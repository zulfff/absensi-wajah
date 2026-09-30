//! Frame quality gate.
//!
//! Every frame — enrolment or attendance — must pass this gate before its
//! embedding is trusted. A frame that is blurry, tiny, badly lit, or badly
//! posed produces an unreliable embedding; using it would corrupt matching.
//!
//! The gate is deliberately conservative: reject, capture again. The cost of a
//! re-capture is a few seconds; the cost of a bad enrolment is a person who can
//! never be recognised, or worse, one who is recognised as somebody else.

use serde::{Deserialize, Serialize};

/// Why a frame was rejected. Serialised to the client so the kiosk/admin UI can
/// tell the user exactly what to fix ("move closer", "hold still", ...).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityRejection {
    /// Zero or more than one face in the frame.
    NotExactlyOneFace { count: usize },
    /// Face bounding box smaller than the minimum on its short side.
    FaceTooSmall { min_px: u32, got_px: u32 },
    /// Laplacian variance below the blur threshold.
    TooBlurry {
        min_variance: f32,
        got_variance: f32,
    },
    /// Mean luminance outside the acceptable band.
    BadExposure { min: f32, max: f32, got: f32 },
    /// Backlit: face darker than the surrounding background by too much.
    Backlit { delta: f32, max_delta: f32 },
    /// Head pose outside the allowed yaw/pitch/roll window.
    PoseOutOfRange {
        max_degrees: f32,
        yaw: f32,
        pitch: f32,
        roll: f32,
    },
    /// Eyes closed / occluded face.
    EyesClosedOrOccluded,
}

impl QualityRejection {
    /// Human-readable guidance for the on-screen prompt.
    pub fn guidance(&self) -> &'static str {
        match self {
            QualityRejection::NotExactlyOneFace { count } if *count == 0 => {
                "Wajah tidak terdeteksi. Posisikan wajah di tengah kamera."
            }
            QualityRejection::NotExactlyOneFace { .. } => {
                "Terdeteksi lebih dari satu wajah. Hanya satu orang di depan kamera."
            }
            QualityRejection::FaceTooSmall { .. } => "Terlalu jauh. Mendekatlah ke kamera.",
            QualityRejection::TooBlurry { .. } => {
                "Gambar buram. Tahan sebentar dan jangan bergerak."
            }
            QualityRejection::BadExposure { .. } => {
                "Pencahayaan kurang. Cari tempat yang lebih terang."
            }
            QualityRejection::Backlit { .. } => {
                "Cahaya dari belakang. Menghadaplah ke arah cahaya."
            }
            QualityRejection::PoseOutOfRange { .. } => "Menghadap lurus ke kamera.",
            QualityRejection::EyesClosedOrOccluded => {
                "Buka mata dan pastikan wajah tidak tertutup."
            }
        }
    }
}

/// Raw, measured quality signals for a single frame.
///
/// This struct carries measurements only; the pass/fail logic lives in
/// [`FrameQuality::evaluate`] so it can be unit-tested without any image code.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QualitySignals {
    /// Number of faces detected in the frame.
    pub face_count: usize,
    /// Short side of the primary face bounding box, in pixels.
    pub face_px: u32,
    /// Variance of the Laplacian (sharpness). Higher is sharper.
    pub laplacian_variance: f32,
    /// Mean luminance of the face region, 0.0..=255.0.
    pub mean_luminance: f32,
    /// Mean luminance of the surrounding background, 0.0..=255.0.
    pub background_luminance: f32,
    /// Head yaw in degrees (positive = turned right).
    pub yaw: f32,
    /// Head pitch in degrees (positive = looking up).
    pub pitch: f32,
    /// Head roll in degrees.
    pub roll: f32,
    /// Whether both eyes are open (from landmarks).
    pub eyes_open: bool,
}

/// Thresholds for the quality gate. Chosen per deployment; never guessed in code.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct QualityThresholds {
    pub min_face_px: u32,
    pub min_laplacian_variance: f32,
    pub min_luminance: f32,
    pub max_luminance: f32,
    pub max_backlight_delta: f32,
    pub max_pose_degrees: f32,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        // Conservative starting values. Section 10 of the plan requires these to
        // be re-measured against the school's own dataset during Phase 5.
        Self {
            min_face_px: 112,
            min_laplacian_variance: 60.0,
            min_luminance: 45.0,
            max_luminance: 220.0,
            max_backlight_delta: 70.0,
            max_pose_degrees: 25.0,
        }
    }
}

/// The result of evaluating one frame: passed, or rejected with a reason.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum FrameQuality {
    Passed {
        /// Composite quality score in `0.0..=1.0`, for ranking frames.
        score: f32,
    },
    Rejected {
        reason: QualityRejection,
    },
}

impl FrameQuality {
    /// Evaluate measured signals against thresholds.
    ///
    /// Order of checks matters for the user experience: the most actionable,
    /// most fundamental problem is reported first (no face -> distance -> blur
    /// -> light -> pose).
    pub fn evaluate(signals: &QualitySignals, t: &QualityThresholds) -> FrameQuality {
        if signals.face_count != 1 {
            return FrameQuality::Rejected {
                reason: QualityRejection::NotExactlyOneFace {
                    count: signals.face_count,
                },
            };
        }
        if signals.face_px < t.min_face_px {
            return FrameQuality::Rejected {
                reason: QualityRejection::FaceTooSmall {
                    min_px: t.min_face_px,
                    got_px: signals.face_px,
                },
            };
        }
        if signals.laplacian_variance < t.min_laplacian_variance {
            return FrameQuality::Rejected {
                reason: QualityRejection::TooBlurry {
                    min_variance: t.min_laplacian_variance,
                    got_variance: signals.laplacian_variance,
                },
            };
        }
        if signals.mean_luminance < t.min_luminance || signals.mean_luminance > t.max_luminance {
            return FrameQuality::Rejected {
                reason: QualityRejection::BadExposure {
                    min: t.min_luminance,
                    max: t.max_luminance,
                    got: signals.mean_luminance,
                },
            };
        }
        let backlight_delta = signals.background_luminance - signals.mean_luminance;
        if backlight_delta > t.max_backlight_delta {
            return FrameQuality::Rejected {
                reason: QualityRejection::Backlit {
                    delta: backlight_delta,
                    max_delta: t.max_backlight_delta,
                },
            };
        }
        if signals.yaw.abs() > t.max_pose_degrees
            || signals.pitch.abs() > t.max_pose_degrees
            || signals.roll.abs() > t.max_pose_degrees
        {
            return FrameQuality::Rejected {
                reason: QualityRejection::PoseOutOfRange {
                    max_degrees: t.max_pose_degrees,
                    yaw: signals.yaw,
                    pitch: signals.pitch,
                    roll: signals.roll,
                },
            };
        }
        if !signals.eyes_open {
            return FrameQuality::Rejected {
                reason: QualityRejection::EyesClosedOrOccluded,
            };
        }
        FrameQuality::Passed {
            score: composite_score(signals, t),
        }
    }

    pub fn is_passed(&self) -> bool {
        matches!(self, FrameQuality::Passed { .. })
    }

    pub fn score(&self) -> f32 {
        match self {
            FrameQuality::Passed { score } => *score,
            FrameQuality::Rejected { .. } => 0.0,
        }
    }
}

/// Composite 0..1 score used only to *rank* passing frames (keep the best N).
/// It is not a gate — the gate is the ordered checks above.
fn composite_score(s: &QualitySignals, t: &QualityThresholds) -> f32 {
    let sharpness = (s.laplacian_variance / (t.min_laplacian_variance * 3.0)).clamp(0.0, 1.0);
    let size = (s.face_px as f32 / (t.min_face_px as f32 * 2.0)).clamp(0.0, 1.0);
    let mid = (t.min_luminance + t.max_luminance) / 2.0;
    let half_band = (t.max_luminance - t.min_luminance) / 2.0;
    let exposure = (1.0 - (s.mean_luminance - mid).abs() / half_band).clamp(0.0, 1.0);
    let pose_ratio = s.yaw.abs().max(s.pitch.abs()).max(s.roll.abs()) / t.max_pose_degrees;
    let pose = (1.0 - pose_ratio).clamp(0.0, 1.0);
    (sharpness + size + exposure + pose) / 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> QualitySignals {
        QualitySignals {
            face_count: 1,
            face_px: 200,
            laplacian_variance: 180.0,
            mean_luminance: 130.0,
            background_luminance: 130.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eyes_open: true,
        }
    }

    #[test]
    fn good_frame_passes() {
        assert!(FrameQuality::evaluate(&good(), &QualityThresholds::default()).is_passed());
    }

    #[test]
    fn zero_faces_rejected() {
        let mut s = good();
        s.face_count = 0;
        let q = FrameQuality::evaluate(&s, &QualityThresholds::default());
        assert_eq!(
            q,
            FrameQuality::Rejected {
                reason: QualityRejection::NotExactlyOneFace { count: 0 }
            }
        );
    }

    #[test]
    fn two_faces_rejected() {
        let mut s = good();
        s.face_count = 2;
        let q = FrameQuality::evaluate(&s, &QualityThresholds::default());
        assert!(matches!(
            q,
            FrameQuality::Rejected {
                reason: QualityRejection::NotExactlyOneFace { count: 2 }
            }
        ));
    }

    #[test]
    fn small_face_rejected() {
        let mut s = good();
        s.face_px = 50;
        let q = FrameQuality::evaluate(&s, &QualityThresholds::default());
        assert!(matches!(
            q,
            FrameQuality::Rejected {
                reason: QualityRejection::FaceTooSmall { .. }
            }
        ));
    }

    #[test]
    fn blurry_rejected() {
        let mut s = good();
        s.laplacian_variance = 5.0;
        let q = FrameQuality::evaluate(&s, &QualityThresholds::default());
        assert!(matches!(
            q,
            FrameQuality::Rejected {
                reason: QualityRejection::TooBlurry { .. }
            }
        ));
    }

    #[test]
    fn dark_and_bright_rejected() {
        let mut dark = good();
        dark.mean_luminance = 10.0;
        assert!(matches!(
            FrameQuality::evaluate(&dark, &QualityThresholds::default()),
            FrameQuality::Rejected {
                reason: QualityRejection::BadExposure { .. }
            }
        ));
        let mut bright = good();
        bright.mean_luminance = 250.0;
        assert!(matches!(
            FrameQuality::evaluate(&bright, &QualityThresholds::default()),
            FrameQuality::Rejected {
                reason: QualityRejection::BadExposure { .. }
            }
        ));
    }

    #[test]
    fn backlit_rejected() {
        let mut s = good();
        s.mean_luminance = 90.0;
        s.background_luminance = 220.0;
        assert!(matches!(
            FrameQuality::evaluate(&s, &QualityThresholds::default()),
            FrameQuality::Rejected {
                reason: QualityRejection::Backlit { .. }
            }
        ));
    }

    #[test]
    fn pose_rejected() {
        let mut s = good();
        s.yaw = 40.0;
        assert!(matches!(
            FrameQuality::evaluate(&s, &QualityThresholds::default()),
            FrameQuality::Rejected {
                reason: QualityRejection::PoseOutOfRange { .. }
            }
        ));
    }

    #[test]
    fn eyes_closed_rejected() {
        let mut s = good();
        s.eyes_open = false;
        assert_eq!(
            FrameQuality::evaluate(&s, &QualityThresholds::default()),
            FrameQuality::Rejected {
                reason: QualityRejection::EyesClosedOrOccluded
            }
        );
    }

    #[test]
    fn ordering_reports_most_fundamental_first() {
        // Both no-face and blurry; no-face must win.
        let s = QualitySignals {
            face_count: 0,
            laplacian_variance: 1.0,
            ..good()
        };
        assert!(matches!(
            FrameQuality::evaluate(&s, &QualityThresholds::default()),
            FrameQuality::Rejected {
                reason: QualityRejection::NotExactlyOneFace { .. }
            }
        ));
    }

    #[test]
    fn composite_score_in_unit_range() {
        let q = FrameQuality::evaluate(&good(), &QualityThresholds::default());
        let score = q.score();
        assert!((0.0..=1.0).contains(&score), "score was {score}");
    }
}
