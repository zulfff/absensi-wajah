//! User management (admin only).
//!
//! Lets an admin create staff accounts (admin or guru), change a role, reset a
//! password, disable, and delete. Every mutation is admin-gated and audited.
//!
//! The guardrails that matter here are lockout guards: an admin must not be able
//! to remove the last way into the system. Demoting, disabling, or deleting the
//! final enabled admin is refused.

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const ROLES: [&str; 2] = ["admin", "guru"];

/// Minimum password length. Short enough to be usable in a school, long enough
/// to not be trivially guessable.
const MIN_PASSWORD_LEN: usize = 8;

#[derive(Serialize)]
pub struct UserView {
    pub id: Uuid,
    pub username: String,
    pub role: String,
    pub disabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<db::models::User> for UserView {
    fn from(u: db::models::User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            role: u.role,
            disabled: u.disabled,
            created_at: u.created_at,
        }
    }
}

fn validate_role(role: &str) -> Result<(), ApiError> {
    if ROLES.contains(&role) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "role tidak valid: harus salah satu dari {ROLES:?}"
        )))
    }
}

fn validate_password(pw: &str) -> Result<(), ApiError> {
    if pw.len() < MIN_PASSWORD_LEN {
        return Err(ApiError::BadRequest(format!(
            "kata sandi minimal {MIN_PASSWORD_LEN} karakter"
        )));
    }
    Ok(())
}

// ---------- list ----------

pub async fn list_users(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
) -> Result<Json<Vec<UserView>>, ApiError> {
    let users = db::user_repo::list(&state.db).await.map_err(ApiError::Db)?;
    Ok(Json(users.into_iter().map(UserView::from).collect()))
}

// ---------- create ----------

#[derive(Deserialize)]
pub struct CreateUser {
    pub username: String,
    pub password: String,
    pub role: String,
}

pub async fn create_user(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    Json(req): Json<CreateUser>,
) -> Result<(StatusCode, Json<UserView>), ApiError> {
    let username = req.username.trim();
    if username.is_empty() {
        return Err(ApiError::BadRequest("username wajib diisi".into()));
    }
    validate_role(&req.role)?;
    validate_password(&req.password)?;

    let user = db::user_repo::create(&state.db, username, &req.password, &req.role)
        .await
        .map_err(|e| match e {
            db::DbError::Sqlx(sqlx::Error::Database(ref d)) if d.is_unique_violation() => {
                ApiError::Conflict("username sudah dipakai".into())
            }
            other => ApiError::Db(other),
        })?;

    db::audit(
        &state.db,
        Some(claims.sub),
        "user_created",
        Some(&user.id.to_string()),
        Some(serde_json::json!({ "username": user.username, "role": user.role })),
    )
    .await
    .ok();

    Ok((StatusCode::CREATED, Json(user.into())))
}

// ---------- update role ----------

#[derive(Deserialize)]
pub struct UpdateUser {
    pub role: Option<String>,
    pub disabled: Option<bool>,
    pub password: Option<String>,
}

pub async fn update_user(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<UpdateUser>,
) -> Result<Json<UserView>, ApiError> {
    let target = db::user_repo::find(&state.db, id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("user {id}")))?;

    // Would this change remove the last enabled admin? Check before applying.
    let demoting_last_admin = req.role.as_deref() == Some("guru") && target.role == "admin";
    let disabling_last_admin = req.disabled == Some(true) && target.role == "admin";
    if (demoting_last_admin || disabling_last_admin)
        && target.role == "admin"
        && !target.disabled
        && db::user_repo::count_active_admins(&state.db)
            .await
            .map_err(ApiError::Db)?
            <= 1
    {
        return Err(ApiError::Conflict(
            "tidak bisa menghapus peran admin aktif terakhir".into(),
        ));
    }

    if let Some(role) = &req.role {
        validate_role(role)?;
        db::user_repo::update_role(&state.db, id, role)
            .await
            .map_err(ApiError::Db)?;
    }
    if let Some(disabled) = req.disabled {
        db::user_repo::set_disabled(&state.db, id, disabled)
            .await
            .map_err(ApiError::Db)?;
    }
    if let Some(password) = &req.password {
        validate_password(password)?;
        db::user_repo::set_password(&state.db, id, password)
            .await
            .map_err(ApiError::Db)?;
    }

    db::audit(
        &state.db,
        Some(claims.sub),
        "user_updated",
        Some(&id.to_string()),
        Some(serde_json::json!({
            "role": req.role,
            "disabled": req.disabled,
            // Never log the password. Note that a password was set, not what it is.
            "password_changed": req.password.is_some(),
        })),
    )
    .await
    .ok();

    let updated = db::user_repo::find(&state.db, id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("user {id}")))?;
    Ok(Json(updated.into()))
}

// ---------- delete ----------

pub async fn delete_user(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if id == claims.sub {
        return Err(ApiError::Conflict(
            "tidak bisa menghapus akun yang sedang dipakai".into(),
        ));
    }

    let target = db::user_repo::find(&state.db, id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("user {id}")))?;

    if target.role == "admin"
        && !target.disabled
        && db::user_repo::count_active_admins(&state.db)
            .await
            .map_err(ApiError::Db)?
            <= 1
    {
        return Err(ApiError::Conflict(
            "tidak bisa menghapus admin aktif terakhir".into(),
        ));
    }

    db::user_repo::delete(&state.db, id)
        .await
        .map_err(ApiError::Db)?;

    db::audit(
        &state.db,
        Some(claims.sub),
        "user_deleted",
        Some(&id.to_string()),
        Some(serde_json::json!({ "username": target.username })),
    )
    .await
    .ok();

    Ok(StatusCode::NO_CONTENT)
}
