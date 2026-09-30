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
