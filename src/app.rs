//! Router assembly.


use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::auth::require_auth;
use crate::handlers;
use crate::state::AppState;

/// Build the complete application router.
pub fn build(state: AppState) -> Router {
    // Public routes: bootstrap check, login, first-run setup.
    let public = Router::new()
        .route("/api/users/bootstrap", get(handlers::users::bootstrap_status))
        .route("/api/users/bootstrap", post(handlers::users::bootstrap))
        .route("/api/auth/login", post(handlers::auth::login));

    // Authenticated routes.
    let api = Router::new()
        .route("/api/auth/logout", post(handlers::auth::logout))
        .route("/api/auth/me", get(handlers::auth::me))
        .route("/api/system/info", get(handlers::system::system_info))
        .route("/api/system/metrics", get(handlers::system::system_metrics))
        .route("/api/users", get(handlers::users::list_users))
        .route("/api/users", post(handlers::users::create_user))
        .route("/api/users/{id}", put(handlers::users::update_user))
        .route("/api/users/{id}", delete(handlers::users::delete_user))
        .route("/api/audit", get(handlers::audit::list))
        // --- module stubs (planned) ---
        .route("/api/sysctl", get(handlers::mod_stubs::sysctl))
        .route("/api/rcconf", get(handlers::mod_stubs::rcconf))
        .route("/api/network", get(handlers::mod_stubs::network))
        .route("/api/services", get(handlers::mod_stubs::services))
        .route("/api/pf", get(handlers::mod_stubs::pf))
        .route("/api/jails", get(handlers::mod_stubs::jails))
        .route("/api/bhyve", get(handlers::mod_stubs::bhyve))
        .route("/api/zfs", get(handlers::mod_stubs::zfs))
        .route("/api/monitor/series", get(crate::monitor::series))
        .route("/api/monitor/latest", get(crate::monitor::latest))
        .layer(from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .merge(public)
        .merge(api)
        .fallback(crate::web_assets::serve_asset)
        .layer(CorsLayer::very_permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
