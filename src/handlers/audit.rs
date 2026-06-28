//! Audit log read endpoint.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::ApiResult;
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    200
}

#[derive(Debug, Serialize)]
pub struct AuditResponse {
    pub entries: Vec<crate::audit::AuditEntry>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> ApiResult<Json<AuditResponse>> {
    let limit = q.limit.min(2000);
    let entries = match &state.audit {
        Some(log) => log.read_recent(limit).unwrap_or_default(),
        None => vec![],
    };
    Ok(Json(AuditResponse { entries }))
}
