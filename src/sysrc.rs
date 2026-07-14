//! rc.conf management via `sysrc`.
//!
//! All sysrc operations across the codebase go through this module.
//! Provides sync variants (for `spawn_blocking` contexts) and async variants
//! (for direct use in axum handlers).

use std::collections::HashMap;

use crate::cmd;
use crate::error::ApiResult;

const SYSRC: &str = "/usr/sbin/sysrc";

// ── sync variants (call from spawn_blocking) ───────────────────────

/// Read a single rc.conf key. Returns `None` if unset or empty.
pub fn get(key: &str) -> Option<String> {
    let s = cmd::run_sync(SYSRC, &["-n", key]).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() || s == "NO" {
        None
    } else {
        Some(s)
    }
}

/// Set an rc.conf key=value. Returns `Err(String)` on failure.
pub fn set(key: &str, value: &str) -> Result<(), String> {
    let assignment = format!("{key}={value}");
    cmd::run_sync_str(SYSRC, &[&assignment]).map(|_| ())
}

/// Set an rc.conf key=value, ignoring errors (fire-and-forget).
pub fn set_forget(key: &str, value: &str) {
    let assignment = format!("{key}={value}");
    cmd::run_forget_sync(SYSRC, &[&assignment]);
}

/// Delete an rc.conf key (fire-and-forget).
pub fn delete(key: &str) {
    cmd::run_forget_sync(SYSRC, &["-x", key]);
}

/// Ensure a key is set to `"YES"`. No-op if already set.
pub fn ensure_yes(key: &str) {
    if get(key).as_deref() != Some("YES") {
        set_forget(key, "YES");
    }
}

/// Read a space-separated list value (e.g. `jail_list`, `vm_list`).
pub fn get_list(key: &str) -> Vec<String> {
    get(key)
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// Read all non-default rc.conf variables as a `HashMap`.
pub fn list_all() -> HashMap<String, String> {
    match cmd::run_sync(SYSRC, &["-e", "-a"]) {
        Ok(s) => parse_export_lines(&s),
        Err(_) => HashMap::new(),
    }
}

// ── async variants (for axum handlers) ─────────────────────────────

/// Async read a single rc.conf key.
pub async fn get_async(key: &str) -> ApiResult<String> {
    let s = cmd::run(SYSRC, &["-n", key]).await?;
    Ok(s.trim_end().to_string())
}

/// Async set a key=value.
pub async fn set_async(key: &str, value: &str) -> ApiResult<()> {
    let assignment = format!("{key}={value}");
    cmd::run(SYSRC, &[&assignment]).await?;
    Ok(())
}

/// Async delete a key.
pub async fn delete_async(key: &str) -> ApiResult<()> {
    cmd::run(SYSRC, &["-x", key]).await?;
    Ok(())
}

/// Async read all non-default rc.conf variables.
pub async fn list_all_async() -> ApiResult<HashMap<String, String>> {
    let raw = cmd::run(SYSRC, &["-e", "-a"]).await?;
    Ok(parse_export_lines(&raw))
}

// ── parsing ────────────────────────────────────────────────────────

/// Parse `sysrc -e -a` output into a `HashMap`.
fn parse_export_lines(raw: &str) -> HashMap<String, String> {
    raw.lines()
        .filter(|l| !l.is_empty())
        .filter_map(parse_export_line)
        .collect()
}

/// Parse one line of `sysrc -e` output (`KEY="VALUE"` or `KEY=VALUE`).
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
