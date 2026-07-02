//! sysctl management — enumerate kernel state variables via sysctl(3) C API.
//!
//! Walks the MIB tree using CTL_SYSCTL_NEXTNOSKIP, then for each OID retrieves its
//! name, type/kind flags, description, and current value — all through the
//! sysctl(3) syscall with zero text parsing and zero subprocess spawning.
//!
//! Determines which variables have been explicitly configured by parsing
//! `/etc/sysctl.conf`.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use libc::{c_int, c_uint};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::AppState;

// ---- FreeBSD sysctl(3) magic constants (from <sys/sysctl.h>) ----

const CTL_SYSCTL_NEXTNOSKIP: c_int = 7;
const CTL_SYSCTL_NAME: c_int = 1;
const CTL_SYSCTL_OIDFMT: c_int = 4;
const CTL_SYSCTL_OIDDESCR: c_int = 5;

const CTLTYPE_MASK: c_uint = 0xf;
const CTLTYPE_NODE: c_uint = 1;
const CTLFLAG_WR: c_uint = 0x4000_0000;
const CTLFLAG_TUN: c_uint = 0x0008_0000;

const CTL_MAXNAME: usize = 24;
const SYSCTL_CONF: &str = "/etc/sysctl.conf";
const MAX_BACKUPS: usize = 5;

// ---- Data model ----

#[derive(Debug, Serialize)]
pub struct SysctlEntry {
    pub name: String,
    pub value: Option<String>,
    #[serde(rename = "type")]
    pub typ: String,
    pub fmt: String,
    pub description: Option<String>,
    pub writable: bool,
    pub modified: bool,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

// ---- sysctl(3) FFI helpers ----

/// Call `sysctl({0, op, ...mib}, ...)` — the CTL_SYSCTL meta-queries.
/// Returns the raw response bytes.
fn sysctl_meta(op: c_int, mib: &[c_int]) -> Option<Vec<u8>> {
    let mut req = vec![0_i32, op];
    req.extend_from_slice(mib);
    let mut len: usize = 0;
    let rc = unsafe {
        libc::sysctl(
            req.as_mut_ptr(),
            req.len() as c_uint,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let rc = unsafe {
        libc::sysctl(
            req.as_mut_ptr(),
            req.len() as c_uint,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    Some(buf)
}

/// Read the value of a MIB directly via `sysctl(mib, ...)`.
fn sysctl_value(mib: &[c_int]) -> Option<Vec<u8>> {
    let mut mib = mib.to_vec();
    let mut len: usize = 0;
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as c_uint,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as c_uint,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    Some(buf)
}

// ---- Value formatting ----

/// Map a CTLTYPE_* constant to a human-readable type name.
fn type_name(typ: c_uint) -> &'static str {
    match typ {
        2 => "integer",
        3 => "string",
        4 => "s64",
        5 => "opaque",
        6 => "unsigned integer",
        7 => "long integer",
        8 => "unsigned long",
        9 => "uint64",
        0xa => "uint8",
        0xb => "uint16",
        0xc => "int8",
        0xd => "int16",
        0xe => "int32",
        0xf => "uint32",
        _ => "unknown",
    }
}

/// Format a raw sysctl value buffer into a display string, based on type.
fn format_value(buf: &[u8], typ: c_uint) -> Option<String> {
    fn read_int<T: Copy + Default + Into<i128>>(buf: &[u8]) -> Option<T> {
        if buf.len() < std::mem::size_of::<T>() {
            return None;
        }
        let mut arr = [0u8; 32];
        arr[..std::mem::size_of::<T>()].copy_from_slice(&buf[..std::mem::size_of::<T>()]);
        Some(unsafe { std::ptr::read_unaligned(arr.as_ptr() as *const T) })
    }

    match typ {
        3 => {
            // STRING: trim trailing NUL
            let end = buf.len();
            let s = if buf.ends_with(&[0]) { &buf[..end - 1] } else { buf };
            Some(String::from_utf8_lossy(s).into_owned())
        }
        2 => read_int::<i32>(buf).map(|v| v.to_string()),
        6 => read_int::<u32>(buf).map(|v| v.to_string()),
        7 => read_int::<i64>(buf).map(|v| v.to_string()),
        8 => read_int::<u64>(buf).map(|v| v.to_string()),
        4 => read_int::<i64>(buf).map(|v| v.to_string()),
        9 => read_int::<u64>(buf).map(|v| v.to_string()),
        0xa => read_int::<u8>(buf).map(|v| v.to_string()),
        0xb => read_int::<u16>(buf).map(|v| v.to_string()),
        0xc => read_int::<i8>(buf).map(|v| v.to_string()),
        0xd => read_int::<i16>(buf).map(|v| v.to_string()),
        0xe => read_int::<i32>(buf).map(|v| v.to_string()),
        0xf => read_int::<u32>(buf).map(|v| v.to_string()),
        5 => {
            // OPAQUE: show byte count; caller can use fmt for more detail
            Some(format!("opaque ({} bytes)", buf.len()))
        }
        _ => None,
    }
}

// ---- Tree walk ----

/// One OID entry collected during the MIB tree walk.
struct RawOid {
    mib: Vec<c_int>,
    name: String,
    kind: c_uint,
    fmt: String,
    description: Option<String>,
}

/// Walk the entire sysctl MIB tree via CTL_SYSCTL_NEXTNOSKIP, collecting metadata
/// for every OID (name, kind, format, description).
fn walk_tree() -> Vec<RawOid> {
    let mut result = Vec::new();
    let mut mib: Vec<c_int> = Vec::new();

    loop {
        // Query CTL_SYSCTL_NEXTNOSKIP to get the next MIB in sequence
        // (NEXTNOSKIP includes OIDs with CTLFLAG_SKIP, e.g. compat aliases).
        let next_buf = match sysctl_meta(CTL_SYSCTL_NEXTNOSKIP, &mib) {
            Some(b) => b,
            None => break,
        };
        // The returned buffer is an array of c_int representing the next MIB.
        let count = next_buf.len() / std::mem::size_of::<c_int>();
        if count == 0 || count > CTL_MAXNAME {
            break;
        }
        mib = (0..count)
            .map(|i| {
                let off = i * std::mem::size_of::<c_int>();
                i32::from_ne_bytes([
                    next_buf[off],
                    next_buf[off + 1],
                    next_buf[off + 2],
                    next_buf[off + 3],
                ])
            })
            .collect();

        // Get name.
        let name = sysctl_meta(CTL_SYSCTL_NAME, &mib)
            .and_then(|b| {
                let s = if b.ends_with(&[0]) { &b[..b.len() - 1] } else { &b };
                String::from_utf8(s.to_vec()).ok()
            })
            .unwrap_or_default();

        // Get kind + format string (OIDFMT returns: u32 kind, then fmt chars).
        let (kind, fmt) = sysctl_meta(CTL_SYSCTL_OIDFMT, &mib)
            .map(|b| {
                let k = if b.len() >= 4 {
                    c_uint::from_ne_bytes([b[0], b[1], b[2], b[3]])
                } else {
                    0
                };
                let f = if b.len() > 4 {
                    String::from_utf8_lossy(&b[4..]).into_owned()
                } else {
                    String::new()
                };
                (k, f)
            })
            .unwrap_or((0, String::new()));

        // Get description (may be empty).
        let description = sysctl_meta(CTL_SYSCTL_OIDDESCR, &mib).and_then(|b| {
            let s = if b.ends_with(&[0]) { &b[..b.len() - 1] } else { &b };
            String::from_utf8(s.to_vec()).ok().filter(|s| !s.is_empty())
        });

        result.push(RawOid {
            mib: mib.clone(),
            name,
            kind,
            fmt,
            description,
        });
    }

    result
}

/// Parse /etc/sysctl.conf, returning the set of variable names that have been
/// explicitly configured. Lines starting with `#` or empty lines are skipped.
fn parse_sysctl_conf() -> HashSet<String> {
    let content = match std::fs::read_to_string("/etc/sysctl.conf") {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let eq = l.find('=')?;
            Some(l[..eq].trim().to_string())
        })
        .collect()
}

// ---- Handler ----

/// GET /api/sysctl — list all sysctl variables with type, description, value,
/// writable flag, and modification status. Optional `?q=` filters by name.
pub async fn list(Query(q): Query<SearchQuery>) -> ApiResult<Json<Vec<SysctlEntry>>> {
    let raw = walk_tree();
    let modified = parse_sysctl_conf();

    let mut entries: Vec<SysctlEntry> = raw
        .into_iter()
        .filter(|r| r.kind & CTLTYPE_MASK != CTLTYPE_NODE)
        .map(|r| {
            let typ = r.kind & CTLTYPE_MASK;
            let name = r.name.clone();
            let fmt = r.fmt.clone();
            let desc = r.description.clone();
            let value = sysctl_value(&r.mib)
                .and_then(|buf| format_value(&buf, typ));
            SysctlEntry {
                name,
                value,
                typ: type_name(typ).to_string(),
                fmt,
                description: desc,
                writable: r.kind & CTLFLAG_WR != 0,
                modified: modified.contains(&r.name),
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    if let Some(ref query) = q.q {
        let ql = query.to_lowercase();
        entries.retain(|e| e.name.to_lowercase().contains(&ql));
    }

    Ok(Json(entries))
}

// ---- Write / persist / reset ----

/// Validate a sysctl OID name: `[a-zA-Z0-9._]+`, 1–128 chars.
fn validate_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 128 {
        return Err(ApiError::BadRequest("invalid sysctl name length".into()));
    }
    let re = Regex::new(r"^[a-zA-Z0-9._]+$").unwrap();
    if !re.is_match(name) {
        return Err(ApiError::BadRequest(
            "sysctl name must match [a-zA-Z0-9._]+".into(),
        ));
    }
    Ok(())
}

/// Reject values that could corrupt sysctl.conf (newlines / null bytes).
fn validate_value(value: &str) -> ApiResult<()> {
    if value.contains('\0') || value.contains('\n') || value.contains('\r') {
        return Err(ApiError::BadRequest(
            "value must not contain newlines or null bytes".into(),
        ));
    }
    Ok(())
}

/// Look up the MIB for a sysctl by name via CTL_SYSCTL_NAME2OID.
fn name_to_mib(name: &str) -> Option<Vec<c_int>> {
    let cname = std::ffi::CString::new(name).ok()?;
    let mib = [0_i32, 3]; // {CTL_SYSCTL, CTL_SYSCTL_NAME2OID}
    let mut buf = [0_i32; CTL_MAXNAME];
    let mut len = std::mem::size_of_val(&buf);
    let rc = unsafe {
        libc::sysctl(
            mib.as_ptr() as *mut libc::c_int,
            2,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            cname.as_ptr() as *const libc::c_void,
            cname.as_bytes().len() + 1, // include NUL
        )
    };
    if rc != 0 {
        return None;
    }
    let count = len / std::mem::size_of::<c_int>();
    Some(buf[..count].to_vec())
}

/// Set a sysctl value at runtime via `sysctl(3)` syscall.
/// The value string is parsed into bytes based on the OID's type.
fn set_runtime_value(mib: &[c_int], typ: c_uint, value: &str) -> ApiResult<()> {
    let mut mib = mib.to_vec();
    let buf = encode_value(value, typ)?;
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as c_uint,
            std::ptr::null_mut(),
            &mut 0,
            buf.as_ptr() as *const libc::c_void,
            buf.len(),
        )
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        return Err(ApiError::Command(format!(
            "sysctl set failed: {}",
            err
        )));
    }
    Ok(())
}

/// Encode a value string into bytes for sysctl(3), based on type.
fn encode_value(value: &str, typ: c_uint) -> ApiResult<Vec<u8>> {
    match typ {
        3 => {
            // STRING: NUL-terminated
            let mut buf = value.as_bytes().to_vec();
            buf.push(0);
            Ok(buf)
        }
        2 | 0xe => value
            .parse::<i32>()
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| ApiError::BadRequest(format!("expected integer, got '{value}'"))),
        6 | 0xf => value
            .parse::<u32>()
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| ApiError::BadRequest(format!("expected unsigned integer, got '{value}'"))),
        7 => value
            .parse::<i64>()
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| ApiError::BadRequest(format!("expected long integer, got '{value}'"))),
        8 => value
            .parse::<u64>()
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| ApiError::BadRequest(format!("expected unsigned long, got '{value}'"))),
        4 => value
            .parse::<i64>()
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| ApiError::BadRequest(format!("expected s64, got '{value}'"))),
        9 => value
            .parse::<u64>()
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| ApiError::BadRequest(format!("expected u64, got '{value}'"))),
        0xa => value
            .parse::<u8>()
            .map(|v| vec![v])
            .map_err(|_| ApiError::BadRequest(format!("expected uint8, got '{value}'"))),
        0xb => value
            .parse::<u16>()
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| ApiError::BadRequest(format!("expected uint16, got '{value}'"))),
        0xc => value
            .parse::<i8>()
            .map(|v| vec![v as u8])
            .map_err(|_| ApiError::BadRequest(format!("expected int8, got '{value}'"))),
        0xd => value
            .parse::<i16>()
            .map(|v| v.to_ne_bytes().to_vec())
            .map_err(|_| ApiError::BadRequest(format!("expected int16, got '{value}'"))),
        _ => Err(ApiError::BadRequest(format!(
            "cannot set sysctl of type '{}' ({})",
            type_name(typ),
            typ
        ))),
    }
}

/// Derive the sysctl.conf backup directory (sibling `sysctl-backup/`).
fn backup_dir(state: &AppState) -> std::path::PathBuf {
    state
        .config
        .paths
        .db
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/var/db/fwp"))
        .join("sysctl-backup")
}

/// Copy current /etc/sysctl.conf into `backup_dir`, then prune to newest 5.
fn backup_sysctl_conf(state: &AppState) {
    let dir = backup_dir(state);
    let ts = state.now_ts();
    let dest = dir.join(format!("sysctl.conf.{ts}"));
    if let Err(e) = fs::create_dir_all(&dir)
        .and_then(|_| fs::copy(SYSCTL_CONF, &dest).map(|_| ()))
    {
        tracing::warn!(error = %e, "sysctl.conf backup failed (non-blocking)");
        return;
    }
    prune_backups(&dir, "sysctl.conf.", MAX_BACKUPS);
}

/// Keep at most `max` backup files matching `prefix` in `dir`.
fn prune_backups(dir: &Path, prefix: &str, max: usize) {
    let mut entries: Vec<(u64, std::path::PathBuf)> = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for ent in rd.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if let Some(suffix) = name.strip_prefix(prefix) {
                if let Ok(ts) = suffix.parse::<u64>() {
                    entries.push((ts, ent.path()));
                }
            }
        }
    }
    if entries.len() <= max {
        return;
    }
    entries.sort_unstable_by_key(|(ts, _)| *ts);
    for (_, path) in entries.iter().take(entries.len() - max) {
        let _ = fs::remove_file(path);
    }
}

/// Atomically replace a system file (tmp + rename), keeping mode 0644.
fn atomic_write(path: &str, content: &str) -> ApiResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = format!("{path}.fwp.tmp");
    fs::write(&tmp, content)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Set or replace a `name=value` line in /etc/sysctl.conf, preserving comments
/// and other entries. If the name already exists, its value is updated;
/// otherwise a new line is appended.
fn upsert_sysctl_conf(name: &str, value: &str) -> ApiResult<()> {
    let content = fs::read_to_string(SYSCTL_CONF).unwrap_or_default();
    let target = format!("{name}={value}");
    let prefix = format!("{name}=");
    let mut found = false;
    let mut out_lines: Vec<String> = content
        .lines()
        .map(|l| {
            let trimmed = l.trim();
            if trimmed.starts_with(&prefix) && !trimmed.starts_with('#') {
                found = true;
                target.clone()
            } else {
                l.to_string()
            }
        })
        .collect();
    if !found {
        out_lines.push(target);
    }
    out_lines.push(String::new()); // trailing newline
    let out = out_lines.join("\n");
    atomic_write(SYSCTL_CONF, &out)
}

/// Remove all `name=value` lines for `name` from /etc/sysctl.conf, preserving
/// comments and other entries.
fn remove_from_sysctl_conf(name: &str) -> ApiResult<()> {
    let content = fs::read_to_string(SYSCTL_CONF).unwrap_or_default();
    let prefix = format!("{name}=");
    let out_lines: Vec<String> = content
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            !(trimmed.starts_with(&prefix) && !trimmed.starts_with('#'))
        })
        .map(|l| l.to_string())
        .collect();
    let mut out = out_lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    atomic_write(SYSCTL_CONF, &out)
}

#[derive(Debug, Deserialize)]
pub struct SetBody {
    pub value: String,
    pub persist: Option<bool>,
}

/// PUT /api/sysctl/{name} — set a sysctl value at runtime, optionally persisting
/// to /etc/sysctl.conf (with backup).
pub async fn set(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath(name): AxumPath<String>,
    Json(body): Json<SetBody>,
) -> ApiResult<Json<SysctlEntry>> {
    validate_name(&name)?;
    validate_value(&body.value)?;

    // Look up MIB and kind for this OID.
    let mib = name_to_mib(&name)
        .ok_or_else(|| ApiError::NotFound(format!("sysctl '{name}' not found")))?;

    // Get kind to check writable + type.
    let kind = sysctl_meta(CTL_SYSCTL_OIDFMT, &mib)
        .and_then(|b| {
            if b.len() >= 4 {
                Some(c_uint::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            } else {
                None
            }
        })
        .ok_or_else(|| ApiError::Internal("failed to query sysctl kind".into()))?;

    if kind & CTLFLAG_WR == 0 {
        return Err(ApiError::BadRequest(format!(
            "sysctl '{name}' is read-only"
        )));
    }

    let typ = kind & CTLTYPE_MASK;
    if typ == CTLTYPE_NODE {
        return Err(ApiError::BadRequest(format!(
            "sysctl '{name}' is a container node, not settable"
        )));
    }

    // Set runtime value.
    set_runtime_value(&mib, typ, &body.value)?;

    // Persist to sysctl.conf if requested.
    let persist = body.persist.unwrap_or(false);
    if persist {
        backup_sysctl_conf(&state);
        upsert_sysctl_conf(&name, &body.value)?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        &format!("/api/sysctl/{name}"),
        200,
        Some(format!(
            "set sysctl '{name}' = '{}'{}",
            body.value,
            if persist { " (persisted)" } else { "" }
        )),
    );

    // Read back current value.
    let current = sysctl_value(&mib).and_then(|buf| format_value(&buf, typ));
    let modified_set = parse_sysctl_conf();
    let is_modified = modified_set.contains(&name);

    Ok(Json(SysctlEntry {
        name,
        value: current,
        typ: type_name(typ).to_string(),
        fmt: String::new(),
        description: None,
        writable: true,
        modified: is_modified,
    }))
}

/// DELETE /api/sysctl/{name} — remove a sysctl from /etc/sysctl.conf (revert to
/// system default). The runtime value is NOT changed — that requires a reboot
/// or explicit re-set after removing the config entry.
pub async fn reset(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath(name): AxumPath<String>,
) -> ApiResult<StatusCode> {
    validate_name(&name)?;

    backup_sysctl_conf(&state);
    remove_from_sysctl_conf(&name)?;

    audit::record(
        &state,
        Some(&auth.username),
        "DELETE",
        &format!("/api/sysctl/{name}"),
        200,
        Some(format!("reset sysctl '{name}' (removed from sysctl.conf)")),
    );

    Ok(StatusCode::NO_CONTENT)
}
