//! Rsync sync-task management — HTTP handlers.
//!
//! Manages user-defined rsync sync tasks (push/pull/local) stored in SQLite,
//! plus an init step that installs the `rsync` pkg (not in FreeBSD base).
//! Pattern follows smb init: `GET /status` + `POST /init` streaming task, and
//! reuses the unified bgtask SSE endpoint for both init and manual sync runs.

use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use regex::Regex;
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::bgtask;
use crate::cmd;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const RSYNC: &str = "/usr/local/bin/rsync";
const PKG: &str = "/usr/sbin/pkg";
const RSYNC_PKG: &str = "rsync";

// ── Status ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RsyncStatus {
    pub installed: bool,
    pub version: Option<String>,
}

pub fn check_status() -> RsyncStatus {
    let installed = Path::new(RSYNC).exists();
    let version = if installed {
        cmd::run_sync(RSYNC, &["--version"])
            .ok()
            .and_then(|s| {
                // First line: "rsync  version 3.4.1  protocol version 32"
                s.lines().next().and_then(|l| {
                    l.split_whitespace()
                        .skip_while(|w| !w.eq_ignore_ascii_case("version"))
                        .nth(1) // skip "version" itself, take the number
                        .map(str::to_owned)
                })
            })
    } else {
        None
    };
    RsyncStatus { installed, version }
}

/// GET /api/rsync/status
pub async fn status() -> ApiResult<Json<RsyncStatus>> {
    let s = tokio::task::spawn_blocking(check_status)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(s))
}

// ── Init ────────────────────────────────────────────────────────────

/// POST /api/rsync/init — streaming background task that installs rsync.
pub async fn init(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<serde_json::Value>> {
    let st = tokio::task::spawn_blocking(check_status)
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    if st.installed {
        return Err(ApiError::Conflict("rsync already installed".into()));
    }

    let id = bgtask::create("rsync-init", "Install rsync");
    let tid = id.clone();
    let state2 = state.clone();
    let username = user.username.clone();
    tokio::spawn(async move {
        bgtask::push_line(&tid, "=== Installing rsync ===");
        let exit =
            bgtask::run_streaming_cmd(&tid, PKG, &["install", "-y", RSYNC_PKG]).await;
        let ok = exit == 0;
        let msg = if ok {
            "rsync installed.".to_string()
        } else {
            format!("Installation failed (exit code {exit}).")
        };
        bgtask::push_line(&tid, &msg);
        bgtask::set_status(
            &tid,
            if ok {
                bgtask::TaskStatus::Done
            } else {
                bgtask::TaskStatus::Failed
            },
            Some(exit),
        );
        audit::record(
            &state2,
            Some(&username),
            "POST",
            "/api/rsync/init",
            if ok { 200 } else { 500 },
            Some(if ok {
                "rsync installed".into()
            } else {
                "rsync install failed".into()
            }),
        );
    });
    Ok(Json(serde_json::json!({ "task_id": id })))
}

// ── Data model ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RsyncTask {
    pub id: i64,
    pub description: String,
    pub source: String,
    pub dest: String,
    pub archive: bool,
    pub compress: bool,
    pub delete: bool,
    pub verbose: bool,
    pub port: Option<i32>,
    pub extra_args: String,
    pub run_user: String,
    pub cron_expr: String,
    pub cron_enabled: bool,
    pub last_run_at: Option<i64>,
    pub last_status: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

const SELECT_COLS: &str = "id, description, source, dest, archive, compress, \"delete\", \
     verbose, port, extra_args, run_user, cron_enabled, cron_expr, \
     last_run_at, last_status, created_at, updated_at";

fn row_to_task(r: &Row) -> rusqlite::Result<RsyncTask> {
    Ok(RsyncTask {
        id: r.get(0)?,
        description: r.get(1)?,
        source: r.get(2)?,
        dest: r.get(3)?,
        archive: r.get::<_, i64>(4)? != 0,
        compress: r.get::<_, i64>(5)? != 0,
        delete: r.get::<_, i64>(6)? != 0,
        verbose: r.get::<_, i64>(7)? != 0,
        port: r.get::<_, Option<i64>>(8)?.map(|v| v as i32),
        extra_args: r.get(9)?,
        run_user: r.get(10)?,
        cron_enabled: r.get::<_, i64>(11)? != 0,
        cron_expr: r.get(12)?,
        last_run_at: r.get(13)?,
        last_status: r.get(14)?,
        created_at: r.get(15)?,
        updated_at: r.get(16)?,
    })
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskReq {
    pub description: String,
    pub source: String,
    pub dest: String,
    pub archive: Option<bool>,
    pub compress: Option<bool>,
    pub delete: Option<bool>,
    pub verbose: Option<bool>,
    pub port: Option<i32>,
    pub extra_args: Option<String>,
    pub run_user: Option<String>,
    pub cron_expr: Option<String>,
    pub cron_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct RunReq {
    pub dry_run: Option<bool>,
}

// ── Validation ──────────────────────────────────────────────────────


fn ensure_installed() -> ApiResult<()> {
    if !Path::new(RSYNC).exists() {
        return Err(ApiError::Conflict("rsync is not installed".into()));
    }
    Ok(())
}

fn validate_description(desc: &str) -> ApiResult<()> {
    if desc.is_empty() {
        return Err(ApiError::BadRequest("description must not be empty".into()));
    }
    if desc.len() > 128 {
        return Err(ApiError::BadRequest("description too long (max 128 chars)".into()));
    }
    if desc.chars().any(|c| c < ' ' || c == '\x7f') {
        return Err(ApiError::BadRequest("description contains control characters".into()));
    }
    Ok(())
}

/// Reject control characters and NUL. Path specs may be local (absolute) or
/// remote (`user@host:path`); we do not over-restrict the syntax.
fn validate_path_spec(p: &str) -> ApiResult<()> {
    if p.is_empty() {
        return Err(ApiError::BadRequest("path must not be empty".into()));
    }
    if p.chars().any(|c| c < ' ' || c == '\x7f') {
        return Err(ApiError::BadRequest("path contains control characters".into()));
    }
    Ok(())
}

fn validate_extra_args(s: &str) -> ApiResult<()> {
    if s.chars().any(|c| c < ' ' || c == '\x7f') {
        return Err(ApiError::BadRequest(
            "extra args contain control characters".into(),
        ));
    }
    Ok(())
}

/// A run-as username (system crontab `who` column): ASCII, no control chars,
/// max 32. Empty means root (default).
fn validate_run_user(u: &str) -> ApiResult<()> {
    if u.is_empty() {
        return Ok(());
    }
    if u.len() > 32 || u.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')) {
        return Err(ApiError::BadRequest("run_user must be a valid username".into()));
    }
    Ok(())
}

/// A 5-field cron expression (minute hour dom month dow). Empty means "not
/// scheduled". Each field allows only digits and the cron metacharacters
/// `* / , -`.
fn validate_cron_expr(expr: &str) -> ApiResult<()> {
    if expr.is_empty() {
        return Ok(());
    }
    let fields: Vec<&str> = expr.split_whitespace().collect();
    // @-style special aliases (single token like @daily, @hourly).
    const SPECIALS: &[&str] = &[
        "@reboot", "@yearly", "@annually", "@monthly",
        "@weekly", "@daily", "@midnight", "@hourly",
    ];
    if fields.len() == 1 && SPECIALS.contains(&fields[0]) {
        return Ok(());
    }
    if fields.len() != 5 {
        return Err(ApiError::BadRequest("cron_expr must have 5 fields or a @-alias".into()));
    }
    for f in fields {
        if !f.chars().all(|c| c.is_ascii_digit() || c == '*' || c == '/' || c == ',' || c == '-') {
            return Err(ApiError::BadRequest("cron field has invalid characters".into()));
        }
    }
    Ok(())
}


/// A path spec is "remote" if it does not start with `/` and contains `:`
/// (matches rsync's own `host:path` / `host::module` detection).
fn is_remote(p: &str) -> bool {
    !p.starts_with('/') && p.contains(':')
}

// ── DB helpers ──────────────────────────────────────────────────────

fn list_tasks_db(conn: &Connection) -> ApiResult<Vec<RsyncTask>> {
    let mut stmt = conn.prepare(&format!("SELECT {SELECT_COLS} FROM rsync_tasks ORDER BY id"))?;
    let rows = stmt.query_map([], row_to_task)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn get_task_db(conn: &Connection, id: i64) -> ApiResult<Option<RsyncTask>> {
    let mut stmt =
        conn.prepare(&format!("SELECT {SELECT_COLS} FROM rsync_tasks WHERE id = ?1"))?;
    let mut rows = stmt.query_map(params![id], row_to_task)?;
    match rows.next() {
        Some(r) => Ok(Some(r?)),
        None => Ok(None),
    }
}

fn update_run_result(conn: &Connection, id: i64, ts: i64, status: &str) -> ApiResult<()> {
    conn.execute(
        "UPDATE rsync_tasks SET last_run_at = ?1, last_status = ?2 WHERE id = ?3",
        params![ts, status, id],
    )?;
    Ok(())
}

// ── CRUD handlers ───────────────────────────────────────────────────

/// GET /api/rsync/tasks
pub async fn list_tasks(State(state): State<AppState>) -> ApiResult<Json<Vec<RsyncTask>>> {
    let conn = state.db.lock().await;
    Ok(Json(list_tasks_db(&conn)?))
}

/// POST /api/rsync/tasks
pub async fn create_task(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateTaskReq>,
) -> ApiResult<(StatusCode, Json<RsyncTask>)> {
    ensure_installed()?;
    validate_description(&req.description)?;
    validate_path_spec(&req.source)?;
    validate_path_spec(&req.dest)?;
    validate_extra_args(req.extra_args.as_deref().unwrap_or(""))?;
    let run_user = req.run_user.as_deref().unwrap_or("").trim().to_string();
    validate_run_user(&run_user)?;
    let cron_expr = req.cron_expr.as_deref().unwrap_or("").trim().to_string();
    validate_cron_expr(&cron_expr)?;
    if let Some(port) = req.port {
        if port != 0 && !(1..=65535).contains(&port) {
            return Err(ApiError::BadRequest("port must be 1-65535".into()));
        }
    }
    let port = req.port.filter(|&p| p != 0);
    let now = state.now_ts();

    let id = {
        let conn = state.db.lock().await;
        conn.execute(
            "INSERT INTO rsync_tasks \
             (description, source, dest, archive, compress, \"delete\", verbose, \
              port, extra_args, run_user, cron_enabled, cron_expr, created_at, updated_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                req.description,
                req.source,
                req.dest,
                req.archive.unwrap_or(true) as i64,
                req.compress.unwrap_or(false) as i64,
                req.delete.unwrap_or(false) as i64,
                req.verbose.unwrap_or(true) as i64,
                port,
                req.extra_args.unwrap_or_default(),
                run_user,
                req.cron_enabled.unwrap_or(false) as i64,
                cron_expr,
                now,
                now,
            ],
        )?;
        conn.last_insert_rowid()
    };
    audit::record(
        &state,
        Some(&user.username),
        "POST",
        "/api/rsync/tasks",
        201,
        Some(format!("created rsync task '{}'", req.description)),
    );
    let conn = state.db.lock().await;
    let task = get_task_db(&conn, id)?
        .ok_or_else(|| ApiError::Internal("created task not found".into()))?;
    // Reflect the schedule in /etc/crontab after the row is committed.
    sync_cron(&task)?;
    Ok((StatusCode::CREATED, Json(task)))
}

/// PUT /api/rsync/tasks/{id}
pub async fn update_task(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
    Json(req): Json<CreateTaskReq>,
) -> ApiResult<Json<RsyncTask>> {
    ensure_installed()?;
    validate_description(&req.description)?;
    validate_path_spec(&req.source)?;
    validate_path_spec(&req.dest)?;
    validate_extra_args(req.extra_args.as_deref().unwrap_or(""))?;
    let run_user = req.run_user.as_deref().unwrap_or("").trim().to_string();
    validate_run_user(&run_user)?;
    let cron_expr = req.cron_expr.as_deref().unwrap_or("").trim().to_string();
    validate_cron_expr(&cron_expr)?;
    if let Some(port) = req.port {
        if port != 0 && !(1..=65535).contains(&port) {
            return Err(ApiError::BadRequest("port must be 1-65535".into()));
        }
    }
    let port = req.port.filter(|&p| p != 0);
    let now = state.now_ts();

    {
        let conn = state.db.lock().await;
        let affected = conn.execute(
            "UPDATE rsync_tasks SET \
             description=?1, source=?2, dest=?3, archive=?4, compress=?5, \
             \"delete\"=?6, verbose=?7, port=?8, extra_args=?9, \
             run_user=?10, cron_enabled=?11, cron_expr=?12, updated_at=?13 \
             WHERE id=?14",
            params![
                req.description,
                req.source,
                req.dest,
                req.archive.unwrap_or(true) as i64,
                req.compress.unwrap_or(false) as i64,
                req.delete.unwrap_or(false) as i64,
                req.verbose.unwrap_or(true) as i64,
                port,
                req.extra_args.unwrap_or_default(),
                run_user,
                req.cron_enabled.unwrap_or(false) as i64,
                cron_expr,
                now,
                id,
            ],
        )?;
        if affected == 0 {
            return Err(ApiError::NotFound("rsync task not found".into()));
        }
    }
    audit::record(
        &state,
        Some(&user.username),
        "PUT",
        &format!("/api/rsync/tasks/{id}"),
        200,
        Some(format!("updated rsync task '{}'", req.description)),
    );
    let conn = state.db.lock().await;
    let task = get_task_db(&conn, id)?
        .ok_or_else(|| ApiError::Internal("updated task not found".into()))?;
    // Reflect the (possibly changed) schedule in /etc/crontab.
    sync_cron(&task)?;
    Ok(Json(task))
}

/// DELETE /api/rsync/tasks/{id}
pub async fn delete_task(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
) -> ApiResult<Json<serde_json::Value>> {
    let desc = {
        let conn = state.db.lock().await;
        let task = get_task_db(&conn, id)?
            .ok_or_else(|| ApiError::NotFound("rsync task not found".into()))?;
        conn.execute("DELETE FROM rsync_tasks WHERE id=?1", params![id])?;
        task.description
    };
    // Remove the block from /etc/crontab (no-op if unscheduled).
    remove_cron_block(id)?;
    audit::record(
        &state,
        Some(&user.username),
        "DELETE",
        &format!("/api/rsync/tasks/{id}"),
        200,
        Some(format!("deleted rsync task '{desc}'")),
    );
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Run handler ─────────────────────────────────────────────────────

/// Build the rsync argv from a task definition.
///
/// Options are emitted as separate argv elements (never a shell string), so
/// there is no shell-injection surface. `extra_args` is split on whitespace
/// into individual argv elements. When either endpoint is remote (`host:path`),
/// an SSH wrapper with `BatchMode=yes` is injected so the task fails fast
/// instead of hanging on an interactive password prompt.
fn build_rsync_args(task: &RsyncTask, dry_run: bool) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if task.archive {
        args.push("-a".into());
    }
    if task.verbose {
        args.push("-v".into());
    }
    if task.compress {
        args.push("-z".into());
    }
    if task.delete {
        args.push("--delete".into());
    }
    if dry_run {
        args.push("-n".into());
    }
    if is_remote(&task.source) || is_remote(&task.dest) {
        let mut ssh = String::from("ssh -o BatchMode=yes");
        if let Some(port) = task.port {
            if port > 0 {
                ssh.push_str(&format!(" -p {port}"));
            }
        }
        args.push("-e".into());
        args.push(ssh);
    }
    for a in task.extra_args.split_whitespace() {
        args.push(a.into());
    }
    args.push(task.source.clone());
    args.push(task.dest.clone());
    args
}
// ── Cron scheduling ─────────────────────────────────────────────────
//
// A scheduled task is materialised in `/etc/crontab` as a two-line block:
//
//     # [fwp-rsync=<id>] <description>
//     <cron_expr> <who> /usr/local/bin/rsync <args...>
//
// The comment line anchors the block to the task: its tag `[fwp-rsync=<id>]`
// uses the DB AUTOINCREMENT id, which is never reused after deletion. We
// scan for the tag to find/update/remove the block, so crontab line numbers
// shifting never breaks the association. The `who` column is the run_user
// (or `root`), so cron itself runs rsync as that user.

const ETC_CRONTAB: &str = "/etc/crontab";

/// Shell-quote one argv element. Elements containing only the safe set
/// (alphanumerics and `-_./:=,@+`) are left bare; everything else is wrapped
/// in single quotes with embedded `'` escaped as `'\''`.
fn shell_quote_one(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./:=,@+".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Join argv elements into a `/bin/sh -c`-safe command string.
fn shell_join(parts: &[String]) -> String {
    parts.iter().map(|p| shell_quote_one(p)).collect::<Vec<_>>().join(" ")
}

/// The comment line anchoring a task in `/etc/crontab`. The leading
/// `[fwp-managed, ...]` tag marks it as owned by fwp so the crontab API
/// refuses to edit/delete it; `rsync=<id>` ties it to this task.
/// `# [fwp-managed, rsync=<id>] <description> (managed by FreeBSD-Web-Panel — do not edit manually)`.
fn cron_comment_line(task: &RsyncTask) -> String {
    format!(
        "# [fwp-managed, rsync={}] {} (managed by FreeBSD-Web-Panel — do not edit manually)",
        task.id, task.description
    )
}
fn cron_cmd_line(task: &RsyncTask) -> String {
    let mut parts = vec![RSYNC.to_string()];
    parts.extend(build_rsync_args(task, false));
    let who = if task.run_user.is_empty() {
        "root"
    } else {
        &task.run_user
    };
    format!("{} {} {}", task.cron_expr, who, shell_join(&parts))
}

/// Parse `rsync=<id>` from an `[fwp-managed, rsync=<id>]` comment line.
fn parse_fwp_tag(line: &str) -> Option<i64> {
    let l = line.trim_start().strip_prefix('#')?.trim_start();
    if !l.contains("[fwp-managed") {
        return None;
    }
    let tag = l.split(']').next()?;
    let rsync_part = tag.split("rsync=").nth(1)?;
    rsync_part.trim().parse::<i64>().ok()
}

/// Atomically rewrite `/etc/crontab` (tmp + rename, mode 0644).
fn atomic_write_crontab(content: &str) -> ApiResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = format!("{ETC_CRONTAB}.fwp.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&tmp, ETC_CRONTAB)?;
    Ok(())
}

/// Insert or update the crontab block for `task`. If the task's block already
/// exists (matched by tag), it is replaced in place; otherwise appended.
fn upsert_cron_block(task: &RsyncTask) -> ApiResult<()> {
    let content = std::fs::read_to_string(ETC_CRONTAB).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let mut written = false;
    while i < lines.len() {
        if let Some(tag_id) = parse_fwp_tag(lines[i]) {
            if tag_id == task.id {
                out.push(cron_comment_line(task));
                out.push(cron_cmd_line(task));
                written = true;
                i += 1; // skip old comment line
                if i < lines.len() && parse_fwp_tag(lines[i]).is_none() {
                    i += 1; // skip paired command line
                }
                continue;
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    if !written {
        if !out.is_empty() && !out.last().unwrap().is_empty() {
            out.push(String::new()); // blank separator
        }
        out.push(cron_comment_line(task));
        out.push(cron_cmd_line(task));
    }
    let mut result = out.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    atomic_write_crontab(&result)
}

/// Remove the crontab block for task `id` if present. No-op if absent.
fn remove_cron_block(id: i64) -> ApiResult<()> {
    let content = std::fs::read_to_string(ETC_CRONTAB).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let mut removed = false;
    while i < lines.len() {
        if let Some(tag_id) = parse_fwp_tag(lines[i]) {
            if tag_id == id {
                removed = true;
                i += 1; // skip comment line
                if i < lines.len() && parse_fwp_tag(lines[i]).is_none() {
                    i += 1; // skip paired command line
                }
                if out.last().map(|l| l.is_empty()).unwrap_or(false) {
                    out.pop(); // drop preceding blank separator
                }
                continue;
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    if !removed {
        return Ok(());
    }
    let mut result = out.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    atomic_write_crontab(&result)
}

/// Reconcile `/etc/crontab` with a task's schedule: write the block only when
/// the schedule is both enabled (`cron_enabled`) and has a rule (`cron_expr`);
/// otherwise remove it. Disabling keeps `cron_expr` intact in the DB.
fn sync_cron(task: &RsyncTask) -> ApiResult<()> {
    if task.cron_enabled && !task.cron_expr.is_empty() {
        upsert_cron_block(task)
    } else {
        remove_cron_block(task.id)
    }
}

/// POST /api/rsync/tasks/{id}/run — run a sync now as a streaming task.
pub async fn run_task(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(id): AxumPath<i64>,
    Json(req): Json<RunReq>,
) -> ApiResult<Json<serde_json::Value>> {
    ensure_installed()?;
    let dry_run = req.dry_run.unwrap_or(false);
    let task = {
        let conn = state.db.lock().await;
        get_task_db(&conn, id)?
            .ok_or_else(|| ApiError::NotFound("rsync task not found".into()))?
    };

    let arg_strings = build_rsync_args(&task, dry_run);
    // If a run_user is set, wrap the rsync invocation in `su <user> -c '<cmd>'`
    // so it executes as that user (matching the crontab `who` column). Root
    // (empty run_user) runs rsync directly.
    let (exec_cmd, exec_args, display_cmd) = if task.run_user.is_empty() {
        let disp = format!("rsync {}", arg_strings.join(" "));
        (RSYNC.to_string(), arg_strings.clone(), disp)
    } else {
        let mut full = vec![RSYNC.to_string()];
        full.extend(arg_strings.clone());
        let quoted = shell_join(&full);
        let disp = format!("su {} -c {}", task.run_user, quoted);
        let a = vec!["-".to_string(), "-c".to_string(), quoted.clone()];
        ("su".to_string(), a, disp)
    };
    let label = format!(
        "rsync {}{}",
        task.description,
        if dry_run { " (dry-run)" } else { "" }
    );
    let bgid = bgtask::create("rsync-run", &label);
    let tid = bgid.clone();
    let db = state.db.clone();
    let state2 = state.clone();
    let username = user.username.clone();
    let task_id = task.id;
    let task_desc = task.description.clone();
    let path = format!("/api/rsync/tasks/{id}/run");
    let exec_cmd = exec_cmd.clone();
    let exec_args: Vec<String> = exec_args.clone();

    tokio::spawn(async move {
        let arg_refs: Vec<&str> = exec_args.iter().map(|s| s.as_str()).collect();
        bgtask::push_line(&tid, &format!("$ {display_cmd}"));
        let exit = bgtask::run_streaming_cmd(&tid, &exec_cmd, &arg_refs).await;
        let ok = exit == 0;
        let now = state2.now_ts();
        {
            let conn = db.lock().await;
            let _ = update_run_result(
                &conn,
                task_id,
                now,
                if ok { "success" } else { "failed" },
            );
        }
        bgtask::set_status(
            &tid,
            if ok {
                bgtask::TaskStatus::Done
            } else {
                bgtask::TaskStatus::Failed
            },
            Some(exit),
        );
        audit::record(
            &state2,
            Some(&username),
            "POST",
            &path,
            if ok { 200 } else { 500 },
            Some(if ok {
                format!("rsync task '{task_desc}' succeeded")
            } else {
                format!("rsync task '{task_desc}' failed (exit {exit})")
            }),
        );
    });
    Ok(Json(serde_json::json!({ "task_id": bgid })))
}

// ── Remote directory browse ────────────────────────────────────────
//
// Browse a remote host's directory tree over SSH (`ls -1Ap`) so users can
// pick source/dest paths interactively — mirroring the local FilePicker.
// The remote command is single-quoted; the path is validated to contain no
// single quotes or control chars, so there is no shell-injection surface.

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    /// Full rsync-style spec: `[user@]host:/abs/path`.
    pub spec: String,
    pub port: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct RsyncBrowseEntry {
    pub name: String,
    /// Full spec for this child: `[user@]host:/abs/path/name`.
    pub path: String,
    pub is_dir: bool,
}

static RE_REMOTE_HOST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9._@\-]+$").unwrap());

/// Split `[user@]host:/path` into `(host_part, abs_remote_path)`.
/// Mirrors rsync's own remote detection: the first `:` whose left side holds
/// no `/` separates host from path.
fn parse_remote_spec(spec: &str) -> ApiResult<(String, String)> {
    let colon = spec
        .find(':')
        .ok_or_else(|| ApiError::BadRequest("spec must be host:path".into()))?;
    let host_part = &spec[..colon];
    let path = &spec[colon + 1..];
    if host_part.is_empty() || host_part.contains('/') {
        return Err(ApiError::BadRequest("invalid host part".into()));
    }
    if !RE_REMOTE_HOST.is_match(host_part) {
        return Err(ApiError::BadRequest("invalid host".into()));
    }
    if !path.starts_with('/') {
        return Err(ApiError::BadRequest("remote path must be absolute".into()));
    }
    // The remote command wraps the path in single quotes, so it must not
    // contain a single quote itself; reject control chars too.
    if path.contains('\'') || path.chars().any(|c| c < ' ' || c == '\x7f') {
        return Err(ApiError::BadRequest("invalid remote path".into()));
    }
    Ok((host_part.to_string(), path.to_string()))
}

/// GET /api/rsync/browse?spec=[user@]host:/path&port=N
///
/// Lists the immediate children of a remote directory over SSH. Only needs
/// the `ssh` client (BatchMode fails fast on missing credentials instead of
/// hanging). `StrictHostKeyChecking=accept-new` auto-accepts first contact.
pub async fn browse(Query(q): Query<BrowseQuery>) -> ApiResult<Json<Vec<RsyncBrowseEntry>>> {
    let (host_part, remote_path) = parse_remote_spec(&q.spec)?;
    let port = q.port.unwrap_or(22);
    if !(1..=65535).contains(&port) {
        return Err(ApiError::BadRequest("port must be 1-65535".into()));
    }
    let port_s = port.to_string();
    let remote_cmd = format!("ls -1Ap -- '{}'", remote_path);
    let host_part_for_paths = host_part.clone();

    let output = tokio::task::spawn_blocking(move || -> std::io::Result<std::process::Output> {
        Command::new("ssh")
            .arg("-p")
            .arg(&port_s)
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "StrictHostKeyChecking=accept-new",
            ])
            .arg(&host_part)
            .arg(&remote_cmd)
            .output()
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(ApiError::Io)?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let msg = if err.is_empty() {
            format!(
                "remote listing failed (exit {})",
                output.status.code().unwrap_or(-1)
            )
        } else {
            err
        };
        return Err(ApiError::Command(msg));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sep = if remote_path.ends_with('/') { "" } else { "/" };
    let mut entries = Vec::new();
    for line in stdout.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let is_dir = line.ends_with('/');
        let name = line.trim_end_matches('/').to_string();
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        entries.push(RsyncBrowseEntry {
            name: name.clone(),
            path: format!("{}:{}{}{}", host_part_for_paths, remote_path, sep, name),
            is_dir,
        });
    }
    // Directories first, then case-insensitive name order.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Json(entries))
}
