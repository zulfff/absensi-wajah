//! Shared application state.

use crate::config::Config;
use crate::gallery::GalleryCache;
use crate::rate_limit::RateLimiter;
use db::Db;
use std::sync::Arc;

/// The face pipeline as a trait object, so the server can run either the real
/// ONNX pipeline or the deterministic one without generics leaking everywhere.
pub trait FaceEngine: Send + Sync {
    /// Process one frame: detect + quality + liveness + embed.
    fn process(
        &self,
        image: &face_core::RgbImage,
    ) -> Result<face_core::PipelineOutput, face_core::FaceError>;

    fn quality_thresholds(&self) -> &domain::quality::QualityThresholds;
}

/// Adapter turning a concrete `Pipeline<D, E, L>` into a [`FaceEngine`].
pub struct PipelineEngine<D, E, L>
where
    D: face_core::FaceDetector,
    E: face_core::FaceEmbedder,
    L: face_core::LivenessChecker,
{
    inner: face_core::Pipeline<D, E, L>,
}

impl<D, E, L> PipelineEngine<D, E, L>
where
    D: face_core::FaceDetector,
    E: face_core::FaceEmbedder,
    L: face_core::LivenessChecker,
{
    pub fn new(inner: face_core::Pipeline<D, E, L>) -> Self {
        Self { inner }
    }
}

impl<D, E, L> FaceEngine for PipelineEngine<D, E, L>
where
    D: face_core::FaceDetector + 'static,
    E: face_core::FaceEmbedder + 'static,
    L: face_core::LivenessChecker + 'static,
{
    fn process(
        &self,
        image: &face_core::RgbImage,
    ) -> Result<face_core::PipelineOutput, face_core::FaceError> {
        self.inner.process(image)
    }

    fn quality_thresholds(&self) -> &domain::quality::QualityThresholds {
        self.inner.quality_thresholds()
    }
}

/// Deterministic engine wrapper (model-free).
pub struct DeterministicEngine {
    inner: face_core::pipeline::DeterministicPipeline,
}

impl DeterministicEngine {
    pub fn new(thresholds: domain::quality::QualityThresholds) -> Self {
        Self {
            inner: face_core::pipeline::DeterministicPipeline::new(thresholds),
        }
    }
}

impl FaceEngine for DeterministicEngine {
    fn process(
        &self,
        image: &face_core::RgbImage,
    ) -> Result<face_core::PipelineOutput, face_core::FaceError> {
        self.inner.process(image)
    }

    fn quality_thresholds(&self) -> &domain::quality::QualityThresholds {
        &DETERMINISTIC_THRESHOLDS
    }
}

/// The deterministic engine ignores quality gating (it has no real detector),
/// so it advertises permissive thresholds. Keep in sync with `new`.
static DETERMINISTIC_THRESHOLDS: domain::quality::QualityThresholds =
    domain::quality::QualityThresholds {
        min_face_px: 8,
        min_laplacian_variance: 0.0,
        min_luminance: 0.0,
        max_luminance: 255.0,
        max_backlight_delta: 255.0,
        max_pose_degrees: 180.0,
    };

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    pub face: Arc<dyn FaceEngine>,
    pub gallery: Arc<GalleryCache>,
    pub login_limiter: Arc<RateLimiter>,
    /// A valid Argon2 PHC hash of a random, never-matching password.
    ///
    /// Login verifies against this when the username is unknown, so the same
    /// Argon2 work runs whether or not the account exists. It MUST be a
    /// parseable hash: a malformed dummy makes `verify_password` return early
    /// (no hashing), reintroducing the username-enumeration timing oracle.
    pub dummy_password_hash: Arc<String>,
}

impl AppState {
    pub fn new(
        db: Db,
        config: Arc<Config>,
        face: Arc<dyn FaceEngine>,
        gallery: Arc<GalleryCache>,
    ) -> Self {
        // Build a real hash at startup from random bytes, so it cannot be
        // mistyped and always parses.
        let mut random = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut random);
        let dummy_password_hash = Arc::new(
            db::user_repo::hash_password(&hex_encode(&random))
                .expect("hashing a random string cannot fail"),
        );
        Self {
            db,
            config,
            face,
            gallery,
            login_limiter: Arc::new(RateLimiter::new(10, 60)),
            dummy_password_hash,
        }
    }
}

/// Lowercase hex encoding, used only to turn random bytes into a password
/// string for the dummy hash. Avoids pulling in a hex crate for one call.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
