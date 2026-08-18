//! Time management — system clock, timezone, and NTP (ntpd) configuration.
//!
//! ## Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/api/time/status` | Time overview (clock + timezone + NTP status) |
//! | PUT | `/api/time/datetime` | Manually set system time |
//! | POST | `/api/time/sync` | One-shot NTP sync via `sntp` |
//! | PUT | `/api/time/timezone` | Set system timezone (`tzsetup`) |
//! | PUT | `/api/time/rtc-mode` | Switch RTC mode (UTC ↔ local) |
//! | GET | `/api/time/zones` | List available timezones |
//! | PUT | `/api/time/ntp/sync-on-start` | Toggle `ntpd_sync_on_start` |
//! | POST | `/api/time/ntp/enable` | Enable + start ntpd |
//! | POST | `/api/time/ntp/disable` | Stop + disable ntpd |
//! | POST | `/api/time/ntp/restart` | Restart ntpd |

use std::collections::BTreeMap;
use std::path::Path;

use axum::extract::State;
use axum::Json;
use chrono::{Local, Offset, Utc};
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::cmd;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::sysinfo;
use crate::sysrc;

const DATE: &str = "/bin/date";
const TZSETUP: &str = "/usr/sbin/tzsetup";
const SNTP: &str = "/usr/sbin/sntp";
const NTPQ: &str = "/usr/bin/ntpq";
const SYSCTL: &str = "/sbin/sysctl";
const ADJKERNTZ: &str = "/sbin/adjkerntz";
const SERVICE: &str = "/usr/sbin/service";

const NTP_CONF: &str = "/etc/ntp.conf";
const LOCALTIME: &str = "/etc/localtime";
const WALL_CMOS_CLOCK: &str = "/etc/wall_cmos_clock";
const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";
const NTP_PIDFILE: &str = "/var/db/ntp/ntpd.pid";
const DRIFT_FILES: &[&str] = &["/var/db/ntp/ntpd.drift", "/var/db/ntpd.drift"];

// ═══════════════════════════════════════════════════════════════════
//  Data models
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Serialize)]
pub struct TimeStatus {
    pub local_time: String,
    pub utc_time: String,
    pub utc_offset: String,
    pub epoch: i64,
    pub boot_time: i64,
    pub uptime_seconds: u64,
    pub timezone: String,
    pub timezone_abbr: String,
    pub rtc_local: bool,
    pub ntp: NtpStatus,
}

#[derive(Debug, Serialize)]
pub struct NtpStatus {
    pub enabled: bool,
    pub running: bool,
    pub sync_on_start: bool,
    pub stratum: Option<u8>,
    pub offset_ms: Option<f64>,
    pub system_peer: Option<String>,
    pub peers: Vec<NtpPeer>,
    pub drift: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct NtpPeer {
    pub remote: String,
    pub refid: String,
    pub stratum: u8,
    pub state: String,
    pub delay_ms: f64,
    pub offset_ms: f64,
    pub jitter_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct NtpConfig {
    pub servers: Vec<ServerEntry>,
    pub sync_on_start: bool,
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    pub kind: String,
    pub host: String,
    pub options: String,
}

#[derive(Debug, Serialize)]
pub struct ZoneList {
    pub regions: Vec<ZoneRegion>,
}

#[derive(Debug, Serialize)]
pub struct ZoneRegion {
    pub name: String,
    pub zones: Vec<String>,
}

// ═══════════════════════════════════════════════════════════════════
//  Helper functions (synchronous)
// ═══════════════════════════════════════════════════════════════════

/// Extract IANA timezone name from `/etc/localtime` symlink target.
fn current_timezone() -> String {
    if let Ok(target) = std::fs::read_link(LOCALTIME) {
        let s = target.to_string_lossy().into_owned();
        let prefix = format!("{ZONEINFO_DIR}/");
        if let Some(zone) = s.strip_prefix(&prefix) {
            return zone.to_string();
        }
        // Relative or custom path — use canonicalize as fallback
        if let Ok(canon) = std::fs::canonicalize(LOCALTIME) {
            let cs = canon.to_string_lossy().into_owned();
            if let Some(zone) = cs.strip_prefix(&prefix) {
                return zone.to_string();
            }
        }
        return s;
    }
    // Not a symlink — can't determine IANA name
    "Unknown".to_string()
}

/// Read the RTC mode: `true` if `/etc/wall_cmos_clock` exists (RTC = local).
fn rtc_is_local() -> bool {
    Path::new(WALL_CMOS_CLOCK).exists()
}

/// Check if ntpd is running by reading its pidfile and signaling the process.
fn is_ntpd_running() -> bool {
    let pid = std::fs::read_to_string(NTP_PIDFILE)
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok());
    match pid {
        Some(pid) if pid > 0 => {
            // SAFETY: kill(pid, 0) checks process existence without sending a signal.
            unsafe { libc::kill(pid, 0) == 0 }
        }
        _ => false,
    }
}

/// Read the NTP drift value from the drift file.
fn read_drift() -> Option<f64> {
    for path in DRIFT_FILES {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(val) = content.trim().parse::<f64>() {
                return Some(val);
            }
        }
    }
    None
}

/// Parse `ntpq -p` output into peer list. Skips `.POOL.` virtual entries.
/// Returns `(peers, system_peer_name, stratum, offset_ms)`.
fn parse_ntpq(output: &str) -> (Vec<NtpPeer>, Option<String>, Option<u8>, Option<f64>) {
    let mut peers = Vec::new();
    let mut past_header = false;

    for line in output.lines() {
        if !past_header {
            if line.starts_with('=') {
                past_header = true;
            }
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }

        let status_char = line.chars().next().unwrap_or(' ');
        let fields: Vec<&str> = line[1..].split_whitespace().collect();
        if fields.len() < 10 {
            continue;
        }

        // Skip virtual pool entries
        if fields[1] == ".POOL." {
            continue;
        }

        let state = match status_char {
            '*' => "sync",
            '+' => "candidate",
            '-' => "outlier",
            'x' => "false",
            '#' => "backup",
            _ => "unselected",
        }
        .to_string();

        peers.push(NtpPeer {
            remote: fields[0].to_string(),
            refid: fields[1].to_string(),
            stratum: fields[2].parse().unwrap_or(0),
            state,
            delay_ms: fields[7].parse().unwrap_or(0.0),
            offset_ms: fields[8].parse().unwrap_or(0.0),
            jitter_ms: fields[9].parse().unwrap_or(0.0),
        });
    }

    // The `*` peer is the system sync source
    let sys_peer = peers.iter().find(|p| p.state == "sync");
    let system_peer = sys_peer.map(|p| p.remote.clone());
    let stratum = sys_peer.map(|p| p.stratum + 1);
    let offset_ms = sys_peer.map(|p| p.offset_ms);

    (peers, system_peer, stratum, offset_ms)
}

/// Recursively collect zone names from a zoneinfo subdirectory.
fn collect_zones(dir: &Path, prefix: &str, zones: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip metadata files
        if name.ends_with(".tab") || name.ends_with(".list") || name == "Factory" {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let full = entry.path();
        if ft.is_dir() {
            let new_prefix = format!("{prefix}/{name}");
            collect_zones(&full, &new_prefix, zones);
        } else if ft.is_file() || ft.is_symlink() {
            zones.push(format!("{prefix}/{name}"));
        }
    }
}

/// Scan `/usr/share/zoneinfo/` and return zones grouped by region.
fn scan_zones() -> Vec<ZoneRegion> {
    let mut regions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Directories to skip (duplicates or metadata)
    let excluded = ["posix", "right", "src", "locale", "SystemV"];

    let Ok(entries) = std::fs::read_dir(ZONEINFO_DIR) else {
        return vec![];
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".tab") || name.ends_with(".list") || name == "Factory" {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        let path = entry.path();

        if ft.is_dir() {
            if excluded.contains(&name.as_str()) {
                continue;
            }
            let mut zones = Vec::new();
            collect_zones(&path, &name, &mut zones);
            zones.sort();
            if !zones.is_empty() {
                regions.insert(name, zones);
            }
        } else if ft.is_file() || ft.is_symlink() {
            // Top-level zones (UTC, GMT, CET, etc.)
            regions.entry("Misc".into()).or_default().push(name);
        }
    }

    regions.into_iter()
        .map(|(name, mut zones)| {
            zones.sort();
            ZoneRegion { name, zones }
        })
        .collect()
}

// ═══════════════════════════════════════════════════════════════════
//  Status — GET /api/time/status
// ═══════════════════════════════════════════════════════════════════

fn collect_status() -> ApiResult<TimeStatus> {
    let now = Local::now();
    let utc = Utc::now();

    // Format UTC offset as "+0800"
    let offset_secs = now.offset().fix().local_minus_utc();
    let sign = if offset_secs >= 0 { '+' } else { '-' };
    let abs = offset_secs.unsigned_abs();
    let utc_offset = format!("{sign}{:02}{:02}", abs / 3600, (abs % 3600) / 60);

    let boot_time = sysinfo::boot_time();
    let epoch = now.timestamp();
    let uptime = if boot_time > 0 {
        (epoch - boot_time).max(0) as u64
    } else {
        0
    };

    let timezone = current_timezone();
    let timezone_abbr = now.format("%Z").to_string();
    let rtc_local = rtc_is_local();

    // NTP status
    let ntp_enabled = sysrc::is_yes("ntpd_enable");
    let sync_on_start = sysrc::is_yes("ntpd_sync_on_start");
    let running = is_ntpd_running();

    let (peers, system_peer, stratum, offset_ms, drift) = if running {
        let ntpq_out = cmd::run_sync(NTPQ, &["-p"]).unwrap_or_default();
        let (peers, sp, st, off) = parse_ntpq(&ntpq_out);
        (peers, sp, st, off, read_drift())
    } else {
        (vec![], None, None, None, None)
    };

    Ok(TimeStatus {
        local_time: now.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
        utc_time: utc.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        utc_offset,
        epoch,
        boot_time,
        uptime_seconds: uptime,
        timezone,
        timezone_abbr,
        rtc_local,
        ntp: NtpStatus {
            enabled: ntp_enabled,
            running,
            sync_on_start,
            stratum,
            offset_ms,
            system_peer,
            peers,
            drift,
        },
    })
}

pub async fn status() -> ApiResult<Json<TimeStatus>> {
    let result = tokio::task::spawn_blocking(collect_status)
        .await
        .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;
    Ok(Json(result))
}

// ═══════════════════════════════════════════════════════════════════
//  Set datetime — PUT /api/time/datetime
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct SetDatetimeRequest {
    pub datetime: String, // ISO 8601 local: "2026-08-14T11:30:00"
}

pub async fn set_datetime(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetDatetimeRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    // Parse and validate the datetime
    let nd = chrono::NaiveDateTime::parse_from_str(&body.datetime, "%Y-%m-%dT%H:%M:%S")
        .map_err(|e| ApiError::BadRequest(format!("invalid datetime: {e}")))?;

    // Format as FreeBSD date(1) operand: CCYYMMDDHHMM.ss
    let formatted = nd.format("%Y%m%d%H%M.%S").to_string();

    let dt_str = body.datetime.clone();
    let result = tokio::task::spawn_blocking(move || {
        cmd::run_sync(DATE, &[&formatted])?;
        Ok::<_, ApiError>(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/time/datetime",
        200,
        Some(format!("set datetime to {dt_str}")),
    );

    let _ = result;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ═══════════════════════════════════════════════════════════════════
//  One-shot sync — POST /api/time/sync
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct SyncRequest {
    pub server: Option<String>,
}

pub async fn sync_now(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SyncRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let server = body.server.unwrap_or_else(|| "pool.ntp.org".into());
    // Validate hostname/IP
    if !server
        .chars()
        .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == ':')
    {
        return Err(ApiError::BadRequest("invalid server name".into()));
    }

    let srv = server.clone();
    // -K /dev/null: suppress KoD db warnings (the file may be absent on
    // systems where ntpd's package didn't create it).
    tokio::task::spawn_blocking(move || cmd::run_sync(SNTP, &["-K", "/dev/null", "-s", &srv]))
        .await
        .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/time/sync",
        200,
        Some(format!("sntp sync with {server}")),
    );

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ═══════════════════════════════════════════════════════════════════
//  Set timezone — PUT /api/time/timezone
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct SetTimezoneRequest {
    pub zone: String,
}

pub async fn set_timezone(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetTimezoneRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    // Validate zone name: no path traversal, must be a valid zoneinfo path
    let zone = &body.zone;
    if zone.starts_with('/')
        || zone.contains("..")
        || !zone
            .chars()
            .all(|c| c.is_alphanumeric() || c == '/' || c == '_' || c == '-' || c == '+')
    {
        return Err(ApiError::BadRequest("invalid timezone name".into()));
    }

    // Verify the zone file exists
    let zone_path = format!("{ZONEINFO_DIR}/{zone}");
    if !Path::new(&zone_path).exists() {
        return Err(ApiError::BadRequest(format!("timezone not found: {zone}")));
    }

    let zone_clone = zone.clone();
    tokio::task::spawn_blocking(move || cmd::run_sync(TZSETUP, &[&zone_clone]))
        .await
        .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/time/timezone",
        200,
        Some(format!("set timezone to {zone}")),
    );

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ═══════════════════════════════════════════════════════════════════
//  Set RTC mode — PUT /api/time/rtc-mode
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct RtcModeRequest {
    pub local: bool,
}

pub async fn set_rtc_mode(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<RtcModeRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let local = body.local;
    let mode_str = if local { "local" } else { "UTC" };

    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        if local {
            // Switch to local time mode
            std::fs::File::create(WALL_CMOS_CLOCK)?;
            cmd::run_sync(SYSCTL, &["machdep.wall_cmos_clock=1"])?;
            // adjkerntz adjusts the kernel timezone offset for local RTC
            let _ = cmd::run_sync(ADJKERNTZ, &["-a"]);
        } else {
            // Switch to UTC mode
            let _ = std::fs::remove_file(WALL_CMOS_CLOCK);
            cmd::run_sync(SYSCTL, &["machdep.wall_cmos_clock=0"])?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/time/rtc-mode",
        200,
        Some(format!("set RTC mode to {mode_str}")),
    );

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ═══════════════════════════════════════════════════════════════════
//  List zones — GET /api/time/zones
// ═══════════════════════════════════════════════════════════════════

pub async fn list_zones() -> ApiResult<Json<ZoneList>> {
    let regions = tokio::task::spawn_blocking(scan_zones)
        .await
        .map_err(|e| ApiError::Internal(format!("task join error: {e}")))?;
    Ok(Json(ZoneList { regions }))
}

// ═══════════════════════════════════════════════════════════════════
//  NTP config — GET /api/time/ntp/conf
// ═══════════════════════════════════════════════════════════════════

/// Parse `/etc/ntp.conf` to extract server/pool entries.
fn parse_ntp_conf(content: &str) -> Vec<ServerEntry> {
    let mut servers = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("server ") || trimmed.starts_with("pool ") {
            let mut parts = trimmed.splitn(3, char::is_whitespace);
            let kind = parts.next().unwrap_or("").to_string();
            let host = parts.next().unwrap_or("").to_string();
            let options = parts.next().unwrap_or("").trim().to_string();
            if !host.is_empty() {
                servers.push(ServerEntry { kind, host, options });
            }
        }
    }
    servers
}

pub async fn get_ntp_conf() -> ApiResult<Json<NtpConfig>> {
    let result = tokio::task::spawn_blocking(|| -> ApiResult<NtpConfig> {
        let raw = std::fs::read_to_string(NTP_CONF)
            .map_err(|e| ApiError::Internal(format!("failed to read ntp.conf: {e}")))?;
        let servers = parse_ntp_conf(&raw);
        let sync_on_start = sysrc::is_yes("ntpd_sync_on_start");
        Ok(NtpConfig {
            servers,
            sync_on_start,
            raw,
        })
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;

    Ok(Json(result))
}

// ═══════════════════════════════════════════════════════════════════
//  Update NTP config — PUT /api/time/ntp/conf
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct UpdateNtpConfRequest {
    pub servers: Vec<ServerEntry>,
}

/// Rebuild ntp.conf: replace all server/pool lines with the new list,
/// preserving all other lines (comments, restrict, tos, leapfile, …).
fn rebuild_ntp_conf(original: &str, new_servers: &[ServerEntry]) -> String {
    let mut lines: Vec<String> = original.lines().map(|l| l.to_string()).collect();

    // Find insertion point = position of first server/pool line
    let first_pos = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with("server ") || t.starts_with("pool ")
    });

    // Remove all existing server/pool lines
    lines.retain(|l| {
        let t = l.trim_start();
        !t.starts_with("server ") && !t.starts_with("pool ")
    });

    // Generate new lines
    let new_lines: Vec<String> = new_servers
        .iter()
        .map(|s| {
            if s.options.is_empty() {
                format!("{} {}", s.kind, s.host)
            } else {
                format!("{} {} {}", s.kind, s.host, s.options)
            }
        })
        .collect();

    // Insert at the original position (or append at end if none existed)
    let insert_pos = first_pos.unwrap_or(lines.len());
    for (i, line) in new_lines.into_iter().enumerate() {
        lines.insert(insert_pos + i, line);
    }

    let mut result = lines.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

pub async fn set_ntp_conf(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<UpdateNtpConfRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    // Validate all server entries
    for s in &body.servers {
        if !matches!(s.kind.as_str(), "server" | "pool") {
            return Err(ApiError::BadRequest(format!("invalid server type: {}", s.kind)));
        }
        if s.host.is_empty() {
            return Err(ApiError::BadRequest("server host is empty".into()));
        }
        if !s
            .host
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == ':')
        {
            return Err(ApiError::BadRequest(format!("invalid hostname: {}", s.host)));
        }
    }

    let servers = body.servers.clone();
    let n_servers = servers.len();
    let state_for_backup = state.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        // Snapshot the current ntp.conf into the unified conf_backup/ dir
        // (non-blocking on failure).
        crate::backup::backup_file(&state_for_backup, NTP_CONF);
        let original = std::fs::read_to_string(NTP_CONF).unwrap_or_default();
        let new_content = rebuild_ntp_conf(&original, &servers);

        // Atomic write: temp file + rename (same directory)
        let tmp = "/etc/ntp.conf.fwp-tmp";
        std::fs::write(tmp, &new_content)?;
        std::fs::rename(tmp, NTP_CONF)?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;


    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/time/ntp/conf",
        200,
        Some(format!("updated ntp.conf ({n_servers} servers)")),
    );

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ═══════════════════════════════════════════════════════════════════
//  Toggle sync-on-start — PUT /api/time/ntp/sync-on-start
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Deserialize)]
pub struct SyncOnStartRequest {
    pub enabled: bool,
}

pub async fn set_sync_on_start(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SyncOnStartRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let value = if body.enabled { "YES" } else { "NO" };
    sysrc::set_async("ntpd_sync_on_start", value).await?;

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/time/ntp/sync-on-start",
        200,
        Some(format!("ntpd_sync_on_start={value}")),
    );

    Ok(Json(serde_json::json!({ "status": "ok" })))
}

// ═══════════════════════════════════════════════════════════════════
//  NTP service control — enable / disable / restart
// ═══════════════════════════════════════════════════════════════════

/// Enable ntpd: set rc.conf + start service.
pub async fn ntp_enable(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<serde_json::Value>> {
    sysrc::set_async("ntpd_enable", "YES").await?;
    let output = cmd::run(SERVICE, &["ntpd", "start"]).await.unwrap_or_else(|e| {
        // Service start may fail if already running — not fatal
        e.to_string()
    });

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/time/ntp/enable",
        200,
        Some("ntpd enabled + started".into()),
    );

    Ok(Json(serde_json::json!({ "status": "ok", "output": output.trim() })))
}

/// Disable ntpd: stop service + set rc.conf.
pub async fn ntp_disable(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<serde_json::Value>> {
    let output = cmd::run(SERVICE, &["ntpd", "stop"]).await.unwrap_or_else(|e| e.to_string());
    sysrc::set_async("ntpd_enable", "NO").await?;

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/time/ntp/disable",
        200,
        Some("ntpd stopped + disabled".into()),
    );

    Ok(Json(serde_json::json!({ "status": "ok", "output": output.trim() })))
}

/// Restart ntpd.
pub async fn ntp_restart(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<serde_json::Value>> {
    let output = cmd::run(SERVICE, &["ntpd", "restart"]).await?;

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/time/ntp/restart",
        200,
        Some("ntpd restarted".into()),
    );

    Ok(Json(serde_json::json!({ "status": "ok", "output": output.trim() })))
}

// ═══════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn test_parse_ntpq() {
        let sample = "\
     remote           refid      st t when poll reach   delay   offset  jitter
==============================================================================
 0.freebsd.pool. .POOL.          16 p    -   64    0    0.000   +0.000   0.000
*time.neu.edu.cn .GNSS.           1 u  739 1024  377   38.405   -1.045   1.299
+203.107.6.88    10.137.38.86     2 u  870 1024  177   26.458   +2.672   1.475
";
        let (peers, sys_peer, stratum, offset) = parse_ntpq(sample);
        assert_eq!(peers.len(), 2); // .POOL. entry skipped
        assert_eq!(peers[0].remote, "time.neu.edu.cn");
        assert_eq!(peers[0].state, "sync");
        assert_eq!(peers[0].stratum, 1);
        assert_eq!(peers[1].remote, "203.107.6.88");
        assert_eq!(peers[1].state, "candidate");
        assert_eq!(sys_peer.as_deref(), Some("time.neu.edu.cn"));
        assert_eq!(stratum, Some(2)); // peer stratum 1 + 1
        assert!((offset.unwrap() - (-1.045)).abs() < 0.01);
    }

    #[test]
    fn test_parse_ntpq_empty() {
        let (peers, sys_peer, stratum, offset) = parse_ntpq("");
        assert!(peers.is_empty());
        assert_eq!(sys_peer, None);
        assert_eq!(stratum, None);
        assert_eq!(offset, None);
    }

    #[test]
    fn test_rebuild_ntp_conf() {
        let original = "\
# Comment line
tos minclock 3 maxclock 6
pool 0.freebsd.pool.ntp.org iburst
server ntp.aliyun.com iburst
# Another comment
restrict default limited kod nomodify notrap noquery nopeer
";
        let new_servers = vec![
            ServerEntry {
                kind: "server".into(),
                host: "time.google.com".into(),
                options: "iburst".into(),
            },
            ServerEntry {
                kind: "pool".into(),
                host: "0.cn.pool.ntp.org".into(),
                options: String::new(),
            },
        ];
        let result = rebuild_ntp_conf(original, &new_servers);
        assert!(result.contains("# Comment line"));
        assert!(result.contains("tos minclock 3 maxclock 6"));
        assert!(result.contains("server time.google.com iburst"));
        assert!(result.contains("pool 0.cn.pool.ntp.org"));
        assert!(result.contains("restrict default limited"));
        assert!(!result.contains("ntp.aliyun.com"));
        assert!(!result.contains("0.freebsd.pool.ntp.org"));
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_rebuild_ntp_conf_no_existing() {
        let original = "# Only comments\ntos minclock 3\n";
        let new_servers = vec![ServerEntry {
            kind: "server".into(),
            host: "pool.ntp.org".into(),
            options: "iburst".into(),
        }];
        let result = rebuild_ntp_conf(original, &new_servers);
        assert!(result.contains("server pool.ntp.org iburst"));
        assert!(result.contains("# Only comments"));
    }

    #[test]
    fn test_parse_ntp_conf() {
        let content = "\
# Config
tos minclock 3
pool 0.freebsd.pool.ntp.org iburst
server ntp.aliyun.com iburst
server ntp1.aliyun.com
restrict default
";
        let servers = parse_ntp_conf(content);
        assert_eq!(servers.len(), 3);
        assert_eq!(servers[0].kind, "pool");
        assert_eq!(servers[0].host, "0.freebsd.pool.ntp.org");
        assert_eq!(servers[0].options, "iburst");
        assert_eq!(servers[2].kind, "server");
        assert_eq!(servers[2].host, "ntp1.aliyun.com");
        assert_eq!(servers[2].options, "");
    }
}
