//! Errors from the face pipeline.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FaceError {
    #[error("image is {width}x{height} but expected {expected} bytes, got {got}")]
    ImageShape {
        width: u32,
        height: u32,
        expected: usize,
        got: usize,
    },

    #[error("failed to decode image: {0}")]
    Decode(#[from] image::ImageError),

    #[error("detector found no faces")]
    NoFace,

    #[error("detector found {0} faces, expected exactly one")]
    MultipleFaces(usize),

    #[error("face bounding box is out of image bounds")]
    BoxOutOfBounds,

    #[error("ONNX runtime error: {0}")]
    Onnx(String),

    #[error("model output had unexpected shape: {0}")]
    OutputShape(String),

    #[error("embedding error: {0}")]
    Domain(#[from] domain::DomainError),
}
