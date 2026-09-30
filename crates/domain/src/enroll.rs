//! Enrolment validation (Section 4 of the plan).
//!
//! Enrolment is where a bad gallery is born, and a bad gallery cannot be fixed
//! at match time. Three checks protect it:
//!
//! 1. **Internal consistency** — the frames a student submits must agree with
//!    each other. Outlier frames (a different person walked past, or one bad
//!    capture) are dropped, not averaged in.
//! 2. **Anti-duplicate** — the new student must not look like an already
//!    enrolled student. This catches the same person enrolled twice, or the
//!    admin selecting the wrong student record.
//! 3. **Separation margin** — even if not a duplicate, the student must be
//!    *distinguishable* from their nearest neighbour in the gallery, or they
//!    will be confused at match time. Flag for review.
//!
//! All pure logic; no images, no I/O.

use crate::embedding::{centroid, cosine_similarity, Embedding};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tunables for enrolment validation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnrollThresholds {
    /// Minimum cosine between a frame and the student's centroid to keep it.
    pub min_internal_similarity: f32,
    /// Two templates from *different* students must be below this to be
    /// considered distinct identities. Above it -> blocked as a duplicate.
    pub duplicate_similarity: f32,
    /// Minimum separation (1 - cosine, or the distance in similarity space)
    /// between this student and their nearest neighbour. Below -> risky.
    pub min_separation: f32,
    /// Minimum number of frames that must survive outlier removal.
    pub min_frames: usize,
}

impl Default for EnrollThresholds {
    fn default() -> Self {
        // Calibrated against the production pipeline (see crates/face-core
        // examples/measure_scores.rs): same-person captures cluster at >= 0.98,
        // different people at <= 0.07. These values therefore sit in the wide,
        // empty band between the two clusters, not near either one.
        //
        // - min_internal_similarity 0.60: a frame must agree strongly with the
        //   session medoid to be kept; a stray second face (a passer-by) scores
        //   ~0 and is dropped, while all genuine captures stay.
        // - duplicate_similarity 0.60: another student whose centroid is already
        //   this close to the new one is almost certainly the same person
        //   (genuine pairs start at 0.98), so block before the gallery is
        //   polluted.
        // - min_separation 0.30 -> max_allowed 0.70: keep a wide gap from the
        //   nearest neighbour so a look-alike cannot be confused later.
        Self {
            min_internal_similarity: 0.60,
            duplicate_similarity: 0.60,
            min_separation: 0.30,
            min_frames: 5,
        }
    }
}

/// A named reference template already in the gallery (for the duplicate check).
#[derive(Clone, Debug)]
pub struct GalleryEntry {
    pub student_id: Uuid,
    pub embedding: Embedding,
}

/// The verdict of enrolling a set of frames for one student.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EnrollVerdict {
    /// All checks passed. `kept` are the frames to store.
    Accepted {
        kept_frames: usize,
        /// The new student's similarity to their nearest neighbour (lower = safer).
        nearest_neighbour_similarity: f32,
        nearest_neighbour_id: Option<Uuid>,
    },
    /// Some frames dropped, but enough remained and all checks passed.
    AcceptedWithOutliersDropped {
        kept_frames: usize,
        dropped_frames: usize,
        nearest_neighbour_similarity: f32,
        nearest_neighbour_id: Option<Uuid>,
    },
    /// Blocked: looks like an already-enrolled student.
    Duplicate {
        conflicts_with: Uuid,
        similarity: f32,
        threshold: f32,
    },
    /// Passed the duplicate check but too close to a neighbour to be safe.
    RiskySeparation {
        nearest_neighbour_id: Uuid,
        similarity: f32,
        max_allowed: f32,
    },
    /// Not enough usable frames after outlier removal.
    InsufficientFrames { kept: usize, needed: usize },
    /// No frames at all.
    NoFrames,
}

/// Find the medoid: the frame with the greatest agreement (summed cosine) to
/// all other frames.
///
/// This is the robust reference an outlier filter must measure against. A
/// centroid is *not* robust: if the input is split between two people, the
/// centroid falls between them and every frame looks "consistent" — the exact
/// failure this filter exists to prevent. The medoid always sits inside the
/// largest cluster, so frames from a different person score low against it.
fn medoid(frames: &[Embedding]) -> Option<usize> {
    if frames.is_empty() {
        return None;
    }
    let mut best: Option<(usize, f32)> = None;
    for (i, a) in frames.iter().enumerate() {
        let mut agreement = 0.0f32;
        for (j, b) in frames.iter().enumerate() {
            if i == j {
                continue;
            }
            let sim = cosine_similarity(a, b);
            // Only genuine agreement counts; unrelated frames must not add
            // positive support to a candidate medoid via noise.
            if sim > 0.0 {
                agreement += sim;
            }
        }
        if best.map(|(_, s)| agreement > s).unwrap_or(true) {
            best = Some((i, agreement));
        }
    }
    best.map(|(i, _)| i)
}

/// Which frames are outliers relative to the rest.
///
/// Strategy: take the medoid as the reference (see [`medoid`]) and keep only
/// frames whose similarity to it is at least `min_internal_similarity`. Unlike
/// a centroid, the medoid does not drift toward a second person, so a
/// two-person (bimodal) input has its minority cluster dropped instead of
/// silently merged into one enrolment.
///
/// One pass is enough for the small N (10-15) of an enrolment session.
pub fn filter_outliers(frames: &[Embedding], min_similarity: f32) -> Vec<usize> {
    if frames.is_empty() {
        return Vec::new();
    }
    let Some(medoid_index) = medoid(frames) else {
        return (0..frames.len()).collect();
    };
    let reference = &frames[medoid_index];
    frames
        .iter()
        .enumerate()
        .filter(|(_, f)| cosine_similarity(f, reference) >= min_similarity)
        .map(|(i, _)| i)
        .collect()
}

/// Full enrolment evaluation.
///
/// `frames` = embeddings from the enrolment captures.
/// `gallery` = all other students' active templates.
pub fn evaluate_enrollment(
    frames: &[Embedding],
    gallery: &[GalleryEntry],
    t: &EnrollThresholds,
) -> EnrollVerdict {
    if frames.is_empty() {
        return EnrollVerdict::NoFrames;
    }

    let kept_indices = filter_outliers(frames, t.min_internal_similarity);
    let dropped = frames.len() - kept_indices.len();
    if kept_indices.len() < t.min_frames {
        return EnrollVerdict::InsufficientFrames {
            kept: kept_indices.len(),
            needed: t.min_frames,
        };
    }

    // Build the candidate centroid from the kept frames.
    let kept: Vec<Embedding> = kept_indices.iter().map(|i| frames[*i].clone()).collect();
    let Ok(candidate) = centroid(&kept) else {
        return EnrollVerdict::NoFrames;
    };

    // Find the nearest neighbour among other students.
    let mut nearest: Option<(Uuid, f32)> = None;
    for entry in gallery {
        let sim = cosine_similarity(&candidate, &entry.embedding);
        if nearest.map(|(_, s)| sim > s).unwrap_or(true) {
            nearest = Some((entry.student_id, sim));
        }
    }

    if let Some((id, sim)) = nearest {
        if sim >= t.duplicate_similarity {
            return EnrollVerdict::Duplicate {
                conflicts_with: id,
                similarity: sim,
                threshold: t.duplicate_similarity,
            };
        }
        let max_allowed = 1.0 - t.min_separation;
        if sim > max_allowed {
            return EnrollVerdict::RiskySeparation {
                nearest_neighbour_id: id,
                similarity: sim,
                max_allowed,
            };
        }
        let (sim_out, id_out) = (sim, Some(id));
        if dropped > 0 {
            EnrollVerdict::AcceptedWithOutliersDropped {
                kept_frames: kept.len(),
                dropped_frames: dropped,
                nearest_neighbour_similarity: sim_out,
                nearest_neighbour_id: id_out,
            }
        } else {
            EnrollVerdict::Accepted {
                kept_frames: kept.len(),
                nearest_neighbour_similarity: sim_out,
                nearest_neighbour_id: id_out,
            }
        }
    } else if dropped > 0 {
        EnrollVerdict::AcceptedWithOutliersDropped {
            kept_frames: kept.len(),
            dropped_frames: dropped,
            nearest_neighbour_similarity: 0.0,
            nearest_neighbour_id: None,
        }
    } else {
        EnrollVerdict::Accepted {
            kept_frames: kept.len(),
            nearest_neighbour_similarity: 0.0,
            nearest_neighbour_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::EMBEDDING_DIM;

    /// A deterministic unit vector. Vectors with the same `cluster` share a
    /// direction (high cosine) but differ slightly with `noise`; vectors in
    /// different clusters are near-orthogonal (low cosine). This models
    /// "same person, different capture" vs "different person".
    fn vec_for(cluster: usize, noise: f32) -> Embedding {
        let mut v = vec![0.0f32; EMBEDDING_DIM];
        // Give each cluster a distinct set of active basis dimensions.
        let base = (cluster % 32) * 16;
        for i in 0..16 {
            v[(base + i) % EMBEDDING_DIM] = 1.0;
        }
        // Add a small per-capture perturbation across all dims.
        for (i, slot) in v.iter_mut().enumerate() {
            *slot += ((i as f32) * 0.7 + noise).sin() * 0.05;
        }
        Embedding::new_normalized(v).unwrap()
    }

    fn enroll_defaults() -> EnrollThresholds {
        EnrollThresholds::default()
    }

    #[test]
    fn no_frames_is_rejected() {
        assert_eq!(
            evaluate_enrollment(&[], &[], &enroll_defaults()),
            EnrollVerdict::NoFrames
        );
    }

    #[test]
    fn too_few_frames_is_rejected() {
        let frames = vec![vec_for(0, 0.0), vec_for(0, 0.01)];
        assert!(matches!(
            evaluate_enrollment(&frames, &[], &enroll_defaults()),
            EnrollVerdict::InsufficientFrames { .. }
        ));
    }

    #[test]
    fn consistent_frames_accepted() {
        let frames: Vec<_> = (0..10).map(|i| vec_for(0, i as f32 * 0.001)).collect();
        assert!(matches!(
            evaluate_enrollment(&frames, &[], &enroll_defaults()),
            EnrollVerdict::Accepted { .. }
        ));
    }

    #[test]
    fn duplicate_blocked_when_matches_gallery() {
        let base = vec_for(0, 0.0);
        let frames: Vec<_> = (0..10).map(|i| vec_for(0, i as f32 * 0.001)).collect();
        let gallery = vec![GalleryEntry {
            student_id: Uuid::from_u128(99),
            embedding: base,
        }];
        match evaluate_enrollment(&frames, &gallery, &enroll_defaults()) {
            EnrollVerdict::Duplicate { conflicts_with, .. } => {
                assert_eq!(conflicts_with, Uuid::from_u128(99));
            }
            other => panic!("expected duplicate, got {other:?}"),
        }
    }

    #[test]
    fn outlier_is_dropped() {
        // 9 captures of cluster 0 plus one capture of a different cluster.
        let mut frames: Vec<_> = (0..9).map(|i| vec_for(0, i as f32 * 0.001)).collect();
        frames.push(vec_for(20, 0.0));
        match evaluate_enrollment(&frames, &[], &enroll_defaults()) {
            EnrollVerdict::AcceptedWithOutliersDropped { dropped_frames, .. } => {
                assert_eq!(dropped_frames, 1);
            }
            other => panic!("expected outliers dropped, got {other:?}"),
        }
    }

    #[test]
    fn too_many_outliers_leads_to_insufficient() {
        // Only 2 consistent frames (cluster 0), 8 from other clusters.
        let mut frames: Vec<_> = (0..2).map(|i| vec_for(0, i as f32 * 0.001)).collect();
        frames.extend((0..8).map(|i| vec_for(20 + i, 0.0)));
        assert!(matches!(
            evaluate_enrollment(&frames, &[], &enroll_defaults()),
            EnrollVerdict::InsufficientFrames { .. }
        ));
    }

    #[test]
    fn filter_outliers_keeps_majority() {
        let mut frames: Vec<_> = (0..8).map(|i| vec_for(0, i as f32 * 0.001)).collect();
        frames.push(vec_for(20, 0.0));
        let kept = filter_outliers(&frames, 0.45);
        assert_eq!(kept.len(), 8);
        assert!(!kept.contains(&8));
    }

    #[test]
    fn bimodal_two_people_is_not_merged() {
        // Regression: two different people (near-orthogonal embeddings) split
        // evenly must NOT be accepted as one enrolment. The old centroid-based
        // filter kept both clusters because the centroid sat between them.
        let mut frames: Vec<_> = (0..5).map(|i| vec_for(0, i as f32 * 0.001)).collect();
        frames.extend((0..5).map(|i| vec_for(5, i as f32 * 0.001)));

        let kept = filter_outliers(&frames, 0.45);
        assert!(
            kept.len() < frames.len(),
            "a second person must be excluded, kept all {} frames",
            frames.len()
        );
        // Every kept frame must belong to a single cluster (all low or all high
        // indices here); none from the other cluster may survive.
        let left = kept.iter().filter(|i| **i < 5).count();
        let right = kept.len() - left;
        assert!(
            left == 0 || right == 0,
            "kept frames span both people: {kept:?}"
        );

        // And the full evaluation must not silently accept the merged pair.
        match evaluate_enrollment(&frames, &[], &enroll_defaults()) {
            EnrollVerdict::Accepted { kept_frames, .. }
            | EnrollVerdict::AcceptedWithOutliersDropped { kept_frames, .. } => {
                assert!(
                    kept_frames < frames.len(),
                    "merged two people into an accepted enrolment"
                );
            }
            _ => {}
        }
    }
}
