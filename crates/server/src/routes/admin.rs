//! Admin API handlers (auth: JWT, role admin/guru).

use crate::auth::{AdminAuth, UserAuth};
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------- health ----------

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

// ---------- auth ----------

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub role: String,
    pub username: String,
}

pub async fn login(
    State(state): State<AppState>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    // Rate-limit by client IP to slow brute force.
    let key = format!("login:{}", addr.ip());
    if !state.login_limiter.check(&key) {
        return Err(ApiError::TooManyRequests);
    }

    let user = db::user_repo::find_by_username(&state.db, &req.username)
        .await
        .map_err(ApiError::Db)?;

    // Always run a hash comparison to avoid a timing oracle on username existence.
    let Some(user) = user else {
        db::user_repo::verify_password(&req.password, &state.dummy_password_hash);
        return Err(ApiError::Unauthorized);
    };

    if user.disabled || !db::user_repo::verify_password(&req.password, &user.password_hash) {
        return Err(ApiError::Unauthorized);
    }

    state.login_limiter.reset(&key);
    let token = crate::auth::issue_token(
        &user,
        &state.config.jwt_secret,
        state.config.jwt_ttl_seconds,
    )?;

    db::audit(
        &state.db,
        Some(user.id),
        "login",
        Some(&user.username),
        None,
    )
    .await
    .ok();

    Ok(Json(LoginResponse {
        token,
        role: user.role,
        username: user.username,
    }))
}

/// The caller's identity, read from the live user row so a role change is
/// reflected immediately rather than staying stale in the token until expiry.
pub async fn me(
    State(state): State<AppState>,
    UserAuth(claims): UserAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = db::user_repo::find(&state.db, claims.sub)
        .await
        .map_err(ApiError::Db)?
        .ok_or(ApiError::Unauthorized)?;
    Ok(Json(serde_json::json!({
        "username": user.username,
        "role": user.role,
    })))
}

// ---------- students ----------

#[derive(Deserialize)]
pub struct CreateStudent {
    pub nis: String,
    pub nama: String,
    pub kelas_id: Option<Uuid>,
}

pub async fn create_student(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
    Json(req): Json<CreateStudent>,
) -> Result<(StatusCode, Json<db::models::Student>), ApiError> {
    // Trim before storing: a NIS of " 123 " and "123" are different rows to the
    // UNIQUE index but the same student to a human, so an untrimmed value
    // silently creates duplicates and breaks lookups.
    let nis = req.nis.trim().to_string();
    let nama = req.nama.trim().to_string();
    if nis.is_empty() || nama.is_empty() {
        return Err(ApiError::BadRequest("nis dan nama wajib diisi".into()));
    }
    let student = db::student_repo::create(
        &state.db,
        db::student_repo::NewStudent {
            nis,
            nama,
            kelas_id: req.kelas_id,
        },
    )
    .await
    .map_err(|e| match e {
        db::DbError::Sqlx(sqlx::Error::Database(ref d)) if d.is_unique_violation() => {
            ApiError::Conflict("NIS sudah terdaftar".into())
        }
        other => ApiError::Db(other),
    })?;
    Ok((StatusCode::CREATED, Json(student)))
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    100
}

/// A student row for the list view, with the enrollment counts the UI needs so
/// it does not have to fetch each student individually.
#[derive(Serialize)]
pub struct StudentListRow {
    #[serde(flatten)]
    pub student: db::models::Student,
    pub template_count: i64,
    pub active_template_count: i64,
}

pub async fn list_students(
    State(state): State<AppState>,
    UserAuth(_): UserAuth,
    Query(q): Query<ListQuery>,
) -> Result<Json<Vec<StudentListRow>>, ApiError> {
    let limit = q.limit.clamp(1, 500);
    let rows = db::student_repo::list_with_counts(&state.db, limit, q.offset.max(0))
        .await
        .map_err(ApiError::Db)?;
    let out = rows
        .into_iter()
        .map(|r| StudentListRow {
            student: r.student,
            template_count: r.template_count,
            active_template_count: r.active_template_count,
        })
        .collect();
    Ok(Json(out))
}

pub async fn get_student(
    State(state): State<AppState>,
    UserAuth(_): UserAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let student = db::student_repo::find(&state.db, id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("student {id}")))?;
    let templates = db::face_repo::count_for_student(&state.db, id)
        .await
        .map_err(ApiError::Db)?;
    Ok(Json(serde_json::json!({
        "student": student,
        "template_count": templates,
    })))
}

#[derive(Deserialize)]
pub struct UpdateStudent {
    pub nama: Option<String>,
    pub kelas_id: Option<Uuid>,
    pub status: Option<String>,
}

pub async fn update_student(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<UpdateStudent>,
) -> Result<Json<db::models::Student>, ApiError> {
    if let Some(status) = &req.status {
        if !["active", "inactive", "graduated"].contains(&status.as_str()) {
            return Err(ApiError::BadRequest("status tidak valid".into()));
        }
    }
    let nama = req
        .nama
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let student = db::student_repo::update(&state.db, id, nama, req.kelas_id, req.status)
        .await
        .map_err(ApiError::Db)?;
    Ok(Json(student))
}

pub async fn delete_student(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    db::student_repo::delete(&state.db, id)
        .await
        .map_err(ApiError::Db)?;
    state.gallery.reload(&state.db).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Record parental/guardian consent to process this student's biometrics.
///
/// The enrollment endpoints refuse to run without it (plan Section 9), so this
/// is a prerequisite for the whole enrollment flow — without it a new student
/// can never be enrolled.
#[derive(Deserialize)]
pub struct ConsentBody {
    /// Who gave consent (guardian name, or "tertulis" for a signed form).
    pub consent_by: Option<String>,
}

pub async fn set_consent(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(body): Json<ConsentBody>,
) -> Result<Json<db::models::Student>, ApiError> {
    // Confirm the student exists so a typo'd id is a 404, not a silent no-op.
    db::student_repo::find(&state.db, id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("student {id}")))?;

    let by = body
        .consent_by
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("orang tua/wali");

    db::student_repo::set_consent(&state.db, id, by)
        .await
        .map_err(ApiError::Db)?;

    db::audit(
        &state.db,
        Some(claims.sub),
        "consent_granted",
        Some(&id.to_string()),
        Some(serde_json::json!({ "consent_by": by })),
    )
    .await
    .ok();

    let student = db::student_repo::find(&state.db, id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("student {id}")))?;
    Ok(Json(student))
}

// ---------- devices ----------

#[derive(Deserialize)]
pub struct CreateDevice {
    pub nama: String,
    pub lokasi: Option<String>,
}

#[derive(Serialize)]
pub struct DeviceCreated {
    pub device: db::models::Device,
    /// Shown once; never retrievable again.
    pub token: String,
}

pub async fn create_device(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
    Json(req): Json<CreateDevice>,
) -> Result<(StatusCode, Json<DeviceCreated>), ApiError> {
    let (device, token) = db::device_repo::create(&state.db, &req.nama, req.lokasi.as_deref())
        .await
        .map_err(ApiError::Db)?;
    Ok((StatusCode::CREATED, Json(DeviceCreated { device, token })))
}

pub async fn list_devices(
    State(state): State<AppState>,
    UserAuth(_): UserAuth,
) -> Result<Json<Vec<db::models::Device>>, ApiError> {
    let devices = db::device_repo::list(&state.db)
        .await
        .map_err(ApiError::Db)?;
    Ok(Json(devices))
}

pub async fn revoke_device(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    db::device_repo::revoke(&state.db, id)
        .await
        .map_err(ApiError::Db)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_device(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    db::device_repo::delete(&state.db, id)
        .await
        .map_err(ApiError::Db)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- attendance ----------

#[derive(Deserialize)]
pub struct AttendanceQuery {
    pub tanggal: Option<NaiveDate>,
    pub kelas_id: Option<Uuid>,
}

#[derive(Serialize)]
pub struct AttendanceRow {
    pub id: Uuid,
    pub student_id: Uuid,
    pub nama: String,
    pub nis: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub similarity: f32,
    pub margin: f32,
    pub liveness_score: f32,
    pub status: String,
}

pub async fn list_attendance(
    State(state): State<AppState>,
    UserAuth(_): UserAuth,
    Query(q): Query<AttendanceQuery>,
) -> Result<Json<Vec<AttendanceRow>>, ApiError> {
    let date = q.tanggal.unwrap_or_else(|| state.config.today_local());
    let rows = db::attendance_repo::list_for_date_with_student(
        &state.db,
        date,
        q.kelas_id,
        &state.config.timezone,
    )
    .await
    .map_err(ApiError::Db)?;

    let out = rows
        .into_iter()
        .map(|r| AttendanceRow {
            id: r.id,
            student_id: r.student_id,
            nama: r.nama,
            nis: r.nis,
            timestamp: r.timestamp,
            similarity: r.similarity,
            margin: r.margin,
            liveness_score: r.liveness_score,
            status: r.status,
        })
        .collect();
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct CorrectBody {
    pub note: Option<String>,
}

pub async fn correct_attendance(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(body): Json<CorrectBody>,
) -> Result<StatusCode, ApiError> {
    db::attendance_repo::correct(
        &state.db,
        id,
        body.note.as_deref().unwrap_or("dikoreksi admin"),
    )
    .await
    .map_err(ApiError::Db)?;
    db::audit(
        &state.db,
        Some(claims.sub),
        "correct_attendance",
        Some(&id.to_string()),
        None,
    )
    .await
    .ok();
    Ok(StatusCode::NO_CONTENT)
}

// ---------- monitoring ----------

pub async fn monitoring_summary(
    State(state): State<AppState>,
    UserAuth(_): UserAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let since = chrono::Utc::now() - chrono::Duration::hours(24);
    let summary = db::attendance_repo::outcome_summary(&state.db, since)
        .await
        .map_err(ApiError::Db)?;
    let enrolled = db::student_repo::count_active_templates(&state.db)
        .await
        .map_err(ApiError::Db)?;
    Ok(Json(serde_json::json!({
        "outcomes": summary.into_iter().map(|(outcome, n, avg_top1, avg_margin)| {
            serde_json::json!({
                "outcome": outcome,
                "count": n,
                "avg_top1": avg_top1,
                "avg_margin": avg_margin,
            })
        }).collect::<Vec<_>>(),
        "enrolled_students": enrolled,
        "gallery_students": state.gallery.student_count(),
    })))
}

pub async fn reload_gallery(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    let n = state.gallery.reload(&state.db).await;
    Ok(Json(serde_json::json!({ "students": n })))
}
