//! Jail container management — list running jails, view details, and manage
//! base systems.
//!
//! Runtime queries use the libjail C API (`jailparam_*`) via `crate::jail`.
//! Base system management uses a JSON registry file + ZFS/filesystem ops.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::jail;
use crate::state::AppState;

const ZFS: &str = "/sbin/zfs";
const CP: &str = "/bin/cp";

// ── Running jail list / detail ────────────────────────────────────

/// Unified jail info struct. Used in all list and detail responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JailInfo {
    pub name: String,
    /// JID > 0 if running, 0 if stopped.
    #[serde(default)]
    pub jid: i32,
    // ── basic info (always present, for list display) ──
    pub hostname: String,
    pub path: String,
    pub ip4_addr: String,
    pub ip6_addr: String,
    // ── detail-only fields ──
    /// Config params from jail.conf (merged with globals, variable-substituted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<HashMap<String, String>>,
    /// Runtime info from libjail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<JailRuntime>,
}

/// Runtime information from libjail (only for running jails in detail view).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JailRuntime {
    pub jid: i32,
    /// "running" or "dying".
    pub state: String,
    /// All runtime parameters from libjail.
    pub params: HashMap<String, String>,
}

fn split_addrs(val: &str) -> Vec<String> {
    if val.is_empty() {
        return Vec::new();
    }
    val.split(',').map(|s| s.trim().to_string()).collect()
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// "true" = only running, "false" or absent = all (from jail.conf).
    #[serde(default)]
    pub running: Option<String>,
}

/// List jails. By default returns all jails from jail.conf with running
/// status merged from libjail. Pass ?running=true to get only running jails.
pub async fn list(Query(q): Query<ListQuery>) -> ApiResult<Json<Vec<JailInfo>>> {
    let only_running = q.running.as_deref() == Some("true");

    if only_running {
        // Fast path: query libjail directly.
        let jails: Vec<JailInfo> = jail::list_jails()
            .map_err(ApiError::Internal)?
            .iter()
            .filter_map(|p| {
                let jid: i32 = p.get("jid")?.parse().ok()?;
                Some(JailInfo {
                    name: p.get("name")?.clone(),
                    jid,
                    hostname: p.get("host.hostname").cloned().unwrap_or_default(),
                    path: p.get("path").cloned().unwrap_or_default(),
                    ip4_addr: p.get("ip4.addr").cloned().unwrap_or_default(),
                    ip6_addr: p.get("ip6.addr").cloned().unwrap_or_default(),
                    params: None,
                    runtime: None,
                })
            })
            .collect();
        return Ok(Json(jails));
    }

    // Default: all jails from jail.conf + running status from libjail.
    let entries = parse_jail_conf()?;

    let running: HashMap<String, i32> = jail::list_jails()
        .map_err(ApiError::Internal)?
        .iter()
        .filter_map(|p| {
            let name = p.get("name")?;
            let jid: i32 = p.get("jid")?.parse().ok()?;
            Some((name.clone(), jid))
        })
        .collect();

    let result: Vec<JailInfo> = entries
        .into_iter()
        .map(|pj| {
            let jid = running.get(&pj.name).copied().unwrap_or(0);
            JailInfo {
                hostname: pj.params.get("host.hostname").cloned().unwrap_or_else(|| pj.name.clone()),
                path: pj.params.get("path").cloned().unwrap_or_default(),
                ip4_addr: pj.params.get("ip4.addr").cloned().unwrap_or_else(|| pj.params.get("ip4").cloned().unwrap_or_default()),
                ip6_addr: pj.params.get("ip6.addr").cloned().unwrap_or_else(|| pj.params.get("ip6").cloned().unwrap_or_default()),
                name: pj.name,
                jid,
                params: None,
                runtime: None,
            }
        })
        .collect();

    Ok(Json(result))
}

// ── jail.conf parsing ─────────────────────────────────────────────

/// A jail parsed from /etc/jail.conf (internal type, converted to JailInfo).
struct ParsedJail {
    name: String,
    params: HashMap<String, String>,
}

/// Parse `/etc/jail.conf` into a list of jail entries with variable
/// substitution applied.
fn parse_jail_conf() -> ApiResult<Vec<ParsedJail>> {
    let content = fs::read_to_string("/etc/jail.conf")
        .map_err(|_| ApiError::NotFound("/etc/jail.conf not found".into()))?;

    let mut entries = Vec::new();
    let mut global_params: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, Vec<(String, String)>)> = None;
    let mut in_block_comment = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();

        // Block comments.
        if in_block_comment {
            if line.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if line.starts_with("/*") {
            if !line.contains("*/") {
                in_block_comment = true;
            }
            continue;
        }

        // Strip line comments.
        let line = if let Some(pos) = line.find('#') {
            &line[..pos]
        } else {
            line
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Block start: "name {" or "name{".
        if line.ends_with('{') {
            let name = line.trim_end_matches('{').trim();
            // Skip if it looks like a parameter assignment.
            if !name.contains('=') && !name.contains(';') && !name.is_empty() {
                current = Some((name.to_string(), Vec::new()));
            }
            continue;
        }

        // Block end: "}".
        if line.starts_with('}') {
            if let Some((name, params)) = current.take() {
                let mut merged: Vec<(String, String)> = global_params.clone();
                merged.extend(params);
                entries.push((name, merged));
            }
            continue;
        }

        // Parse parameter: "key = value;" or "key += value;" or "key;".
        let param = parse_param(line);
        if let Some((k, v)) = param {
            if let Some((_, ref mut params)) = current {
                params.push((k, v));
            } else {
                global_params.push((k, v));
            }
        }
    }

    // Resolve variable substitution and build entries.
    let mut result = Vec::new();
    for (name, params) in &entries {
        let mut map: HashMap<String, String> = params.iter().cloned().collect();
        substitute_vars(&mut map, name);
        result.push(ParsedJail { name: name.clone(), params: map });
    }

    Ok(result)
}

/// Parse a single parameter line into (key, value).
fn parse_param(line: &str) -> Option<(String, String)> {
    let line = line.trim_end_matches(';').trim();

    if let Some(eq) = line.find('=') {
        let key = line[..eq].trim().trim_end_matches('+').trim();
        let value = line[eq + 1..].trim();
        // Strip surrounding quotes.
        let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            &value[1..value.len() - 1]
        } else {
            value
        };
        Some((key.to_string(), value.to_string()))
    } else {
        // Boolean parameter: "key" → (key, "true").
        Some((line.to_string(), "true".to_string()))
    }
}

/// Replace `${name}`, `${path}`, `${host.hostname}` in parameter values.
fn substitute_vars(map: &mut HashMap<String, String>, name: &str) {
    let path = map.get("path").cloned().unwrap_or_default();
    let hostname = map
        .get("host.hostname")
        .cloned()
        .unwrap_or_else(|| name.to_string());

    for val in map.values_mut() {
        let mut result = val.clone();
        result = result.replace("${name}", name);
        result = result.replace("${path}", &path);
        result = result.replace("${host.hostname}", &hostname);
        // Also handle $name (without braces).
        result = result.replace("$name", name);
        *val = result;
    }
}

/// Get detailed information about a specific jail.
/// Always returns params (from jail.conf); includes runtime if running.
pub async fn detail(Path(name): Path<String>) -> ApiResult<Json<JailInfo>> {
    validate_jail_name(&name)?;

    // Config from jail.conf.
    let entries = parse_jail_conf().unwrap_or_default();
    let pj = entries
        .into_iter()
        .find(|e| e.name == name)
        .ok_or_else(|| ApiError::NotFound(format!("jail \"{name}\" not found")))?;

    // Runtime from libjail (if running).
    let rt_params = jail::get_jail(&name).map_err(ApiError::Internal)?;
    let mut runtime = None;
    let mut jid = 0;

    if let Some(ref params) = rt_params {
        let rt_jid: i32 = params
            .get("jid")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        jid = rt_jid;
        let dying = params.get("dying").map(|v| v == "true").unwrap_or(false);
        runtime = Some(JailRuntime {
            jid: rt_jid,
            state: if dying { "dying".into() } else { "running".into() },
            params: params.clone(),
        });
    }

    Ok(Json(JailInfo {
        hostname: pj.params.get("host.hostname").cloned().unwrap_or_else(|| pj.name.clone()),
        path: pj.params.get("path").cloned().unwrap_or_default(),
        ip4_addr: pj.params.get("ip4.addr").cloned().unwrap_or_else(|| pj.params.get("ip4").cloned().unwrap_or_default()),
        ip6_addr: pj.params.get("ip6.addr").cloned().unwrap_or_else(|| pj.params.get("ip6").cloned().unwrap_or_default()),
        name: pj.name,
        jid,
        params: Some(pj.params),
        runtime,
    }))
}

// ── Base systems ──────────────────────────────────────────────────

/// A registered base system.
///
/// Two types:
/// - ZFS (`type_ = "zfs"`): clones from registered snapshots of a dataset.
/// - SharedFS (`type_ = "sharedfs"`): copies a template skeleton, mounts
///   a shared read-only binaries directory via nullfs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseSystem {
    pub name: String,
    /// "zfs" or "sharedfs".
    #[serde(rename = "type")]
    pub type_: String,
    /// ZFS: dataset name (e.g. "zroot/jails/bases/freebsd-15.1").
    /// SharedFS: template directory path.
    pub source_path: String,
    /// ZFS: snapshots selected at import time (full names like "pool/ds@snap").
    /// SharedFS: empty.
    pub snapshots: Vec<String>,
    /// SharedFS only: path to the shared read-only binaries directory.
    pub sharedfs_path: Option<String>,
    pub created_at: i64,
}

/// Extra info returned in list responses (includes live ZFS snapshot check).
#[derive(Debug, Serialize)]
pub struct BaseSystemInfo {
    #[serde(flatten)]
    pub base: BaseSystem,
}

#[derive(Debug, Deserialize)]
pub struct BaseImportBody {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    /// ZFS: dataset name. SharedFS: template directory path.
    pub source_path: String,
    /// ZFS: list of snapshot full names to register. SharedFS: absent.
    pub snapshots: Option<Vec<String>>,
    /// SharedFS: shared binaries directory path. ZFS: absent.
    pub sharedfs_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImageCreateBody {
    /// "zfs-clone" | "sharedfs"
    pub method: String,
    /// Snapshot full name — required for zfs-clone.
    pub snapshot: Option<String>,
    /// Target ZFS dataset name — required for zfs-clone.
    pub dataset: Option<String>,
    /// Target directory path.
    pub target: String,
}

/// Minimal FreeBSD system structure markers.
const REQUIRED_SYSTEM_DIRS: &[&str] = &["bin", "sbin", "usr/bin", "usr/lib", "etc"];
/// Minimal SharedFS template markers (symlinks to /sharedfs + config dirs).
const REQUIRED_TEMPLATE_DIRS: &[&str] = &["etc", "sharedfs"];
/// Minimal SharedFS binaries markers.
const REQUIRED_SHAREDFS_DIRS: &[&str] = &["bin", "lib", "sbin", "usr/bin"];

#[derive(Debug, Deserialize)]
pub struct NameQuery {
    pub name: String,
}

fn validate_jail_name(name: &str) -> ApiResult<()> {
    if name.is_empty()
        || name.len() > 256
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        return Err(ApiError::BadRequest("invalid name".into()));
    }
    Ok(())
}

/// Validate a source path — either an absolute filesystem path or a ZFS dataset name.
fn validate_source_path(p: &str) -> ApiResult<()> {
    if p.is_empty() || p.contains('\0') || p.contains('\n') {
        return Err(ApiError::BadRequest("invalid path".into()));
    }
    if p.starts_with('/') {
        // Filesystem path — reject shell metacharacters.
        if p.contains('$') || p.contains('`') || p.contains('\\') {
            return Err(ApiError::BadRequest("path contains invalid characters".into()));
        }
    } else {
        // ZFS dataset name — validate format.
        let valid = p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | '@' | ':'));
        if !valid || p.starts_with('.') {
            return Err(ApiError::BadRequest("invalid ZFS dataset name".into()));
        }
    }
    Ok(())
}

/// Validate a target filesystem path — must be absolute.
fn validate_target_path(p: &str) -> ApiResult<()> {
    if p.is_empty() || !p.starts_with('/') {
        return Err(ApiError::BadRequest("target path must be absolute".into()));
    }
    if p.contains('\0') || p.contains('\n') {
        return Err(ApiError::BadRequest("path contains invalid characters".into()));
    }
    Ok(())
}

/// Path to the JSON registry file that stores registered base systems.
fn bases_file(state: &AppState) -> PathBuf {
    state
        .config
        .paths
        .db
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/var/db/fwp"))
        .join("jail-bases.json")
}

fn read_bases(state: &AppState) -> Vec<BaseSystem> {
    match fs::read_to_string(bases_file(state)) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn write_bases(state: &AppState, bases: &[BaseSystem]) -> ApiResult<()> {
    let content =
        serde_json::to_string_pretty(bases).map_err(|e| ApiError::Internal(e.to_string()))?;
    fs::write(bases_file(state), content)?;
    Ok(())
}

/// Check whether a path is a ZFS dataset.
fn is_zfs_dataset(path: &str) -> bool {
    Command::new(ZFS)
        .args(["list", "-H", "-o", "name", path])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Resolve a source path to a filesystem path. If it's a ZFS dataset name,
/// look up its mountpoint.
fn resolve_fs_path(path: &str) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    // Try ZFS mountpoint lookup.
    if let Ok(output) = Command::new(ZFS)
        .args(["list", "-H", "-o", "mountpoint", path])
        .output()
    {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
    }
    path.to_string()
}

/// Generate the fstab file path for a jail image, derived from the target path.
fn image_fstab_path(state: &AppState, target: &str) -> PathBuf {
    let sanitized = target
        .replace('/', "_")
        .trim_start_matches('_')
        .to_string();
    bases_file(state)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/var/db/fwp"))
        .join("jail-fstabs")
        .join(format!("{sanitized}.fstab"))
}

/// List snapshots for a ZFS dataset (full names like "pool/ds@snap").
fn zfs_snapshots(dataset: &str) -> Vec<String> {
    let output = Command::new(ZFS)
        .args(["list", "-t", "snapshot", "-H", "-o", "name", "-d", "1", dataset])
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn run(cmd: &str, args: &[&str]) -> ApiResult<()> {
    let output = Command::new(cmd).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::Command(if stderr.is_empty() {
            format!("{cmd} failed")
        } else {
            stderr
        }));
    }
    Ok(())
}

/// Check that a directory contains the expected subdirectories.
fn check_dirs(path: &str, required: &[&str]) -> bool {
    let base = std::path::Path::new(path);
    required.iter().all(|d| base.join(d).exists())
}

/// Validate a ZFS snapshot by checking its content via mountpoint lookup.
/// Uses `zfs send` is too heavy; instead, verify the snapshot exists.
fn validate_zfs_snapshots(dataset: &str, snaps: &[String]) -> ApiResult<()> {
    let available = zfs_snapshots(dataset);
    for snap in snaps {
        if !available.contains(snap) {
            return Err(ApiError::BadRequest(format!(
                "snapshot \"{snap}\" not found on dataset \"{dataset}\""
            )));
        }
    }
    Ok(())
}

/// List ZFS snapshots for a dataset (for import dialog).
pub async fn zfs_snapshot_list(
    Query(q): Query<NameQuery>,
) -> ApiResult<Json<Vec<String>>> {
    validate_source_path(&q.name)?;
    if !is_zfs_dataset(&q.name) {
        return Err(ApiError::BadRequest(format!(
            "\"{}\" is not a ZFS dataset",
            q.name
        )));
    }
    Ok(Json(zfs_snapshots(&q.name)))
}

/// List all registered base systems.
pub async fn base_list(State(state): State<AppState>) -> ApiResult<Json<Vec<BaseSystemInfo>>> {
    let bases = read_bases(&state);
    let infos = bases
        .into_iter()
        .map(|b| BaseSystemInfo { base: b })
        .collect();
    Ok(Json(infos))
}

/// Register a new base system.
pub async fn base_import(
    State(state): State<AppState>,
    Json(body): Json<BaseImportBody>,
) -> ApiResult<(StatusCode, Json<BaseSystem>)> {
    validate_jail_name(&body.name)?;

    let mut bases = read_bases(&state);
    if bases.iter().any(|b| b.name == body.name) {
        return Err(ApiError::Conflict(format!(
            "base system \"{}\" already exists",
            body.name
        )));
    }

    let base = match body.type_.as_str() {
        "zfs" => {
            validate_source_path(&body.source_path)?;
            if !is_zfs_dataset(&body.source_path) {
                return Err(ApiError::BadRequest(format!(
                    "\"{}\" is not a ZFS dataset",
                    body.source_path
                )));
            }

            let snaps = body.snapshots.as_deref().unwrap_or(&[]);
            if snaps.is_empty() {
                return Err(ApiError::BadRequest(
                    "at least one snapshot must be selected".into(),
                ));
            }
            validate_zfs_snapshots(&body.source_path, snaps)?;

            // Verify the dataset mountpoint has basic FreeBSD structure.
            let mp = resolve_fs_path(&body.source_path);
            if !check_dirs(&mp, REQUIRED_SYSTEM_DIRS) {
                return Err(ApiError::BadRequest(format!(
                    "dataset mountpoint \"{mp}\" does not contain a valid FreeBSD system structure"
                )));
            }

            BaseSystem {
                name: body.name.clone(),
                type_: "zfs".into(),
                source_path: body.source_path.clone(),
                snapshots: snaps.to_vec(),
                sharedfs_path: None,
                created_at: state.now_ts(),
            }
        }

        "sharedfs" => {
            let template = &body.source_path;
            let sharedfs = body.sharedfs_path.as_deref().ok_or_else(|| {
                ApiError::BadRequest("sharedfs_path is required for sharedfs type".into())
            })?;

            validate_target_path(template)?;
            validate_target_path(sharedfs)?;

            if !std::path::Path::new(template).exists() {
                return Err(ApiError::BadRequest(format!(
                    "template path does not exist: {template}"
                )));
            }
            if !std::path::Path::new(sharedfs).exists() {
                return Err(ApiError::BadRequest(format!(
                    "sharedfs path does not exist: {sharedfs}"
                )));
            }

            // Verify structure.
            if !check_dirs(template, REQUIRED_TEMPLATE_DIRS) {
                return Err(ApiError::BadRequest(format!(
                    "template directory \"{template}\" does not have a valid structure (missing etc/ or sharedfs/)"
                )));
            }
            if !check_dirs(sharedfs, REQUIRED_SHAREDFS_DIRS) {
                return Err(ApiError::BadRequest(format!(
                    "sharedfs directory \"{sharedfs}\" does not have a valid FreeBSD binaries structure"
                )));
            }

            BaseSystem {
                name: body.name.clone(),
                type_: "sharedfs".into(),
                source_path: body.source_path.clone(),
                snapshots: Vec::new(),
                sharedfs_path: Some(sharedfs.to_string()),
                created_at: state.now_ts(),
            }
        }

        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown base system type: \"{other}\""
            )));
        }
    };

    bases.push(base.clone());
    write_bases(&state, &bases)?;

    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/jails/bases",
        201,
        Some(format!("imported base system {} ({})", body.name, base.type_)),
    );

    Ok((StatusCode::CREATED, Json(base)))
}

/// Remove a base system registration (does not delete the source files).
pub async fn base_destroy(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    let mut bases = read_bases(&state);
    let before = bases.len();
    bases.retain(|b| b.name != name);
    if bases.len() == before {
        return Err(ApiError::NotFound(format!(
            "base system \"{name}\" not found"
        )));
    }
    write_bases(&state, &bases)?;

    crate::audit::record(
        &state,
        None,
        "DELETE",
        &format!("/api/jails/bases/{name}"),
        200,
        Some(format!("removed base system {name}")),
    );

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct BaseUpdateBody {
    /// New list of allowed snapshots (full names). Only for ZFS type.
    pub snapshots: Vec<String>,
}

/// Update a base system (currently only ZFS snapshots list).
pub async fn base_update(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<BaseUpdateBody>,
) -> ApiResult<Json<BaseSystem>> {
    let mut bases = read_bases(&state);
    let base = bases
        .iter_mut()
        .find(|b| b.name == name)
        .ok_or_else(|| ApiError::NotFound(format!("base system \"{name}\" not found")))?;

    if base.type_ != "zfs" {
        return Err(ApiError::BadRequest(
            "snapshot update is only supported for ZFS base systems".into(),
        ));
    }

    if body.snapshots.is_empty() {
        return Err(ApiError::BadRequest("at least one snapshot is required".into()));
    }

    validate_zfs_snapshots(&base.source_path, &body.snapshots)?;

    base.snapshots = body.snapshots.clone();
    let updated = base.clone();
    write_bases(&state, &bases)?;

    crate::audit::record(
        &state,
        None,
        "PUT",
        &format!("/api/jails/bases/{name}"),
        200,
        Some(format!(
            "updated base system {name} snapshots: [{}]",
            updated.snapshots.join(", ")
        )),
    );

    Ok(Json(updated))
}

/// Create a jail image from a base system.
pub async fn base_create_image(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<ImageCreateBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let bases = read_bases(&state);
    let base = bases
        .iter()
        .find(|b| b.name == name)
        .ok_or_else(|| ApiError::NotFound(format!("base system \"{name}\" not found")))?;

    validate_target_path(&body.target)?;

    match body.method.as_str() {
        "zfs-clone" => {
            if base.type_ != "zfs" {
                return Err(ApiError::BadRequest(
                    "zfs-clone requires a ZFS base system".into(),
                ));
            }
            let snapshot = body
                .snapshot
                .as_deref()
                .ok_or_else(|| ApiError::BadRequest("snapshot is required for zfs-clone".into()))?;
            let dataset = body
                .dataset
                .as_deref()
                .ok_or_else(|| ApiError::BadRequest("dataset is required for zfs-clone".into()))?;

            validate_zfs_name(dataset)?;

            // Snapshot must be in the registered list.
            if !base.snapshots.contains(&snapshot.to_string()) {
                return Err(ApiError::BadRequest(format!(
                    "snapshot \"{snapshot}\" is not registered for base system \"{name}\""
                )));
            }

            // Clone the snapshot to a new dataset.
            run(ZFS, &["clone", snapshot, dataset])?;
            // Set the mountpoint to the target path.
            run(ZFS, &["set", &format!("mountpoint={}", body.target), dataset])?;

            crate::audit::record(
                &state,
                None,
                "POST",
                &format!("/api/jails/bases/{name}/image"),
                201,
                Some(format!(
                    "zfs clone {snapshot} → {dataset} at {}",
                    body.target
                )),
            );
        }

        "sharedfs" => {
            if base.type_ != "sharedfs" {
                return Err(ApiError::BadRequest(
                    "sharedfs requires a SharedFS base system".into(),
                ));
            }
            let sharedfs_path = base.sharedfs_path.as_deref().ok_or_else(|| {
                ApiError::Internal("sharedfs base system missing sharedfs_path".into())
            })?;
            let template = &base.source_path;
            let dst = &body.target;

            // Copy the template skeleton to the target.
            fs::create_dir_all(dst)?;
            let cp_status = Command::new(CP)
                .args(["-R", &format!("{template}/."), dst])
                .output()?;
            if !cp_status.status.success() {
                let stderr = String::from_utf8_lossy(&cp_status.stderr).trim().to_string();
                return Err(ApiError::Command(if stderr.is_empty() {
                    "cp failed".into()
                } else {
                    stderr
                }));
            }

            // Write fstab file for the nullfs ro mount of sharedfs.
            let fstab_path = image_fstab_path(&state, dst);
            if let Some(parent) = fstab_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let fstab_content = format!(
                "{}\t{dst}/sharedfs\tnullfs\tro\t0\t0\n",
                sharedfs_path
            );
            fs::write(&fstab_path, fstab_content)?;

            crate::audit::record(
                &state,
                None,
                "POST",
                &format!("/api/jails/bases/{name}/image"),
                201,
                Some(format!(
                    "sharedfs image at {} (sharedfs: {}, fstab: {})",
                    body.target, sharedfs_path, fstab_path.display()
                )),
            );
        }

        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown method: \"{other}\""
            )));
        }
    }

    // For sharedfs, include the fstab path in the response.
    let fstab_path = if body.method == "sharedfs" {
        Some(image_fstab_path(&state, &body.target).to_string_lossy().into_owned())
    } else {
        None
    };

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "method": body.method,
            "target": body.target,
            "sharedfs_path": base.sharedfs_path,
            "fstab": fstab_path,
        })),
    ))
}

/// Validate a ZFS dataset name (allows '/', '@', '_', '-', '.', ':'  and alphanumerics).
fn validate_zfs_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 256 {
        return Err(ApiError::BadRequest("invalid dataset name".into()));
    }
    let valid = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.' | '@' | ':'));
    if !valid || name.starts_with('.') || name.contains("..") {
        return Err(ApiError::BadRequest("invalid dataset name".into()));
    }
    Ok(())
}

// ── Jail creation ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JailCreateBody {
    pub name: String,
    pub hostname: Option<String>,
    /// "directory" | "base"
    pub location_type: String,
    /// For "directory": the path. For "base": the base system name.
    pub path: Option<String>,
    pub base_name: Option<String>,
    /// ZFS base: snapshot to clone, target dataset, mount point.
    pub snapshot: Option<String>,
    pub target_dataset: Option<String>,
    /// Network.
    pub interface: Option<String>,
    pub ip4: Option<String>,
    pub ip6: Option<String>,
}

const JAIL_CONF: &str = "/etc/jail.conf";

/// Backup jail.conf to /var/db/fwp/backup/ with timestamp.
fn backup_jail_conf(state: &AppState) -> ApiResult<()> {
    let backup_dir = bases_file(state)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/var/db/fwp"))
        .join("backup");
    fs::create_dir_all(&backup_dir)?;
    let ts = state.now_ts();
    let backup_path = backup_dir.join(format!("jail.conf.{ts}"));
    if std::path::Path::new(JAIL_CONF).exists() {
        fs::copy(JAIL_CONF, &backup_path)?;
    }
    Ok(())
}

/// Write a new jail.conf (atomic: write tmp + rename).
fn write_jail_conf_atomic(content: &str) -> ApiResult<()> {
    let tmp = format!("{JAIL_CONF}.fwp.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, JAIL_CONF)?;
    Ok(())
}

/// Generate a jail.conf block for a new jail.
fn generate_jail_block(
    name: &str,
    path: &str,
    hostname: Option<&str>,
    interface: Option<&str>,
    ip4: Option<&str>,
    ip6: Option<&str>,
    fstab_path: Option<&str>,
) -> String {
    let mut lines = vec![format!("{name} {{")];

    // Only emit parameters that differ from the global defaults.
    // Global defaults in this system typically include:
    //   path="/jails/${name}";  host.hostname = "${name}";
    // So we only emit path/hostname if they differ from the pattern.
    let default_path = format!("/jails/{name}");
    if path != default_path {
        lines.push(format!("    path = \"{path}\";"));
    }

    if let Some(hn) = hostname {
        if !hn.is_empty() && hn != name {
            lines.push(format!("    host.hostname = \"{hn}\";"));
        }
    }

    if let Some(iface) = interface {
        if !iface.is_empty() {
            lines.push(format!("    interface = \"{iface}\";"));
        }
    }

    if let Some(ip) = ip4 {
        if !ip.is_empty() {
            if ip == "inherit" {
                lines.push("    ip4 = \"inherit\";".into());
            } else {
                lines.push(format!("    ip4.addr = {ip};"));
            }
        }
    }

    if let Some(ip) = ip6 {
        if !ip.is_empty() {
            if ip == "inherit" {
                lines.push("    ip6 = \"inherit\";".into());
            } else {
                lines.push(format!("    ip6.addr = {ip};"));
            }
        }
    }

    if let Some(fstab) = fstab_path {
        lines.push(format!("    mount.fstab = \"{fstab}\";"));
    }

    lines.push("}".into());
    lines.join("\n")
}

/// Create a new jail: prepare filesystem + write jail.conf entry.
pub async fn jail_create(
    State(state): State<AppState>,
    Json(body): Json<JailCreateBody>,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    validate_jail_name(&body.name)?;

    // Check for duplicate name in jail.conf.
    let existing = parse_jail_conf().unwrap_or_default();
    if existing.iter().any(|e| e.name == body.name) {
        return Err(ApiError::Conflict(format!(
            "jail \"{}\" already exists in jail.conf",
            body.name
        )));
    }

    let mut fstab_path: Option<String> = None;
    let jail_path: String;

    match body.location_type.as_str() {
        "directory" => {
            let p = body
                .path
                .as_deref()
                .ok_or_else(|| ApiError::BadRequest("path is required".into()))?;
            validate_target_path(p)?;
            if !std::path::Path::new(p).exists() {
                fs::create_dir_all(p)?;
            }
            jail_path = p.to_string();
        }

        "base" => {
            let base_name = body.base_name.as_deref().ok_or_else(|| {
                ApiError::BadRequest("base_name is required for base location type".into())
            })?;
            let bases = read_bases(&state);
            let base = bases
                .iter()
                .find(|b| b.name == base_name)
                .ok_or_else(|| {
                    ApiError::NotFound(format!("base system \"{base_name}\" not found"))
                })?;

            match base.type_.as_str() {
                "zfs" => {
                    let snapshot = body.snapshot.as_deref().ok_or_else(|| {
                        ApiError::BadRequest("snapshot is required for ZFS base".into())
                    })?;
                    if !base.snapshots.iter().any(|s| s == snapshot) {
                        return Err(ApiError::BadRequest(format!(
                            "snapshot \"{snapshot}\" is not registered for base \"{base_name}\""
                        )));
                    }

                    // Default dataset: <source_dataset_parent>/<jail_name>
                    let default_ds = {
                        let parent = base
                            .source_path
                            .rfind('/')
                            .map(|i| &base.source_path[..i])
                            .unwrap_or("zroot/jails");
                        format!("{parent}/{}", body.name)
                    };
                    let dataset = body.target_dataset.as_deref().unwrap_or(&default_ds);
                    validate_zfs_name(dataset)?;

                    // Default mount point.
                    let default_mp = format!("/jails/{}", body.name);
                    let mountpoint = body.path.as_deref().unwrap_or(&default_mp);
                    validate_target_path(mountpoint)?;

                    run(ZFS, &["clone", snapshot, dataset])?;
                    run(ZFS, &["set", &format!("mountpoint={mountpoint}"), dataset])?;

                    jail_path = mountpoint.to_string();

                    crate::audit::record(
                        &state, None, "POST", "/api/jails/create", 201,
                        Some(format!("zfs clone {snapshot} → {dataset} at {mountpoint}")),
                    );
                }

                "sharedfs" => {
                    let sharedfs_path = base.sharedfs_path.as_deref().ok_or_else(|| {
                        ApiError::Internal("sharedfs base missing sharedfs_path".into())
                    })?;
                    let template = &base.source_path;

                    let default_target = format!("/jails/{}", body.name);
                    let target = body.path.as_deref().unwrap_or(&default_target);
                    validate_target_path(target)?;

                    // Copy template skeleton.
                    fs::create_dir_all(target)?;
                    let cp_status = Command::new(CP)
                        .args(["-R", &format!("{template}/."), target])
                        .output()?;
                    if !cp_status.status.success() {
                        let stderr =
                            String::from_utf8_lossy(&cp_status.stderr).trim().to_string();
                        return Err(ApiError::Command(if stderr.is_empty() {
                            "cp failed".into()
                        } else {
                            stderr
                        }));
                    }

                    // Write fstab.
                    let fstab = image_fstab_path(&state, target);
                    if let Some(parent) = fstab.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(
                        &fstab,
                        format!("{sharedfs_path}\t{target}/sharedfs\tnullfs\tro\t0\t0\n"),
                    )?;
                    fstab_path = Some(fstab.to_string_lossy().into_owned());

                    jail_path = target.to_string();

                    crate::audit::record(
                        &state, None, "POST", "/api/jails/create", 201,
                        Some(format!("sharedfs image at {target}")),
                    );
                }

                other => {
                    return Err(ApiError::BadRequest(format!(
                        "unknown base type: \"{other}\""
                    )));
                }
            }
        }

        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown location_type: \"{other}\""
            )));
        }
    }

    // Backup and write jail.conf.
    backup_jail_conf(&state)?;

    let conf_content = fs::read_to_string(JAIL_CONF).unwrap_or_default();
    let block = generate_jail_block(
        &body.name,
        &jail_path,
        body.hostname.as_deref(),
        body.interface.as_deref(),
        body.ip4.as_deref(),
        body.ip6.as_deref(),
        fstab_path.as_deref(),
    );

    let new_content = if conf_content.is_empty() {
        block
    } else {
        // Ensure existing content ends with newline before appending.
        let separator = if conf_content.ends_with('\n') { "" } else { "\n" };
        // Add a blank line between existing content and new block.
        format!("{conf_content}{separator}\n{block}\n")
    };

    write_jail_conf_atomic(&new_content)?;

    crate::audit::record(
        &state, None, "POST", "/api/jails/create", 201,
        Some(format!("created jail {} at {}", body.name, jail_path)),
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "name": body.name,
            "path": jail_path,
            "fstab": fstab_path,
        })),
    ))
}

// ── Jail lifecycle control (start/stop/delete) ────────────────────

/// Start a jail.
pub async fn jail_start(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_jail_name(&name)?;
    jail::start_jail(&name).map_err(ApiError::Command)?;
    crate::audit::record(
        &state, None, "POST", &format!("/api/jails/{name}/start"), 200,
        Some(format!("started jail {name}")),
    );
    Ok(Json(serde_json::json!({"name": name, "action": "start"})))
}

/// Stop a jail.
pub async fn jail_stop(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_jail_name(&name)?;
    jail::stop_jail(&name).map_err(ApiError::Command)?;
    crate::audit::record(
        &state, None, "POST", &format!("/api/jails/{name}/stop"), 200,
        Some(format!("stopped jail {name}")),
    );
    Ok(Json(serde_json::json!({"name": name, "action": "stop"})))
}

#[derive(Debug, Deserialize)]
pub struct JailDeleteQuery {
    /// If "true", remove the jail's filesystem (zfs destroy or rm -rf).
    #[serde(default)]
    pub remove_files: String,
}

/// Delete a jail: stop if running, remove from jail.conf, optionally
/// destroy its filesystem.
pub async fn jail_delete(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(q): Query<JailDeleteQuery>,
) -> ApiResult<StatusCode> {
    validate_jail_name(&name)?;

    // Stop if running.
    if jail::is_jail_running(&name) {
        jail::stop_jail(&name).map_err(ApiError::Command)?;
    }

    // Extract jail path and mount.fstab BEFORE modifying jail.conf.
    let conf_content = fs::read_to_string(JAIL_CONF).unwrap_or_default();
    let entries = parse_jail_conf_from_str(&conf_content).unwrap_or_default();
    let jail_entry = entries.iter().find(|e| e.name == name);
    let jail_path = jail_entry.and_then(|e| {
        let p = e.params.get("path")?;
        if p.is_empty() { None } else { Some(p.clone()) }
    });
    // Check mount.fstab in the raw params.
    let jail_fstab = jail_entry.and_then(|e| e.params.get("mount.fstab").cloned());

    // Backup and remove from jail.conf.
    backup_jail_conf(&state)?;
    let new_content = remove_jail_block(&conf_content, &name);
    write_jail_conf_atomic(&new_content)?;

    let remove_files = q.remove_files == "true";
    let mut detail_msg = format!("removed jail {name} from jail.conf");

    if remove_files {
        if let Some(ref path) = jail_path {
            // Determine if the path is its own ZFS dataset or just a directory
            // inside a parent dataset. `zfs list` resolves to the nearest parent
            // dataset, so we MUST verify the mountpoint matches exactly.
            let zfs_info = Command::new(ZFS)
                .args(["list", "-H", "-o", "name,mountpoint", path])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    let line = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() >= 2 {
                        Some((parts[0].to_string(), parts[1].to_string()))
                    } else {
                        None
                    }
                });

            // Only treat as ZFS dataset if the mountpoint matches the path
            // exactly — otherwise it's a subdirectory of a parent dataset
            // and must be removed as a plain directory.
            let is_dedicated_dataset = zfs_info
                .as_ref()
                .map(|(_, mp)| mp == path)
                .unwrap_or(false);

            if is_dedicated_dataset {
                if let Some((dataset, _)) = zfs_info {
                    match run(ZFS, &["destroy", "-r", &dataset]) {
                        Ok(()) => detail_msg.push_str(&format!(", destroyed dataset {dataset}")),
                        Err(e) => tracing::warn!("failed to destroy dataset {dataset}: {e}"),
                    }
                }
            } else if std::path::Path::new(path).exists() {
                // Plain directory (possibly inside a ZFS dataset) — rm -rf.
                match std::fs::remove_dir_all(path) {
                    Ok(()) => detail_msg.push_str(&format!(", removed directory {path}")),
                    Err(e) => tracing::warn!("failed to remove {path}: {e}"),
                }
            }
        }

        // Remove fstab file if referenced.
        if let Some(ref fstab) = jail_fstab {
            let fstab_path = std::path::Path::new(fstab);
            if fstab_path.exists() {
                let _ = std::fs::remove_file(fstab_path);
                detail_msg.push_str(&format!(", removed fstab {fstab}"));
            }
        }
    }

    crate::audit::record(
        &state, None, "DELETE", &format!("/api/jails/{name}"), 200,
        Some(detail_msg),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Parse jail.conf from a string (for backup file parsing).
fn parse_jail_conf_from_str(content: &str) -> ApiResult<Vec<ParsedJail>> {
    let mut entries = Vec::new();
    let mut global_params: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, Vec<(String, String)>)> = None;
    let mut in_block_comment = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if in_block_comment {
            if line.contains("*/") { in_block_comment = false; }
            continue;
        }
        if line.starts_with("/*") {
            if !line.contains("*/") { in_block_comment = true; }
            continue;
        }
        let line = if let Some(pos) = line.find('#') { &line[..pos] } else { line };
        let line = line.trim();
        if line.is_empty() { continue; }
        if line.ends_with('{') {
            let name = line.trim_end_matches('{').trim();
            if !name.contains('=') && !name.contains(';') && !name.is_empty() {
                current = Some((name.to_string(), Vec::new()));
            }
            continue;
        }
        if line.starts_with('}') {
            if let Some((name, params)) = current.take() {
                let mut merged: Vec<(String, String)> = global_params.clone();
                merged.extend(params);
                entries.push((name, merged));
            }
            continue;
        }
        let param = parse_param(line);
        if let Some((k, v)) = param {
            if let Some((_, ref mut params)) = current {
                params.push((k, v));
            } else {
                global_params.push((k, v));
            }
        }
    }
    let mut result = Vec::new();
    for (name, params) in &entries {
        let mut map: HashMap<String, String> = params.iter().cloned().collect();
        substitute_vars(&mut map, name);
        result.push(ParsedJail {
            name: name.clone(),
            params: map,
        });
    }
    Ok(result)
}

/// Remove a jail block from jail.conf content.
fn remove_jail_block(conf: &str, name: &str) -> String {
    // Find the block boundaries: "name {" ... "}".
    let lines: Vec<&str> = conf.lines().collect();
    let mut result = Vec::new();
    let mut skipping = false;

    for line in &lines {
        let trimmed = line.trim();

        if !skipping {
            // Check if this line starts the target block.
            if trimmed == format!("{name} {{")
                || trimmed == format!("{name}{{")
                || trimmed.starts_with(&format!("{name} {{"))
            {
                // But make sure it's a jail name, not a parameter.
                // A jail block start has the name followed by optional whitespace and '{'.
                let before_brace = trimmed.trim_end_matches('{').trim();
                if before_brace == name {
                    skipping = true;
                    continue;
                }
            }
            result.push(*line);
        } else {
            // Inside the block being removed.
            if trimmed.starts_with('}') {
                skipping = false;
                // Skip the blank line after the block if present.
                continue;
            }
        }
    }

    // Clean up: remove trailing blank lines, ensure single trailing newline.
    let content = result.join("\n");
    let content = content.trim_end_matches('\n');
    if content.is_empty() {
        String::new()
    } else {
        format!("{content}\n")
    }
}
