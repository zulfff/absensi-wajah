//! Database layer: connection, migrations, and typed queries.
//!
//! Uses `sqlx` with PostgreSQL + `pgvector`. All SQL lives here so the server
//! crate stays free of SQL strings.

pub mod attendance_repo;
pub mod device_repo;
pub mod error;
pub mod face_repo;
pub mod models;
pub mod pool;
pub mod student_repo;
pub mod user_repo;

pub use error::DbError;
pub use models::*;
pub use pool::{connect, migrate, Db};

use uuid::Uuid;

/// Append an audit-log entry (plan Section 9: every biometric access/change is
/// audited).
pub async fn audit(
    db: &Db,
    user_id: Option<Uuid>,
    action: &str,
    target: Option<&str>,
    detail: Option<serde_json::Value>,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO audit_log (user_id, action, target, detail)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(user_id)
    .bind(action)
    .bind(target)
    .bind(detail)
    .execute(db)
    .await?;
    Ok(())
}

/// Embedding dimension stored in the DB. Must match the model and the
/// `vector(512)` column in the migration.
pub const EMBEDDING_DIM: usize = 512;
