//! Authentication: admin JWTs and kiosk device tokens.

use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use db::models::{Device, User};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Claims embedded in an admin session JWT.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject — the user id.
    pub sub: Uuid,
    pub username: String,
    pub role: String,
    /// Expiry (unix seconds).
    pub exp: i64,
    /// Issued at.
    pub iat: i64,
}

/// Mint a JWT for an authenticated user.
pub fn issue_token(user: &User, secret: &str, ttl_seconds: i64) -> Result<String, ApiError> {
    let now = chrono::Utc::now().timestamp();
    let claims = Claims {
        sub: user.id,
        username: user.username.clone(),
        role: user.role.clone(),
        exp: now + ttl_seconds,
        iat: now,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!("jwt encode: {e}")))
}

/// Verify and decode a JWT.
pub fn verify_token(token: &str, secret: &str) -> Result<Claims, ApiError> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| ApiError::Unauthorized)?;
    Ok(data.claims)
}

/// Extractor: **any** authenticated staff member (admin or guru).
///
/// Re-validates the user row so a deleted or disabled account stops working
/// immediately, not when its token happens to expire. The lookup is by primary
/// key, so the cost is one indexed read. Role is *not* checked here — use
/// [`AdminAuth`] for anything that mutates data.
pub struct UserAuth(pub Claims);

impl<S> FromRequestParts<S> for UserAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let claims = bearer_claims(parts, &app)?;

        let user = db::user_repo::find(&app.db, claims.sub)
            .await
            .map_err(ApiError::Db)?
            .ok_or(ApiError::Unauthorized)?;
        if user.disabled {
            return Err(ApiError::Unauthorized);
        }
        Ok(UserAuth(claims))
    }
}

/// Extractor: an authenticated **admin**, re-checked against the database.
///
/// Beyond decoding the token, this loads the user row and rejects if the user
/// was deleted, disabled, or demoted since the token was issued. That closes the
/// window where a revoked account keeps working for the token's whole TTL.
pub struct AdminAuth(pub Claims);

impl<S> FromRequestParts<S> for AdminAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let claims = bearer_claims(parts, &app)?;

        // Re-validate against the live row: the token's role may be stale.
        let user = db::user_repo::find(&app.db, claims.sub)
            .await
            .map_err(ApiError::Db)?
            .ok_or(ApiError::Unauthorized)?;
        if user.disabled || user.role != "admin" {
            return Err(ApiError::Forbidden);
        }
        Ok(AdminAuth(claims))
    }
}

/// Pull and verify the `Authorization: Bearer <jwt>` header into claims.
fn bearer_claims(parts: &Parts, app: &AppState) -> Result<Claims, ApiError> {
    let header = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError::Unauthorized)?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or(ApiError::Unauthorized)?;
    verify_token(token, &app.config.jwt_secret)
}

/// Require one of the given roles for a claim set.
pub fn require_role(claims: &Claims, allowed: &[&str]) -> Result<(), ApiError> {
    if allowed.contains(&claims.role.as_str()) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Extractor: the request must carry a valid device token via
/// `X-Device-Token`. Returns the resolved device.
pub struct DeviceAuth(pub Device);

impl<S> FromRequestParts<S> for DeviceAuth
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let token = parts
            .headers
            .get("x-device-token")
            .and_then(|v| v.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;

        match db::device_repo::authenticate(&app.db, token).await {
            Ok(Some(device)) => Ok(DeviceAuth(device)),
            Ok(None) => Err(ApiError::Unauthorized),
            Err(e) => Err(ApiError::Db(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn user() -> User {
        User {
            id: Uuid::from_u128(7),
            username: "admin".into(),
            password_hash: "x".into(),
            role: "admin".into(),
            disabled: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn issue_and_verify_roundtrip() {
        let token = issue_token(&user(), "secret", 3600).unwrap();
        let claims = verify_token(&token, "secret").unwrap();
        assert_eq!(claims.sub, Uuid::from_u128(7));
        assert_eq!(claims.role, "admin");
    }

    #[test]
    fn wrong_secret_rejected() {
        let token = issue_token(&user(), "secret", 3600).unwrap();
        assert!(verify_token(&token, "other").is_err());
    }

    #[test]
    fn expired_token_rejected() {
        // TTL well beyond the default 60s leeway.
        let token = issue_token(&user(), "secret", -3600).unwrap();
        assert!(verify_token(&token, "secret").is_err());
    }

    #[test]
    fn role_check() {
        let token = issue_token(&user(), "secret", 3600).unwrap();
        let claims = verify_token(&token, "secret").unwrap();
        assert!(require_role(&claims, &["admin"]).is_ok());
        assert!(require_role(&claims, &["admin", "guru"]).is_ok());
        assert!(require_role(&claims, &["guru"]).is_err());
    }
}
