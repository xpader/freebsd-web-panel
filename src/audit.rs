//! Append-only structured audit log for all write operations.

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: i64,
    pub user: Option<String>,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub detail: Option<String>,
}

/// Shared append-only audit log writer.
pub struct AuditLog {
    file: Mutex<std::fs::File>,
    path: Box<Path>,
}

impl AuditLog {
    pub fn open(path: &Path) -> std::io::Result<Arc<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Arc::new(AuditLog {
            file: Mutex::new(file),
            path: path.into(),
        }))
    }

    pub fn write(&self, entry: &AuditEntry) {
        let line = serde_json::to_string(entry).unwrap_or_else(|_| "{}".into());
        use std::io::Write;
        let mut f = self.file.lock();
        if let Err(e) = writeln!(f, "{line}") {
            tracing::warn!(error = %e, "audit log write failed");
        }
    }

    /// Read the last `limit` entries from the log file.
    pub fn read_recent(&self, limit: usize) -> std::io::Result<Vec<AuditEntry>> {
        let raw = std::fs::read_to_string(&self.path)?;
        let mut entries: Vec<AuditEntry> = raw
            .lines()
            .filter(|l| !l.is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        entries.reverse();
        entries.truncate(limit);
        Ok(entries)
    }
}

/// Convenience to record an audit entry from within a handler.
pub fn record(
    state: &AppState,
    user: Option<&str>,
    method: &str,
    path: &str,
    status: u16,
    detail: Option<String>,
) {
    if let Some(log) = &state.audit {
        log.write(&AuditEntry {
            ts: state.now_ts(),
            user: user.map(str::to_owned),
            method: method.into(),
            path: path.into(),
            status,
            detail,
        });
    }
}
