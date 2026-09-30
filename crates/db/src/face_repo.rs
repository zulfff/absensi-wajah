//! Face template storage and gallery matching via pgvector.

use crate::error::DbError;
use crate::models::GalleryRow;
use crate::Db;
use domain::Embedding;
use pgvector::Vector;
use uuid::Uuid;

fn to_pg(embedding: &Embedding) -> Vector {
    Vector::from(embedding.as_slice().to_vec())
}

fn from_pg(vector: Vector) -> Result<Embedding, DbError> {
    Embedding::new(vector.to_vec()).map_err(|e| DbError::Invalid(e.to_string()))
}

/// Insert a face template for a student.
///
/// Kept for single-template use (tests, tooling). The enrollment flow uses
/// [`replace_templates`] instead, so it writes atomically.
#[allow(dead_code)]
pub async fn insert_template(
    db: &Db,
    student_id: Uuid,
    embedding: &Embedding,
    quality_score: f32,
    source_image_ref: Option<&str>,
    is_centroid: bool,
) -> Result<Uuid, DbError> {
    let (id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO face_templates
            (student_id, embedding, quality_score, source_image_ref, is_centroid)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(student_id)
    .bind(to_pg(embedding))
    .bind(quality_score)
    .bind(source_image_ref)
    .bind(is_centroid)
    .fetch_one(db)
    .await?;
    Ok(id)
}

/// Replace a student's templates atomically: delete any existing rows, then
/// insert the centroid plus every kept frame in one transaction.
///
/// All-or-nothing on purpose — a partial write would leave a student with a
/// gallery that matches some captures and silently ignores the rest. Also one
/// round trip instead of one INSERT per frame.
///
/// `frames` are (embedding, quality_score, is_centroid).
pub async fn replace_templates(
    db: &Db,
    student_id: Uuid,
    frames: &[(Embedding, f32, bool)],
) -> Result<usize, DbError> {
    if frames.is_empty() {
        return Err(DbError::Invalid("no frames to store".into()));
    }
    let mut tx = db.begin().await?;

    sqlx::query("DELETE FROM face_templates WHERE student_id = $1")
        .bind(student_id)
        .execute(&mut *tx)
        .await?;

    // Build a single multi-row INSERT. `UNNEST` lets one statement take all
    // rows, so the round-trip count stays constant regardless of frame count.
    let embeddings: Vec<Vector> = frames.iter().map(|(e, _, _)| to_pg(e)).collect();
    let quality: Vec<f32> = frames.iter().map(|(_, q, _)| *q).collect();
    let centroid: Vec<bool> = frames.iter().map(|(_, _, c)| *c).collect();

    sqlx::query(
        r#"
        INSERT INTO face_templates (student_id, embedding, quality_score, is_centroid)
        SELECT $1, e, q, c
        FROM UNNEST($2::vector[], $3::real[], $4::bool[]) AS t(e, q, c)
        "#,
    )
    .bind(student_id)
    .bind(&embeddings)
    .bind(&quality)
    .bind(&centroid)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(frames.len())
}

/// Activate all templates for a student (admin clicks "Aktifkan").
pub async fn activate_templates(db: &Db, student_id: Uuid) -> Result<u64, DbError> {
    let res = sqlx::query("UPDATE face_templates SET active = TRUE WHERE student_id = $1")
        .bind(student_id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(DbError::NotFound(format!(
            "no templates for student {student_id}"
        )));
    }
    Ok(res.rows_affected())
}

pub async fn deactivate_templates(db: &Db, student_id: Uuid) -> Result<(), DbError> {
    sqlx::query("UPDATE face_templates SET active = FALSE WHERE student_id = $1")
        .bind(student_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Delete every template for a student (retention / re-enroll).
pub async fn delete_templates(db: &Db, student_id: Uuid) -> Result<u64, DbError> {
    let res = sqlx::query("DELETE FROM face_templates WHERE student_id = $1")
        .bind(student_id)
        .execute(db)
        .await?;
    Ok(res.rows_affected())
}

/// Load every active template as the in-memory gallery.
///
/// For a school, brute-force cosine over the gallery is fast and exact; the
/// HNSW index is a scale option, not a requirement.
pub async fn load_active_gallery(db: &Db) -> Result<Vec<GalleryRow>, DbError> {
    let rows: Vec<(Uuid, Vector)> =
        sqlx::query_as("SELECT student_id, embedding FROM face_templates WHERE active")
            .fetch_all(db)
            .await?;
    rows.into_iter()
        .map(|(student_id, v)| {
            Ok(GalleryRow {
                student_id,
                embedding: from_pg(v)?,
            })
        })
        .collect()
}

/// Load the active gallery excluding one student (used during enrollment to
/// check duplicates against *other* students).
pub async fn load_gallery_excluding(db: &Db, exclude: Uuid) -> Result<Vec<GalleryRow>, DbError> {
    let rows: Vec<(Uuid, Vector)> = sqlx::query_as(
        "SELECT student_id, embedding FROM face_templates WHERE active AND student_id <> $1",
    )
    .bind(exclude)
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(student_id, v)| {
            Ok(GalleryRow {
                student_id,
                embedding: from_pg(v)?,
            })
        })
        .collect()
}

/// Nearest students with cosine similarity and rank, ready for the matcher.
///
/// One row per student (their best matching active template). Uses the cosine
/// distance operator `<=>`; similarity = 1 - distance. The query binds the
/// vector twice because it appears in both the projection and the ordering.
pub async fn nearest_with_scores(
    db: &Db,
    query: &Embedding,
    limit: i64,
) -> Result<Vec<(Uuid, f32)>, DbError> {
    let rows: Vec<(Uuid, f64)> = sqlx::query_as(
        r#"
        SELECT student_id, (1.0 - MIN(embedding <=> $1))::float8 AS similarity
        FROM face_templates
        WHERE active
        GROUP BY student_id
        ORDER BY MIN(embedding <=> $1) ASC
        LIMIT $2
        "#,
    )
    .bind(to_pg(query))
    .bind(limit)
    .fetch_all(db)
    .await?;
    Ok(rows.into_iter().map(|(id, s)| (id, s as f32)).collect())
}

pub async fn count_for_student(db: &Db, student_id: Uuid) -> Result<i64, DbError> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM face_templates WHERE student_id = $1")
        .bind(student_id)
        .fetch_one(db)
        .await?;
    Ok(n)
}
