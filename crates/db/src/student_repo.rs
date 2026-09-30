//! Student queries.

use crate::error::DbError;
use crate::models::Student;
use crate::Db;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct NewStudent {
    pub nis: String,
    pub nama: String,
    pub kelas_id: Option<Uuid>,
}

pub async fn create(db: &Db, new: NewStudent) -> Result<Student, DbError> {
    let row = sqlx::query_as::<_, Student>(
        r#"
        INSERT INTO students (nis, nama, kelas_id)
        VALUES ($1, $2, $3)
        RETURNING id, nis, nama, kelas_id, status, consent_granted, created_at
        "#,
    )
    .bind(&new.nis)
    .bind(&new.nama)
    .bind(new.kelas_id)
    .fetch_one(db)
    .await?;
    Ok(row)
}

pub async fn list(db: &Db, limit: i64, offset: i64) -> Result<Vec<Student>, DbError> {
    let rows = sqlx::query_as::<_, Student>(
        r#"
        SELECT id, nis, nama, kelas_id, status, consent_granted, created_at
        FROM students
        ORDER BY nama
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// A student row plus their template counts, for the students list.
///
/// Computed in one query with a LEFT JOIN + FILTER so the list does not need a
/// per-student follow-up request (the client previously made one request per
/// student just to know whether the face was enrolled).
pub struct StudentListItem {
    pub student: Student,
    pub template_count: i64,
    pub active_template_count: i64,
}

pub async fn list_with_counts(
    db: &Db,
    limit: i64,
    offset: i64,
) -> Result<Vec<StudentListItem>, DbError> {
    let rows = sqlx::query_as::<_, StudentListRow>(
        r#"
        SELECT s.id, s.nis, s.nama, s.kelas_id, s.status, s.consent_granted,
               s.created_at,
               COUNT(f.id)::bigint AS template_count,
               COUNT(f.id) FILTER (WHERE f.active)::bigint AS active_template_count
        FROM students s
        LEFT JOIN face_templates f ON f.student_id = s.id
        GROUP BY s.id
        ORDER BY s.nama
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| StudentListItem {
            student: Student {
                id: r.id,
                nis: r.nis,
                nama: r.nama,
                kelas_id: r.kelas_id,
                status: r.status,
                consent_granted: r.consent_granted,
                created_at: r.created_at,
            },
            template_count: r.template_count,
            active_template_count: r.active_template_count,
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct StudentListRow {
    id: Uuid,
    nis: String,
    nama: String,
    kelas_id: Option<Uuid>,
    status: String,
    consent_granted: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    template_count: i64,
    active_template_count: i64,
}

pub async fn find(db: &Db, id: Uuid) -> Result<Option<Student>, DbError> {
    let row = sqlx::query_as::<_, Student>(
        r#"
        SELECT id, nis, nama, kelas_id, status, consent_granted, created_at
        FROM students WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(row)
}

pub async fn update(
    db: &Db,
    id: Uuid,
    nama: Option<String>,
    kelas_id: Option<Uuid>,
    status: Option<String>,
) -> Result<Student, DbError> {
    let row = sqlx::query_as::<_, Student>(
        r#"
        UPDATE students
        SET nama = COALESCE($2, nama),
            kelas_id = COALESCE($3, kelas_id),
            status = COALESCE($4, status),
            updated_at = now()
        WHERE id = $1
        RETURNING id, nis, nama, kelas_id, status, consent_granted, created_at
        "#,
    )
    .bind(id)
    .bind(nama)
    .bind(kelas_id)
    .bind(status)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| DbError::NotFound(format!("student {id}")))?;
    Ok(row)
}

pub async fn set_consent(db: &Db, id: Uuid, consent_by: &str) -> Result<(), DbError> {
    sqlx::query(
        r#"
        UPDATE students
        SET consent_granted = TRUE, consent_by = $2, consent_at = now(), updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(consent_by)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn delete(db: &Db, id: Uuid) -> Result<(), DbError> {
    let res = sqlx::query("DELETE FROM students WHERE id = $1")
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("student {id}")));
    }
    Ok(())
}

/// Count of enrolled (active-template) students — dashboard metric.
pub async fn count_active_templates(db: &Db) -> Result<i64, DbError> {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(DISTINCT student_id) FROM face_templates WHERE active")
            .fetch_one(db)
            .await?;
    Ok(row.0)
}
