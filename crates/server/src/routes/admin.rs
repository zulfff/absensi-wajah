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
    headers: axum::http::HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    // Rate-limit by client IP to slow brute force.
    //
    // The server runs behind Caddy, so `addr` is the *proxy's* address — the
    // same for every client. Keying on it would make one attacker (or ~11
    // legitimate logins a minute) lock out every user, while giving no per-
    // attacker protection. Prefer the client IP Caddy put in `X-Forwarded-For`.
    //
    // Caddy appends the real peer address and, by default, *ignores* any
    // client-supplied `X-Forwarded-For`, so the right-most entry is the address
    // Caddy itself observed — not attacker-controlled. With no header (direct
    // connection, e.g. dev), fall back to the TCP peer.
    let client_ip = client_ip_from(&headers, addr.ip());
    // Per-IP bucket only. A shared/global bucket would be a trivial DoS: ten
    // failed attempts from anywhere would lock every legitimate user out for
    // the window. The limiter's map is itself bounded (a sweep drops expired
    // windows), so a flood from many IPs cannot exhaust memory either.
    let key = format!("login:ip:{client_ip}");
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

/// The `X-Forwarded-For` header name, lower-cased for `HeaderMap` lookup.
const XFF: &str = "x-forwarded-for";

/// Resolve the client IP for rate limiting, preferring the proxy-set
/// `X-Forwarded-For`.
///
/// Takes the **right-most** entry: Caddy appends the address it actually saw and
/// ignores client-supplied values, so the last element is trustworthy while
/// anything the client prepended is not. Falls back to the TCP peer when the
/// header is absent or malformed. Only used for rate-limit keying; never for
/// authorization.
fn client_ip_from(headers: &axum::http::HeaderMap, peer: std::net::IpAddr) -> std::net::IpAddr {
    headers
        .get(XFF)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .map(str::trim)
        .and_then(|s| s.parse::<std::net::IpAddr>().ok())
        .unwrap_or(peer)
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
    /// `None` = field absent (leave unchanged); `Some(None)` = explicit JSON
    /// `null` (clear the class); `Some(Some(id))` = set the class. Without the
    /// outer/inner distinction a partial update cannot express "unassign".
    #[serde(default, deserialize_with = "double_option")]
    pub kelas_id: Option<Option<Uuid>>,
    pub status: Option<String>,
}

/// Deserialize an optional, nullable field into `Option<Option<T>>`.
///
/// serde cannot distinguish "absent" from "null" for a plain `Option<T>`. This
/// helper does: absent -> `None`, `null` -> `Some(None)`, value -> `Some(Some)`.
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
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
    // Hard cap on rows returned for one day. A school day is bounded by the
    // student count, but an unbounded response array is a resource-exhaustion
    // surface (and a slow query over a large `attendance` slice). 5000 is far
    // above any real school's daily check-in count.
    const MAX_ATTENDANCE_ROWS: i64 = 5000;
    let date = q.tanggal.unwrap_or_else(|| state.config.today_local());
    let rows = db::attendance_repo::list_for_date_with_student(
        &state.db,
        date,
        q.kelas_id,
        &state.config.timezone,
        MAX_ATTENDANCE_ROWS,
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

/// Manual attendance by a teacher (plan Section 6 "fallback manusia"): when the
/// camera cannot recognise a student (broken camera, glasses, lighting, or the
/// model is simply unsure), a staff member records presence by hand. This is the
/// human fallback that makes the fail-closed rule safe to enforce.
#[derive(Deserialize)]
pub struct ManualAttendanceBody {
    pub student_id: Uuid,
    pub note: Option<String>,
}

pub async fn mark_attendance_manual(
    State(state): State<AppState>,
    UserAuth(claims): UserAuth,
    Json(body): Json<ManualAttendanceBody>,
) -> Result<(StatusCode, Json<db::models::Attendance>), ApiError> {
    // The student must exist; a typo must not create an orphan row.
    db::student_repo::find(&state.db, body.student_id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("student {}", body.student_id)))?;

    // Respect the cooldown: if the student was marked in the last window
    // (by the kiosk or by hand), do not add a duplicate. Unlike the kiosk path
    // this reports the existing record rather than silently succeeding.
    let last = db::attendance_repo::last_marked(&state.db, body.student_id)
        .await
        .map_err(ApiError::Db)?;
    let now = chrono::Utc::now();
    if let domain::cooldown::CooldownVerdict::WithinCooldown { remaining_seconds, .. } =
        domain::cooldown::check(last, now, state.config.cooldown_seconds)
    {
        return Err(ApiError::Conflict(format!(
            "siswa sudah ditandai hadir dalam {remaining_seconds} detik terakhir"
        )));
    }

    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("absen manual oleh guru");

    let record = db::attendance_repo::record(
        &state.db,
        db::attendance_repo::NewAttendance {
            student_id: body.student_id,
            device_id: None,
            // No model was involved, so the model scores are explicitly zero —
            // never a fake confidence that would pollute the tuning dataset.
            similarity: 0.0,
            margin: 0.0,
            liveness_score: 0.0,
            status: "manual".into(),
            note: Some(note.into()),
        },
    )
    .await
    .map_err(ApiError::Db)?;

    db::audit(
        &state.db,
        Some(claims.sub),
        "attendance_manual",
        Some(&body.student_id.to_string()),
        Some(serde_json::json!({ "note": note })),
    )
    .await
    .ok();

    Ok((StatusCode::CREATED, Json(record)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use std::net::{IpAddr, Ipv4Addr};

    fn peer() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 99))
    }

    #[test]
    fn uses_rightmost_forwarded_entry() {
        let mut h = HeaderMap::new();
        // A client at 1.2.3.4 through Caddy; a client could prepend a lie, but
        // only the right-most (Caddy-appended) value is trusted.
        h.insert(
            XFF,
            HeaderValue::from_static("1.2.3.4, 5.6.7.8, 9.9.9.9"),
        );
        assert_eq!(
            client_ip_from(&h, peer()),
            IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))
        );
    }

    #[test]
    fn falls_back_to_peer_without_header() {
        assert_eq!(client_ip_from(&HeaderMap::new(), peer()), peer());
    }

    #[test]
    fn falls_back_to_peer_on_garbage() {
        let mut h = HeaderMap::new();
        h.insert(XFF, HeaderValue::from_static("not-an-ip"));
        assert_eq!(client_ip_from(&h, peer()), peer());
    }

    #[test]
    fn parses_single_ipv6_entry() {
        let mut h = HeaderMap::new();
        h.insert(XFF, HeaderValue::from_static("2001:db8::1"));
        assert_eq!(
            client_ip_from(&h, peer()),
            "2001:db8::1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn update_student_distinguishes_absent_from_null_kelas() {
        // Absent -> no change.
        let absent: UpdateStudent = serde_json::from_str(r#"{"nama":"Budi"}"#).unwrap();
        assert_eq!(absent.kelas_id, None);
        // Explicit null -> clear to NULL.
        let cleared: UpdateStudent = serde_json::from_str(r#"{"kelas_id":null}"#).unwrap();
        assert_eq!(cleared.kelas_id, Some(None));
        // A value -> set it.
        let id = Uuid::from_u128(42);
        let set: UpdateStudent =
            serde_json::from_str(&format!(r#"{{"kelas_id":"{id}"}}"#)).unwrap();
        assert_eq!(set.kelas_id, Some(Some(id)));
    }
}
