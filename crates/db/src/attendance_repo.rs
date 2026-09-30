//! Attendance and attempt-audit queries.

use crate::error::DbError;
use crate::models::Attendance;
use crate::Db;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct NewAttendance {
    pub student_id: Uuid,
    pub device_id: Option<Uuid>,
    pub similarity: f32,
    pub margin: f32,
    pub liveness_score: f32,
    pub status: String,
    pub note: Option<String>,
}

pub async fn record(db: &Db, new: NewAttendance) -> Result<Attendance, DbError> {
    let row = sqlx::query_as::<_, Attendance>(
        r#"
        INSERT INTO attendance
            (student_id, device_id, similarity, margin, liveness_score, status, note)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id, student_id, device_id, "timestamp", similarity, margin,
                  liveness_score, status, note
        "#,
    )
    .bind(new.student_id)
    .bind(new.device_id)
    .bind(new.similarity)
    .bind(new.margin)
    .bind(new.liveness_score)
    .bind(new.status)
    .bind(new.note)
    .fetch_one(db)
    .await?;
    Ok(row)
}

/// A student's most recent attendance timestamp, for the cooldown check.
///
/// `MAX()` over an empty set returns a single row with NULL, so the decode
/// must tolerate NULL in column 0 rather than assuming `fetch_optional` gives
/// `None`.
pub async fn last_marked(db: &Db, student_id: Uuid) -> Result<Option<DateTime<Utc>>, DbError> {
    let row: (Option<DateTime<Utc>>,) = sqlx::query_as(
        r#"
        SELECT MAX("timestamp") FROM attendance
        WHERE student_id = $1 AND status <> 'corrected'
        "#,
    )
    .bind(student_id)
    .fetch_one(db)
    .await?;
    Ok(row.0)
}

/// List attendance for a date and optional class.
///
/// `timezone` is the school's IANA zone (e.g. `Asia/Jakarta`). The timestamp is
/// converted into that zone *before* taking the date, so a check-in at 06:30
/// local (23:30 UTC the day before) is bucketed into the correct local day.
pub async fn list_for_date(
    db: &Db,
    date: chrono::NaiveDate,
    kelas_id: Option<Uuid>,
    timezone: &str,
    limit: i64,
) -> Result<Vec<Attendance>, DbError> {
    let rows = sqlx::query_as::<_, Attendance>(
        r#"
        SELECT a.id, a.student_id, a.device_id, a."timestamp", a.similarity,
               a.margin, a.liveness_score, a.status, a.note
        FROM attendance a
        JOIN students s ON s.id = a.student_id
        WHERE (a."timestamp" AT TIME ZONE $3)::date = $1
          AND ($2::uuid IS NULL OR s.kelas_id = $2)
        ORDER BY a."timestamp" DESC
        LIMIT $4
        "#,
    )
    .bind(date)
    .bind(kelas_id)
    .bind(timezone)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Attendance for a date, joined with the student's name and NIS for display.
///
/// One query, not a per-row student lookup — the join keeps it a single round
/// trip regardless of how many rows the day produced.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AttendanceWithStudent {
    pub id: Uuid,
    pub student_id: Uuid,
    pub nama: String,
    pub nis: String,
    pub timestamp: DateTime<Utc>,
    pub similarity: f32,
    pub margin: f32,
    pub liveness_score: f32,
    pub status: String,
}

pub async fn list_for_date_with_student(
    db: &Db,
    date: chrono::NaiveDate,
    kelas_id: Option<Uuid>,
    timezone: &str,
    limit: i64,
) -> Result<Vec<AttendanceWithStudent>, DbError> {
    let rows = sqlx::query_as::<_, AttendanceWithStudent>(
        r#"
        SELECT a.id, a.student_id, s.nama, s.nis, a."timestamp",
               a.similarity, a.margin, a.liveness_score, a.status
        FROM attendance a
        JOIN students s ON s.id = a.student_id
        WHERE (a."timestamp" AT TIME ZONE $3)::date = $1
          AND ($2::uuid IS NULL OR s.kelas_id = $2)
        ORDER BY a."timestamp" DESC
        LIMIT $4
        "#,
    )
    .bind(date)
    .bind(kelas_id)
    .bind(timezone)
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// Admin correction: mark an attendance row as corrected/void.
pub async fn correct(db: &Db, id: Uuid, note: &str) -> Result<(), DbError> {
    let res = sqlx::query("UPDATE attendance SET status = 'corrected', note = $2 WHERE id = $1")
        .bind(id)
        .bind(note)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!("attendance {id}")));
    }
    Ok(())
}

/// Record a raw attempt for audit & threshold tuning.
#[derive(Clone, Debug)]
pub struct NewAttempt {
    pub device_id: Option<Uuid>,
    pub outcome: String,
    pub top1_student_id: Option<Uuid>,
    pub top1_score: Option<f32>,
    pub top2_score: Option<f32>,
    pub margin: Option<f32>,
    pub liveness_score: Option<f32>,
    pub reason: Option<String>,
    pub frame_count: i32,
}

pub async fn record_attempt(db: &Db, a: NewAttempt) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO attempts
            (device_id, outcome, top1_student_id, top1_score, top2_score,
             margin, liveness_score, reason, frame_count)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(a.device_id)
    .bind(a.outcome)
    .bind(a.top1_student_id)
    .bind(a.top1_score)
    .bind(a.top2_score)
    .bind(a.margin)
    .bind(a.liveness_score)
    .bind(a.reason)
    .bind(a.frame_count)
    .execute(db)
    .await?;
    Ok(())
}

/// Aggregate attempt outcomes over a window — for the monitoring dashboard
/// (Section 10.8: reject ratio, average scores).
pub async fn outcome_summary(
    db: &Db,
    since: DateTime<Utc>,
) -> Result<Vec<(String, i64, Option<f64>, Option<f64>)>, DbError> {
    let rows: Vec<(String, i64, Option<f64>, Option<f64>)> = sqlx::query_as(
        r#"
        SELECT outcome,
               COUNT(*) AS n,
               AVG(top1_score)::float8 AS avg_top1,
               AVG(margin)::float8 AS avg_margin
        FROM attempts
        WHERE "timestamp" >= $1
        GROUP BY outcome
        ORDER BY n DESC
        "#,
    )
    .bind(since)
    .fetch_all(db)
    .await?;
    Ok(rows)
}
