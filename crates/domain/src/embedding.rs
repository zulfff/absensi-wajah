//! Face embeddings and similarity.
//!
//! An embedding is a 512-dimensional L2-normalised vector produced by ArcFace.
//! Because vectors are normalised, cosine similarity is a plain dot product in
//! `[-1, 1]`, where higher means more similar.

use crate::error::DomainError;
use serde::{Deserialize, Serialize};

/// ArcFace (buffalo_l / w600k_r50) produces 512-D embeddings.
pub const EMBEDDING_DIM: usize = 512;

/// A face embedding. Invariants enforced on construction:
/// - exactly [`EMBEDDING_DIM`] elements,
/// - every element finite,
/// - not the zero vector.
///
/// Invariants are checked rather than assumed because a NaN reaching the
/// comparison loop makes every similarity `NaN`, and `NaN >= threshold` is
/// `false` — which would silently reject everyone (safe) but also hides a real
/// pipeline bug (not safe). Reject early, loudly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Embedding(Vec<f32>);

impl Embedding {
    /// Construct from a raw vector, validating all invariants.
    pub fn new(raw: Vec<f32>) -> Result<Self, DomainError> {
        if raw.len() != EMBEDDING_DIM {
            return Err(DomainError::DimensionMismatch {
                expected: EMBEDDING_DIM,
                got: raw.len(),
            });
        }
        let mut norm_sq = 0.0f64;
        for (index, value) in raw.iter().enumerate() {
            if !value.is_finite() {
                return Err(DomainError::NonFiniteValue { index });
            }
            norm_sq += (*value as f64) * (*value as f64);
        }
        if norm_sq <= f64::EPSILON {
            return Err(DomainError::DegenerateEmbedding);
        }
        Ok(Self(raw))
    }

    /// Construct from a raw vector, L2-normalising it first.
    ///
    /// Use this for model output that is not guaranteed unit-length. The result
    /// is always unit-length so [`cosine_similarity`] reduces to a dot product.
    pub fn new_normalized(raw: Vec<f32>) -> Result<Self, DomainError> {
        if raw.len() != EMBEDDING_DIM {
            return Err(DomainError::DimensionMismatch {
                expected: EMBEDDING_DIM,
                got: raw.len(),
            });
        }
        let mut norm_sq = 0.0f64;
        for (index, value) in raw.iter().enumerate() {
            if !value.is_finite() {
                return Err(DomainError::NonFiniteValue { index });
            }
            norm_sq += (*value as f64) * (*value as f64);
        }
        let norm = norm_sq.sqrt();
        if norm <= f64::EPSILON {
            return Err(DomainError::DegenerateEmbedding);
        }
        let inv = (1.0 / norm) as f32;
        let normalized: Vec<f32> = raw.iter().map(|v| v * inv).collect();
        Self::new(normalized)
    }

    /// Borrow the underlying slice.
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// Convert back to an owned vector.
    pub fn into_vec(self) -> Vec<f32> {
        self.0
    }

    /// L2 norm. Should be ~1.0 for a normalised embedding.
    pub fn norm(&self) -> f32 {
        self.0.iter().map(|v| v * v).sum::<f32>().sqrt()
    }
}

/// Cosine similarity between two embeddings.
///
/// Both are assumed unit-length (guaranteed by construction), so this is a dot
/// product clamped to `[-1, 1]` to absorb floating-point drift.
pub fn cosine_similarity(a: &Embedding, b: &Embedding) -> f32 {
    let dot: f32 = a
        .as_slice()
        .iter()
        .zip(b.as_slice().iter())
        .map(|(x, y)| x * y)
        .sum();
    dot.clamp(-1.0, 1.0)
}

/// Compute the mean embedding of a set (used to build an enrolment centroid).
///
/// Returns [`DomainError::NoFrames`] for an empty input.
pub fn centroid(embeddings: &[Embedding]) -> Result<Embedding, DomainError> {
    if embeddings.is_empty() {
        return Err(DomainError::NoFrames);
    }
    let mut acc = vec![0.0f64; EMBEDDING_DIM];
    for embedding in embeddings {
        for (slot, value) in acc.iter_mut().zip(embedding.as_slice().iter()) {
            *slot += *value as f64;
        }
    }
    let count = embeddings.len() as f64;
    let mean: Vec<f32> = acc.iter().map(|v| (v / count) as f32).collect();
    // A mean of unit vectors is not unit-length; normalise for storage.
    Embedding::new_normalized(mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vector(seed: f32) -> Vec<f32> {
        // Deterministic non-degenerate vector, then normalised.
        let mut v = vec![0.0f32; EMBEDDING_DIM];
        for (i, slot) in v.iter_mut().enumerate() {
            *slot = ((i as f32 + seed) * 0.017).sin();
        }
        v
    }

    #[test]
    fn rejects_wrong_dimension() {
        let err = Embedding::new(vec![0.1; 10]).unwrap_err();
        assert_eq!(
            err,
            DomainError::DimensionMismatch {
                expected: EMBEDDING_DIM,
                got: 10
            }
        );
    }

    #[test]
    fn rejects_non_finite() {
        let mut v = unit_vector(0.0);
        v[3] = f32::NAN;
        let err = Embedding::new(v).unwrap_err();
        assert!(matches!(err, DomainError::NonFiniteValue { index: 3 }));
    }

    #[test]
    fn rejects_zero_vector() {
        let err = Embedding::new(vec![0.0f32; EMBEDDING_DIM]).unwrap_err();
        assert_eq!(err, DomainError::DegenerateEmbedding);
    }

    #[test]
    fn normalisation_yields_unit_norm() {
        let e = Embedding::new_normalized(unit_vector(1.0)).unwrap();
        assert!((e.norm() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn identical_vectors_are_maximally_similar() {
        let a = Embedding::new_normalized(unit_vector(2.0)).unwrap();
        let b = Embedding::new_normalized(unit_vector(2.0)).unwrap();
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn opposite_vectors_are_minimally_similar() {
        let a = Embedding::new_normalized(unit_vector(3.0)).unwrap();
        let negated: Vec<f32> = a.as_slice().iter().map(|v| -v).collect();
        let b = Embedding::new_normalized(negated).unwrap();
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-5);
    }

    #[test]
    fn centroid_of_empty_is_error() {
        assert_eq!(centroid(&[]).unwrap_err(), DomainError::NoFrames);
    }

    #[test]
    fn centroid_of_identical_is_identical() {
        let a = Embedding::new_normalized(unit_vector(4.0)).unwrap();
        let c = centroid(&[a.clone(), a.clone()]).unwrap();
        assert!(cosine_similarity(&a, &c) > 0.999);
    }
}
