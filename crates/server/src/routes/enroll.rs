//! Enrollment API (plan Section 4).
//!
//! Two-step flow:
//! 1. `POST /api/students/:id/enroll/frame` — send one captured frame, get
//!    immediate quality feedback. Repeated ~10–15 times by the admin UI, which
//!    guides the capture (straight, left, right, up, down, ...).
//! 2. `POST /api/students/:id/enroll/commit` — run consistency + duplicate +
//!    separation checks over the collected frames and store the templates in
//!    `pending` (inactive) state. Admin then activates.

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;
use base64::Engine;
use domain::enroll::{evaluate_enrollment, EnrollThresholds, EnrollVerdict, GalleryEntry};
use domain::Embedding;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Decode a base64 (optionally data-URL) image body into an RGB frame.
fn decode_frame(image: &str) -> Result<face_core::RgbImage, ApiError> {
    let payload = image.split_once(',').map(|(_, b)| b).unwrap_or(image);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|_| ApiError::BadRequest("frame bukan base64 yang valid".into()))?;
    face_core::RgbImage::decode(&bytes)
        .map_err(|e| ApiError::BadRequest(format!("frame tidak dapat didekode: {e}")))
}

#[derive(Deserialize)]
pub struct FrameRequest {
    /// Base64 JPEG/PNG, optionally as a data URL.
    pub image: String,
    /// Index of this capture in the guided sequence (0-based).
    #[serde(default)]
    pub step: usize,
}

#[derive(Serialize)]
pub struct FrameResponse {
    /// Whether this frame passed the quality gate.
    pub accepted: bool,
    /// Quality score 0..1 if accepted.
    pub score: f32,
    /// Machine-readable rejection reason, if rejected.
    pub reason: Option<String>,
    /// Human guidance to show the admin.
    pub guidance: String,
}

/// Assess one enrollment frame (no storage yet — the client collects them).
pub async fn enroll_frame(
    State(state): State<AppState>,
    AdminAuth(_): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<FrameRequest>,
) -> Result<Json<FrameResponse>, ApiError> {
    // Confirm the student exists and has consent (Section 9).
    let student = db::student_repo::find(&state.db, id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("student {id}")))?;
    if !student.consent_granted {
        return Err(ApiError::Forbidden);
    }

    let image = decode_frame(&req.image)?;
    let output = match state.face.process(&image) {
        Ok(o) => o,
        Err(e) => {
            return Ok(Json(FrameResponse {
                accepted: false,
                score: 0.0,
                reason: Some(format!("{e}")),
                guidance: "Wajah tidak terdeteksi dengan jelas.".into(),
            }));
        }
    };

    let qt = state.face.quality_thresholds();
    let quality = domain::quality::FrameQuality::evaluate(&output.quality, qt);
    // Same liveness rule as attendance: a frame too weak to ever pass the
    // decision engine must not be stored as a reference template either.
    let liveness_ok = domain::liveness_passed(
        output.liveness.is_spoof,
        output.liveness.score,
        state.config.thresholds().t_liveness,
    );
    let _ = req.step;

    match quality {
        domain::quality::FrameQuality::Passed { score } => {
            if !liveness_ok {
                Ok(Json(FrameResponse {
                    accepted: false,
                    score,
                    reason: Some("spoof_detected".into()),
                    guidance: "Terdeteksi bukan wajah langsung. Gunakan wajah asli.".into(),
                }))
            } else {
                Ok(Json(FrameResponse {
                    accepted: true,
                    score,
                    reason: None,
                    guidance: "Bagus. Lanjut ke pose berikutnya.".into(),
                }))
            }
        }
        domain::quality::FrameQuality::Rejected { reason } => Ok(Json(FrameResponse {
            accepted: false,
            score: 0.0,
            reason: Some(serde_json::to_string(&reason).unwrap_or_default()),
            guidance: reason.guidance().into(),
        })),
    }
}

#[derive(Deserialize)]
pub struct CommitRequest {
    /// All frames collected during the guided capture.
    pub frames: Vec<String>,
}

/// Hard cap on frames per commit. Each frame runs full ONNX inference, so an
/// unbounded list is a CPU exhaustion vector. 30 is far above the ~15 a guided
/// capture actually sends, so legitimate flows never hit it.
const MAX_COMMIT_FRAMES: usize = 30;

/// Hard cap on one frame's base64 length (~2 MB decoded). The 6 MB body cap
/// bounds the whole request; this bounds a single frame so one huge frame
/// cannot crowd out the rest.
const MAX_FRAME_B64_LEN: usize = 3 * 1024 * 1024;

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CommitResponse {
    Accepted {
        templates_created: usize,
        dropped_frames: usize,
        nearest_neighbour_similarity: f32,
        pending_activation: bool,
    },
    Duplicate {
        conflicts_with: Uuid,
        similarity: f32,
    },
    RiskySeparation {
        nearest_neighbour_id: Uuid,
        similarity: f32,
    },
    InsufficientFrames {
        kept: usize,
        needed: usize,
    },
    NoFrames,
}

/// Finalize enrollment: check and store templates.
pub async fn enroll_commit(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<CommitRequest>,
) -> Result<Json<CommitResponse>, ApiError> {
    let student = db::student_repo::find(&state.db, id)
        .await
        .map_err(ApiError::Db)?
        .ok_or_else(|| ApiError::NotFound(format!("student {id}")))?;
    if !student.consent_granted {
        return Err(ApiError::Forbidden);
    }

    // Bound the work before touching the model.
    if req.frames.len() > MAX_COMMIT_FRAMES {
        return Err(ApiError::BadRequest(format!(
            "terlalu banyak frame: maksimum {MAX_COMMIT_FRAMES}"
        )));
    }
    if req.frames.iter().any(|f| f.len() > MAX_FRAME_B64_LEN) {
        return Err(ApiError::PayloadTooLarge);
    }

    // Embed every frame; skip any that cannot be processed.
    let mut embeddings: Vec<Embedding> = Vec::new();
    let mut total_frames = 0usize;
    let t_liveness = state.config.thresholds().t_liveness;
    for raw in &req.frames {
        total_frames += 1;
        let Ok(image) = decode_frame(raw) else {
            continue;
        };
        let Ok(output) = state.face.process(&image) else {
            continue;
        };
        // Only keep frames that would themselves pass quality + liveness, using
        // the exact same liveness rule as the attendance decision engine.
        let qt = state.face.quality_thresholds();
        if domain::quality::FrameQuality::evaluate(&output.quality, qt).is_passed()
            && domain::liveness_passed(output.liveness.is_spoof, output.liveness.score, t_liveness)
        {
            embeddings.push(output.embedding);
        }
    }

    if embeddings.is_empty() {
        return Ok(Json(CommitResponse::NoFrames));
    }

    // Build the gallery of *other* students for the duplicate check.
    let gallery_rows = db::face_repo::load_gallery_excluding(&state.db, id)
        .await
        .map_err(ApiError::Db)?;
    let gallery: Vec<GalleryEntry> = gallery_rows
        .into_iter()
        .map(|r| GalleryEntry {
            student_id: r.student_id,
            embedding: r.embedding,
        })
        .collect();

    let thresholds = EnrollThresholds::default();
    let verdict = evaluate_enrollment(&embeddings, &gallery, &thresholds);

    match verdict {
        EnrollVerdict::Duplicate {
            conflicts_with,
            similarity,
            ..
        } => {
            db::audit(
                &state.db,
                Some(claims.sub),
                "enroll_duplicate_blocked",
                Some(&id.to_string()),
                Some(serde_json::json!({
                    "conflicts_with": conflicts_with,
                    "similarity": similarity,
                })),
            )
            .await
            .ok();
            Ok(Json(CommitResponse::Duplicate {
                conflicts_with,
                similarity,
            }))
        }
        EnrollVerdict::RiskySeparation {
            nearest_neighbour_id,
            similarity,
            ..
        } => Ok(Json(CommitResponse::RiskySeparation {
            nearest_neighbour_id,
            similarity,
        })),
        EnrollVerdict::InsufficientFrames { kept, needed } => {
            Ok(Json(CommitResponse::InsufficientFrames { kept, needed }))
        }
        EnrollVerdict::NoFrames => Ok(Json(CommitResponse::NoFrames)),
        EnrollVerdict::Accepted {
            nearest_neighbour_similarity,
            kept_frames,
            ..
        }
        | EnrollVerdict::AcceptedWithOutliersDropped {
            nearest_neighbour_similarity,
            kept_frames,
            ..
        } => {
            // Store the centroid (best single representative) plus each kept
            // frame, all inactive until the admin activates. Replaces any prior
            // templates for a clean re-enroll, atomically.
            //
            // Re-derive the kept set at the SAME threshold the verdict used, so
            // what we store is exactly what was validated. The verdict reports
            // only the count (`kept_frames`), not the indices, hence the
            // re-filter; the debug assert guards that the two never diverge.
            let kept: Vec<Embedding> =
                domain::enroll::filter_outliers(&embeddings, thresholds.min_internal_similarity)
                    .into_iter()
                    .map(|i| embeddings[i].clone())
                    .collect();
            debug_assert_eq!(
                kept.len(),
                kept_frames,
                "verdict kept_frames disagreed with a re-filter at the same threshold"
            );

            let centroid = domain::centroid(&kept)
                .map_err(|e| ApiError::Internal(anyhow::anyhow!("centroid: {e}")))?;

            // Centroid first (is_centroid = true, best quality), then the frames.
            let mut to_store: Vec<(Embedding, f32, bool)> = Vec::with_capacity(kept.len() + 1);
            to_store.push((centroid, 1.0, true));
            for e in &kept {
                to_store.push((e.clone(), 0.8, false));
            }
            let stored = db::face_repo::replace_templates(&state.db, id, &to_store)
                .await
                .map_err(ApiError::Db)?;

            // Report drops relative to frames that were actually eligible
            // (passed quality + liveness), not every raw frame submitted — a
            // frame rejected before embedding was never a candidate outlier.
            // `submitted` is still recorded so the raw count stays visible.
            let dropped = embeddings.len().saturating_sub(kept.len());
            db::audit(
                &state.db,
                Some(claims.sub),
                "enroll_committed",
                Some(&id.to_string()),
                Some(serde_json::json!({
                    "kept": kept.len(),
                    "dropped": dropped,
                    "submitted": total_frames,
                })),
            )
            .await
            .ok();

            Ok(Json(CommitResponse::Accepted {
                templates_created: stored,
                dropped_frames: dropped,
                nearest_neighbour_similarity,
                pending_activation: true,
            }))
        }
    }
}

/// Activate a student's templates after review.
pub async fn activate(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let n = db::face_repo::activate_templates(&state.db, id)
        .await
        .map_err(ApiError::Db)?;
    let students = state.gallery.reload(&state.db).await;
    db::audit(
        &state.db,
        Some(claims.sub),
        "enroll_activated",
        Some(&id.to_string()),
        None,
    )
    .await
    .ok();
    Ok(Json(serde_json::json!({
        "activated_templates": n,
        "gallery_students": students,
    })))
}

pub async fn deactivate(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    db::face_repo::deactivate_templates(&state.db, id)
        .await
        .map_err(ApiError::Db)?;
    state.gallery.reload(&state.db).await;
    db::audit(
        &state.db,
        Some(claims.sub),
        "enroll_deactivated",
        Some(&id.to_string()),
        None,
    )
    .await
    .ok();
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Permanently delete a student's biometric data (retention / right to erasure).
pub async fn delete_face(
    State(state): State<AppState>,
    AdminAuth(claims): AdminAuth,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let n = db::face_repo::delete_templates(&state.db, id)
        .await
        .map_err(ApiError::Db)?;
    state.gallery.reload(&state.db).await;
    db::audit(
        &state.db,
        Some(claims.sub),
        "biometric_deleted",
        Some(&id.to_string()),
        None,
    )
    .await
    .ok();
    Ok(Json(serde_json::json!({ "deleted_templates": n })))
}
