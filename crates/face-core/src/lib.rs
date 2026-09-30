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
    ///
    /// Decoding is **bounded**: a small compressed file can declare enormous
    /// dimensions (a "decompression bomb"), and decoding it would allocate
    /// gigabytes from a few hundred KB of input. The input-length caps elsewhere
    /// bound the compressed size only, so the limits here (max side and max
    /// total allocation) are what stop a crafted frame from exhausting memory.
    pub fn decode(bytes: &[u8]) -> Result<Self, FaceError> {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(image::ImageError::IoError)?;
        reader.limits(decode_limits());
        let img = reader.decode()?;
        let rgb = img.to_rgb8();
        let (width, height) = rgb.dimensions();
        Ok(Self {
            width,
            height,
            pixels: rgb.into_raw(),
        })
    }
}

/// Maximum side length of an accepted frame, in pixels. Well above any real
/// camera frame (640x480, or 4K at 3840) and far below a bomb.
const MAX_FRAME_SIDE: u32 = 8192;

/// Maximum bytes the decoder may allocate for one frame. A 8192x8192 RGB image
/// is ~192 MiB; 256 MiB leaves headroom without allowing a multi-GB allocation.
const MAX_FRAME_ALLOC: u64 = 256 * 1024 * 1024;

fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_FRAME_SIDE);
    limits.max_image_height = Some(MAX_FRAME_SIDE);
    limits.max_alloc = Some(MAX_FRAME_ALLOC);
    limits
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbImage as ImgBuf};

    fn encode_png(w: u32, h: u32) -> Vec<u8> {
        let img = ImgBuf::from_pixel(w, h, image::Rgb([10, 20, 30]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn decodes_a_normal_frame() {
        let png = encode_png(64, 48);
        let img = RgbImage::decode(&png).expect("normal frame decodes");
        assert_eq!(img.width, 64);
        assert_eq!(img.height, 48);
        assert_eq!(img.pixels.len(), 64 * 48 * 3);
    }

    #[test]
    fn rejects_over_side_limit_instead_of_allocating() {
        // 9000 px wide exceeds MAX_FRAME_SIDE (8192). Must error, not allocate
        // a huge buffer.
        let png = encode_png(9000, 1);
        assert!(RgbImage::decode(&png).is_err());
    }

    #[test]
    fn rejects_garbage_bytes() {
        assert!(RgbImage::decode(b"not an image").is_err());
    }
}
