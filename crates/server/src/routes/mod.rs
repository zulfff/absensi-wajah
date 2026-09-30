//! Router assembly.

pub mod admin;
pub mod enroll;
pub mod kiosk;
pub mod users;
pub mod ws;

use crate::state::AppState;
use axum::http::{header, HeaderValue, Method};
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

/// Build the full application router.
pub fn router(state: AppState) -> Router {
    // 6 MB request cap: a JPEG frame is well under this; anything larger is a
    // mistake or an attack.
    let body_limit = RequestBodyLimitLayer::new(6 * 1024 * 1024);

    // CORS only for explicitly configured origins (the Vite dev server). The
    // SPA and API share an origin in production, so the default empty list
    // means no cross-origin access — never `Any`, which would let any site
    // call the admin API with a stolen token.
    let allowed = state.config.cors_origins();
    let cors = if allowed.is_empty() {
        CorsLayer::new()
    } else {
        let origins: Vec<HeaderValue> = allowed
            .iter()
            .filter_map(|o| o.parse::<HeaderValue>().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
    };

    let admin = Router::new()
        .route("/api/auth/login", post(admin::login))
        .route("/api/auth/me", get(admin::me))
        .route("/api/health", get(admin::health))
        .route(
            "/api/users",
            get(users::list_users).post(users::create_user),
        )
        .route(
            "/api/users/{id}",
            axum::routing::put(users::update_user).delete(users::delete_user),
        )
        .route(
            "/api/students",
            get(admin::list_students).post(admin::create_student),
        )
        .route(
            "/api/students/{id}",
            get(admin::get_student)
                .put(admin::update_student)
                .delete(admin::delete_student),
        )
        .route("/api/students/{id}/consent", post(admin::set_consent))
        .route(
            "/api/students/{id}/enroll/frame",
            post(enroll::enroll_frame),
        )
        .route(
            "/api/students/{id}/enroll/commit",
            post(enroll::enroll_commit),
        )
        .route("/api/students/{id}/activate", post(enroll::activate))
        .route("/api/students/{id}/deactivate", post(enroll::deactivate))
        .route("/api/students/{id}/face", delete(enroll::delete_face))
        .route(
            "/api/devices",
            get(admin::list_devices).post(admin::create_device),
        )
        .route("/api/devices/{id}/revoke", post(admin::revoke_device))
        .route("/api/devices/{id}", delete(admin::delete_device))
        .route("/api/attendance", get(admin::list_attendance).post(admin::mark_attendance_manual))
        .route(
            "/api/attendance/{id}/correct",
            post(admin::correct_attendance),
        )
        .route("/api/monitoring/summary", get(admin::monitoring_summary))
        .route("/api/gallery/reload", post(admin::reload_gallery));

    Router::new()
        .merge(admin)
        .route("/ws/kiosk", get(ws::kiosk_ws))
        .route("/api/kiosk/info", get(kiosk::info))
        .layer(body_limit)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
