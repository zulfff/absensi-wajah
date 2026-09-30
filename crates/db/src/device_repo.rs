//! Device (kiosk) queries and token handling.

use crate::error::DbError;
use crate::models::Device;
use crate::Db;
use uuid::Uuid;

/// Hash a device token for storage. SHA-256 is correct here (unlike passwords):
/// device tokens are high-entropy random values, not low-entropy user secrets,
/// so a slow KDF is unnecessary and would only add per-frame latency.
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Generate a new random device token (returned once, never stored in clear).
pub fn generate_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("kiosk_{}", base64_url(&bytes))
}

fn base64_url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub async fn create(
    db: &Db,
    nama: &str,
    lokasi: Option<&str>,
) -> Result<(Device, String), DbError> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let row = sqlx::query_as::<_, Device>(
        r#"
        INSERT INTO devices (nama, lokasi, token_hash)
        VALUES ($1, $2, $3)
        RETURNING id, nama, lokasi, revoked, last_seen, created_at
        "#,
    )
    .bind(nama)
    .bind(lokasi)
    .bind(token_hash)
    .fetch_one(db)
    .await?;
    Ok((row, token))
}

pub async fn list(db: &Db) -> Result<Vec<Device>, DbError> {
    let rows = sqlx::query_as::<_, Device>(
        "SELECT id, nama, lokasi, revoked, last_seen, created_at FROM devices ORDER BY created_at DESC",
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Authenticate a device by its token. Returns the device if the token matches
/// and the device is not revoked.
pub async fn authenticate(db: &Db, token: &str) -> Result<Option<Device>, DbError> {
    let hash = hash_token(token);
    let row = sqlx::query_as::<_, Device>(
        r#"
        SELECT id, nama, lokasi, revoked, last_seen, created_at
        FROM devices
        WHERE token_hash = $1 AND NOT revoked
        "#,
    )
    .bind(hash)
    .fetch_optional(db)
    .await?;
    Ok(row)
}

pub async fn touch(db: &Db, id: Uuid) -> Result<(), DbError> {
    sqlx::query("UPDATE devices SET last_seen = now() WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn revoke(db: &Db, id: Uuid) -> Result<(), DbError> {
    let res = sqlx::query("UPDATE devices SET revoked = TRUE WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("device {id}")));
    }
    Ok(())
}

pub async fn delete(db: &Db, id: Uuid) -> Result<(), DbError> {
    let res = sqlx::query("DELETE FROM devices WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("device {id}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hash_is_deterministic() {
        assert_eq!(hash_token("abc"), hash_token("abc"));
        assert_ne!(hash_token("abc"), hash_token("abd"));
    }

    #[test]
    fn generated_tokens_are_unique_and_prefixed() {
        let a = generate_token();
        let b = generate_token();
        assert!(a.starts_with("kiosk_"));
        assert_ne!(a, b);
    }
}
