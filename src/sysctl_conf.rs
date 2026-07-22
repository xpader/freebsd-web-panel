//! /etc/sysctl.conf file editing.
//!
//! Shared between the sysctl handler and other modules (e.g. firewall)
//! that need to persist kernel tunables to sysctl.conf.

use std::fs;

use crate::error::ApiResult;

const SYSCTL_CONF: &str = "/etc/sysctl.conf";

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
pub fn upsert(name: &str, value: &str) -> ApiResult<()> {
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
pub fn remove(name: &str) -> ApiResult<()> {
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
