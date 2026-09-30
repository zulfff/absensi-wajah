//! Absensi Wajah server entrypoint.

use absensi_server::{app, build_state, Config};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let config = Config::load();
    let bind = config.bind_addr;

    if config.jwt_secret == "dev-insecure-change-me" {
        tracing::warn!("JWT_SECRET is the insecure default — set a strong secret in production");
    }

    let thresholds = config.thresholds();
    if let Err(e) = thresholds.validate() {
        anyhow::bail!("invalid decision thresholds: {e}");
    }

    let state = build_state(config).await?;
    let router = app(state);

    let listener = TcpListener::bind(bind).await?;
    tracing::info!(%bind, "absensi-server listening");

    // ConnectInfo is needed for the login rate limiter's client IP.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,absensi_server=debug,db=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}
