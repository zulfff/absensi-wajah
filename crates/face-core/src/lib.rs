//! Face processing pipeline: detect -> align -> quality -> liveness -> embed.
//!
//! The pipeline is expressed as a set of traits ([`FaceDetector`],
//! [`FaceEmbedder`], [`LivenessChecker`]) with a high-level [`Pipeline`] that
//! composes them. Two families of implementation exist:
//!
//! - **Real** (behind the `onnx` feature): SCRFD detection, ArcFace embedding,
//!   MiniFASNet liveness, all via ONNX Runtime.
//! - **Deterministic** (always available): [`DeterministicPipeline`], used in
//!   tests and in dev so the server runs end-to-end without any model files.
//!
//! Everything above this crate (decision logic, DB, HTTP) depends only on the
//! traits, so swapping the implementation never touches the security rules.

pub mod align;
pub mod error;
pub mod landmarks;
pub mod pipeline;
pub mod quality_metrics;
pub mod scrfd;

#[cfg(feature = "onnx")]
pub mod onnx;

pub use error::FaceError;
pub use pipeline::{
    DetectedFace, FaceDetector, FaceEmbedder, LivenessChecker, LivenessResult, Pipeline,
    PipelineOutput,
};
pub use quality_metrics::{laplacian_variance, mean_luminance, of_region};

/// A decoded RGB image with 8-bit channels, row-major.
#[derive(Clone, Debug)]
pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 3` bytes, RGB order.
    pub pixels: Vec<u8>,
}

impl RgbImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, FaceError> {
        let expected = (width as usize) * (height as usize) * 3;
        if pixels.len() != expected {
            return Err(FaceError::ImageShape {
                width,
                height,
                expected,
                got: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// A solid fill image (test helper).
    pub fn solid(width: u32, height: u32, r: u8, g: u8, b: u8) -> Self {
        let mut pixels = Vec::with_capacity((width * height * 3) as usize);
        for _ in 0..(width * height) {
            pixels.push(r);
            pixels.push(g);
            pixels.push(b);
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Decode from encoded bytes (JPEG/PNG) — used for kiosk frame uploads.
    pub fn decode(bytes: &[u8]) -> Result<Self, FaceError> {
        let img = image::load_from_memory(bytes)?;
        let rgb = img.to_rgb8();
        let (width, height) = rgb.dimensions();
        Ok(Self {
            width,
            height,
            pixels: rgb.into_raw(),
        })
    }
}
