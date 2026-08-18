//! Unified config-file backups.
//!
//! Before the panel rewrites a system config file (resolv.conf, sysctl.conf,
//! ntp.conf, jail.conf, crontab, …) it snapshots the current copy into the
//! shared `conf_backup/` directory (sibling of the configured DB path, e.g.
//! `/var/db/fwp/conf_backup/`) as `<file-name>.<unix-seconds>`, keeping the
//! most recent [`MAX_BACKUPS`] copies per file name.
//!
//! Backups are non-blocking by design: a failed backup is logged and never
//! aborts the edit (a missing backup beats a blocked write).

use std::path::{Path, PathBuf};

use tracing::warn;

use crate::state::AppState;

/// Maximum backup copies retained per file name.
const MAX_BACKUPS: usize = 5;

/// The unified backup directory: `<db-dir>/conf_backup/`.
fn backup_dir(state: &AppState) -> PathBuf {
    state
        .config
        .paths
        .db
        .parent()
        .unwrap_or_else(|| Path::new("/var/db/fwp"))
        .join("conf_backup")
}

/// Snapshot an on-disk file into the backup dir (no-op if it doesn't exist).
pub fn backup_file(state: &AppState, path: &str) {
    let src = Path::new(path);
    if !src.exists() {
        return;
    }
    let Some(name) = src.file_name() else { return };
    let name = name.to_string_lossy();
    let dir = backup_dir(state);
    let dest = dir.join(format!("{name}.{}", state.now_ts()));
    if let Err(e) = std::fs::create_dir_all(&dir)
        .and_then(|_| std::fs::copy(src, &dest).map(|_| ()))
    {
        warn!(error = %e, file = %name, "config backup failed (non-blocking)");
        return;
    }
    prune(&dir, &format!("{name}."));
}

/// Snapshot in-memory content (e.g. a user crontab read via `crontab -l`)
/// under the given file name. Empty content is skipped: nothing to back up.
pub fn backup_content(state: &AppState, name: &str, content: &str) {
    if content.is_empty() {
        return;
    }
    let dir = backup_dir(state);
    let dest = dir.join(format!("{name}.{}", state.now_ts()));
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&dest, content)) {
        warn!(error = %e, file = name, "config backup failed (non-blocking)");
        return;
    }
    prune(&dir, &format!("{name}."));
}

/// Keep at most [`MAX_BACKUPS`] backup files matching `prefix` in `dir`,
/// deleting the oldest ones.
fn prune(dir: &Path, prefix: &str) {
    let mut entries: Vec<(u64, PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if let Some(ts) = name
                .strip_prefix(prefix)
                .and_then(|s| s.parse::<u64>().ok())
            {
                entries.push((ts, ent.path()));
            }
        }
    }
    if entries.len() <= MAX_BACKUPS {
        return;
    }
    entries.sort_unstable_by_key(|(ts, _)| *ts);
    for (_, path) in entries.iter().take(entries.len() - MAX_BACKUPS) {
        let _ = std::fs::remove_file(path);
    }
}
