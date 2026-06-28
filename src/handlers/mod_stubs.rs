//! Placeholder routes for FreeBSD management modules.
//!
//! These return a "planned" response so the API surface and frontend
//! navigation are wired up, while the actual functionality is developed in
//! subsequent phases. Each module gets a small set of catch-all routes.

use axum::Json;
use serde::Serialize;

use crate::error::ApiResult;

#[derive(Debug, Serialize)]
pub struct ModuleStatus {
    pub module: &'static str,
    pub status: &'static str,
    pub message: &'static str,
}

macro_rules! status {
    ($name:ident, $module:literal) => {
        pub async fn $name() -> ApiResult<Json<ModuleStatus>> {
            Ok(Json(ModuleStatus {
                module: $module,
                status: "planned",
                message: "This module will be implemented in a later phase.",
            }))
        }
    };
}

status!(sysctl, "sysctl");
status!(rcconf, "rc.conf");
status!(network, "network");
status!(services, "services");
status!(pf, "pf");
status!(jails, "jails");
status!(bhyve, "bhyve");
status!(zfs, "zfs");
