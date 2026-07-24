//! rc.conf management.
//!
//! Two-layer design:
//!
//! - **Reads**: directly parse rc.conf files via `std::fs` (<1ms, no subprocess).
//! - **Writes**: delegate to `/usr/sbin/sysrc` (correct quoting, syntax check, file selection).
//!
//! See `docs/impl/26-sysrc.md` for rationale.

use std::collections::HashMap;

use crate::cmd;
use crate::error::{ApiError, ApiResult};

const SYSRC: &str = "/usr/sbin/sysrc";

// ═══════════════════════════════════════════════════════════════════
//  Reads — direct file parsing (no subprocess)
// ═══════════════════════════════════════════════════════════════════

/// Read a single rc.conf key. Returns `None` if unset, empty, or `"NO"`.
///
/// Reads rc.conf files directly (<1ms). The `"NO"` filter matches FreeBSD
/// rc.conf semantics: unset variables resolve to `"NO"` via defaults.
pub fn get(key: &str) -> Option<String> {
    get_raw(key).filter(|s| !s.is_empty() && s != "NO")
}

/// Read a single rc.conf key without `"NO"` filtering.
/// Used by `get_async` and callers that need to see literal values.
fn get_raw(key: &str) -> Option<String> {
    read_rcconf_files().get(key).cloned()
}

/// Check if a key is set to `"YES"`.
pub fn is_yes(key: &str) -> bool {
    get(key).as_deref() == Some("YES")
}

/// Read a space-separated list value (e.g. `jail_list`, `vm_list`, `cloned_interfaces`).
pub fn get_list(key: &str) -> Vec<String> {
    get(key)
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// Read all rc.conf variables as a `HashMap`.
/// Matches FreeBSD's `source_rc_confs()` resolution order.
pub fn read_rcconf_files() -> HashMap<String, String> {
    let mut map = HashMap::new();

    // Resolve rc_conf_files and source them (with re-scan for overrides).
    let mut files = resolve_rc_conf_files();
    let mut sourced: Vec<String> = Vec::new();
    for pass in 0..2 {
        let mut rc_conf_files_changed = false;
        for file in &files {
            if sourced.contains(file) {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(file) {
                let before = map.get("rc_conf_files").cloned();
                merge_rcconf_lines(&mut map, &content);
                if pass == 0 {
                    if let Some(after) = map.get("rc_conf_files") {
                        if before.as_deref() != Some(after.as_str()) && !after.is_empty() {
                            rc_conf_files_changed = true;
                        }
                    }
                }
                sourced.push(file.clone());
            }
        }
        if !rc_conf_files_changed {
            break;
        }
        if let Some(new_list) = map.get("rc_conf_files") {
            files = new_list.split_whitespace().map(String::from).collect();
        }
    }

    // /etc/rc.conf.d/* (lowest priority, sourced per-service by rc.d scripts).
    if let Ok(entries) = std::fs::read_dir("/etc/rc.conf.d") {
        let mut dfiles: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        dfiles.sort_by_key(|e| e.file_name());
        for entry in dfiles {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                merge_rcconf_lines(&mut map, &content);
            }
        }
    }

    map
}

// ═══════════════════════════════════════════════════════════════════
//  Writes — via sysrc subprocess
// ═══════════════════════════════════════════════════════════════════

/// Set a single `key=value`. Returns `Err(String)` on failure.
pub fn set(key: &str, value: &str) -> Result<(), String> {
    let assignment = format!("{key}={value}");
    cmd::run_sync_str(SYSRC, &[&assignment]).map(|_| ())
}

/// Set multiple `key=value` pairs in a single sysrc call.
///
/// Reduces N subprocess spawns to 1. Each pair becomes a `KEY=VALUE`
/// argument to sysrc (e.g. `sysrc firewall_enable=YES firewall_quiet=YES`).
pub fn set_multi(items: &[(&str, &str)]) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }
    let args: Vec<String> = items.iter().map(|(k, v)| format!("{k}={v}")).collect();
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    cmd::run_sync_str(SYSRC, &arg_refs).map(|_| ())
}

/// Set `key=value`, ignoring errors (fire-and-forget).
pub fn set_forget(key: &str, value: &str) {
    let assignment = format!("{key}={value}");
    cmd::run_forget_sync(SYSRC, &[&assignment]);
}

/// Delete a key (fire-and-forget).
pub fn delete(key: &str) {
    cmd::run_forget_sync(SYSRC, &["-x", key]);
}

/// Ensure a key is set to `"YES"`. No-op if already set.
///
/// Reads from file (<1ms); only spawns sysrc when a write is actually needed.
pub fn ensure_yes(key: &str) {
    if !is_yes(key) {
        set_forget(key, "YES");
    }
}

/// Ensure a key is set to `"NO"`. No-op if already `"NO"` or unset.
///
/// Reads from file (<1ms); only spawns sysrc when a write is actually needed.
pub fn ensure_no(key: &str) {
    if get(key).is_some() {
        set_forget(key, "NO");
    }
}

// ── list helpers (idempotent) ──────────────────────────────────────

/// Add an item to a space-separated list key. No-op if already present.
pub fn list_add(key: &str, item: &str) -> Result<(), String> {
    let mut list = get_list(key);
    if list.iter().any(|i| i == item) {
        return Ok(());
    }
    list.push(item.to_string());
    set(key, &list.join(" "))
}

/// Remove an item from a space-separated list key.
/// Deletes the key entirely if the list becomes empty.
pub fn list_remove(key: &str, item: &str) -> Result<(), String> {
    let mut list = get_list(key);
    if !list.iter().any(|i| i == item) {
        return Ok(());
    }
    list.retain(|i| i != item);
    if list.is_empty() {
        delete(key);
    } else {
        set(key, &list.join(" "))?;
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════
//  Async variants (for axum handlers)
// ═══════════════════════════════════════════════════════════════════

/// Async read a single key (no subprocess — delegates to `get_raw`).
/// Does NOT filter `"NO"` (callers like rcconf CRUD need literal values).
pub async fn get_async(key: &str) -> ApiResult<String> {
    let key = key.to_string();
    let result = tokio::task::spawn_blocking(move || get_raw(&key))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    result.ok_or_else(|| ApiError::NotFound("rc.conf key not set".into()))
}

/// Async set `key=value` via sysrc.
pub async fn set_async(key: &str, value: &str) -> ApiResult<()> {
    let assignment = format!("{key}={value}");
    cmd::run(SYSRC, &[&assignment]).await?;
    Ok(())
}

/// Async delete a key via sysrc.
pub async fn delete_async(key: &str) -> ApiResult<()> {
    cmd::run(SYSRC, &["-x", key]).await?;
    Ok(())
}

/// Async read all non-default rc.conf variables (no subprocess).
pub async fn list_all_async() -> ApiResult<HashMap<String, String>> {
    tokio::task::spawn_blocking(read_rcconf_files)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))
}

// ═══════════════════════════════════════════════════════════════════
//  File parsing
// ═══════════════════════════════════════════════════════════════════

/// Parse `rc_conf_files` from `/etc/defaults/rc.conf`.
/// Falls back to `/etc/rc.conf /etc/rc.conf.local`.
fn resolve_rc_conf_files() -> Vec<String> {
    if let Ok(defaults) = std::fs::read_to_string("/etc/defaults/rc.conf") {
        for line in defaults.lines() {
            let line = line.trim();
            if line.starts_with("rc_conf_files=") {
                let val = &line["rc_conf_files=".len()..];
                let unquoted = val.trim();
                let unquoted = if unquoted.len() >= 2
                    && unquoted.starts_with('"')
                    && unquoted.ends_with('"')
                {
                    &unquoted[1..unquoted.len() - 1]
                } else {
                    unquoted
                };
                let files: Vec<String> = unquoted
                    .split_whitespace()
                    .map(String::from)
                    .collect();
                if !files.is_empty() {
                    return files;
                }
            }
        }
    }
    vec!["/etc/rc.conf".into(), "/etc/rc.conf.local".into()]
}

fn merge_rcconf_lines(map: &mut HashMap<String, String>, content: &str) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = parse_export_line(line) {
            map.insert(k, v);
        }
    }
}

/// Parse one line of rc.conf (`KEY="VALUE"` or `KEY=VALUE`).
fn parse_export_line(line: &str) -> Option<(String, String)> {
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
    Some((key, value))
}

/// Reverse shell-style escaping (`\"` → `"`, `\\` → `\`).
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
