//! rc.conf management — list, set and delete variables via sysrc.
//!
//! All sysrc operations go through the [`crate::sysrc`] module. Reads use
//! `sysrc -e -a` (export format, non-default variables only). Writes use
//! `sysrc KEY=VALUE`; deletes use `sysrc -x KEY`. Inputs are validated before
//! being passed as command arguments (no shell interpolation).

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::sysrc;
use crate::AppState;

/// Validate a rc.conf variable name: must be a shell identifier
/// (`[a-zA-Z_][a-zA-Z0-9_]*`), 1–128 chars.
fn validate_key(key: &str) -> ApiResult<()> {
    if key.is_empty() || key.len() > 128 {
        return Err(ApiError::BadRequest("invalid variable name length".into()));
    }
    let re = Regex::new(r"^[a-zA-Z_][a-zA-Z0-9_]*$").unwrap();
    if !re.is_match(key) {
        return Err(ApiError::BadRequest(
            "variable name must match [a-zA-Z_][a-zA-Z0-9_]*".into(),
        ));
    }
    Ok(())
}

/// Reject values that could corrupt the rc.conf file (newlines / null bytes).
fn validate_value(value: &str) -> ApiResult<()> {
    if value.contains('\0') || value.contains('\n') || value.contains('\r') {
        return Err(ApiError::BadRequest(
            "value must not contain newlines or null bytes".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct RcVar {
    pub key: String,
    pub value: String,
}

/// GET /api/rcconf — list all non-default rc.conf variables (effective values),
/// sorted by key.
pub async fn list() -> ApiResult<Json<Vec<RcVar>>> {
    let map = sysrc::list_all_async().await?;
    let mut vars: Vec<RcVar> = map
        .into_iter()
        .map(|(key, value)| RcVar { key, value })
        .collect();
    vars.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(Json(vars))
}

#[derive(Debug, Deserialize)]
pub struct SetRequest {
    pub key: String,
    pub value: String,
}

/// PUT /api/rcconf — set (create or update) a rc.conf variable via `sysrc`.
pub async fn set(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetRequest>,
) -> ApiResult<(StatusCode, Json<RcVar>)> {
    validate_key(&body.key)?;
    validate_value(&body.value)?;

    sysrc::set_async(&body.key, &body.value).await?;

    // Re-read the effective value so we echo back what sysrc actually stored.
    let stored = sysrc::get_async(&body.key)
        .await
        .unwrap_or_else(|_| body.value.clone());
    let var = RcVar {
        key: body.key.clone(),
        value: stored,
    };

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/rcconf",
        200,
        Some(format!("set rc.conf '{}'", body.key)),
    );

    Ok((StatusCode::OK, Json(var)))
}

#[derive(Debug, Deserialize)]
pub struct KeyQuery {
    pub key: String,
}

/// DELETE /api/rcconf?key=NAME — remove a variable from rc.conf via `sysrc -x`.
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<KeyQuery>,
) -> ApiResult<StatusCode> {
    validate_key(&q.key)?;
    sysrc::delete_async(&q.key).await?;

    audit::record(
        &state,
        Some(&auth.username),
        "DELETE",
        "/api/rcconf",
        200,
        Some(format!("deleted rc.conf '{}'", q.key)),
    );

    Ok(StatusCode::NO_CONTENT)
}
