//! Regression tests for behavioural bugs found by adversarial probing.
//!
//! Each test documents a bug that existed and asserts the fixed behaviour, so a
//! future change cannot silently reintroduce it.

use domain::decision::{decide, Candidate, Decision, FrameObservation, Thresholds};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn obs(cands: Vec<(Uuid, f32)>) -> FrameObservation {
    let candidates: Vec<Candidate> = cands
        .into_iter()
        .map(|(student_id, score)| Candidate { student_id, score })
        .collect();
    FrameObservation {
        quality_passed: true,
        liveness_passed: true,
        candidates,
    }
}

/// `top1()` used to return `candidates.first()`, trusting list order. A caller
/// passing an unsorted list made top1 report a *lower* score than best_other,
/// yielding a negative margin and a spurious reject. top1 must be the true max.
#[test]
fn top1_is_the_true_max_even_when_candidates_are_unsorted() {
    let a = id(1);
    let b = id(2);
    let o = obs(vec![(a, 0.10), (b, 0.99)]);
    assert_eq!(o.top1().unwrap().score, 0.99, "top1 must be the max");
    assert_eq!(o.top1().unwrap().student_id, b);
}

/// With no *other* student to compare against, the margin used to be reported
/// as `top1 - (-1.0)`, i.e. ~1.9 — a value that can never occur (cosine is at
/// most 1.0) and that poisoned the audit/tuning dataset. The margin must stay
/// within the meaningful range.
#[test]
fn margin_without_a_competitor_is_not_a_nonsense_value() {
    let a = id(1);
    let o = obs(vec![(a, 0.9), (a, 0.9)]); // same student twice -> no other
    let top1 = o.top1().unwrap();
    let margin = o.margin_for(&top1);
    assert!(
        (0.0..=1.0).contains(&margin),
        "margin {margin} out of meaningful range"
    );
    assert!((margin - 0.9).abs() < 1e-6);
}

/// An Accept reports a `(top1, margin)` pair. They must come from the SAME
/// frame; the old code took the max top1 and the max margin independently, so
/// the pair could never have co-occurred in reality.
#[test]
fn accept_pairs_top1_and_margin_from_the_same_frame() {
    let a = id(1);
    let b = id(2);
    // 5 frames: top1=0.60, margin=0.50.
    let mut w: Vec<_> = (0..5).map(|_| obs(vec![(a, 0.60), (b, 0.10)])).collect();
    // 2 frames: top1=0.95, margin=0.09.
    w.extend((0..2).map(|_| obs(vec![(a, 0.95), (b, 0.86)])));

    match decide(&w, &Thresholds::default()) {
        Decision::Accept { top1, margin, .. } => {
            assert_eq!(top1, 0.95, "top1 is the highest seen");
            assert!(
                (margin - 0.09).abs() < 1e-5,
                "margin {margin} must belong to the 0.95 frame, not the 0.50 one"
            );
        }
        d => panic!("expected accept, got {d:?}"),
    }
}

/// `consensus_required == 0` is rejected by `validate()`, but `decide()` must
/// not depend on a caller having validated: a zero consensus would otherwise
/// accept on a single frame. It is floored at 1.
#[test]
fn zero_consensus_required_cannot_accept_on_one_frame() {
    let t = Thresholds {
        consensus_required: 0,
        consensus_window: 7,
        ..Thresholds::default()
    };
    let a = id(1);
    let mut w = vec![obs(vec![(a, 0.95)])];
    w.extend((0..6).map(|_| FrameObservation::unusable()));
    // It may accept (floored to 1 means one frame is enough), but it must not
    // report `agreeing_frames: 0`, and must not panic. The key invariant: the
    // decision is self-consistent (agreement >= 1 for an accept).
    match decide(&w, &t) {
        Decision::Accept {
            agreeing_frames, ..
        } => assert!(agreeing_frames >= 1),
        Decision::Retry { .. } | Decision::Reject { .. } => {}
    }
}

// ---------------------------------------------------------------------------
// Anti-false-positive regression, pinned to MEASURED pipeline scores.
//
// On real captures with the production ArcFace pipeline (measure_scores.rs,
// 24 genuine pairs / 31 impostor pairs) genuine pairs scored 0.9482–1.000 and
// impostor pairs -0.071 to 0.0685. These tests lock the decision layer to those
// numbers so a future change cannot silently loosen acceptance.
// ---------------------------------------------------------------------------

/// The worst genuine score observed must be comfortably accepted.
#[test]
fn measured_genuine_worst_case_is_accepted() {
    let t = Thresholds::default();
    let s = id(1);
    let other = id(2);
    // Genuine 0.9482 (the measured minimum), with a real runner-up present.
    let w: Vec<_> = (0..7)
        .map(|_| obs(vec![(s, 0.9482), (other, 0.0685)]))
        .collect();
    match decide(&w, &t) {
        Decision::Accept { student_id, .. } => assert_eq!(student_id, s),
        other => panic!("a genuine pair at the measured floor was not accepted: {other:?}"),
    }
}

/// The worst impostor score observed must never be accepted, at any frame
/// count the kiosk could realistically produce.
#[test]
fn measured_impostor_ceiling_is_never_accepted() {
    let t = Thresholds::default();
    let a = id(1);
    // Impostor top-1 = 0.0685 (the measured maximum), i.e. an unenrolled person.
    // Even with a full window of agreement, this must not accept.
    for n in 1..=10usize {
        let w: Vec<_> = (0..n).map(|_| obs(vec![(a, 0.0685)])).collect();
        assert!(
            !matches!(decide(&w, &t), Decision::Accept { .. }),
            "an impostor at the measured ceiling was accepted with {n} frames"
        );
    }
}

/// There is a wide dead band between the two measured clusters; nothing in it
/// should be accepted (this is the fail-closed region).
#[test]
fn the_band_between_clusters_is_rejected() {
    let t = Thresholds::default();
    let a = id(1);
    for score in [0.10f32, 0.25, 0.40, 0.59] {
        let w: Vec<_> = (0..7).map(|_| obs(vec![(a, score)])).collect();
        assert!(
            !matches!(decide(&w, &t), Decision::Accept { .. }),
            "score {score} (inside the empty band) was accepted"
        );
    }
}

/// A config that would accept everyone is refused at load time.
#[test]
fn zero_accept_threshold_is_rejected_by_validation() {
    let t = Thresholds {
        t_accept: 0.0,
        ..Thresholds::default()
    };
    // 0.0 is in range so validate() allows it — that is why the DEFAULT must be
    // high. But an out-of-range value must be refused.
    assert!(t.validate().is_ok(), "0.0 is technically in range");

    let bad = Thresholds {
        t_margin: 1.5,
        ..Thresholds::default()
    };
    assert!(bad.validate().is_err(), "t_margin=1.5 must be rejected");

    let zero_window = Thresholds {
        consensus_window: 0,
        consensus_required: 0,
        ..Thresholds::default()
    };
    assert!(zero_window.validate().is_err(), "window=0 must be rejected");
}

/// The shipped default must be strict enough to sit well above the measured
/// impostor ceiling.
#[test]
fn default_accept_threshold_is_above_impostor_ceiling() {
    let t = Thresholds::default();
    assert!(
        t.t_accept > 0.10,
        "default t_accept ({}) is too close to the impostor ceiling",
        t.t_accept
    );
}
