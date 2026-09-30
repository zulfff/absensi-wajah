//! The composed face pipeline and its traits.

use crate::{quality_metrics, RgbImage};
use domain::quality::{QualitySignals, QualityThresholds};
use domain::{Embedding, EMBEDDING_DIM};
use serde::{Deserialize, Serialize};

use crate::error::FaceError;

/// A face found in an image, with its bounding box and optional landmarks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectedFace {
    /// Bounding box: x, y, width, height in pixels.
    pub bbox: [f32; 4],
    /// Five landmarks (left eye, right eye, nose, left mouth, right mouth) as
    /// (x, y) pairs — the alignment targets for ArcFace.
    pub landmarks: [[f32; 2]; 5],
    /// Detector confidence 0..1.
    pub confidence: f32,
    /// Head pose estimated from landmarks, in degrees.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// Whether both eyes appear open.
    pub eyes_open: bool,
}

impl DetectedFace {
    /// Short side of the bounding box, in pixels.
    pub fn short_side_px(&self) -> u32 {
        self.bbox[2].min(self.bbox[3]).max(0.0) as u32
    }
}

/// Result of a liveness / anti-spoof check on one aligned face.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LivenessResult {
    /// 0.0 (spoof) .. 1.0 (live).
    pub score: f32,
    /// Whether a passive model or an active challenge flagged this as a spoof.
    pub is_spoof: bool,
}

/// Detects faces in an image.
pub trait FaceDetector: Send + Sync {
    fn detect(&self, image: &RgbImage) -> Result<Vec<DetectedFace>, FaceError>;
}

/// Turns an aligned face crop into a 512-D embedding.
pub trait FaceEmbedder: Send + Sync {
    /// `aligned` is a face cropped and warped to a canonical 112x112 RGB frame.
    fn embed(&self, aligned: &RgbImage) -> Result<Embedding, FaceError>;
}

/// Passive/active anti-spoof check.
///
/// Receives the **original frame** and the detected face, not just the aligned
/// crop: passive liveness models (MiniFASNet) need a *wider* crop than the tight
/// face — the surrounding context is part of what separates a photo held up to
/// the camera from a live person.
pub trait LivenessChecker: Send + Sync {
    fn check(&self, image: &RgbImage, face: &DetectedFace) -> Result<LivenessResult, FaceError>;
}

/// Everything the caller gets back for one frame.
#[derive(Clone, Debug)]
pub struct PipelineOutput {
    /// Quality signals measured on the detected face.
    pub quality: QualitySignals,
    /// Liveness verdict.
    pub liveness: LivenessResult,
    /// The embedding (only present if a single face was found).
    pub embedding: Embedding,
    /// The face itself.
    pub face: DetectedFace,
}

/// Composes detector + embedder + liveness with quality measurement.
pub struct Pipeline<D, E, L> {
    detector: D,
    embedder: E,
    liveness: L,
    quality_thresholds: QualityThresholds,
}

impl<D, E, L> Pipeline<D, E, L>
where
    D: FaceDetector,
    E: FaceEmbedder,
    L: LivenessChecker,
{
    pub fn new(
        detector: D,
        embedder: E,
        liveness: L,
        quality_thresholds: QualityThresholds,
    ) -> Self {
        Self {
            detector,
            embedder,
            liveness,
            quality_thresholds,
        }
    }

    pub fn quality_thresholds(&self) -> &QualityThresholds {
        &self.quality_thresholds
    }

    /// Run the pipeline on a full frame.
    ///
    /// Enforces **exactly one face**: zero or several is an error, because the
    /// downstream decision must never guess which person is present.
    pub fn process(&self, image: &RgbImage) -> Result<PipelineOutput, FaceError> {
        let faces = self.detector.detect(image)?;
        if faces.is_empty() {
            return Err(FaceError::NoFace);
        }
        if faces.len() > 1 {
            return Err(FaceError::MultipleFaces(faces.len()));
        }
        let face = faces.into_iter().next().expect("one face");

        // Align (crop to bbox for the deterministic impl; real impl warps by
        // landmarks). The embedder receives a canonical RGB crop.
        let aligned = align(image, &face)?;

        let (face_grey, gw, gh) = quality_metrics::of_region(
            &image.pixels,
            image.width,
            image.height,
            face.bbox[0].max(0.0) as u32,
            face.bbox[1].max(0.0) as u32,
            face.bbox[2].max(1.0) as u32,
            face.bbox[3].max(1.0) as u32,
        )
        .ok_or(FaceError::BoxOutOfBounds)?;

        let face_luma = quality_metrics::mean_luminance(&face_grey, 1);
        let bg_luma = quality_metrics::mean_luminance(&image.pixels, 3);
        let sharpness = quality_metrics::laplacian_variance(&face_grey, gw, gh);

        let quality = QualitySignals {
            face_count: 1,
            face_px: face.short_side_px(),
            laplacian_variance: sharpness,
            mean_luminance: face_luma,
            background_luminance: bg_luma,
            yaw: face.yaw,
            pitch: face.pitch,
            roll: face.roll,
            eyes_open: face.eyes_open,
        };

        let liveness = self.liveness.check(image, &face)?;
        let embedding = self.embedder.embed(&aligned)?;

        Ok(PipelineOutput {
            quality,
            liveness,
            embedding,
            face,
        })
    }
}

/// Align a detected face to the canonical 112x112 ArcFace input.
///
/// Uses the landmark-based similarity transform ([`crate::align::warp_to_aligned`])
/// so ArcFace sees faces oriented the way it was trained. This is not optional
/// for accuracy: a plain bbox crop measurably shrinks the genuine/impostor
/// margin and raises the false-accept rate.
fn align(image: &RgbImage, face: &DetectedFace) -> Result<RgbImage, FaceError> {
    crate::align::warp_to_aligned(image, &face.landmarks)
}

/// A deterministic, model-free pipeline. Given a fixed image it returns a
/// stable embedding derived from image content, so integration tests and the
/// dev server work without ONNX or model binaries.
///
/// NOT for production recognition — it has no notion of faces. It exists so the
/// HTTP/DB/decision layers can be exercised end-to-end.
pub struct DeterministicPipeline {
    inner: Pipeline<DeterministicDetector, DeterministicEmbedder, DeterministicLiveness>,
}

impl Default for DeterministicPipeline {
    fn default() -> Self {
        Self::new(QualityThresholds::default())
    }
}

impl DeterministicPipeline {
    pub fn new(thresholds: QualityThresholds) -> Self {
        Self {
            inner: Pipeline::new(
                DeterministicDetector,
                DeterministicEmbedder,
                DeterministicLiveness,
                thresholds,
            ),
        }
    }

    pub fn process(&self, image: &RgbImage) -> Result<PipelineOutput, FaceError> {
        self.inner.process(image)
    }
}

/// Treats the whole frame as one face — no real detection.
pub struct DeterministicDetector;

impl FaceDetector for DeterministicDetector {
    fn detect(&self, image: &RgbImage) -> Result<Vec<DetectedFace>, FaceError> {
        // A centred box covering ~70% of the frame.
        let w = image.width as f32 * 0.7;
        let h = image.height as f32 * 0.7;
        let x = (image.width as f32 - w) / 2.0;
        let y = (image.height as f32 - h) / 2.0;
        Ok(vec![DetectedFace {
            bbox: [x, y, w, h],
            landmarks: [
                [x + w * 0.3, y + h * 0.35],
                [x + w * 0.7, y + h * 0.35],
                [x + w * 0.5, y + h * 0.55],
                [x + w * 0.35, y + h * 0.75],
                [x + w * 0.65, y + h * 0.75],
            ],
            confidence: 1.0,
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            eyes_open: true,
        }])
    }
}

/// Derives a stable embedding from the image's *structure*. Two images with
/// similar texture produce similar embeddings; identical images produce
/// identical embeddings; a purely flat colour maps to near-zero variance and
/// is rejected as degenerate (which is honest — a flat frame is not a face).
///
/// It has no notion of faces. It exists so the HTTP/DB/decision layers can be
/// exercised end-to-end without ONNX. It is NOT a substitute for ArcFace.
pub struct DeterministicEmbedder;

impl FaceEmbedder for DeterministicEmbedder {
    fn embed(&self, aligned: &RgbImage) -> Result<Embedding, FaceError> {
        let w = aligned.width as usize;
        let h = aligned.height as usize;
        // Greyscale buffer.
        let grey: Vec<f32> = aligned
            .pixels
            .chunks_exact(3)
            .map(|p| 0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
            .collect();

        // Subtract the global mean so a uniform brightness shift does not
        // dominate; then project horizontal and vertical gradients (structure,
        // not content) into the embedding space via a fixed basis.
        let mean = grey.iter().sum::<f32>() / grey.len().max(1) as f32;
        let mut v = vec![0.0f32; EMBEDDING_DIM];
        let mut idx = 0usize;
        for y in 0..h {
            for x in 0..w {
                let c = grey[idx] - mean;
                let gx = if x + 1 < w {
                    grey[idx + 1] - grey[idx]
                } else {
                    0.0
                };
                let gy = if y + 1 < h {
                    grey[idx + w] - grey[idx]
                } else {
                    0.0
                };
                // Two orthogonal taps per pixel, folded into the vector.
                let t0 = ((y * w + x) * 2) % EMBEDDING_DIM;
                let t1 = ((y * w + x) * 2 + 1) % EMBEDDING_DIM;
                v[t0] += c * 0.5 + gx;
                v[t1] += c * 0.5 + gy;
                idx += 1;
            }
        }
        Ok(Embedding::new_normalized(v)?)
    }
}

/// Always reports "live". The real checker (MiniFASNet + challenge) replaces
/// this behind the `onnx` feature.
pub struct DeterministicLiveness;

impl LivenessChecker for DeterministicLiveness {
    fn check(&self, _image: &RgbImage, _face: &DetectedFace) -> Result<LivenessResult, FaceError> {
        Ok(LivenessResult {
            score: 1.0,
            is_spoof: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A textured image (deterministic pseudo-noise). The deterministic
    /// embedder works on structure, so flat colours are correctly rejected as
    /// degenerate; tests need real texture.
    fn textured(w: u32, h: u32, seed: u32) -> RgbImage {
        let mut pixels = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let n = x
                    .wrapping_mul(2654435761)
                    .wrapping_add(y.wrapping_mul(40503))
                    .wrapping_add(seed.wrapping_mul(374761393));
                let v = (n >> 13) as u8;
                pixels.push(v);
                pixels.push(v.wrapping_add(37));
                pixels.push(v.wrapping_add(91));
            }
        }
        RgbImage::new(w, h, pixels).unwrap()
    }

    #[test]
    fn deterministic_pipeline_produces_consistent_embeddings() {
        let p = DeterministicPipeline::default();
        let img = textured(224, 224, 1);
        let a = p.process(&img).unwrap();
        let b = p.process(&img).unwrap();
        assert!(
            domain::cosine_similarity(&a.embedding, &b.embedding) > 0.999,
            "identical images must embed identically"
        );
    }

    #[test]
    fn different_images_embed_differently() {
        let p = DeterministicPipeline::default();
        let a = p.process(&textured(224, 224, 2)).unwrap();
        let b = p.process(&textured(224, 224, 3)).unwrap();
        assert!(
            domain::cosine_similarity(&a.embedding, &b.embedding) < 0.999,
            "different images must not embed identically"
        );
    }

    #[test]
    fn flat_image_keeps_no_structure() {
        // A flat frame carries no structure. It may embed (float rounding gives
        // a tiny residual direction) or be rejected as degenerate — either is
        // acceptable; what matters is that it must not panic and must not look
        // like a distinctive face. Assert the invariant we actually rely on:
        // two *different* flat colours are near-indistinguishable, so the
        // decision layer's quality gate (not the embedder) is what rejects them.
        let p = DeterministicPipeline::default();
        let a = p.process(&RgbImage::solid(224, 224, 30, 30, 30));
        let b = p.process(&RgbImage::solid(224, 224, 210, 210, 210));
        match (a, b) {
            (Ok(a), Ok(b)) => {
                let sim = domain::cosine_similarity(&a.embedding, &b.embedding);
                assert!(
                    sim > 0.5,
                    "flat frames must not be treated as distinct identities (sim={sim})"
                );
            }
            _ => { /* either rejected: fine */ }
        }
    }

    #[test]
    fn quality_signals_are_populated() {
        let p = DeterministicPipeline::default();
        let out = p.process(&textured(224, 224, 4)).unwrap();
        assert_eq!(out.quality.face_count, 1);
        assert!(out.face.short_side_px() > 100);
    }

    #[test]
    fn tiny_textured_frame_does_not_panic() {
        let p = DeterministicPipeline::default();
        let img = textured(16, 16, 5);
        // Either succeeds or returns an error, but must never panic.
        let _ = p.process(&img);
    }
}
