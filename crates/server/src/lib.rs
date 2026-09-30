//! Absensi Wajah server: HTTP API + kiosk WebSocket + face pipeline.

pub mod attendance;
pub mod auth;
pub mod config;
pub mod error;
pub mod gallery;
pub mod rate_limit;
pub mod routes;
pub mod state;

use std::sync::Arc;

pub use config::Config;
pub use state::{AppState, DeterministicEngine, FaceEngine, PipelineEngine};

/// Build the application state, bootstrapping the DB and the gallery cache.
pub async fn build_state(config: Config) -> anyhow::Result<AppState> {
    let config = Arc::new(config);

    let db = db::connect(&config.database_url, config.db_pool_size, &config.timezone).await?;
    db::migrate(&db).await?;
    bootstrap_admin(&db, &config).await?;

    let gallery = Arc::new(gallery::GalleryCache::new());
    gallery.reload(&db).await;

    let face: Arc<dyn FaceEngine> = build_face_engine(&config)?;

    Ok(AppState::new(db, config, face, gallery))
}

/// Construct the face engine. Uses the deterministic one when explicitly
/// requested; otherwise the ONNX pipeline when the `onnx` feature is compiled.
fn build_face_engine(config: &Config) -> anyhow::Result<Arc<dyn FaceEngine>> {
    let thresholds = domain::quality::QualityThresholds::default();

    if config.use_deterministic_pipeline {
        tracing::warn!("using DETERMINISTIC face pipeline — not for production");
        return Ok(Arc::new(DeterministicEngine::new(thresholds)));
    }

    #[cfg(feature = "onnx")]
    {
        let onnx_config = face_core::onnx::OnnxConfig::from_env();
        let pipeline = face_core::onnx::build_pipeline(&onnx_config, thresholds)
            .map_err(|e| anyhow::anyhow!("failed to load ONNX models: {e}"))?;
        tracing::info!("loaded ONNX face pipeline");
        Ok(Arc::new(PipelineEngine::new(pipeline)))
    }

    #[cfg(not(feature = "onnx"))]
    {
        let _ = thresholds;
        anyhow::bail!(
            "server built without the `onnx` feature and USE_DETERMINISTIC_PIPELINE is false; \
             rebuild with --features onnx or set USE_DETERMINISTIC_PIPELINE=true for development"
        )
    }
}

/// Create the bootstrap admin on first run.
async fn bootstrap_admin(db: &db::Db, config: &Config) -> anyhow::Result<()> {
    let count = db::user_repo::count(db).await?;
    if count == 0 {
        let user = db::user_repo::create(
            db,
            &config.bootstrap_admin_user,
            &config.bootstrap_admin_password,
            "admin",
        )
        .await?;
        tracing::warn!(
            username = %user.username,
            "created bootstrap admin — change the password immediately"
        );
    }
    Ok(())
}

/// Assemble the full router from a freshly built state.
pub fn app(state: AppState) -> axum::Router {
    routes::router(state)
}
