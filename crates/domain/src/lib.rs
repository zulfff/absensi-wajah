//! Pure domain logic for the face-attendance system.
//!
//! This crate contains **no I/O** — no database, no network, no ONNX. It is the
//! decision layer that decides whether a set of observed frames constitutes a
//! confident attendance event. Keeping it pure makes the security-critical rules
//! (thresholds, margin, multi-frame consensus, cooldown, open-set rejection)
//! trivially unit-testable and auditable.
//!
//! Design principle: **fail-closed**. When in doubt, reject and ask the student
//! to try again rather than record the wrong person. A false accept is far worse
//! than a false reject.

pub mod attendance;
pub mod cooldown;
pub mod decision;
pub mod embedding;
pub mod enroll;
pub mod error;
pub mod quality;

pub use attendance::{AttemptOutcome, AttendanceRecord, AttendanceStatus};
pub use decision::{liveness_passed, Decision, DecisionReason, Thresholds};
pub use embedding::{centroid, cosine_similarity, Embedding, EMBEDDING_DIM};
pub use error::DomainError;
pub use quality::{FrameQuality, QualityRejection};
