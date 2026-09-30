//! User (admin/guru) queries and password hashing.

use crate::error::DbError;
use crate::models::User;
use crate::Db;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use rand::RngCore;
use uuid::Uuid;

/// Hash a password with Argon2id and a fresh random salt.
pub fn hash_password(password: &str) -> Result<String, DbError> {
    let mut salt = [0u8; 16];
    rand::rng().fill_bytes(&mut salt);
    Argon2::default()
        .hash_password_with_salt(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| DbError::Invalid(format!("hash: {e}")))
}

/// Verify a password against a stored PHC string. Constant-time inside Argon2.
pub fn verify_password(password: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

pub async fn create(db: &Db, username: &str, password: &str, role: &str) -> Result<User, DbError> {
    let hash = hash_password(password)?;
    let row = sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (username, password_hash, role)
        VALUES ($1, $2, $3)
        RETURNING id, username, password_hash, role, disabled, created_at
        "#,
    )
    .bind(username)
    .bind(hash)
    .bind(role)
    .fetch_one(db)
    .await?;
    Ok(row)
}

pub async fn find_by_username(db: &Db, username: &str) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as::<_, User>(
        r#"
        SELECT id, username, password_hash, role, disabled, created_at
        FROM users WHERE username = $1
        "#,
    )
    .bind(username)
    .fetch_optional(db)
    .await?;
    Ok(row)
}

pub async fn find(db: &Db, id: Uuid) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as::<_, User>(
        r#"
        SELECT id, username, password_hash, role, disabled, created_at
        FROM users WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(row)
}

pub async fn count(db: &Db) -> Result<i64, DbError> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(db)
        .await?;
    Ok(n)
}

/// Number of enabled admin accounts — used to refuse changes that would leave
/// the system without a way to manage it.
pub async fn count_active_admins(db: &Db) -> Result<i64, DbError> {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin' AND NOT disabled")
            .fetch_one(db)
            .await?;
    Ok(n)
}

/// All users, newest first (never returns the password hash to callers that
/// serialise `User` — the field is `#[serde(skip_serializing)]`).
pub async fn list(db: &Db) -> Result<Vec<User>, DbError> {
    let rows = sqlx::query_as::<_, User>(
        r#"
        SELECT id, username, password_hash, role, disabled, created_at
        FROM users
        ORDER BY created_at
        "#,
    )
    .fetch_all(db)
    .await?;
    Ok(rows)
}

pub async fn update_role(db: &Db, id: Uuid, role: &str) -> Result<(), DbError> {
    let res = sqlx::query("UPDATE users SET role = $2 WHERE id = $1")
        .bind(id)
        .bind(role)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("user {id}")));
    }
    Ok(())
}

pub async fn set_disabled(db: &Db, id: Uuid, disabled: bool) -> Result<(), DbError> {
    let res = sqlx::query("UPDATE users SET disabled = $2 WHERE id = $1")
        .bind(id)
        .bind(disabled)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("user {id}")));
    }
    Ok(())
}

pub async fn set_password(db: &Db, id: Uuid, password: &str) -> Result<(), DbError> {
    let hash = hash_password(password)?;
    let res = sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
        .bind(id)
        .bind(hash)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("user {id}")));
    }
    Ok(())
}

pub async fn delete(db: &Db, id: Uuid) -> Result<(), DbError> {
    let res = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("user {id}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_roundtrip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn hash_is_salted_and_differs_per_call() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "salt must make identical passwords hash differently");
        assert!(verify_password("same", &a));
        assert!(verify_password("same", &b));
    }

    #[test]
    fn verify_rejects_garbage_hash() {
        assert!(!verify_password("x", "not-a-hash"));
    }
}

#[cfg(test)]
mod dummy_probe {
    use super::{hash_password, verify_password};

    /// A dummy hash used to equalise login timing MUST be parseable, or
    /// `verify_password` returns early and the timing oracle comes back. This
    /// guards the invariant: a freshly generated hash parses and actually runs
    /// Argon2 (verification takes materially longer than the parse-failure path).
    #[test]
    fn generated_dummy_hash_is_parseable_and_does_argon2_work() {
        let dummy = hash_password("absensi-wajah-timing-equalizer").unwrap();
        assert!(
            argon2::password_hash::phc::PasswordHash::new(&dummy).is_ok(),
            "dummy hash must parse"
        );
        assert!(!verify_password("guess", &dummy));
        // A malformed hash returns without hashing and is far faster.
        let t_bad = std::time::Instant::now();
        assert!(!verify_password("guess", "not-a-hash"));
        let bad = t_bad.elapsed();
        let t_good = std::time::Instant::now();
        assert!(!verify_password("guess", &dummy));
        let good = t_good.elapsed();
        assert!(
            good > bad,
            "valid dummy ({good:?}) should do more work than an unparseable one ({bad:?})"
        );
    }
}
