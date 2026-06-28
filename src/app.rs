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
        // --- ZFS ---
        .route("/api/zfs/pools", get(handlers::zfs::pool_list))
        .route("/api/zfs/pools/{name}", get(handlers::zfs::pool_status))
        .route("/api/zfs/pools/{name}/scrub", post(handlers::zfs::pool_scrub))
        .route("/api/zfs/pools/{name}/scrub/stop", post(handlers::zfs::pool_scrub_stop))
        .route("/api/zfs/datasets", get(handlers::zfs::dataset_list).post(handlers::zfs::dataset_create))
        .route("/api/zfs/dataset/destroy", delete(handlers::zfs::dataset_destroy))
        .route("/api/zfs/dataset/properties", get(handlers::zfs::dataset_properties).put(handlers::zfs::dataset_set))
        .route("/api/zfs/snapshots", get(handlers::zfs::snapshot_list).post(handlers::zfs::snapshot_create))
        .route("/api/zfs/snapshot/destroy", delete(handlers::zfs::snapshot_destroy))
        .route("/api/zfs/snapshot/rollback", post(handlers::zfs::snapshot_rollback))
        .route("/api/zfs/snapshot/clone", post(handlers::zfs::snapshot_clone))
        .route("/api/filesystem/overview", get(handlers::filesystem::overview))
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
