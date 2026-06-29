//! rc.conf management — list, set and delete variables via sysrc.
//!
//! All operations go through `/usr/sbin/sysrc`. Reads use `sysrc -e -a`
//! (export format, non-default variables only). Writes use `sysrc KEY=VALUE`;
//! deletes use `sysrc -x KEY`. Inputs are validated before being passed as
//! command arguments (no shell interpolation).

use std::process::Command;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::AppState;

const SYSRC: &str = "/usr/sbin/sysrc";

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

/// Run a command and return its stdout, or an ApiError on failure.
fn run(cmd: &str, args: &[&str]) -> ApiResult<String> {
    let output = Command::new(cmd).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::Command(if stderr.is_empty() {
            format!("{cmd} failed")
        } else {
            stderr
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[derive(Debug, Serialize)]
pub struct RcVar {
    pub key: String,
    pub value: String,
}

/// Reverse sysrc's shell-style export escaping (`\"` → `"`, `\\` → `\`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse one line of `sysrc -e` output (`KEY="VALUE"`) into an RcVar.
fn parse_export_line(line: &str) -> Option<RcVar> {
    let eq = line.find('=')?;
    let key = line[..eq].trim().to_string();
    if key.is_empty() {
        return None;
    }
    let raw = &line[eq + 1..];
    let value = if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        unescape(&raw[1..raw.len() - 1])
    } else {
        raw.to_string()
    };
    Some(RcVar { key, value })
}

/// GET /api/rcconf — list all non-default rc.conf variables (effective values),
/// sorted by key.
pub async fn list() -> ApiResult<Json<Vec<RcVar>>> {
    let raw = run(SYSRC, &["-e", "-a"])?;
    let mut vars: Vec<RcVar> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(parse_export_line)
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

    let assignment = format!("{}={}", body.key, body.value);
    run(SYSRC, &[&assignment])?;

    // Re-read the effective value so we echo back what sysrc actually stored.
    let stored = run(SYSRC, &["-n", &body.key])
        .map(|s| s.trim_end().to_string())
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
    run(SYSRC, &["-x", &q.key])?;

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
