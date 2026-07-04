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

// ── jail.conf parsing ─────────────────────────────────────────────

/// A jail defined in /etc/jail.conf (may or may not be running).
#[derive(Debug, Serialize)]
pub struct JailConfEntry {
    pub name: String,
    pub running: bool,
    pub path: String,
    pub hostname: String,
    pub interface: String,
    pub ip4: String,
    pub ip4_addr: String,
    /// All raw parameters from the jail block (merged with globals).
    pub params: HashMap<String, String>,
}

/// Parse `/etc/jail.conf` into a list of jail entries with variable
/// substitution applied.
fn parse_jail_conf() -> ApiResult<Vec<JailConfEntry>> {
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

        let path = map.get("path").cloned().unwrap_or_default();
        let hostname = map
            .get("host.hostname")
            .cloned()
            .unwrap_or_else(|| name.clone());
        let interface = map.get("interface").cloned().unwrap_or_default();
        let ip4 = map.get("ip4").cloned().unwrap_or_default();
        let ip4_addr = map.get("ip4.addr").cloned().unwrap_or_default();

        result.push(JailConfEntry {
            name: name.clone(),
            running: false, // filled in by caller
            path,
            hostname,
            interface,
            ip4,
            ip4_addr,
            params: map,
        });
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

/// List all jails from /etc/jail.conf with running status from libjail.
pub async fn conf_list() -> ApiResult<Json<Vec<JailConfEntry>>> {
    let mut entries = parse_jail_conf()?;

    // Get running jail names from libjail.
    let running: std::collections::HashSet<String> = jail::list_jails()
        .map_err(ApiError::Internal)?
        .iter()
        .filter_map(|p| p.get("name").cloned())
        .collect();

    for entry in &mut entries {
        entry.running = running.contains(&entry.name);
    }

    Ok(Json(entries))
}

/// Get detailed information about a specific jail by name or JID.
pub async fn detail(Path(name): Path<String>) -> ApiResult<Json<JailDetail>> {
    validate_jail_name(&name)?;

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
