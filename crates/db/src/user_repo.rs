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

/// Apply role, disabled, and/or password changes atomically, refusing any change
/// that would leave the system with zero enabled admins.
///
/// Guard and writes share one transaction. `SELECT ... FOR UPDATE` locks every
/// admin row while the count is taken, so two concurrent requests cannot both
/// observe "2 admins" and both demote — the race that could otherwise leave no
/// way into the system. `None` for any field means "leave unchanged";
/// `password_hash` is the already-computed Argon2 PHC string.
pub async fn update_guarding_last_admin(
    db: &Db,
    id: Uuid,
    role: Option<&str>,
    disabled: Option<bool>,
    password_hash: Option<&str>,
) -> Result<(), DbError> {
    let mut tx = db.begin().await?;

    // Lock all admin rows so the count below cannot change under us.
    let _locked: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM users WHERE role = 'admin' FOR UPDATE")
            .fetch_all(&mut *tx)
            .await?;

    let target: Option<(String, bool)> =
        sqlx::query_as("SELECT role, disabled FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (target_role, target_disabled) =
        target.ok_or_else(|| DbError::NotFound(format!("user {id}")))?;

    let new_role = role.unwrap_or(&target_role).to_string();
    let new_disabled = disabled.unwrap_or(target_disabled);

    // Would this remove the last enabled admin?
    let loses_admin = target_role == "admin"
        && !target_disabled
        && (new_role != "admin" || new_disabled);
    if loses_admin {
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin' AND NOT disabled")
                .fetch_one(&mut *tx)
                .await?;
        if n <= 1 {
            return Err(DbError::Conflict(
                "tidak bisa menghapus peran admin aktif terakhir".into(),
            ));
        }
    }

    sqlx::query(
        "UPDATE users
         SET role = $2,
             disabled = $3,
             password_hash = COALESCE($4, password_hash)
         WHERE id = $1",
    )
    .bind(id)
    .bind(new_role)
    .bind(new_disabled)
    .bind(password_hash)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Delete a user, refusing to remove the last enabled admin. Returns the
/// deleted user's username.
///
/// Guard and delete share one transaction, with admin rows locked, so two
/// concurrent deletes cannot both observe "one admin left" and both proceed.
pub async fn delete_guarding_last_admin(db: &Db, id: Uuid) -> Result<String, DbError> {
    let mut tx = db.begin().await?;

    let _locked: Vec<(Uuid,)> =
        sqlx::query_as("SELECT id FROM users WHERE role = 'admin' FOR UPDATE")
            .fetch_all(&mut *tx)
            .await?;

    let target: Option<(String, String, bool)> =
        sqlx::query_as("SELECT username, role, disabled FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (username, role, disabled) =
        target.ok_or_else(|| DbError::NotFound(format!("user {id}")))?;

    if role == "admin" && !disabled {
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM users WHERE role = 'admin' AND NOT disabled")
                .fetch_one(&mut *tx)
                .await?;
        if n <= 1 {
            return Err(DbError::Conflict(
                "tidak bisa menghapus admin aktif terakhir".into(),
            ));
        }
    }

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(username)
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
