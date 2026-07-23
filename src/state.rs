//! Shared application state, factored out to avoid module cycles.

use std::path::PathBuf;
use std::sync::Arc;

use crate::audit::AuditLog;
use crate::auth::LoginGuard;
use crate::config::Config;
use crate::db::Db;
use crate::handlers::debug::TokioMetricsAccum;

/// Shared state accessible to all handlers.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    pub audit: Option<Arc<AuditLog>>,
    pub web_root: Option<PathBuf>,
    pub tokio_accumulator: Arc<parking_lot::Mutex<TokioMetricsAccum>>,
    pub login_guard: LoginGuard,
}

impl AppState {
    /// Current Unix timestamp (seconds).
    pub fn now_ts(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}
