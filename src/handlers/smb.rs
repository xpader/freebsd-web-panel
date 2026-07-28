//! SMB (Samba) file sharing management — HTTP handlers.
//!
//! Manages smb4.conf, Samba users, and the samba_server service.
//! Requires the `samba416` (or compatible) pkg — not in FreeBSD base system.
//! Pattern follows bhyve init: `GET /status` + `POST /init` streaming task.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::LazyLock;

use axum::extract::{Path as AxumPath, State};
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::bgtask;
use crate::cmd;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const SAMBA_SMBD: &str = "/usr/local/sbin/smbd";
const SAMBA_CONF: &str = "/usr/local/etc/smb4.conf";
const SMBPASSWD: &str = "/usr/local/bin/smbpasswd";
const PDBEDIT: &str = "/usr/local/bin/pdbedit";
const SERVICE: &str = "/usr/sbin/service";
const RC_NAME: &str = "samba_server";
const PKG: &str = "/usr/sbin/pkg";
const SAMBA_PKG: &str = "samba416";

// ── Status ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SmbStatus {
    pub installed: bool,
    pub enabled: bool,
    pub initialized: bool,
    pub service_running: bool,
    pub version: Option<String>,
}

/// Check if Samba is running by reading pid files and verifying the processes exist.
fn is_samba_running() -> bool {
    for name in ["smbd", "nmbd"] {
        let pid_str = match std::fs::read_to_string(format!("/var/run/samba4/{name}.pid")) {
            Ok(s) => s.trim().to_string(),
            Err(_) => return false,
        };
        let pid: i32 = match pid_str.parse() {
            Ok(p) => p,
            Err(_) => return false,
        };
        if pid <= 0 || unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
    }
    true
}

pub fn check_status() -> SmbStatus {
    let installed = Path::new(SAMBA_SMBD).exists();
    let rc = crate::sysrc::read_rcconf_files();
    let enabled = rc
        .get("samba_server_enable")
        .map(|v| v == "YES")
        .unwrap_or(false);
    let initialized = Path::new(SAMBA_CONF).exists();
    let service_running = installed && is_samba_running();
    let version = if installed {
        cmd::run_sync(SAMBA_SMBD, &["--version"])
            .ok()
            .and_then(|s| {
                s.split_whitespace()
                    .skip_while(|w| w.eq_ignore_ascii_case("version"))
                    .next()
                    .map(|v| v.trim_end_matches(',').to_string())
            })
    } else {
        None
    };
    SmbStatus {
        installed,
        enabled,
        initialized,
        service_running,
        version,
    }
}

/// GET /api/smb/status
pub async fn status() -> ApiResult<Json<SmbStatus>> {
    let s = tokio::task::spawn_blocking(check_status)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(s))
}

// ── Init ────────────────────────────────────────────────────────────

/// POST /api/smb/init — streaming background task that installs Samba,
/// enables the service, and generates a default smb4.conf.
pub async fn init(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<serde_json::Value>> {
    let st = tokio::task::spawn_blocking(check_status)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    if st.installed && st.initialized {
        return Err(ApiError::Conflict("Samba already initialized".into()));
    }

    let id = bgtask::create("smb-init", "Install Samba");
    let tid = id.clone();
    let state2 = state.clone();
    let username = user.username.clone();
    tokio::spawn(async move {
        let ok = run_init_streaming(&tid).await;
        bgtask::set_status(
            &tid,
            if ok {
                bgtask::TaskStatus::Done
            } else {
                bgtask::TaskStatus::Failed
            },
            Some(if ok { 0 } else { 1 }),
        );
        audit::record(
            &state2,
            Some(&username),
            "POST",
            "/api/smb/init",
            if ok { 200 } else { 500 },
            Some(if ok {
                "Samba initialized".into()
            } else {
                "Samba init failed".into()
            }),
        );
    });
    Ok(Json(serde_json::json!({ "task_id": id })))
}

async fn run_init_streaming(task_id: &str) -> bool {
    macro_rules! fail {
        ($msg:expr) => {{
            let m: String = $msg.into();
            bgtask::push_line(task_id, &m);
            bgtask::set_status(task_id, bgtask::TaskStatus::Failed, None);
            return false;
        }};
    }

    // Step 1: Install Samba
    bgtask::push_line(task_id, "=== [1/3] Installing samba416 ===");
    let exit =
        bgtask::run_streaming_cmd(task_id, PKG, &["install", "-y", SAMBA_PKG]).await;
    if exit != 0 {
        fail!(format!("Package installation failed (exit code {exit})"));
    }
    bgtask::push_line(task_id, "Samba installed.");

    // Step 2: Enable service
    bgtask::push_line(task_id, "=== [2/3] Enabling samba_server ===");
    let rc_result = tokio::task::spawn_blocking(|| {
        crate::sysrc::set("samba_server_enable", "YES")
    })
    .await;
    match rc_result {
        Ok(Ok(())) => bgtask::push_line(task_id, "samba_server_enable=YES"),
        Ok(Err(e)) => fail!(format!("sysrc failed: {e}")),
        Err(e) => fail!(format!("sysrc task panicked: {e}")),
    }

    // Step 3: Generate default config + start
    bgtask::push_line(task_id, "=== [3/3] Generating smb4.conf ===");
    let conf_result =
        tokio::task::spawn_blocking(|| write_default_conf().map(|_| ())).await;
    match conf_result {
        Ok(Ok(())) => {
            bgtask::push_line(task_id, "smb4.conf created.");
        }
        Ok(Err(e)) => fail!(format!("Failed to write smb4.conf: {e}")),
        Err(e) => fail!(format!("Config task panicked: {e}")),
    }

    // Start the service
    bgtask::push_line(task_id, "Starting samba_server...");
    let exit = bgtask::run_streaming_cmd(task_id, SERVICE, &[RC_NAME, "start"]).await;
    if exit != 0 {
        bgtask::push_line(task_id, "Warning: service did not start cleanly.");
    }

    bgtask::push_line(task_id, "=== Initialization complete ===");
    true
}

// ── smb4.conf data model ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_workgroup")]
    pub workgroup: String,
    #[serde(default = "default_server_string")]
    pub server_string: String,
    #[serde(default = "default_server_role")]
    pub server_role: String,
    #[serde(default = "default_map_to_guest")]
    pub map_to_guest: String,
    #[serde(default = "default_passdb_backend")]
    pub passdb_backend: String,
    #[serde(default = "default_server_min_protocol")]
    pub server_min_protocol: String,
    #[serde(default = "default_dns_proxy")]
    pub dns_proxy: String,
    #[serde(default = "default_load_printers")]
    pub load_printers: String,
    #[serde(default = "default_log_level")]
    pub log_level: u8,
    /// macOS compatibility: when true, loads fruit + streams_xattr VFS modules
    /// and enables AAPL protocol negotiation, metadata streaming, and encoding.
    #[serde(default)]
    pub fruit_enabled: bool,
}

fn default_workgroup() -> String { "WORKGROUP".into() }
fn default_server_string() -> String { "FreeBSD Samba Server".into() }
fn default_server_role() -> String { "standalone".into() }
fn default_map_to_guest() -> String { "Bad User".into() }
fn default_passdb_backend() -> String { "tdbsam".into() }
fn default_server_min_protocol() -> String { "SMB2".into() }
fn default_dns_proxy() -> String { "no".into() }
fn default_load_printers() -> String { "no".into() }
fn default_log_level() -> u8 { 1 }

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            workgroup: default_workgroup(),
            server_string: default_server_string(),
            server_role: default_server_role(),
            map_to_guest: default_map_to_guest(),
            passdb_backend: default_passdb_backend(),
            server_min_protocol: default_server_min_protocol(),
            dns_proxy: default_dns_proxy(),
            load_printers: default_load_printers(),
            log_level: default_log_level(),
            fruit_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmbShare {
    pub name: String,
    #[serde(default)]
    pub comment: String,
    pub path: String,
    #[serde(default = "default_true")]
    pub browseable: bool,
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub guest_ok: bool,
    #[serde(default)]
    pub valid_users: Vec<String>,
    #[serde(default = "default_create_mask")]
    pub create_mask: String,
    #[serde(default = "default_directory_mask")]
    pub directory_mask: String,
    /// Expose this share as a macOS Time Machine backup target.
    #[serde(default)]
    pub time_machine: bool,
    /// Optional capacity quota for the Time Machine share (e.g. "1T", "500G").
    /// Empty string means no limit.
    #[serde(default)]
    pub time_machine_max_size: String,
}

fn default_true() -> bool { true }
fn default_create_mask() -> String { "0664".into() }
fn default_directory_mask() -> String { "0775".into() }

#[derive(Debug, Clone, Serialize)]
pub struct SmbConfig {
    pub global: GlobalConfig,
    pub shares: Vec<SmbShare>,
}

// ── smb4.conf parser ────────────────────────────────────────────────

/// Parse smb4.conf (INI format) into structured config.
/// Returns default config if file doesn't exist.
fn parse_conf() -> SmbConfig {
    let content = match std::fs::read_to_string(SAMBA_CONF) {
        Ok(c) => c,
        Err(_) => {
            return SmbConfig {
                global: GlobalConfig::default(),
                shares: vec![],
            };
        }
    };

    let mut global = GlobalConfig::default();
    let mut shares: Vec<SmbShare> = vec![];
    let mut current_section: Option<(String, HashMap<String, String>)> = None;
    let mut global_extra: HashMap<String, String> = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            // Save previous section
            if let Some((name, props)) = current_section.take() {
                if name.eq_ignore_ascii_case("global") {
                    apply_global(&mut global, &props, &mut global_extra);
                } else {
                    shares.push(props_to_share(&name, &props));
                }
            }
            let name = line[1..line.len() - 1].trim().to_string();
            current_section = Some((name, HashMap::new()));
            continue;
        }
        if let Some(eq) = line.find('=') {
            let key = line[..eq].trim().to_lowercase();
            let value = line[eq + 1..].trim().to_string();
            if let Some((_, props)) = &mut current_section {
                props.insert(key, value);
            }
        }
    }
    // Save last section
    if let Some((name, props)) = current_section.take() {
        if name.eq_ignore_ascii_case("global") {
            apply_global(&mut global, &props, &mut global_extra);
        } else {
            shares.push(props_to_share(&name, &props));
        }
    }

    SmbConfig { global, shares }
}

fn apply_global(
    global: &mut GlobalConfig,
    props: &HashMap<String, String>,
    _extra: &mut HashMap<String, String>,
) {
    if let Some(v) = props.get("workgroup") { global.workgroup = v.clone(); }
    if let Some(v) = props.get("server string") { global.server_string = v.clone(); }
    if let Some(v) = props.get("server role") { global.server_role = v.clone(); }
    if let Some(v) = props.get("map to guest") { global.map_to_guest = v.clone(); }
    if let Some(v) = props.get("passdb backend") { global.passdb_backend = v.clone(); }
    if let Some(v) = props.get("server min protocol") { global.server_min_protocol = v.clone(); }
    if let Some(v) = props.get("dns proxy") { global.dns_proxy = v.clone(); }
    if let Some(v) = props.get("load printers") { global.load_printers = v.clone(); }
    if let Some(v) = props.get("log level") {
        if let Ok(n) = v.parse::<u8>() { global.log_level = n; }
    }
    // Detect macOS compatibility: vfs objects containing "fruit" or fruit:aapl = yes
    if let Some(v) = props.get("vfs objects") {
        if v.split_whitespace().any(|o| o.eq_ignore_ascii_case("fruit")) {
            global.fruit_enabled = true;
        }
    }
}

fn props_to_share(name: &str, props: &HashMap<String, String>) -> SmbShare {
    SmbShare {
        name: name.to_string(),
        comment: props.get("comment").cloned().unwrap_or_default(),
        path: props.get("path").cloned().unwrap_or_default(),
        browseable: parse_bool(props.get("browseable").map(|v| v.as_str())),
        writable: parse_bool(props.get("writable").map(|v| v.as_str())),
        guest_ok: parse_bool(props.get("guest ok").map(|v| v.as_str())),
        valid_users: props
            .get("valid users")
            .map(|v| v.split_whitespace().map(String::from).collect())
            .unwrap_or_default(),
        create_mask: props
            .get("create mask")
            .cloned()
            .unwrap_or_else(default_create_mask),
        directory_mask: props
            .get("directory mask")
            .cloned()
            .unwrap_or_else(default_directory_mask),
        time_machine: props
            .get("fruit:time machine")
            .map(|v| v.eq_ignore_ascii_case("yes") || v == "true" || v == "1")
            .unwrap_or(false),
        time_machine_max_size: props
            .get("fruit:time machine max size")
            .cloned()
            .unwrap_or_default(),
    }
}

fn parse_bool(v: Option<&str>) -> bool {
    match v {
        Some(s) => s.eq_ignore_ascii_case("yes") || s == "true" || s == "1",
        None => true, // Samba defaults browseable=yes, writable=no; callers override
    }
}

// ── smb4.conf generator ─────────────────────────────────────────────

fn generate_conf(config: &SmbConfig) -> String {
    let g = &config.global;
    let mut out = String::new();
    out.push_str("# Managed by FreeBSD Web Panel\n\n");

    out.push_str("[global]\n");
    let _ = writeln!(out, "    workgroup = {}", g.workgroup);
    let _ = writeln!(out, "    server string = {}", g.server_string);
    let _ = writeln!(out, "    server role = {}", g.server_role);
    let _ = writeln!(out, "    map to guest = {}", g.map_to_guest);
    let _ = writeln!(out, "    passdb backend = {}", g.passdb_backend);
    let _ = writeln!(out, "    server min protocol = {}", g.server_min_protocol);
    let _ = writeln!(out, "    dns proxy = {}", g.dns_proxy);
    let _ = writeln!(out, "    load printers = {}", g.load_printers);
    let _ = writeln!(out, "    log level = {}", g.log_level);
    out.push_str("    logging = file\n");
    out.push_str("    log file = /var/log/samba4/log.%m\n");
    out.push_str("    max log size = 50\n");
    if g.fruit_enabled {
        out.push_str("    vfs objects = fruit streams_xattr\n");
        out.push_str("    fruit:aapl = yes\n");
        out.push_str("    fruit:metadata = stream\n");
        out.push_str("    fruit:encoding = native\n");
    }
    out.push('\n');

    for share in &config.shares {
        out.push_str(&format!("[{}]\n", share.name));
        let _ = writeln!(out, "    comment = {}", share.comment);
        let _ = writeln!(out, "    path = {}", share.path);
        let _ = writeln!(out, "    browseable = {}", yn(share.browseable));
        let _ = writeln!(out, "    writable = {}", yn(share.writable));
        let _ = writeln!(out, "    guest ok = {}", yn(share.guest_ok));
        if !share.valid_users.is_empty() {
            let _ = writeln!(out, "    valid users = {}", share.valid_users.join(" "));
        }
        let _ = writeln!(out, "    create mask = {}", share.create_mask);
        let _ = writeln!(out, "    directory mask = {}", share.directory_mask);
        if share.time_machine {
            // Time Machine requires the fruit VFS module; load it at share level
            // if the global macOS compatibility toggle hasn't already.
            if !config.global.fruit_enabled {
                out.push_str("    vfs objects = fruit streams_xattr\n");
            }
            out.push_str("    fruit:time machine = yes\n");
            if !share.time_machine_max_size.trim().is_empty() {
                let _ = writeln!(
                    out,
                    "    fruit:time machine max size = {}",
                    share.time_machine_max_size.trim()
                );
            }
        }
        out.push('\n');
    }

    out
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn write_default_conf() -> ApiResult<()> {
    let config = SmbConfig {
        global: GlobalConfig::default(),
        shares: vec![],
    };
    write_conf(&config)
}

fn write_conf(config: &SmbConfig) -> ApiResult<()> {
    let content = generate_conf(config);
    let tmp = format!("{SAMBA_CONF}.tmp");
    std::fs::write(&tmp, &content)?;
    std::fs::rename(&tmp, SAMBA_CONF)?;
    Ok(())
}

/// Reload smb4.conf into the live config and save it.
/// Reads current config, applies mutation, writes back, reloads service.
fn modify_conf<F: FnOnce(&mut SmbConfig)>(f: F) -> ApiResult<()> {
    let mut config = parse_conf();
    f(&mut config);
    write_conf(&config)
}

// ── Validation ──────────────────────────────────────────────────────

static RE_SHARE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_.$-]{1,64}$").unwrap());

static RE_USERNAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_.-]{1,32}$").unwrap());

fn ensure_initialized() -> ApiResult<()> {
    let st = check_status();
    if !st.installed || !st.initialized {
        return Err(ApiError::BadRequest("Samba not initialized".into()));
    }
    Ok(())
}

fn validate_share_name(name: &str) -> ApiResult<()> {
    if !RE_SHARE_NAME.is_match(name) {
        return Err(ApiError::BadRequest(
            "Invalid share name (allowed: letters, digits, _, ., $, -, max 64 chars)".into(),
        ));
    }
    Ok(())
}

fn validate_username(name: &str) -> ApiResult<()> {
    if !RE_USERNAME.is_match(name) {
        return Err(ApiError::BadRequest(
            "Invalid username (allowed: letters, digits, _, ., -, max 32 chars)".into(),
        ));
    }
    Ok(())
}

fn validate_system_user(username: &str) -> ApiResult<()> {
    validate_username(username)?;
    let content = std::fs::read_to_string("/etc/passwd")
        .map_err(|e| ApiError::Internal(format!("read /etc/passwd: {e}")))?;
    let exists = content
        .lines()
        .any(|l| l.starts_with(&format!("{username}:")));
    if !exists {
        return Err(ApiError::BadRequest(format!(
            "System user '{username}' does not exist"
        )));
    }
    Ok(())
}

fn validate_path(path: &str) -> ApiResult<()> {
    if !path.starts_with('/') {
        return Err(ApiError::BadRequest("Path must be absolute".into()));
    }
    if path.contains('\0') {
        return Err(ApiError::BadRequest("Invalid path".into()));
    }
    Ok(())
}

// ── Config handlers ─────────────────────────────────────────────────

/// GET /api/smb/config
pub async fn get_config() -> ApiResult<Json<GlobalConfig>> {
    ensure_initialized()?;
    let config = tokio::task::spawn_blocking(|| parse_conf().global)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(config))
}

/// PUT /api/smb/config
pub async fn update_config(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<GlobalConfig>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_initialized()?;
    tokio::task::spawn_blocking(move || {
        modify_conf(|c| c.global = req.clone())?;
        cmd::run_sync(SERVICE, &[RC_NAME, "reload"])
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&user.username),
        "PUT",
        "/api/smb/config",
        200,
        Some("Updated SMB global config".into()),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

// ── Shares handlers ─────────────────────────────────────────────────

/// GET /api/smb/shares
pub async fn list_shares() -> ApiResult<Json<Vec<SmbShare>>> {
    ensure_initialized()?;
    let config = tokio::task::spawn_blocking(parse_conf)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(config.shares))
}

#[derive(Debug, Deserialize)]
pub struct CreateShareReq {
    pub name: String,
    #[serde(default)]
    pub comment: String,
    pub path: String,
    #[serde(default = "default_true")]
    pub browseable: bool,
    #[serde(default)]
    pub writable: bool,
    #[serde(default)]
    pub guest_ok: bool,
    #[serde(default)]
    pub valid_users: Vec<String>,
    #[serde(default = "default_create_mask")]
    pub create_mask: String,
    #[serde(default = "default_directory_mask")]
    pub directory_mask: String,
    #[serde(default)]
    pub time_machine: bool,
    #[serde(default)]
    pub time_machine_max_size: String,
}

/// POST /api/smb/shares
pub async fn create_share(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateShareReq>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_initialized()?;
    validate_share_name(&req.name)?;
    validate_path(&req.path)?;

    let name = req.name.clone();
    let username = user.username.clone();
    let result = tokio::task::spawn_blocking(move || -> ApiResult<()> {
        modify_conf(|c| {
            // Reject duplicate
            if c.shares.iter().any(|s| s.name.eq_ignore_ascii_case(&req.name)) {
                return;
            }
            c.shares.push(SmbShare {
                name: req.name,
                comment: req.comment,
                path: req.path,
                browseable: req.browseable,
                writable: req.writable,
                guest_ok: req.guest_ok,
                valid_users: req.valid_users,
                create_mask: req.create_mask,
                directory_mask: req.directory_mask,
                time_machine: req.time_machine,
                time_machine_max_size: req.time_machine_max_size,
            });
        })?;
        cmd::run_sync(SERVICE, &[RC_NAME, "reload"])?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    // Check if share was actually added (not a duplicate)
    let config = parse_conf();
    if !config.shares.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
        return Err(ApiError::Conflict(format!("Share '{name}' already exists")));
    }

    let _ = result;
    audit::record(
        &state,
        Some(&username),
        "POST",
        "/api/smb/shares",
        200,
        Some(format!("Created SMB share '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

/// PUT /api/smb/shares/{name}
pub async fn update_share(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
    Json(req): Json<CreateShareReq>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_initialized()?;
    validate_path(&req.path)?;
    let target = name.clone();
    let username = user.username.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        modify_conf(|c| {
            if let Some(share) = c.shares.iter_mut().find(|s| s.name.eq_ignore_ascii_case(&target)) {
                share.comment = req.comment;
                share.path = req.path;
                share.browseable = req.browseable;
                share.writable = req.writable;
                share.guest_ok = req.guest_ok;
                share.valid_users = req.valid_users;
                share.create_mask = req.create_mask;
                share.directory_mask = req.directory_mask;
                share.time_machine = req.time_machine;
                share.time_machine_max_size = req.time_machine_max_size;
            }
        })?;
        cmd::run_sync(SERVICE, &[RC_NAME, "reload"])?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&username),
        "PUT",
        "/api/smb/shares",
        200,
        Some(format!("Updated SMB share '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

/// DELETE /api/smb/shares/{name}
pub async fn delete_share(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_initialized()?;
    let target = name.clone();
    let username = user.username.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        modify_conf(|c| c.shares.retain(|s| !s.name.eq_ignore_ascii_case(&target)))?;
        cmd::run_sync(SERVICE, &[RC_NAME, "reload"])?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&username),
        "DELETE",
        "/api/smb/shares",
        200,
        Some(format!("Deleted SMB share '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

// ── Users handlers ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SmbUser {
    pub username: String,
    pub uid: u32,
}

/// GET /api/smb/users
pub async fn list_users() -> ApiResult<Json<Vec<SmbUser>>> {
    ensure_initialized()?;
    let users = tokio::task::spawn_blocking(|| -> ApiResult<Vec<SmbUser>> {
        let output = cmd::run_sync(PDBEDIT, &["-L"])?;
        let mut users = vec![];
        for line in output.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 2 {
                let username = parts[0].trim().to_string();
                let uid = parts[1].trim().parse::<u32>().unwrap_or(0);
                users.push(SmbUser { username, uid });
            }
        }
        Ok(users)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    Ok(Json(users))
}

/// GET /api/smb/sysusers — list system users that can be Samba users.
/// Filters out system accounts (uid < 1000) and existing Samba users.
pub async fn list_sysusers() -> ApiResult<Json<Vec<SysUser>>> {
    ensure_initialized()?;
    let users = tokio::task::spawn_blocking(|| -> ApiResult<Vec<SysUser>> {
        let passwd = std::fs::read_to_string("/etc/passwd")?;
        let smb_output = cmd::run_sync(PDBEDIT, &["-L"]).unwrap_or_default();
        let existing: std::collections::HashSet<String> = smb_output
            .lines()
            .filter_map(|l| {
                let parts: Vec<&str> = l.split(':').collect();
                parts.first().map(|s| s.trim().to_string())
            })
            .collect();

        let mut users = vec![];
        for line in passwd.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() < 7 { continue; }
            let username = parts[0];
            let uid: u32 = parts[2].parse().unwrap_or(0);
            let shell = parts[6];
            // Skip system accounts, nologin shells, and existing Samba users
            if uid < 1000 || shell.ends_with("/nologin") || shell.ends_with("/false") {
                continue;
            }
            if existing.contains(username) {
                continue;
            }
            users.push(SysUser {
                username: username.to_string(),
                uid,
                gecos: parts[4].to_string(),
            });
        }
        Ok(users)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    Ok(Json(users))
}

#[derive(Debug, Serialize)]
pub struct SysUser {
    pub username: String,
    pub uid: u32,
    pub gecos: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserReq {
    pub username: String,
    pub password: String,
}

/// POST /api/smb/users
pub async fn create_user(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateUserReq>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_initialized()?;
    validate_system_user(&req.username)?;

    let username = req.username.clone();
    let password = req.password.clone();
    let auth_user = user.username.clone();
    let smb_user = username.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        let stdin = format!("{password}\n{password}\n");
        cmd::run_sync_stdin(SMBPASSWD, &["-a", "-s", &smb_user], stdin.as_bytes())?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth_user),
        "POST",
        "/api/smb/users",
        200,
        Some(format!("Created Samba user '{username}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

/// DELETE /api/smb/users/{name}
pub async fn delete_user(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_initialized()?;
    validate_username(&name)?;
    let target = name.clone();
    let username = user.username.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        cmd::run_sync(PDBEDIT, &["-x", "-u", &target])?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&username),
        "DELETE",
        "/api/smb/users",
        200,
        Some(format!("Deleted Samba user '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordReq {
    pub password: String,
}

/// PUT /api/smb/users/{name}/password
pub async fn change_password(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
    Json(req): Json<ChangePasswordReq>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_initialized()?;
    validate_username(&name)?;
    let target = name.clone();
    let password = req.password.clone();
    let username = user.username.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        let stdin = format!("{password}\n{password}\n");
        cmd::run_sync_stdin(SMBPASSWD, &["-a", "-s", &target], stdin.as_bytes())?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&username),
        "PUT",
        "/api/smb/users/password",
        200,
        Some(format!("Changed password for Samba user '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

// ── Service control ─────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ServiceActionReq {}

/// POST /api/smb/service/{action}
/// action: start | stop | restart | reload
/// start also sets samba_server_enable=YES, stop also sets samba_server_enable=NO.
pub async fn service_control(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(action): AxumPath<String>,
    Json(_req): Json<ServiceActionReq>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_initialized()?;
    let action = action.to_lowercase();
    match action.as_str() {
        "start" | "stop" | "restart" | "reload" => {}
        _ => return Err(ApiError::BadRequest("Invalid service action".into())),
    }

    let act = action.clone();
    let username = user.username.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        let was_enabled = crate::sysrc::is_yes("samba_server_enable");
        if act == "start" {
            crate::sysrc::set("samba_server_enable", "YES")
                .map_err(ApiError::Command)?;
        }
        if act != "reload" || is_samba_running() {
            let svc_act = if act == "stop" && !was_enabled { "onestop" } else { &act };
            cmd::run_sync(SERVICE, &[RC_NAME, svc_act])?;
        }
        if act == "stop" {
            crate::sysrc::set("samba_server_enable", "NO")
                .map_err(ApiError::Command)?;
        }
        // Verify the service reached the expected state
        let running = is_samba_running();
        if act == "start" && !running {
            return Err(ApiError::Command("service did not start".into()));
        }
        if act == "stop" && running {
            return Err(ApiError::Command("service did not stop".into()));
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&username),
        "POST",
        "/api/smb/service",
        200,
        Some(format!("Samba service {action}")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}
