//! Kiosk HTTP handlers (auth: device token).

use crate::auth::DeviceAuth;
use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::State;
use axum::Json;

pub async fn info(
    State(state): State<AppState>,
    DeviceAuth(device): DeviceAuth,
) -> Result<Json<serde_json::Value>, ApiError> {
    db::device_repo::touch(&state.db, device.id)
        .await
        .map_err(ApiError::Db)?;
    Ok(Json(serde_json::json!({
        "device": device,
        "gallery_students": state.gallery.student_count(),
        "consensus_required": state.config.consensus_required,
        "consensus_window": state.config.consensus_window,
    })))
}
