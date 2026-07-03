//! Jail container management — list running jails and view details.
//!
//! Uses the libjail C API (`jailparam_*`) via the `crate::jail` module.
//! No subprocess spawning.

use std::collections::HashMap;

use axum::extract::Path;
use axum::Json;
use serde::Serialize;

use crate::error::{ApiError, ApiResult};
use crate::jail;

/// A running jail's essential runtime information (list view).
#[derive(Debug, Serialize)]
pub struct JailInfo {
    pub jid: i32,
    pub name: String,
    pub hostname: String,
    pub path: String,
    pub ip4_addr: Vec<String>,
    pub ip6_addr: Vec<String>,
    /// "running" or "dying".
    pub state: String,
    pub persist: bool,
}

/// Complete jail detail including all parameters (detail view).
#[derive(Debug, Serialize)]
pub struct JailDetail {
    pub jid: i32,
    pub name: String,
    pub hostname: String,
    pub path: String,
    pub ip4_addr: Vec<String>,
    pub ip6_addr: Vec<String>,
    pub state: String,
    pub persist: bool,
    /// All parameters from libjail, keyed by parameter name.
    pub params: HashMap<String, String>,
}

fn split_addrs(val: &str) -> Vec<String> {
    if val.is_empty() {
        return Vec::new();
    }
    val.split(',').map(|s| s.trim().to_string()).collect()
}

/// Build a `JailInfo` from a libjail parameter map.
fn jail_info(p: &HashMap<String, String>) -> Option<JailInfo> {
    let jid: i32 = p.get("jid")?.parse().ok()?;
    let name = p.get("name")?.clone();
    let hostname = p.get("host.hostname").cloned().unwrap_or_default();
    let path = p.get("path").cloned().unwrap_or_default();
    let ip4_addr = split_addrs(p.get("ip4.addr").map(|s| s.as_str()).unwrap_or(""));
    let ip6_addr = split_addrs(p.get("ip6.addr").map(|s| s.as_str()).unwrap_or(""));
    let dying = p.get("dying").map(|v| v == "true").unwrap_or(false);
    let persist = p.get("persist").map(|v| v == "true").unwrap_or(false);
    Some(JailInfo {
        jid,
        name,
        hostname,
        path,
        ip4_addr,
        ip6_addr,
        state: if dying { "dying".to_string() } else { "running".to_string() },
        persist,
    })
}

/// List all running jails (including dying ones).
pub async fn list() -> ApiResult<Json<Vec<JailInfo>>> {
    let jails = jail::list_jails()
        .map_err(ApiError::Internal)?
        .iter()
        .filter_map(|p| jail_info(p))
        .collect();
    Ok(Json(jails))
}

/// Get detailed information about a specific jail by name or JID.
pub async fn detail(Path(name): Path<String>) -> ApiResult<Json<JailDetail>> {
    if name.is_empty()
        || name.len() > 256
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return Err(ApiError::BadRequest("invalid jail name".into()));
    }

    let params = jail::get_jail(&name).map_err(ApiError::Internal)?;
    let params = params.ok_or_else(|| ApiError::NotFound(format!("jail \"{name}\" not found")))?;

    let info = jail_info(&params)
        .ok_or_else(|| ApiError::Internal("failed to parse jail parameters".into()))?;

    Ok(Json(JailDetail {
        jid: info.jid,
        name: info.name,
        hostname: info.hostname,
        path: info.path,
        ip4_addr: info.ip4_addr,
        ip6_addr: info.ip6_addr,
        state: info.state,
        persist: info.persist,
        params,
    }))
}
