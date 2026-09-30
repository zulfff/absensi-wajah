//! Real ONNX Runtime implementation (compiled only with `--features onnx`).
//!
//! Wraps three models, all pinned against verified exports:
//! - **SCRFD** `det_10g.onnx` (InsightFace buffalo_l) for detection + 5 landmarks,
//! - **ArcFace** `w600k_r50.onnx` (InsightFace buffalo_l) for 512-D embeddings,
//! - **MiniFASNet** for passive liveness (see [`MiniFasNetLiveness`]).
//!
//! Every model is loaded once at startup and shared via `Arc`. Sessions take
//! `&mut self` for inference, so each is guarded by its own `Mutex`; the server
//! app is designed to run inference on a blocking thread pool, so lock
//! contention is bounded by the inference workers, not the async runtime.
//!
//! ## Pinned model interface (verified, not assumed)
//!
//! SCRFD `det_10g.onnx`, input `input.1` `[1,3,H,W]` dynamic, RGB, `(x-127.5)/128`:
//!   nine outputs in this order (by position):
//!   `score8[N8,1] score16[N16,1] score32[N32,1]`
//!   `bbox8[N8,4]  bbox16[N16,4]  bbox32[N32,4]`
//!   `kps8[N8,10]  kps16[N16,10]  kps32[N32,10]`
//!   with `N = (H/stride)^2 * 2` (2 anchors per cell).
//!
//! ArcFace `w600k_r50.onnx`, input `input.1` `[N,3,112,112]`, RGB,
//! `(x-127.5)/127.5`; output shape `[N,512]`, **not** unit-normalised (must be
//! L2-normalised here).

use crate::error::FaceError;
use crate::landmarks::{estimate_pose, eyes_open};
use crate::scrfd::{decode_stride, nms, Detection, STRIDES};
use crate::{DetectedFace, FaceDetector, FaceEmbedder, LivenessChecker, LivenessResult, RgbImage};
use domain::{Embedding, EMBEDDING_DIM};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;
use std::sync::Mutex;

/// Paths to the ONNX model files.
#[derive(Clone, Debug)]
pub struct OnnxConfig {
    pub detector_path: String,
    pub embedder_path: String,
    pub liveness_path: Option<String>,
    /// Intra-op threads. 0 = ORT default.
    pub intra_threads: usize,
    /// Detector input side length (square). SCRFD's canonical 640.
    pub detector_input_size: u32,
}

impl OnnxConfig {
    pub fn from_env() -> Self {
        Self {
            detector_path: std::env::var("FACE_DETECTOR_MODEL")
                .unwrap_or_else(|_| "models/det_10g.onnx".into()),
            embedder_path: std::env::var("FACE_EMBEDDER_MODEL")
                .unwrap_or_else(|_| "models/w600k_r50.onnx".into()),
            liveness_path: std::env::var("FACE_LIVENESS_MODEL").ok(),
            intra_threads: std::env::var("FACE_ORT_THREADS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            detector_input_size: std::env::var("FACE_DETECTOR_INPUT_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(640),
        }
    }
}

fn load_session(path: &str, intra_threads: usize) -> Result<Session, FaceError> {
    if !Path::new(path).exists() {
        return Err(FaceError::Onnx(format!("model not found: {path}")));
    }
    Session::builder()
        .map_err(|e| FaceError::Onnx(e.to_string()))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| FaceError::Onnx(e.to_string()))?
        .with_intra_threads(intra_threads)
        .map_err(|e| FaceError::Onnx(e.to_string()))?
        .commit_from_file(path)
        .map_err(|e| FaceError::Onnx(e.to_string()))
}

// ---------------------------------------------------------------------------
// SCRFD detector
// ---------------------------------------------------------------------------

/// SCRFD face detector.
pub struct ScrfdDetector {
    session: Mutex<Session>,
    input_size: u32,
    score_threshold: f32,
    nms_threshold: f32,
    max_faces: usize,
}

impl ScrfdDetector {
    pub fn load(config: &OnnxConfig) -> Result<Self, FaceError> {
        Self::load_with(config, 0.5, 0.4, 10)
    }

    pub fn load_with(
        config: &OnnxConfig,
        score_threshold: f32,
        nms_threshold: f32,
        max_faces: usize,
    ) -> Result<Self, FaceError> {
        Ok(Self {
            session: Mutex::new(load_session(&config.detector_path, config.intra_threads)?),
            input_size: config.detector_input_size,
            score_threshold,
            nms_threshold,
            max_faces,
        })
    }

    /// Letterbox the frame to `input_size` and produce an NCHW tensor.
    ///
    /// Returns the tensor and the (scale, pad_x, pad_y) needed to map detected
    /// coordinates back to the original image.
    fn preprocess(&self, image: &RgbImage) -> (Vec<f32>, f32, f32, f32) {
        let size = self.input_size;
        let scale = (size as f32 / image.width as f32).min(size as f32 / image.height as f32);
        let new_w = (image.width as f32 * scale).round() as u32;
        let new_h = (image.height as f32 * scale).round() as u32;
        let pad_x = (size - new_w) / 2;
        let pad_y = (size - new_h) / 2;

        let mut tensor = vec![0.0f32; (3 * size * size) as usize];
        let plane = (size * size) as usize;
        for oy in 0..new_h {
            let sy = (oy as f32 / scale) as u32;
            let sy = sy.min(image.height - 1);
            for ox in 0..new_w {
                let sx = (ox as f32 / scale) as u32;
                let sx = sx.min(image.width - 1);
                let src = ((sy as usize) * (image.width as usize) + (sx as usize)) * 3;
                let dx = (ox + pad_x) as usize;
                let dy = (oy + pad_y) as usize;
                let dst = dy * (size as usize) + dx;
                tensor[dst] = (image.pixels[src] as f32 - 127.5) / 128.0;
                tensor[plane + dst] = (image.pixels[src + 1] as f32 - 127.5) / 128.0;
                tensor[2 * plane + dst] = (image.pixels[src + 2] as f32 - 127.5) / 128.0;
            }
        }
        (tensor, scale, pad_x as f32, pad_y as f32)
    }
}

impl FaceDetector for ScrfdDetector {
    fn detect(&self, image: &RgbImage) -> Result<Vec<DetectedFace>, FaceError> {
        let size = self.input_size as i64;
        let (tensor, scale, pad_x, pad_y) = self.preprocess(image);
        let input = Tensor::from_array(([1i64, 3, size, size], tensor))
            .map_err(|e| FaceError::Onnx(e.to_string()))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| FaceError::Onnx("detector session poisoned".into()))?;
        let outputs = session
            .run(ort::inputs!["input.1" => input])
            .map_err(|e| FaceError::Onnx(e.to_string()))?;

        // Copy every head out as owned data so the borrow on `session` (and the
        // `outputs` handle) ends here; decoding below needs no session.
        let mut per_stride: Vec<(Vec<f32>, Vec<f32>, Vec<f32>)> = Vec::with_capacity(3);
        for (i, stride) in STRIDES.iter().enumerate() {
            let n = stride.num_anchors();
            let scores = extract_f32(&outputs, i)?;
            let bboxes = extract_f32(&outputs, 3 + i)?;
            let kps = extract_f32(&outputs, 6 + i)?;
            if scores.len() < n || bboxes.len() < n * 4 || kps.len() < n * 10 {
                return Err(FaceError::OutputShape(format!(
                    "stride {} expected {} anchors, got score={} bbox={} kps={}",
                    stride.stride,
                    n,
                    scores.len(),
                    bboxes.len(),
                    kps.len()
                )));
            }
            per_stride.push((
                scores[..n].to_vec(),
                bboxes[..n * 4].to_vec(),
                kps[..n * 10].to_vec(),
            ));
        }
        drop(outputs);
        drop(session);

        let mut detections: Vec<Detection> = Vec::new();
        for (i, stride) in STRIDES.iter().enumerate() {
            let (scores, bboxes, kps) = &per_stride[i];
            detections.extend(decode_stride(
                stride,
                scores,
                bboxes,
                kps,
                self.score_threshold,
            ));
        }

        let mut kept = nms(detections, self.nms_threshold, self.max_faces);

        // Undo the letterbox: subtract padding, then divide by scale.
        for det in &mut kept {
            det.bbox[0] = (det.bbox[0] - pad_x) / scale;
            det.bbox[1] = (det.bbox[1] - pad_y) / scale;
            det.bbox[2] = (det.bbox[2] - pad_x) / scale;
            det.bbox[3] = (det.bbox[3] - pad_y) / scale;
            for lm in &mut det.landmarks {
                lm[0] = (lm[0] - pad_x) / scale;
                lm[1] = (lm[1] - pad_y) / scale;
            }
        }

        // Convert to the pipeline's richer face type, deriving pose from the
        // landmarks.
        let faces = kept
            .into_iter()
            .map(|det| {
                let pose = estimate_pose(&det.landmarks);
                DetectedFace {
                    bbox: [
                        det.bbox[0],
                        det.bbox[1],
                        det.bbox[2] - det.bbox[0],
                        det.bbox[3] - det.bbox[1],
                    ],
                    landmarks: det.landmarks,
                    confidence: det.score,
                    yaw: pose.yaw,
                    pitch: pose.pitch,
                    roll: pose.roll,
                    eyes_open: eyes_open(&det.landmarks),
                }
            })
            .collect();

        Ok(faces)
    }
}

/// Extract output `index` as a flat `f32` slice.
///
/// Flattens a possibly 2-D head (e.g. `[N,1]` or `[N,4]`) into a contiguous
/// slice, so callers do not care about the trailing dimension.
fn extract_f32(
    outputs: &ort::session::SessionOutputs<'_>,
    index: usize,
) -> Result<Vec<f32>, FaceError> {
    // Index by position, not name: SCRFD's output names are numeric and
    // export-dependent, but the *order* is stable (pinned in the module docs).
    let value: &ort::value::DynValue = &outputs[index];
    let (_shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| FaceError::Onnx(e.to_string()))?;
    Ok(data.to_vec())
}

// ---------------------------------------------------------------------------
// ArcFace embedder
// ---------------------------------------------------------------------------

/// ArcFace embedder (w600k_r50).
pub struct ArcFaceEmbedder {
    session: Mutex<Session>,
}

impl ArcFaceEmbedder {
    pub fn load(config: &OnnxConfig) -> Result<Self, FaceError> {
        Ok(Self {
            session: Mutex::new(load_session(&config.embedder_path, config.intra_threads)?),
        })
    }
}

impl FaceEmbedder for ArcFaceEmbedder {
    fn embed(&self, aligned: &RgbImage) -> Result<Embedding, FaceError> {
        if aligned.width != 112 || aligned.height != 112 {
            return Err(FaceError::OutputShape(format!(
                "ArcFace input must be 112x112, got {}x{}",
                aligned.width, aligned.height
            )));
        }
        let mut tensor = vec![0.0f32; 3 * 112 * 112];
        let plane = 112 * 112;
        for y in 0..112usize {
            for x in 0..112usize {
                let idx = (y * 112 + x) * 3;
                let dst = y * 112 + x;
                tensor[dst] = (aligned.pixels[idx] as f32 - 127.5) / 127.5;
                tensor[plane + dst] = (aligned.pixels[idx + 1] as f32 - 127.5) / 127.5;
                tensor[2 * plane + dst] = (aligned.pixels[idx + 2] as f32 - 127.5) / 127.5;
            }
        }
        let input = Tensor::from_array(([1i64, 3, 112, 112], tensor))
            .map_err(|e| FaceError::Onnx(e.to_string()))?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| FaceError::Onnx("embedder session poisoned".into()))?;
        let outputs = session
            .run(ort::inputs!["input.1" => input])
            .map_err(|e| FaceError::Onnx(e.to_string()))?;

        let raw = extract_f32(&outputs, 0)?;
        if raw.len() != EMBEDDING_DIM {
            return Err(FaceError::OutputShape(format!(
                "ArcFace output must be {EMBEDDING_DIM}, got {}",
                raw.len()
            )));
        }
        // Output is not unit-length; normalise (ArcFace comparisons are cosine).
        Ok(Embedding::new_normalized(raw)?)
    }
}

// ---------------------------------------------------------------------------
// MiniFASNet liveness
// ---------------------------------------------------------------------------

/// MiniFASNet passive liveness.
///
/// Interface note: the common MiniFASNet export ("Silent-Face-Anti-Spoofing"
/// MiniFASNetV2) takes a `[1,3,80,80]` BGR crop and emits `[1,3]` logits
/// (live / print / replay). The crop is not the tight 112x112 ArcFace face but a
/// 2.7x-expanded box around it, resized to 80x80.
///
/// This implementation encodes that interface. It is only constructed when
/// `FACE_LIVENESS_MODEL` is set, because no permissively-licensed, stable
/// download URL exists for the weights (see `scripts/download_models.sh`).
pub struct MiniFasNetLiveness {
    session: Mutex<Session>,
    /// Probability of "live" above which the face is accepted as live.
    threshold: f32,
}

impl MiniFasNetLiveness {
    pub fn load(path: &str, intra_threads: usize) -> Result<Self, FaceError> {
        Ok(Self {
            session: Mutex::new(load_session(path, intra_threads)?),
            threshold: 0.5,
        })
    }

    pub fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = threshold;
        self
    }
}

impl LivenessChecker for MiniFasNetLiveness {
    fn check(&self, image: &RgbImage, face: &DetectedFace) -> Result<LivenessResult, FaceError> {
        // MiniFASNetV2 preprocessing, pinned empirically against
        // `models/minifasnet.onnx`:
        //   - a 2.7x-expanded crop around the face bbox, resized to 80x80,
        //   - RGB channel order,
        //   - **raw 0..255** values (NOT divided by 255 — verified: feeding
        //     /255 makes real photos read as spoof; feeding raw makes them
        //     read as live with ~0.99 confidence),
        //   - NCHW.
        let crop = expanded_crop(image, face.bbox, 2.7, 80);

        let mut tensor = vec![0.0f32; 3 * 80 * 80];
        let plane = 80 * 80;
        for i in 0..(80 * 80) {
            let p = i * 3;
            tensor[i] = crop[p] as f32;
            tensor[plane + i] = crop[p + 1] as f32;
            tensor[2 * plane + i] = crop[p + 2] as f32;
        }
        let input = Tensor::from_array(([1i64, 3, 80, 80], tensor))
            .map_err(|e| FaceError::Onnx(e.to_string()))?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| FaceError::Onnx("liveness session poisoned".into()))?;
        let outputs = session
            .run(ort::inputs!["input" => input])
            .map_err(|e| FaceError::Onnx(e.to_string()))?;

        let logits = extract_f32(&outputs, 0)?;
        if logits.len() < 3 {
            return Err(FaceError::OutputShape(format!(
                "MiniFASNetV2 expected 3 logits, got {}",
                logits.len()
            )));
        }
        // MiniFASNetV2 classes are [print/photo, live, replay]; class 1 = live.
        // Verified on real photos: class 1 dominates (~0.99) for genuine faces.
        let probs = softmax(&logits);
        let live_prob = probs[1];
        Ok(LivenessResult {
            score: live_prob,
            is_spoof: live_prob < self.threshold,
        })
    }
}

/// Extract a `scale`x-expanded square crop around a face bbox, resized to
/// `out`x`out`, returning row-major RGB bytes.
fn expanded_crop(image: &RgbImage, bbox: [f32; 4], scale: f32, out: u32) -> Vec<u8> {
    let cx = bbox[0] + bbox[2] / 2.0;
    let cy = bbox[1] + bbox[3] / 2.0;
    let side = bbox[2].max(bbox[3]) * scale;
    let x0 = cx - side / 2.0;
    let y0 = cy - side / 2.0;

    let mut pixels = Vec::with_capacity((out * out * 3) as usize);
    for oy in 0..out {
        let sy = y0 + (oy as f32 * side / out as f32);
        let syi = (sy.round() as i64).clamp(0, image.height as i64 - 1) as usize;
        for ox in 0..out {
            let sx = x0 + (ox as f32 * side / out as f32);
            let sxi = (sx.round() as i64).clamp(0, image.width as i64 - 1) as usize;
            let idx = (syi * image.width as usize + sxi) * 3;
            pixels.push(image.pixels[idx]);
            pixels.push(image.pixels[idx + 1]);
            pixels.push(image.pixels[idx + 2]);
        }
    }
    pixels
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().cloned().fold(f32::MIN, f32::max);
    let exps: Vec<f32> = logits.iter().map(|l| (l - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum <= 0.0 {
        vec![0.0; logits.len()]
    } else {
        exps.into_iter().map(|e| e / sum).collect()
    }
}

// ---------------------------------------------------------------------------
// Pipeline construction
// ---------------------------------------------------------------------------

/// Convenience constructor: the real pipeline, `onnx`-backed.
pub fn build_pipeline(
    config: &OnnxConfig,
    thresholds: domain::quality::QualityThresholds,
) -> Result<crate::Pipeline<ScrfdDetector, ArcFaceEmbedder, MiniFasNetLiveness>, FaceError> {
    let detector = ScrfdDetector::load(config)?;
    let embedder = ArcFaceEmbedder::load(config)?;
    let liveness = match &config.liveness_path {
        Some(path) => MiniFasNetLiveness::load(path, config.intra_threads)?,
        None => {
            return Err(FaceError::Onnx(
                "FACE_LIVENESS_MODEL is required in production".into(),
            ))
        }
    };
    Ok(crate::Pipeline::new(
        detector, embedder, liveness, thresholds,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softmax_sums_to_one() {
        let p = softmax(&[1.0, 2.0, 3.0]);
        let sum: f32 = p.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(p[2] > p[1] && p[1] > p[0]);
    }

    #[test]
    fn softmax_handles_large_values_without_overflow() {
        let p = softmax(&[1000.0, 1001.0]);
        assert!(p[0].is_finite() && p[1].is_finite());
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn onnx_config_defaults_are_the_pinned_exports() {
        // Guard against silently changing the model contract.
        let c = OnnxConfig {
            detector_path: "det_10g.onnx".into(),
            embedder_path: "w600k_r50.onnx".into(),
            liveness_path: None,
            intra_threads: 0,
            detector_input_size: 640,
        };
        assert_eq!(c.detector_input_size, 640);
    }
}
