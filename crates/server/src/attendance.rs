//! Attendance service: turns a stream of frames into a decision + record.
//!
//! This is where the pure [`domain`] logic meets the DB and pipeline. It owns a
//! per-session frame window (the last N observations) and applies the decision
//! rules, the cooldown, and the audit log together.

use crate::error::ApiError;
use crate::gallery::GalleryCache;
use crate::state::{AppState, FaceEngine};
use chrono::Utc;
use db::models::Device;
use domain::decision::{decide, Candidate, Decision, DecisionReason, FrameObservation};
use std::sync::Arc;
use uuid::Uuid;

/// Per-session rolling window of frame observations.
///
/// A session corresponds to one kiosk "interaction" — one person standing in
/// front of the camera. It is reset once a decision is reached.
pub struct AttendanceSession {
    device: Device,
    window: Vec<FrameObservation>,
    window_size: usize,
}

/// What the caller should do after pushing a frame.
#[derive(Debug)]
pub enum SessionVerdict {
    /// Keep capturing; show a prompt.
    Continue { prompt: String, frames_seen: usize },
    /// Record attendance (or show "already marked").
    Accepted {
        student_id: Uuid,
        similarity: f32,
        margin: f32,
        /// True if this was within cooldown — no new DB row, just a message.
        already_marked: bool,
    },
    /// Confident rejection — not a known student.
    Rejected { message: String },
}

impl AttendanceSession {
    pub fn new(device: Device, window_size: usize) -> Self {
        Self {
            device,
            window: Vec::with_capacity(window_size),
            window_size,
        }
    }

    /// A session with no device, used only as a `mem::replace` placeholder when
    /// moving the real session into a blocking task. Never used for decisions.
    pub fn new_placeholder(window_size: usize) -> Self {
        Self {
            device: Device {
                id: Uuid::nil(),
                nama: String::new(),
                lokasi: None,
                revoked: false,
                last_seen: None,
                created_at: Utc::now(),
            },
            window: Vec::with_capacity(window_size),
            window_size,
        }
    }

    /// Process one frame (already decoded and, if desired, pre-processed).
    ///
    /// This is synchronous in the CPU-bound pipeline; the caller runs it on a
    /// blocking task. DB writes for the final decision happen in the async
    /// wrapper [`AttendanceSession::push_frame`].
    pub fn observe(
        &mut self,
        face: &Arc<dyn FaceEngine>,
        image: &face_core::RgbImage,
        gallery: &GalleryCache,
        thresholds: &domain::decision::Thresholds,
    ) -> ObservationResult {
        let mut candidates = Vec::new();
        let (quality_passed, liveness_passed, liveness_score) = match face.process(image) {
            Ok(output) => {
                let q = output.quality;
                let qt = face.quality_thresholds();
                let quality_passed = domain::quality::FrameQuality::evaluate(&q, qt).is_passed();
                let liveness_passed = domain::liveness_passed(
                    output.liveness.is_spoof,
                    output.liveness.score,
                    thresholds.t_liveness,
                );
                // Rank against the gallery.
                let ranked = gallery.rank(&output.embedding);
                for (student_id, score) in ranked.into_iter().take(5) {
                    candidates.push(Candidate { student_id, score });
                }
                (quality_passed, liveness_passed, output.liveness.score)
            }
            Err(e) => {
                tracing::debug!(error = %e, "frame could not be processed");
                (false, false, 0.0)
            }
        };

        let observation = FrameObservation {
            quality_passed,
            liveness_passed,
            candidates,
        };

        if self.window.len() >= self.window_size {
            self.window.remove(0);
        }
        self.window.push(observation);

        let decision = decide(&self.window, thresholds);
        ObservationResult {
            decision,
            liveness_score,
            frames_seen: self.window.len(),
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    pub fn reset(&mut self) {
        self.window.clear();
    }
}

pub struct ObservationResult {
    pub decision: Decision,
    pub liveness_score: f32,
    pub frames_seen: usize,
}

/// Turn a decision into a [`SessionVerdict`] and persist side effects.
pub async fn resolve(
    state: &AppState,
    session: &mut AttendanceSession,
    observation: ObservationResult,
) -> Result<SessionVerdict, ApiError> {
    let thresholds = state.config.thresholds();
    // Copy scalars out before `decision` is moved by the match.
    let frames_seen = observation.frames_seen;
    let liveness = observation.liveness_score;
    match observation.decision {
        Decision::Accept {
            student_id,
            top1,
            margin,
            agreeing_frames,
        } => {
            // Cooldown: if the student was marked recently, do not double-record.
            let last = db::attendance_repo::last_marked(&state.db, student_id)
                .await
                .map_err(ApiError::Db)?;
            let now = Utc::now();
            let verdict = domain::cooldown::check(last, now, state.config.cooldown_seconds);

            if let domain::cooldown::CooldownVerdict::WithinCooldown { .. } = verdict {
                db::attendance_repo::record_attempt(
                    &state.db,
                    db::attendance_repo::NewAttempt {
                        device_id: Some(session.device().id),
                        outcome: "cooldown_shown".into(),
                        top1_student_id: Some(student_id),
                        top1_score: Some(top1),
                        top2_score: None,
                        margin: Some(margin),
                        liveness_score: Some(liveness),
                        reason: Some("within cooldown".into()),
                        frame_count: agreeing_frames as i32,
                    },
                )
                .await
                .map_err(ApiError::Db)?;

                session.reset();
                return Ok(SessionVerdict::Accepted {
                    student_id,
                    similarity: top1,
                    margin,
                    already_marked: true,
                });
            }

            // Persist the attendance row.
            db::attendance_repo::record(
                &state.db,
                db::attendance_repo::NewAttendance {
                    student_id,
                    device_id: Some(session.device().id),
                    similarity: top1,
                    margin,
                    liveness_score: liveness,
                    status: "present".into(),
                    note: None,
                },
            )
            .await
            .map_err(ApiError::Db)?;

            db::attendance_repo::record_attempt(
                &state.db,
                db::attendance_repo::NewAttempt {
                    device_id: Some(session.device().id),
                    outcome: "accepted".into(),
                    top1_student_id: Some(student_id),
                    top1_score: Some(top1),
                    top2_score: None,
                    margin: Some(margin),
                    liveness_score: Some(liveness),
                    reason: None,
                    frame_count: agreeing_frames as i32,
                },
            )
            .await
            .map_err(ApiError::Db)?;

            session.reset();
            Ok(SessionVerdict::Accepted {
                student_id,
                similarity: top1,
                margin,
                already_marked: false,
            })
        }
        Decision::Reject { reason } => {
            let outcome = match &reason {
                DecisionReason::BelowAcceptThreshold { .. } => "rejected_unknown",
                DecisionReason::InsufficientMargin { .. } => "rejected_margin",
                DecisionReason::LivenessFailed { .. } => "rejected_liveness",
                _ => "rejected_unknown",
            };
            log_attempt(state, session, outcome, reason, liveness, frames_seen).await?;
            session.reset();
            Ok(SessionVerdict::Rejected {
                message: "Tidak dikenali. Coba lagi atau hubungi guru.".into(),
            })
        }
        Decision::Retry { reason } => {
            let prompt = retry_prompt(&reason, &thresholds);
            Ok(SessionVerdict::Continue {
                prompt,
                frames_seen,
            })
        }
    }
}

async fn log_attempt(
    state: &AppState,
    session: &AttendanceSession,
    outcome: &str,
    reason: DecisionReason,
    liveness_score: f32,
    frames_seen: usize,
) -> Result<(), ApiError> {
    let (top1_student, top1) = match &reason {
        DecisionReason::BelowAcceptThreshold { top1, .. } => (None, Some(*top1)),
        DecisionReason::InsufficientMargin { top1, .. } => (None, Some(*top1)),
        _ => (None, None),
    };
    db::attendance_repo::record_attempt(
        &state.db,
        db::attendance_repo::NewAttempt {
            device_id: Some(session.device().id),
            outcome: outcome.to_string(),
            top1_student_id: top1_student,
            top1_score: top1,
            top2_score: None,
            margin: None,
            liveness_score: Some(liveness_score),
            reason: Some(format!("{reason:?}")),
            frame_count: frames_seen as i32,
        },
    )
    .await
    .map_err(ApiError::Db)?;
    Ok(())
}

fn retry_prompt(reason: &DecisionReason, t: &domain::decision::Thresholds) -> String {
    match reason {
        DecisionReason::NoUsableFrames => "Pastikan wajah terlihat jelas di kamera.".into(),
        DecisionReason::AwaitingConsensus { usable, needed } => {
            format!("Tetap di depan kamera... ({usable}/{needed})")
        }
        DecisionReason::InsufficientMargin { .. } => {
            "Wajah mirip dengan data lain. Coba lagi dengan pencahayaan lebih baik.".into()
        }
        DecisionReason::LivenessFailed { .. } => {
            "Verifikasi keaslian gagal. Pastikan ini wajah langsung, bukan foto.".into()
        }
        DecisionReason::BelowAcceptThreshold { .. } => {
            "Tidak dikenali. Coba mendekat dan menghadap lurus.".into()
        }
        DecisionReason::ConfidentMatch { .. } => format!(
            "Coba lagi. (konsensus {}/{})",
            t.consensus_required, t.consensus_window
        ),
    }
}
