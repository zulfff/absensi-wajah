//! Persisted entity types. These mirror the DB rows and are the types crossing
//! the API boundary (serialised).

use chrono::{DateTime, Utc};
use domain::Embedding;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub role: String,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Class {
    pub id: Uuid,
    pub nama: String,
    pub tingkat: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Student {
    pub id: Uuid,
    pub nis: String,
    pub nama: String,
    pub kelas_id: Option<Uuid>,
    pub status: String,
    pub consent_granted: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct FaceTemplate {
    pub id: Uuid,
    pub student_id: Uuid,
    pub embedding: Embedding,
    pub quality_score: f32,
    pub is_centroid: bool,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Device {
    pub id: Uuid,
    pub nama: String,
    pub lokasi: Option<String>,
    pub revoked: bool,
    pub last_seen: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct Attendance {
    pub id: Uuid,
    pub student_id: Uuid,
    pub device_id: Option<Uuid>,
    pub timestamp: DateTime<Utc>,
    pub similarity: f32,
    pub margin: f32,
    pub liveness_score: f32,
    pub status: String,
    pub note: Option<String>,
}

/// A gallery entry used by the matcher: a student id and one embedding.
#[derive(Clone, Debug)]
pub struct GalleryRow {
    pub student_id: Uuid,
    pub embedding: Embedding,
}
