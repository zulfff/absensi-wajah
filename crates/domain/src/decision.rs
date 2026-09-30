//! The attendance decision engine — the security-critical core.
//!
//! Given a stream of observed frames (each with a top-1 candidate, its score,
//! the runner-up score, and a liveness verdict), decide one of:
//!
//! - **Accept** a student — only when *every* condition holds, over enough
//!   consecutive frames.
//! - **Reject** — a confident "this is not a known student".
//! - **Retry** — insufficient evidence; ask for another attempt.
//!
//! The rules implement Section 6 of the plan (strategies to suppress false
//! positives). Every condition is an AND; there is no path to Accept that skips
//! one. That is what "fail-closed" means in code.

use crate::quality::QualitySignals;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A candidate a frame matched against the enrolment gallery.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub student_id: Uuid,
    /// Cosine similarity to the best-matching template of this student.
    pub score: f32,
}

/// The outcome of processing one frame against the gallery.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FrameObservation {
    /// Whether the frame passed the quality gate.
    pub quality_passed: bool,
    /// Whether the frame passed liveness / anti-spoof.
    pub liveness_passed: bool,
    /// Ranked match candidates (highest first). Empty = no templates or all below floor.
    pub candidates: Vec<Candidate>,
}

impl FrameObservation {
    /// A frame that cannot contribute evidence (failed quality or liveness).
    pub fn unusable() -> Self {
        Self {
            quality_passed: false,
            liveness_passed: false,
            candidates: Vec::new(),
        }
    }

    /// The best (highest-scoring) candidate, if any.
    ///
    /// Computed as the true maximum rather than trusting list order: the
    /// `candidates` field is public and a caller may not have sorted it. A
    /// wrongly-ordered list would otherwise make `top1()` return a *lower*
    /// score than `best_other()`, producing a negative margin and silently
    /// rejecting a genuine match.
    pub fn top1(&self) -> Option<Candidate> {
        self.candidates.iter().copied().max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// The best candidate belonging to a *different* student than `exclude`.
    /// This is the margin comparison target: how close is the nearest impostor?
    pub fn best_other(&self, exclude: Uuid) -> Option<Candidate> {
        self.candidates
            .iter()
            .filter(|c| c.student_id != exclude)
            .copied()
            .max_by(|a, b| {
                a.score
                    .partial_cmp(&b.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Margin for `top1`: `top1 - best_other`.
    ///
    /// When there is no *other* student at all (an empty gallery, or the only
    /// candidate is the same student), there is no competitor, so the margin is
    /// taken as the full remaining headroom above zero rather than a fake
    /// sentinel. Returning `top1.score - 0.0` keeps the value meaningful for
    /// the audit log (it can never exceed the 1.0 similarity ceiling) instead
    /// of the previously-reportable nonsense margin of ~1.9.
    pub fn margin_for(&self, top1: &Candidate) -> f32 {
        match self.best_other(top1.student_id) {
            Some(other) => top1.score - other.score,
            None => top1.score.max(0.0),
        }
    }

    /// A frame only contributes to consensus if it is usable and has a top-1.
    fn is_usable(&self) -> bool {
        self.quality_passed && self.liveness_passed && self.top1().is_some()
    }

    /// A frame that passed quality and produced a candidate, but was rejected
    /// by liveness. Used to distinguish a spoof attempt from a bad capture.
    fn is_liveness_only_failure(&self) -> bool {
        self.quality_passed && !self.liveness_passed && self.top1().is_some()
    }
}

/// Decision thresholds. These are **measured**, never guessed — see Section 10
/// of the plan. Defaults here are deliberately strict placeholders and MUST be
/// replaced by values derived from the school's own ROC curve in Phase 5.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// Minimum top-1 cosine similarity to accept a match.
    pub t_accept: f32,
    /// Minimum required margin (top1 - best_other) to accept. Guards against
    /// two students who look alike.
    pub t_margin: f32,
    /// How many of the last `window` frames must agree on the same identity.
    pub consensus_required: usize,
    /// Sliding window size for consensus.
    pub consensus_window: usize,
    /// Minimum liveness score, 0..=1.
    pub t_liveness: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        // Strict, fail-closed starting point. Re-measure in Phase 5.
        Self {
            t_accept: 0.50,
            t_margin: 0.08,
            consensus_required: 5,
            consensus_window: 7,
            t_liveness: 0.85,
        }
    }
}

impl Thresholds {
    /// Guard against a misconfigured threshold set at load time.
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=1.0).contains(&self.t_accept) {
            return Err(format!("t_accept out of range: {}", self.t_accept));
        }
        if self.t_margin < 0.0 {
            return Err(format!("t_margin must be non-negative: {}", self.t_margin));
        }
        if self.consensus_required == 0 || self.consensus_required > self.consensus_window {
            return Err(format!(
                "consensus_required ({}) must be in 1..=consensus_window ({})",
                self.consensus_required, self.consensus_window
            ));
        }
        if !(0.0..=1.0).contains(&self.t_liveness) {
            return Err(format!("t_liveness out of range: {}", self.t_liveness));
        }
        Ok(())
    }
}

/// Why a decision came out the way it did. Recorded for every attempt (audit &
/// threshold tuning, Section 5.8).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionReason {
    /// A known student was confidently and consistently identified.
    ConfidentMatch {
        student_id: Uuid,
        top1: f32,
        margin: f32,
        agreeing_frames: usize,
    },
    /// No usable frames yet (all failed quality/liveness).
    NoUsableFrames,
    /// Not enough usable frames yet to reach consensus — keep capturing.
    AwaitingConsensus { usable: usize, needed: usize },
    /// Best score below `t_accept`: this is probably not an enrolled student.
    BelowAcceptThreshold { top1: f32, threshold: f32 },
    /// Top-1 vs top-2 too close: risk of confusing two students.
    InsufficientMargin {
        top1: f32,
        best_other: f32,
        margin: f32,
        required: f32,
    },
    /// Frames passed the quality gate but failed liveness (photo / screen /
    /// replay). This is a deliberate spoof signal, not a bad-capture signal, so
    /// it is reported distinctly from [`DecisionReason::NoUsableFrames`].
    LivenessFailed {
        /// How many otherwise-usable frames were rejected by liveness.
        failing_frames: usize,
        required: f32,
    },
}

/// The decision for the current window of observations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// Record attendance for this student.
    Accept {
        student_id: Uuid,
        top1: f32,
        margin: f32,
        agreeing_frames: usize,
    },
    /// Not a known student (or clear conflict) — do not guess a name.
    Reject { reason: DecisionReason },
    /// Not enough evidence — prompt to try again.
    Retry { reason: DecisionReason },
}

/// Evaluate a window of observations and produce a decision.
///
/// Pure function: same inputs always produce the same decision. This is the
/// single place where the accept conditions are AND-ed together.
///
/// Defensive by construction: it does not trust that [`Thresholds::validate`]
/// was called. A `consensus_required` of `0` would otherwise let a single frame
/// accept, so it is floored at `1` here as well — the accept path must never
/// depend on a caller having validated its config.
pub fn decide(window: &[FrameObservation], thresholds: &Thresholds) -> Decision {
    let consensus_required = thresholds.consensus_required.max(1);
    let usable: Vec<&FrameObservation> = window.iter().filter(|o| o.is_usable()).collect();

    if usable.is_empty() {
        // Distinguish a spoof attempt from a merely bad capture: if any frame
        // was otherwise good but failed liveness, say so — the kiosk and the
        // audit log must not report a photograph as "no face".
        let liveness_failures = window
            .iter()
            .filter(|o| o.is_liveness_only_failure())
            .count();
        if liveness_failures > 0 {
            return Decision::Retry {
                reason: DecisionReason::LivenessFailed {
                    failing_frames: liveness_failures,
                    required: thresholds.t_liveness,
                },
            };
        }
        return Decision::Retry {
            reason: DecisionReason::NoUsableFrames,
        };
    }

    // Tally agreement on a single identity among usable frames. A frame agrees
    // on identity X only if X is its top-1 AND clears t_accept AND clears the
    // margin against the nearest other student.
    let mut tally: std::collections::HashMap<Uuid, usize> = std::collections::HashMap::new();
    // Per student, the (top1, margin) pair from the *single* frame with the
    // highest top-1. Storing the pair together — not independent maxima —
    // guarantees the numbers reported for an Accept come from one observation,
    // so the audit dataset never contains a top1 and a margin that never co-
    // occurred.
    let mut best_seen: std::collections::HashMap<Uuid, (f32, f32)> =
        std::collections::HashMap::new();

    for obs in &usable {
        let Some(top1) = obs.top1() else { continue };
        if top1.score < thresholds.t_accept {
            continue;
        }
        let margin = obs.margin_for(&top1);
        if margin < thresholds.t_margin {
            continue;
        }
        *tally.entry(top1.student_id).or_insert(0) += 1;
        let entry = best_seen
            .entry(top1.student_id)
            .or_insert((f32::MIN, margin));
        if top1.score > entry.0 {
            *entry = (top1.score, margin);
        }
    }

    // Find the identity with the most agreeing frames.
    let leader = tally
        .iter()
        .max_by_key(|(_, count)| **count)
        .map(|(id, count)| (*id, *count));

    if let Some((student_id, agreeing)) = leader {
        if agreeing >= consensus_required {
            let (top1, margin) = best_seen[&student_id];
            return Decision::Accept {
                student_id,
                top1,
                margin,
                agreeing_frames: agreeing,
            };
        }
        // Somebody is matching but not consistently enough yet.
        return Decision::Retry {
            reason: DecisionReason::AwaitingConsensus {
                usable: agreeing,
                needed: consensus_required,
            },
        };
    }

    // Nobody cleared both t_accept and t_margin. Diagnose *why* from the best
    // usable frame, so the retry message is honest.
    let best = usable
        .iter()
        .filter_map(|o| o.top1().map(|c| (o, c)))
        .max_by(|a, b| {
            a.1.score
                .partial_cmp(&b.1.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    let Some((obs, top1)) = best else {
        return Decision::Retry {
            reason: DecisionReason::NoUsableFrames,
        };
    };

    if top1.score < thresholds.t_accept {
        // Below accept. This is a confident-enough reject if we have enough
        // usable frames; otherwise ask to try again.
        if usable.len() >= consensus_required {
            return Decision::Reject {
                reason: DecisionReason::BelowAcceptThreshold {
                    top1: top1.score,
                    threshold: thresholds.t_accept,
                },
            };
        }
        return Decision::Retry {
            reason: DecisionReason::BelowAcceptThreshold {
                top1: top1.score,
                threshold: thresholds.t_accept,
            },
        };
    }

    // Score is high enough but margin is not: two students look alike.
    let best_other = obs
        .best_other(top1.student_id)
        .map(|c| c.score)
        .unwrap_or(0.0);
    let margin = top1.score - best_other;
    if margin < thresholds.t_margin {
        return Decision::Retry {
            reason: DecisionReason::InsufficientMargin {
                top1: top1.score,
                best_other,
                margin,
                required: thresholds.t_margin,
            },
        };
    }

    // Should be unreachable: cleared accept + margin implies the frame would
    // have been tallied above. Kept as a safe fallback rather than an unwrap.
    Decision::Retry {
        reason: DecisionReason::AwaitingConsensus {
            usable: 0,
            needed: consensus_required,
        },
    }
}

/// The single source of truth for "does this frame pass liveness?".
///
/// Both enrolment (quality gate before storing a template) and attendance (the
/// decision engine) must answer this identically, or a face can be enrolled but
/// never recognised. The rule is the AND of two independent signals: the passive
/// model must not have flagged a spoof, AND the live score must clear the
/// threshold. Using only one of the two is the bug this function exists to
/// prevent.
pub fn liveness_passed(is_spoof: bool, score: f32, t_liveness: f32) -> bool {
    !is_spoof && score >= t_liveness
}

/// Convenience: build a [`FrameObservation`] from the pieces the pipeline has.
pub fn observation(
    quality: &QualitySignals,
    quality_thresholds: &crate::quality::QualityThresholds,
    liveness_score: f32,
    is_spoof: bool,
    thresholds: &Thresholds,
    candidates: Vec<Candidate>,
) -> FrameObservation {
    let quality_passed =
        crate::quality::FrameQuality::evaluate(quality, quality_thresholds).is_passed();
    FrameObservation {
        quality_passed,
        liveness_passed: liveness_passed(is_spoof, liveness_score, thresholds.t_liveness),
        candidates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn obs(student: Uuid, score: f32, other: Option<(Uuid, f32)>) -> FrameObservation {
        let mut candidates = vec![Candidate {
            student_id: student,
            score,
        }];
        if let Some((oid, oscore)) = other {
            candidates.push(Candidate {
                student_id: oid,
                score: oscore,
            });
            candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        }
        FrameObservation {
            quality_passed: true,
            liveness_passed: true,
            candidates,
        }
    }

    fn thresholds() -> Thresholds {
        Thresholds::default()
    }

    #[test]
    fn empty_window_retries() {
        assert!(matches!(
            decide(&[], &thresholds()),
            Decision::Retry {
                reason: DecisionReason::NoUsableFrames
            }
        ));
    }

    #[test]
    fn all_unusable_retries() {
        let w = vec![FrameObservation::unusable(); 7];
        assert!(matches!(
            decide(&w, &thresholds()),
            Decision::Retry {
                reason: DecisionReason::NoUsableFrames
            }
        ));
    }

    #[test]
    fn consistent_high_scores_accept() {
        let s = id(1);
        let other = id(2);
        let w: Vec<_> = (0..7).map(|_| obs(s, 0.72, Some((other, 0.40)))).collect();
        match decide(&w, &thresholds()) {
            Decision::Accept {
                student_id,
                agreeing_frames,
                ..
            } => {
                assert_eq!(student_id, s);
                assert_eq!(agreeing_frames, 7);
            }
            other => panic!("expected accept, got {other:?}"),
        }
    }

    #[test]
    fn below_accept_threshold_rejects_when_enough_frames() {
        let s = id(1);
        let w: Vec<_> = (0..7).map(|_| obs(s, 0.30, None)).collect();
        match decide(&w, &thresholds()) {
            Decision::Reject {
                reason: DecisionReason::BelowAcceptThreshold { .. },
            } => {}
            other => panic!("expected reject, got {other:?}"),
        }
    }

    #[test]
    fn below_accept_threshold_retries_when_scarce_frames() {
        let s = id(1);
        let w: Vec<_> = (0..2).map(|_| obs(s, 0.30, None)).collect();
        assert!(matches!(
            decide(&w, &thresholds()),
            Decision::Retry {
                reason: DecisionReason::BelowAcceptThreshold { .. }
            }
        ));
    }

    #[test]
    fn tight_margin_blocks_accept_even_with_high_score() {
        // Top-1 = 0.80, impostor = 0.78 -> margin 0.02 < 0.08.
        let s = id(1);
        let impostor = id(2);
        let w: Vec<_> = (0..7)
            .map(|_| obs(s, 0.80, Some((impostor, 0.78))))
            .collect();
        match decide(&w, &thresholds()) {
            Decision::Retry {
                reason: DecisionReason::InsufficientMargin { .. },
            } => {}
            other => panic!("expected margin retry, got {other:?}"),
        }
    }

    #[test]
    fn consensus_requires_minimum_frames() {
        // Only 3 of 7 frames agree -> below consensus_required (5).
        let s = id(1);
        let other = id(2);
        let mut w: Vec<_> = (0..3).map(|_| obs(s, 0.72, Some((other, 0.30)))).collect();
        // 4 frames that are unusable (e.g. liveness failed).
        w.extend((0..4).map(|_| FrameObservation::unusable()));
        match decide(&w, &thresholds()) {
            Decision::Retry {
                reason: DecisionReason::AwaitingConsensus { usable, needed },
            } => {
                assert_eq!(usable, 3);
                assert_eq!(needed, 5);
            }
            other => panic!("expected awaiting-consensus retry, got {other:?}"),
        }
    }

    #[test]
    fn exactly_consensus_boundary_accepts() {
        let s = id(1);
        let other = id(2);
        let mut w: Vec<_> = (0..5).map(|_| obs(s, 0.72, Some((other, 0.30)))).collect();
        w.extend((0..2).map(|_| FrameObservation::unusable()));
        assert!(matches!(decide(&w, &thresholds()), Decision::Accept { .. }));
    }

    #[test]
    fn liveness_failure_removes_frame_from_consensus() {
        let s = id(1);
        let other = id(2);
        // 7 frames match, but only 2 pass liveness.
        let mut w: Vec<_> = (0..2).map(|_| obs(s, 0.72, Some((other, 0.30)))).collect();
        w.extend((0..5).map(|_| {
            let mut o = obs(s, 0.72, Some((other, 0.30)));
            o.liveness_passed = false;
            o
        }));
        match decide(&w, &thresholds()) {
            Decision::Retry {
                reason: DecisionReason::AwaitingConsensus { usable, .. },
            } => assert_eq!(usable, 2),
            other => panic!("expected retry, got {other:?}"),
        }
    }

    #[test]
    fn all_frames_failing_liveness_reports_liveness_not_no_frames() {
        // Quality and candidates are fine; every frame fails liveness. This is
        // a spoof signal and must be reported as such, not as "no usable frames".
        let s = id(1);
        let w: Vec<_> = (0..7)
            .map(|_| {
                let mut o = obs(s, 0.90, None);
                o.liveness_passed = false;
                o
            })
            .collect();
        match decide(&w, &thresholds()) {
            Decision::Retry {
                reason:
                    DecisionReason::LivenessFailed {
                        failing_frames,
                        required,
                    },
            } => {
                assert_eq!(failing_frames, 7);
                assert_eq!(required, thresholds().t_liveness);
            }
            other => panic!("expected liveness-failed retry, got {other:?}"),
        }
    }

    #[test]
    fn quality_failure_without_liveness_is_still_no_usable_frames() {
        // A frame that fails quality but has a candidate and passed liveness is
        // a bad capture, not a spoof — it must remain NoUsableFrames.
        let s = id(1);
        let w: Vec<_> = (0..7)
            .map(|_| {
                let mut o = obs(s, 0.90, None);
                o.quality_passed = false;
                o
            })
            .collect();
        assert!(matches!(
            decide(&w, &thresholds()),
            Decision::Retry {
                reason: DecisionReason::NoUsableFrames
            }
        ));
    }

    #[test]
    fn split_identity_never_reaches_consensus() {
        // 4 frames say A, 3 say B; neither reaches 5.
        let a = id(1);
        let b = id(2);
        let mut w: Vec<_> = (0..4).map(|_| obs(a, 0.65, Some((b, 0.20)))).collect();
        w.extend((0..3).map(|_| obs(b, 0.65, Some((a, 0.20)))));
        assert!(matches!(decide(&w, &thresholds()), Decision::Retry { .. }));
    }

    #[test]
    fn best_other_ignores_same_student() {
        let s = id(1);
        // Two templates of the same student; best_other must ignore both.
        let o = FrameObservation {
            quality_passed: true,
            liveness_passed: true,
            candidates: vec![
                Candidate {
                    student_id: s,
                    score: 0.9,
                },
                Candidate {
                    student_id: s,
                    score: 0.8,
                },
            ],
        };
        assert_eq!(o.best_other(s), None);
    }

    #[test]
    fn threshold_validation_rejects_bad_config() {
        let t = Thresholds {
            consensus_required: 8, // > window(7)
            ..Thresholds::default()
        };
        assert!(t.validate().is_err());

        let t2 = Thresholds {
            t_accept: 1.5,
            ..Thresholds::default()
        };
        assert!(t2.validate().is_err());

        assert!(Thresholds::default().validate().is_ok());
    }

    #[test]
    fn unknown_person_never_guessed() {
        // No candidates at all (empty gallery match) -> never accept.
        let o = FrameObservation {
            quality_passed: true,
            liveness_passed: true,
            candidates: vec![],
        };
        let w = vec![o; 7];
        assert!(!matches!(
            decide(&w, &thresholds()),
            Decision::Accept { .. }
        ));
    }
}
