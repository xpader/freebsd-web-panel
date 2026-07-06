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
const TAR: &str = "/usr/bin/tar";
const FETCH: &str = "/usr/bin/fetch";

/// FreeBSD mirror list for base.txz downloads.
const FREEBSD_MIRRORS: &[(&str, &str)] = &[
    ("Official (download.freebsd.org)", "https://download.freebsd.org"),
    ("China (ftp.cn.freebsd.org)", "https://ftp.cn.freebsd.org"),
    ("Japan (ftp.jp.freebsd.org)", "https://ftp.jp.freebsd.org"),
    ("Taiwan (ftp.tw.freebsd.org)", "https://ftp.tw.freebsd.org"),
    ("Germany (ftp.de.freebsd.org)", "https://ftp.de.freebsd.org"),
    ("USA - NY (ftp.nyi.net)", "https://ftp.nyi.net"),
];

/// Directories that stay in sharedfs as shared read-only (symlinked from template).
const SHAREDFS_SHARED_TOP: &[&str] = &["bin", "lib", "libexec", "sbin"];
const SHAREDFS_SHARED_USR: &[&str] = &[
    "bin", "include", "lib", "lib32", "libdata", "libexec",
    "ports", "sbin", "share", "src",
];
/// Top-level dirs to move from extracted sharedfs into template (per-jail).
const TEMPLATE_REAL_TOP: &[&str] = &["etc", "var", "root", "tmp"];
/// usr/ subdirs to move from extracted sharedfs/usr into template/usr.
const TEMPLATE_REAL_USR: &[&str] = &["local", "obj", "tests"];
/// Empty dirs to create in template (standard FreeBSD layout).
const TEMPLATE_EMPTY_DIRS: &[&str] = &["dev", "media", "mnt", "net", "proc", "sharedfs"];

// ── Running jail list / detail ────────────────────────────────────

/// Read the jail_list from rc.conf (via sysrc) and return as a HashSet of jail names.
fn read_jail_list() -> std::collections::HashSet<String> {
    Command::new("/usr/sbin/sysrc")
        .args(["-n", "jail_list"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().split_whitespace().map(|n| n.to_string()).collect())
        .unwrap_or_default()
}

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
    /// Whether the jail is in rc.conf jail_list (auto-start on boot).
    #[serde(default)]
    pub auto_start: bool,
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
        let auto = read_jail_list();
        let jails: Vec<JailInfo> = jail::list_jails()
            .map_err(ApiError::Internal)?
            .iter()
            .filter_map(|p| {
                let jid: i32 = p.get("jid")?.parse().ok()?;
                let name = p.get("name")?.clone();
                Some(JailInfo {
                    auto_start: auto.contains(&name),
                    name,
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
    let auto = read_jail_list();

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
                auto_start: auto.contains(&pj.name),
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
    let auto = read_jail_list();
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
        auto_start: auto.contains(&name),
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
    /// "import" | "from-txz" | "download". Default: "import".
    #[serde(default)]
    pub method: String,
    #[serde(rename = "type")]
    pub type_: String,

    // ── "import" method fields ──
    /// ZFS: existing dataset name. SharedFS: existing template directory path.
    pub source_path: Option<String>,
    /// ZFS: list of snapshot full names to register.
    pub snapshots: Option<Vec<String>>,
    /// SharedFS: existing shared binaries directory path.
    /// Also reused for "from-txz"/"download" SharedFS: new sharedfs dir to create.
    pub sharedfs_path: Option<String>,

    // ── "from-txz" / "download" common fields ──
    /// "from-txz": path to existing base.txz file on the system.
    pub txz_path: Option<String>,

    // ── "from-txz" / "download" ZFS fields ──
    /// New ZFS dataset to create and extract into.
    pub dataset: Option<String>,
    /// Snapshot name to create after extraction (e.g. "clean").
    pub snapshot_name: Option<String>,

    // ── "from-txz" / "download" SharedFS fields ──
    /// New template directory to create.
    pub template_path: Option<String>,

    // ── "download" fields ──
    /// Full download URL for base.txz (e.g. "https://download.freebsd.org/releases/amd64/14.2-RELEASE/base.txz").
    pub download_url: Option<String>,
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

/// List available FreeBSD mirrors for download.
pub async fn mirror_list() -> ApiResult<Json<Vec<MirrorInfo>>> {
    let mirrors = FREEBSD_MIRRORS
        .iter()
        .map(|(name, url)| MirrorInfo {
            name: name.to_string(),
            url: url.to_string(),
        })
        .collect();
    Ok(Json(mirrors))
}

#[derive(Debug, Serialize)]
pub struct MirrorInfo {
    pub name: String,
    pub url: String,
}

/// Register a new base system — supports three creation methods.
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

    let method = if body.method.is_empty() { "import" } else { body.method.as_str() };
    let base = match method {
        "import" => create_base_import(&state, &body)?,
        "from-txz" => create_base_from_txz(&state, &body, None)?,
        "download" => {
            let txz_path = download_base_txz(&body)?;
            let result = create_base_from_txz(&state, &body, Some(&txz_path));
            // Clean up the downloaded temp file regardless of success/failure.
            let _ = fs::remove_file(&txz_path);
            result?
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown creation method: \"{other}\""
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
        Some(format!("created base system {} ({}, {})", body.name, method, base.type_)),
    );

    Ok((StatusCode::CREATED, Json(base)))
}

/// "import" method: register an existing directory or ZFS dataset.
fn create_base_import(state: &AppState, body: &BaseImportBody) -> ApiResult<BaseSystem> {
    match body.type_.as_str() {
        "zfs" => {
            let source_path = body.source_path.as_deref().ok_or_else(|| {
                ApiError::BadRequest("source_path is required".into())
            })?;
            validate_source_path(source_path)?;
            if !is_zfs_dataset(source_path) {
                return Err(ApiError::BadRequest(format!(
                    "\"{}\" is not a ZFS dataset",
                    source_path
                )));
            }

            let snaps = body.snapshots.as_deref().unwrap_or(&[]);
            if snaps.is_empty() {
                return Err(ApiError::BadRequest(
                    "at least one snapshot must be selected".into(),
                ));
            }
            validate_zfs_snapshots(source_path, snaps)?;

            let mp = resolve_fs_path(source_path);
            if !check_dirs(&mp, REQUIRED_SYSTEM_DIRS) {
                return Err(ApiError::BadRequest(format!(
                    "dataset mountpoint \"{mp}\" does not contain a valid FreeBSD system structure"
                )));
            }

            Ok(BaseSystem {
                name: body.name.clone(),
                type_: "zfs".into(),
                source_path: source_path.to_string(),
                snapshots: snaps.to_vec(),
                sharedfs_path: None,
                created_at: state.now_ts(),
            })
        }

        "sharedfs" => {
            let template = body.source_path.as_deref().ok_or_else(|| {
                ApiError::BadRequest("source_path (template path) is required".into())
            })?;
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

            Ok(BaseSystem {
                name: body.name.clone(),
                type_: "sharedfs".into(),
                source_path: template.to_string(),
                snapshots: Vec::new(),
                sharedfs_path: Some(sharedfs.to_string()),
                created_at: state.now_ts(),
            })
        }

        other => Err(ApiError::BadRequest(format!(
            "unknown base system type: \"{other}\""
        ))),
    }
}

/// "from-txz" / "download" method: create a new base system from a base.txz file.
/// If `txz_override` is provided (download method), it is used instead of body.txz_path.
fn create_base_from_txz(
    state: &AppState,
    body: &BaseImportBody,
    txz_override: Option<&str>,
) -> ApiResult<BaseSystem> {
    let txz_path = txz_override
        .map(|s| s.to_string())
        .or_else(|| body.txz_path.clone())
        .ok_or_else(|| ApiError::BadRequest("txz_path is required".into()))?;

    // Validate the txz file exists and has .txz extension.
    let p = std::path::Path::new(&txz_path);
    if !p.exists() || !p.is_file() {
        return Err(ApiError::BadRequest(format!(
            "base.txz file not found: {txz_path}"
        )));
    }
    validate_target_path(&txz_path)?;

    match body.type_.as_str() {
        "zfs" => create_base_from_txz_zfs(state, body, &txz_path),
        "sharedfs" => create_base_from_txz_sharedfs(state, body, &txz_path),
        other => Err(ApiError::BadRequest(format!(
            "unknown base system type: \"{other}\""
        ))),
    }
}

/// Create a ZFS base system from base.txz:
/// create dataset → extract → snapshot → register.
fn create_base_from_txz_zfs(
    state: &AppState,
    body: &BaseImportBody,
    txz_path: &str,
) -> ApiResult<BaseSystem> {
    let dataset = body.dataset.as_deref().ok_or_else(|| {
        ApiError::BadRequest("dataset is required for from-txz ZFS creation".into())
    })?;
    validate_zfs_name(dataset)?;

    // Dataset must not already exist.
    if is_zfs_dataset(dataset) {
        return Err(ApiError::BadRequest(format!(
            "dataset \"{dataset}\" already exists"
        )));
    }

    let snapshot_name = body.snapshot_name.as_deref().filter(|s| !s.is_empty());
    if let Some(sn) = snapshot_name {
        validate_snapshot_name(sn)?;
    }

    // Create the dataset.
    run(ZFS, &["create", dataset])?;

    // Get mountpoint.
    let mountpoint = resolve_fs_path(dataset);
    if mountpoint.is_empty() || mountpoint == dataset {
        let _ = run(ZFS, &["destroy", "-r", dataset]);
        return Err(ApiError::Internal(format!(
            "could not determine mountpoint for dataset \"{dataset}\""
        )));
    }

    // Extract base.txz into the mountpoint.
    if let Err(e) = run(TAR, &["-xf", txz_path, "-C", &mountpoint]) {
        let _ = run(ZFS, &["destroy", "-r", dataset]);
        return Err(e);
    }

    // Verify structure.
    if !check_dirs(&mountpoint, REQUIRED_SYSTEM_DIRS) {
        let _ = run(ZFS, &["destroy", "-r", dataset]);
        return Err(ApiError::BadRequest(format!(
            "extracted content at \"{mountpoint}\" does not contain a valid FreeBSD system structure"
        )));
    }

    // Create snapshot (optional).
    let full_snap = if let Some(sn) = snapshot_name {
        let snap = format!("{dataset}@{sn}");
        if let Err(e) = run(ZFS, &["snapshot", &snap]) {
            let _ = run(ZFS, &["destroy", "-r", dataset]);
            return Err(e);
        }
        crate::audit::record(
            state, None, "POST", "/api/jails/bases", 201,
            Some(format!("created ZFS dataset {dataset} from {txz_path}, snapshot {snap}")),
        );
        vec![snap]
    } else {
        crate::audit::record(
            state, None, "POST", "/api/jails/bases", 201,
            Some(format!("created ZFS dataset {dataset} from {txz_path} (no snapshot)")),
        );
        vec![]
    };

    Ok(BaseSystem {
        name: body.name.clone(),
        type_: "zfs".into(),
        source_path: dataset.to_string(),
        snapshots: full_snap,
        sharedfs_path: None,
        created_at: state.now_ts(),
    })
}

/// Create a SharedFS base system from base.txz:
/// extract to sharedfs dir → build template structure → register.
fn create_base_from_txz_sharedfs(
    state: &AppState,
    body: &BaseImportBody,
    txz_path: &str,
) -> ApiResult<BaseSystem> {
    let sharedfs_dir = body.sharedfs_path.as_deref().ok_or_else(|| {
        ApiError::BadRequest("sharedfs_path is required for from-txz SharedFS creation".into())
    })?;
    let template_dir = body.template_path.as_deref().ok_or_else(|| {
        ApiError::BadRequest("template_path is required for from-txz SharedFS creation".into())
    })?;

    validate_target_path(sharedfs_dir)?;
    validate_target_path(template_dir)?;

    // Target directories must not already exist (we are creating new ones).
    if std::path::Path::new(sharedfs_dir).exists() {
        return Err(ApiError::BadRequest(format!(
            "sharedfs directory already exists: {sharedfs_dir}"
        )));
    }
    if std::path::Path::new(template_dir).exists() {
        return Err(ApiError::BadRequest(format!(
            "template directory already exists: {template_dir}"
        )));
    }

    // Extract base.txz into sharedfs directory.
    fs::create_dir_all(sharedfs_dir)?;
    if let Err(e) = run(TAR, &["-xf", txz_path, "-C", sharedfs_dir]) {
        let _ = fs::remove_dir_all(sharedfs_dir);
        return Err(e);
    }

    // Build template structure.
    if let Err(e) = build_sharedfs_template(sharedfs_dir, template_dir) {
        let _ = fs::remove_dir_all(template_dir);
        return Err(e);
    }

    // Verify structure.
    if !check_dirs(template_dir, REQUIRED_TEMPLATE_DIRS) {
        let _ = fs::remove_dir_all(template_dir);
        return Err(ApiError::BadRequest(format!(
            "template directory \"{template_dir}\" does not have a valid structure after creation"
        )));
    }
    if !check_dirs(sharedfs_dir, REQUIRED_SHAREDFS_DIRS) {
        let _ = fs::remove_dir_all(template_dir);
        return Err(ApiError::BadRequest(format!(
            "sharedfs directory \"{sharedfs_dir}\" does not have a valid binaries structure after creation"
        )));
    }

    crate::audit::record(
        state, None, "POST", "/api/jails/bases", 201,
        Some(format!("created SharedFS base from {txz_path}: sharedfs={sharedfs_dir}, template={template_dir}")),
    );

    Ok(BaseSystem {
        name: body.name.clone(),
        type_: "sharedfs".into(),
        source_path: template_dir.to_string(),
        snapshots: Vec::new(),
        sharedfs_path: Some(sharedfs_dir.to_string()),
        created_at: state.now_ts(),
    })
}

/// Transform an extracted full FreeBSD system (in `sharedfs_dir`) into a
/// SharedFS + Template structure, following the qjail layout:
///
/// **sharedfs** keeps only shared read-only binaries (bin, lib, libexec, sbin, usr/...).
/// **template** gets per-jail dirs (etc, var, root, ...) + symlinks to /sharedfs/* +
/// empty standard dirs (dev, proc, media, ...) + the sharedfs mount point.
fn build_sharedfs_template(sharedfs_dir: &str, template_dir: &str) -> ApiResult<()> {
    use std::os::unix::fs::symlink;

    fs::create_dir_all(template_dir)?;
    let template_usr = format!("{template_dir}/usr");
    fs::create_dir_all(&template_usr)?;

    // 1. Move per-jail top-level dirs (etc, var, root, tmp) from sharedfs → template.
    for dir in TEMPLATE_REAL_TOP {
        let src = format!("{sharedfs_dir}/{dir}");
        let dst = format!("{template_dir}/{dir}");
        if std::path::Path::new(&src).exists() {
            fs::rename(&src, &dst)?;
        } else {
            fs::create_dir_all(&dst)?;
        }
    }

    // 2. Move per-jail usr/ subdirs (local, obj, tests) from sharedfs/usr → template/usr.
    let sharedfs_usr = format!("{sharedfs_dir}/usr");
    for dir in TEMPLATE_REAL_USR {
        let src = format!("{sharedfs_usr}/{dir}");
        let dst = format!("{template_usr}/{dir}");
        if std::path::Path::new(&src).exists() {
            fs::rename(&src, &dst)?;
        } else {
            fs::create_dir_all(&dst)?;
        }
    }

    // 3. Remove dirs from sharedfs that don't belong (boot, rescue, media, mnt, etc.).
    //    Keep only bin, lib, libexec, sbin, sys, usr.
    if let Ok(entries) = fs::read_dir(sharedfs_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let path = entry.path();
            if path.is_dir() {
                let is_shared = SHAREDFS_SHARED_TOP.contains(&name_str.as_ref())
                    || name_str == "usr";
                let is_moved = TEMPLATE_REAL_TOP.contains(&name_str.as_ref());
                // sys is a symlink, not a dir, so it won't be caught here.
                if !is_shared && !is_moved {
                    let _ = fs::remove_dir_all(&path);
                }
            } else if path.is_file() {
                // Move top-level files (.profile, .cshrc, COPYRIGHT, ...) to template.
                let dst = format!("{template_dir}/{name_str}");
                let _ = fs::rename(&path, &dst);
            }
        }
    }

    // 4. Remove non-shared subdirs from sharedfs/usr (keep only the shared set).
    if let Ok(entries) = fs::read_dir(&sharedfs_usr) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !SHAREDFS_SHARED_USR.contains(&name_str.as_ref()) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }

    // 5. Create symlinks in template → /sharedfs/* for shared top-level dirs.
    for dir in SHAREDFS_SHARED_TOP {
        let src_check = format!("{sharedfs_dir}/{dir}");
        let link = format!("{template_dir}/{dir}");
        if std::path::Path::new(&src_check).exists() && !std::path::Path::new(&link).exists() {
            symlink(format!("/sharedfs/{dir}"), &link)?;
        }
    }

    // 6. Create symlinks in template/usr → /sharedfs/usr/* for shared usr subdirs.
    for dir in SHAREDFS_SHARED_USR {
        let src_check = format!("{sharedfs_usr}/{dir}");
        let link = format!("{template_usr}/{dir}");
        if std::path::Path::new(&src_check).exists() && !std::path::Path::new(&link).exists() {
            symlink(format!("/sharedfs/usr/{dir}"), &link)?;
        }
    }

    // 7. Copy the sys symlink if it exists in sharedfs.
    let sharedfs_sys = format!("{sharedfs_dir}/sys");
    let template_sys = format!("{template_dir}/sys");
    if std::path::Path::new(&sharedfs_sys).exists() && !std::path::Path::new(&template_sys).exists()
    {
        symlink("/sharedfs/sys", &template_sys)?;
    }

    // 8. Create empty standard directories in template.
    for dir in TEMPLATE_EMPTY_DIRS {
        fs::create_dir_all(format!("{template_dir}/{dir}"))?;
    }

    // 9. Create home → usr/home symlink (like qjail).
    let home_link = format!("{template_dir}/home");
    if !std::path::Path::new(&home_link).exists() {
        symlink("usr/home", &home_link)?;
    }

    Ok(())
}

/// Download base.txz from a user-provided URL.
/// Returns the path to the downloaded temp file.
fn download_base_txz(body: &BaseImportBody) -> ApiResult<String> {
    let url = body.download_url.as_deref().ok_or_else(|| {
        ApiError::BadRequest("download_url is required for download method".into())
    })?;
    validate_url(url)?;

    // Download to a temp file.
    let tmp_dir = std::env::temp_dir();
    let tmp_file = tmp_dir.join("fwp-base-download.txz");
    let tmp_path = tmp_file.to_string_lossy().into_owned();

    tracing::info!("downloading base.txz from {url} to {tmp_path}");

    let output = Command::new(FETCH)
        .args(["-o", &tmp_path, url])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let _ = fs::remove_file(&tmp_path);
        return Err(ApiError::Command(if stderr.is_empty() {
            format!("failed to download {url}")
        } else {
            stderr
        }));
    }

    // Verify the file was downloaded and is non-empty.
    let metadata = fs::metadata(&tmp_path)?;
    if metadata.len() < 1024 {
        let _ = fs::remove_file(&tmp_path);
        return Err(ApiError::BadRequest(format!(
            "downloaded file is too small ({:.0} bytes), may be an error page",
            metadata.len()
        )));
    }

    Ok(tmp_path)
}

/// Validate a ZFS snapshot name (the short part after @).
fn validate_snapshot_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 256 {
        return Err(ApiError::BadRequest("invalid snapshot name".into()));
    }
    let valid = name.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | ' ')
    });
    if !valid || name.contains('@') {
        return Err(ApiError::BadRequest("invalid snapshot name".into()));
    }
    Ok(())
}

/// Validate a download URL — must be https:// or http://, no shell metacharacters.
fn validate_url(url: &str) -> ApiResult<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(ApiError::BadRequest("download URL must start with https:// or http://".into()));
    }
    if url.contains('\0') || url.contains('\n') || url.contains(' ') {
        return Err(ApiError::BadRequest("invalid download URL".into()));
    }
    Ok(())
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

// ── Jail editing ──────────────────────────────────────────────────

/// Boolean jail.conf parameters (emitted as `key;` not `key = "value";`).
const JAIL_BOOL_PARAMS: &[&str] = &[
    "persist",
    "mount.devfs",
    "mount.fdescfs",
    "mount.procfs",
    "exec.clean",
    "vnet",
];

/// Read-only parameters that cannot be set in jail.conf (filtered out on save).
const JAIL_READONLY_PARAMS: &[&str] = &[
    "jid",
    "dying",
    "lastjid",
    "children.cur",
    "osrelease",
    "osreldate",
    "cpuset.id",
    "ip4.saddrsel",
    "ip6.saddrsel",
];

#[derive(Debug, Deserialize)]
pub struct JailUpdateBody {
    /// Full parameter map for the jail (key → value).
    /// Boolean params use "true"/"false".
    pub params: HashMap<String, String>,
    /// Whether to add/remove this jail from rc.conf jail_list (auto-start).
    #[serde(default)]
    pub auto_start: Option<bool>,
}

/// Parse only the global parameters (before any jail block) from jail.conf.
fn parse_global_params() -> HashMap<String, String> {
    let content = fs::read_to_string(JAIL_CONF).unwrap_or_default();
    let mut globals: Vec<(String, String)> = Vec::new();
    let mut in_block_comment = false;
    let mut in_jail_block = false;

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

        // Once we enter a jail block, stop collecting globals.
        if line.ends_with('{') {
            in_jail_block = true;
            continue;
        }
        if line.starts_with('}') {
            in_jail_block = false;
            continue;
        }
        if in_jail_block { continue; }

        if let Some((k, v)) = parse_param(line) {
            globals.push((k, v));
        }
    }

    // Apply variable substitution with a dummy name for ${name} resolution.
    let mut map: HashMap<String, String> = globals.into_iter().collect();
    substitute_vars(&mut map, "");
    map
}

/// Generate a jail.conf block from a parameter map.
/// Parameters whose value matches the global default are skipped.
fn generate_jail_block_from_params(
    name: &str,
    params: &HashMap<String, String>,
    globals: &HashMap<String, String>,
) -> String {
    let mut lines = vec![format!("{name} {{")];

    let mut keys: Vec<&String> = params.keys().collect();
    keys.sort();

    for key in keys {
        if JAIL_READONLY_PARAMS.contains(&key.as_str()) {
            continue;
        }

        let value = params[key].trim();

        if JAIL_BOOL_PARAMS.contains(&key.as_str()) || key.starts_with("allow.") {
            // Boolean: skip if global default is also enabled.
            if value == "true" || value == "1" {
                let global_val = globals.get(key.as_str());
                if global_val.map(|v| v == "true" || v == "1").unwrap_or(false) {
                    continue;
                }
                lines.push(format!("    {key};"));
            }
            continue;
        }

        if value.is_empty() {
            continue;
        }

        // Skip if value matches the global default.
        if let Some(global_val) = globals.get(key.as_str()) {
            if global_val.trim() == value {
                continue;
            }
        }

        lines.push(format!("    {key} = \"{value}\";"));
    }

    lines.push("}".into());
    lines.join("\n")
}

/// Update a jail's configuration in jail.conf.
pub async fn jail_update(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<JailUpdateBody>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_jail_name(&name)?;

    let entries = parse_jail_conf().unwrap_or_default();
    if !entries.iter().any(|e| e.name == name) {
        return Err(ApiError::NotFound(format!("jail \"{name}\" not found")));
    }

    backup_jail_conf(&state)?;

    let conf_content = fs::read_to_string(JAIL_CONF).unwrap_or_default();
    let without_old = remove_jail_block(&conf_content, &name);
    let globals = parse_global_params();
    let new_block = generate_jail_block_from_params(&name, &body.params, &globals);

    let new_content = if without_old.is_empty() {
        format!("{new_block}\n")
    } else {
        let separator = if without_old.ends_with('\n') { "" } else { "\n" };
        format!("{without_old}{separator}\n{new_block}\n")
    };

    write_jail_conf_atomic(&new_content)?;

    // Update rc.conf jail_list if auto_start was provided.
    if let Some(want_auto) = body.auto_start {
        let mut list = read_jail_list();
        let was_in = list.contains(&name);
        if want_auto && !was_in {
            list.insert(name.clone());
            let val = list.iter().cloned().collect::<Vec<_>>().join(" ");
            let _ = Command::new("/usr/sbin/sysrc")
                .args([&format!("jail_list={val}")])
                .output();
            crate::audit::record(
                &state, None, "PUT", &format!("/api/jails/{name}"), 200,
                Some(format!("added {name} to jail_list (auto-start)")),
            );
        } else if !want_auto && was_in {
            list.remove(&name);
            let val = list.iter().cloned().collect::<Vec<_>>().join(" ");
            let _ = Command::new("/usr/sbin/sysrc")
                .args([&format!("jail_list={val}")])
                .output();
            crate::audit::record(
                &state, None, "PUT", &format!("/api/jails/{name}"), 200,
                Some(format!("removed {name} from jail_list (auto-start)")),
            );
        }
    }

    crate::audit::record(
        &state, None, "PUT", &format!("/api/jails/{name}"), 200,
        Some(format!("updated jail {name} configuration")),
    );

    Ok(Json(serde_json::json!({"name": name})))
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
