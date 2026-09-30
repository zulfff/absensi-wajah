//! Domain error type. Every variant maps to a fail-closed outcome.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("embedding dimension mismatch: expected {expected}, got {got}")]
    DimensionMismatch { expected: usize, got: usize },

    #[error("embedding contains a non-finite value at index {index}")]
    NonFiniteValue { index: usize },

    #[error("embedding vector is empty or all zeros")]
    DegenerateEmbedding,

    #[error("no candidate frames supplied")]
    NoFrames,
}
