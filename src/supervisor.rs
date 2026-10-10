//! Process supervisor — orchestrate user-defined daemon(8)-guarded processes.
//!
//! Design (docs/plan/42-supervisor.md): each managed process is guarded by
//! its own `daemon(8)` instance, completely independent of fwp — if fwp dies
//! or restarts, crash-restart supervision continues. fwp only stores the
//! definition (SQLite), spawns `daemon`, sends signals, and reads pidfiles
//! and logs; there is no adoption logic because the supervisor's PPID is 1.
//!
//! Runtime layout under the configured dir (default /var/db/fwp/supervisor):
//!
//! ```text
//! <name>.sup.pid     daemon supervisor PID  (-P)
//! <name>.child.pid   guarded process PID    (-p)
//! <name>.log         child stdout+stderr    (-o, append; custom `log_file` overrides)
//! ```
//!
//! daemon(8) semantics this relies on (verified on 15.1-RELEASE):
//! - SIGTERM to the supervisor is forwarded to the child; both exit and the
//!   pidfiles are removed (pidfile(3)).
//! - `-r -R N [-C M]`: restart on ANY exit (including exit 0), N seconds
//!   delay, at most M restarts after the first run.
//! - `-H`: SIGHUP closes and reopens the `-o` log (used for rotation).
//! - exec failure makes the immediate parent exit 0 with the error only on
//!   stderr → we pre-validate the path and re-check liveness after spawn.
//!
//! All signals are sent to exact PIDs read from our own pidfiles — never
//! by name or pattern match.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::sysinfo;

const DAEMON: &str = "/usr/sbin/daemon";

/// Logs are rotated by the hourly scheduler job once they exceed this size.
const LOG_ROTATE_BYTES: u64 = 10 * 1024 * 1024;

static RE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_.-]{1,64}$").unwrap());
/// Environment entry: `NAME=...` (value may be empty).
static RE_ENV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*=").unwrap());

// ── Model ───────────────────────────────────────────────────────────

/// A stored process definition (one row of `supervisor_procs`).
#[derive(Debug, Clone, Serialize)]
pub struct ProcDef {
    pub id: i64,
    pub name: String,
    pub path: String,
    /// argv elements, passed through to execvp — never a shell string.
    pub args: Vec<String>,
    pub workdir: Option<String>,
    pub user: Option<String>,
    /// `K=V` entries, exported to the child via the daemon process.
    pub env: Vec<String>,
    /// Capture child stdout+stderr into `<name>.log`. When false, daemon(8)
    /// pipes child output to its supervisor which discards it (no file).
    pub logging: bool,
    /// Custom log file path; None = `<dir>/<name>.log`. A custom path is
    /// never auto-deleted (it may point at a shared system log).
    pub log_file: Option<String>,
    /// Let the hourly scheduler rotate the log once it exceeds 10 MiB.
    pub log_rotate: bool,
    pub restart: bool,
    /// Start this entry automatically when fwp starts (v6 column).
    pub autostart: bool,
    pub restart_delay: i64,
    pub restart_max: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Runtime state of one entry, derived from pidfiles + kill(pid, 0).
#[derive(Debug, Clone, Serialize)]
pub struct ProcStatus {
    /// `running` | `starting` | `orphaned` | `stopped`
    pub state: &'static str,
    pub sup_pid: Option<i32>,
    pub pid: Option<i32>,
    /// Start time (Unix seconds) of the current child incarnation.
    pub started_at: Option<i64>,
}

/// Request body for create/update.
#[derive(Debug, Deserialize)]
pub struct ProcReq {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub workdir: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default = "yes")]
    pub logging: bool,
    /// Absolute path to a custom log file; empty = default `<dir>/<name>.log`.
    #[serde(default)]
    pub log_file: String,
    #[serde(default = "yes")]
    pub log_rotate: bool,
    #[serde(default = "yes")]
    pub restart: bool,
    /// Start automatically when fwp starts (defaults to false).
    #[serde(default)]
    pub autostart: bool,
    #[serde(default = "one")]
    pub restart_delay: i64,
    #[serde(default)]
    pub restart_max: Option<i64>,
}

fn yes() -> bool {
    true
}
fn one() -> i64 {
    1
}

/// A validated definition ready to store.
pub struct DefInput {
    pub name: String,
    pub path: String,
    pub args: Vec<String>,
    pub workdir: Option<String>,
    pub user: Option<String>,
    pub env: Vec<String>,
    pub logging: bool,
    pub log_file: Option<String>,
    pub log_rotate: bool,
    pub restart: bool,
    pub autostart: bool,
    pub restart_delay: i64,
    pub restart_max: Option<i64>,
}

// ── Validation ──────────────────────────────────────────────────────

/// Validate a create/update request. The executable path is checked up front
/// because daemon(8) reports exec failure by exiting 0 with the error only
/// on stderr — a pre-check turns the common mistake into a clear 400.
pub fn validate_input(raw: &ProcReq) -> ApiResult<DefInput> {
    let name = raw.name.trim();
    if !RE_NAME.is_match(name) {
        return Err(ApiError::BadRequest(
            "invalid name: 1-64 chars of [a-zA-Z0-9_.-]".into(),
        ));
    }

    let path = raw.path.trim();
    if !path.starts_with('/') {
        return Err(ApiError::BadRequest("path must be absolute".into()));
    }
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {
            use std::os::unix::fs::PermissionsExt;
            if m.permissions().mode() & 0o111 == 0 {
                return Err(ApiError::BadRequest(format!(
                    "path is not executable: {path}"
                )));
            }
        }
        _ => return Err(ApiError::BadRequest(format!("path not found: {path}"))),
    }

    if raw.args.len() > 128 {
        return Err(ApiError::BadRequest("too many arguments (max 128)".into()));
    }
    for a in &raw.args {
        if a.contains('\0') || a.len() > 4096 {
            return Err(ApiError::BadRequest(
                "invalid argument (empty/NUL/too long)".into(),
            ));
        }
    }

    let workdir = optional_abs_dir(&raw.workdir, "workdir")?;

    let user = raw.user.trim();
    let user = if user.is_empty() {
        None
    } else {
        if !user_exists(user) {
            return Err(ApiError::BadRequest(format!("unknown user: {user}")));
        }
        Some(user.to_string())
    };

    if raw.env.len() > 128 {
        return Err(ApiError::BadRequest(
            "too many environment entries (max 128)".into(),
        ));
    }
    for e in &raw.env {
        if !RE_ENV.is_match(e) || e.contains('\0') || e.len() > 8192 {
            return Err(ApiError::BadRequest(format!(
                "invalid environment entry (expected KEY=VALUE): {e}"
            )));
        }
    }

    let log_file = validate_log_file(&raw.log_file)?;

    let restart_delay = raw.restart_delay;
    if !(1..=3600).contains(&restart_delay) {
        return Err(ApiError::BadRequest(
            "restart delay must be 1-3600 seconds".into(),
        ));
    }
    let restart_max = match raw.restart_max {
        None => None,
        Some(0) => {
            return Err(ApiError::BadRequest(
                "restart max must be >= 1 (or empty for unlimited)".into(),
            ))
        }
        Some(m) if m > 0 && m <= 1_000_000 => Some(m),
        Some(_) => {
            return Err(ApiError::BadRequest(
                "restart max must be 1-1000000 (or empty for unlimited)".into(),
            ))
        }
    };

    Ok(DefInput {
        name: name.to_string(),
        path: path.to_string(),
        args: raw.args.clone(),
        workdir,
        user,
        env: raw.env.clone(),
        logging: raw.logging,
        log_file,
        log_rotate: raw.log_rotate,
        restart: raw.restart,
        autostart: raw.autostart,
        restart_delay,
        restart_max,
    })
}

/// Empty string → None; otherwise must be an existing absolute directory.
fn optional_abs_dir(raw: &str, what: &str) -> ApiResult<Option<String>> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if !s.starts_with('/') {
        return Err(ApiError::BadRequest(format!("{what} must be absolute")));
    }
    match std::fs::metadata(s) {
        Ok(m) if m.is_dir() => Ok(Some(s.to_string())),
        _ => Err(ApiError::BadRequest(format!("{what} is not a directory: {s}"))),
    }
}

/// Empty string → None (default `<dir>/<name>.log`); otherwise an absolute
/// file path whose parent directory exists. The file itself need not exist —
/// daemon(8) creates/appends on open.
fn validate_log_file(raw: &str) -> ApiResult<Option<String>> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if !s.starts_with('/') {
        return Err(ApiError::BadRequest("log file must be an absolute path".into()));
    }
    if s.ends_with('/') {
        return Err(ApiError::BadRequest(
            "log file must be a file path, not a directory".into(),
        ));
    }
    let parent = std::path::Path::new(s)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("/"));
    if !parent.is_dir() {
        return Err(ApiError::BadRequest(format!(
            "log file directory does not exist: {}",
            parent.display()
        )));
    }
    Ok(Some(s.to_string()))
}

/// Check a username via getpwnam(3).
fn user_exists(name: &str) -> bool {
    match std::ffi::CString::new(name) {
        Ok(c) => !unsafe { libc::getpwnam(c.as_ptr()) }.is_null(),
        Err(_) => false,
    }
}

// ── DB access ───────────────────────────────────────────────────────

const SELECT_COLS: &str =
    "id, name, path, args, workdir, user, env, logging, log_file, log_rotate, restart, autostart, restart_delay, restart_max, created_at, updated_at";

fn row_to_def(r: &Row) -> rusqlite::Result<ProcDef> {
    let args_json: String = r.get("args")?;
    let env_json: String = r.get("env")?;
    Ok(ProcDef {
        id: r.get("id")?,
        name: r.get("name")?,
        path: r.get("path")?,
        args: serde_json::from_str(&args_json).unwrap_or_default(),
        workdir: r.get("workdir")?,
        user: r.get("user")?,
        env: serde_json::from_str(&env_json).unwrap_or_default(),
        logging: r.get::<_, i64>("logging")? != 0,
        log_file: r.get("log_file")?,
        log_rotate: r.get::<_, i64>("log_rotate")? != 0,
        restart: r.get::<_, i64>("restart")? != 0,
        autostart: r.get::<_, i64>("autostart")? != 0,
        restart_delay: r.get("restart_delay")?,
        restart_max: r.get("restart_max")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
    })
}

pub fn list_procs(conn: &Connection) -> ApiResult<Vec<ProcDef>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM supervisor_procs ORDER BY name"
    ))?;
    let defs = stmt
        .query_map([], row_to_def)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(defs)
}

pub fn get_proc(conn: &Connection, id: i64) -> ApiResult<Option<ProcDef>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLS} FROM supervisor_procs WHERE id = ?1"),
        params![id],
        row_to_def,
    )
    .optional()
    .map_err(ApiError::Database)
}

pub fn get_proc_by_name(conn: &Connection, name: &str) -> ApiResult<Option<ProcDef>> {
    conn.query_row(
        &format!("SELECT {SELECT_COLS} FROM supervisor_procs WHERE name = ?1"),
        params![name],
        row_to_def,
    )
    .optional()
    .map_err(ApiError::Database)
}

pub fn create_proc(conn: &Connection, input: &DefInput, now: i64) -> ApiResult<ProcDef> {
    let args = serde_json::to_string(&input.args).unwrap_or_else(|_| "[]".into());
    let env = serde_json::to_string(&input.env).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO supervisor_procs
            (name, path, args, workdir, user, env, logging, log_file, log_rotate,
             restart, autostart, restart_delay, restart_max, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            input.name,
            input.path,
            args,
            input.workdir,
            input.user,
            env,
            input.logging as i64,
            input.log_file,
            input.log_rotate as i64,
            input.restart as i64,
            input.autostart as i64,
            input.restart_delay,
            input.restart_max,
            now,
            now
        ],
    )?;
    let id = conn.last_insert_rowid();
    get_proc(conn, id)?.ok_or_else(|| ApiError::Internal("insert failed".into()))
}

pub fn update_proc(conn: &Connection, id: i64, input: &DefInput, now: i64) -> ApiResult<ProcDef> {
    let args = serde_json::to_string(&input.args).unwrap_or_else(|_| "[]".into());
    let env = serde_json::to_string(&input.env).unwrap_or_else(|_| "[]".into());
    let n = conn.execute(
        "UPDATE supervisor_procs SET
            name = ?1, path = ?2, args = ?3, workdir = ?4, user = ?5, env = ?6,
            logging = ?7, log_file = ?8, log_rotate = ?9,
            restart = ?10, autostart = ?11, restart_delay = ?12, restart_max = ?13, updated_at = ?14
         WHERE id = ?15",
        params![
            input.name,
            input.path,
            args,
            input.workdir,
            input.user,
            env,
            input.logging as i64,
            input.log_file,
            input.log_rotate as i64,
            input.restart as i64,
            input.autostart as i64,
            input.restart_delay,
            input.restart_max,
            now,
            id
        ],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound(format!("process {id} not found")));
    }
    get_proc(conn, id)?.ok_or_else(|| ApiError::Internal("update failed".into()))
}

pub fn delete_proc(conn: &Connection, id: i64) -> ApiResult<()> {
    conn.execute("DELETE FROM supervisor_procs WHERE id = ?1", params![id])?;
    Ok(())
}

// ── Runtime files & liveness ────────────────────────────────────────

fn sup_pidfile(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.sup.pid"))
}
fn child_pidfile(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.child.pid"))
}

/// Log file of an entry: a custom absolute path when set, otherwise the
/// default `<dir>/<name>.log`. Custom files are never auto-deleted (they may
/// point at shared system logs).
fn logfile(dir: &Path, def: &ProcDef) -> PathBuf {
    match &def.log_file {
        Some(custom) => PathBuf::from(custom),
        None => dir.join(format!("{}.log", def.name)),
    }
}

/// Default-location log path (delete/rename cleanup only — never custom paths).
fn default_logfile(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.log"))
}

fn read_pidfile(path: &Path) -> Option<i32> {
    let s = std::fs::read_to_string(path).ok()?;
    let pid: i32 = s.trim().parse().ok()?;
    (pid > 0).then_some(pid)
}

/// kill(pid, 0): success or EPERM (process exists, not ours) means alive.
fn pid_alive(pid: i32) -> bool {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn alive_pid_from(path: &Path) -> Option<i32> {
    read_pidfile(path).filter(|p| pid_alive(*p))
}

fn send_signal(pid: i32, sig: i32) {
    unsafe {
        libc::kill(pid, sig);
    }
}

fn wait_exit(pid: i32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Derive the runtime state of an entry from its pidfiles.
pub fn status(dir: &Path, name: &str) -> ProcStatus {
    let sup_pf = sup_pidfile(dir, name);
    let child_pf = child_pidfile(dir, name);
    match (alive_pid_from(&sup_pf), alive_pid_from(&child_pf)) {
        (Some(sp), Some(cp)) => ProcStatus {
            state: "running",
            sup_pid: Some(sp),
            pid: Some(cp),
            started_at: sysinfo::read_proc_start(cp),
        },
        // Supervisor alive, child not: restart-delay window or just started
        // (daemon(8) truncates the child pidfile between incarnations).
        (Some(sp), None) => ProcStatus {
            state: "starting",
            sup_pid: Some(sp),
            pid: None,
            started_at: None,
        },
        // Supervisor was killed externally; the child keeps running.
        (None, Some(cp)) => ProcStatus {
            state: "orphaned",
            sup_pid: None,
            pid: Some(cp),
            started_at: sysinfo::read_proc_start(cp),
        },
        (None, None) => {
            // Both dead: clean stale pidfiles so `starting` is never faked
            // by leftover files (daemon(8) normally removes them itself, but
            // SIGKILL leaves them behind).
            let _ = std::fs::remove_file(&sup_pf);
            let _ = std::fs::remove_file(&child_pf);
            ProcStatus {
                state: "stopped",
                sup_pid: None,
                pid: None,
                started_at: None,
            }
        }
    }
}

// ── Lifecycle ───────────────────────────────────────────────────────

/// Build the daemon(8) argv. Options are separate argv elements and the
/// command path/args come after `--`, so nothing passes through a shell.
fn build_daemon_cmd(dir: &Path, def: &ProcDef) -> Command {
    let mut cmd = Command::new(DAEMON);
    if def.restart {
        cmd.arg("-r");
        cmd.arg("-R").arg(def.restart_delay.to_string());
        if let Some(m) = def.restart_max {
            cmd.arg("-C").arg(m.to_string());
        }
    }
    cmd.arg("-P").arg(sup_pidfile(dir, &def.name));
    cmd.arg("-p").arg(child_pidfile(dir, &def.name));
    if def.logging {
        cmd.arg("-o").arg(logfile(dir, def));
        // SIGHUP reopens the log file — enables rename-then-HUP rotation.
        cmd.arg("-H");
    }
    // Without `-o`: daemon(8) still dup2's child stdout/stderr into the
    // supervisor pipe, but do_output() discards it — no file, no blocking.
    cmd.arg("-t").arg(format!("fwp-guard:{}", def.name));
    if let Some(u) = &def.user {
        cmd.arg("-u").arg(u);
    }
    if let Some(wd) = &def.workdir {
        // daemon inherits our cwd; the forked child keeps it.
        cmd.current_dir(wd);
    }
    cmd.arg("--");
    cmd.arg(&def.path);
    for a in &def.args {
        cmd.arg(a);
    }
    for e in &def.env {
        if let Some((k, v)) = e.split_once('=') {
            cmd.env(k, v);
        }
    }
    cmd
}

/// Spawn the daemon(8) guard. Sync — call from `spawn_blocking`.
pub fn start(dir: &Path, def: &ProcDef) -> ApiResult<()> {
    let st = status(dir, &def.name);
    match st.state {
        "running" | "starting" => {
            return Err(ApiError::Conflict(format!(
                "process '{}' is already running",
                def.name
            )))
        }
        "orphaned" => {
            return Err(ApiError::Conflict(format!(
                "process '{}' has an orphaned child (pid {}); stop it first",
                def.name,
                st.pid.unwrap_or(0)
            )))
        }
        _ => {}
    }

    std::fs::create_dir_all(dir)?;
    let sup_pf = sup_pidfile(dir, &def.name);
    let child_pf = child_pidfile(dir, &def.name);
    let _ = std::fs::remove_file(&sup_pf);
    let _ = std::fs::remove_file(&child_pf);

    // daemon(3) makes the process we spawn exit immediately, but the
    // long-lived supervisor inherits our fds — redirect to a temp file, not
    // a pipe (see handlers/services.rs for the `.output()` hang this avoids).
    let tmp = std::env::temp_dir().join(format!(
        "fwp-supv-{}-{}.err",
        def.name,
        std::process::id()
    ));
    let err_out = std::fs::File::create(&tmp)?;
    let err_err = std::fs::OpenOptions::new().append(true).open(&tmp)?;
    build_daemon_cmd(dir, def)
        .stdin(Stdio::null())
        .stdout(err_out)
        .stderr(err_err)
        .status()?;

    // Wait up to 2s for the supervisor pidfile + liveness. A present child
    // pidfile is also success (restart-delay window / fast-exiting child).
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if alive_pid_from(&sup_pf).is_some() {
            let _ = std::fs::remove_file(&tmp);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let err = std::fs::read_to_string(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    Err(ApiError::Command(if err.trim().is_empty() {
        format!("daemon for '{}' exited before the supervisor came up", def.name)
    } else {
        err.trim().to_string()
    }))
}

/// Stop protocol. Sync — call from `spawn_blocking`. Idempotent.
///
/// Normal path: SIGTERM the supervisor; daemon(8) forwards it to the child,
/// both exit, pidfiles are removed. Stubborn child: SIGKILL the child — the
/// supervisor is gone or terminating, so it will not restart it. Orphaned
/// child (supervisor already dead): signal the child directly.
pub fn stop(dir: &Path, name: &str) -> ApiResult<()> {
    let sup_pf = sup_pidfile(dir, name);
    let child_pf = child_pidfile(dir, name);
    let sup = alive_pid_from(&sup_pf);
    let child = alive_pid_from(&child_pf);

    match sup {
        Some(sp) => {
            send_signal(sp, libc::SIGTERM);
            wait_exit(sp, Duration::from_secs(10));
            if let Some(cp) = alive_pid_from(&child_pf) {
                send_signal(cp, libc::SIGKILL);
                wait_exit(cp, Duration::from_secs(2));
            }
            if pid_alive(sp) {
                send_signal(sp, libc::SIGKILL);
                wait_exit(sp, Duration::from_secs(1));
            }
        }
        None => {
            if let Some(cp) = child {
                send_signal(cp, libc::SIGTERM);
                wait_exit(cp, Duration::from_secs(10));
                if pid_alive(cp) {
                    send_signal(cp, libc::SIGKILL);
                    wait_exit(cp, Duration::from_secs(2));
                }
            }
        }
    }

    let sup_left = alive_pid_from(&sup_pf);
    let child_left = alive_pid_from(&child_pf);
    let _ = std::fs::remove_file(&sup_pf);
    let _ = std::fs::remove_file(&child_pf);

    if let Some(cp) = child_left {
        return Err(ApiError::Command(format!(
            "child pid {cp} did not exit"
        )));
    }
    if let Some(sp) = sup_left {
        return Err(ApiError::Command(format!(
            "supervisor pid {sp} did not exit"
        )));
    }
    Ok(())
}

/// Start every `autostart` entry that is currently stopped. Spawned once by
/// main after the listener binds; failures are logged, never fatal — the
/// panel must come up regardless. Guards of already-running entries are
/// untouched (they are independent of fwp anyway).
pub async fn autostart(state: crate::state::AppState) {
    let defs = {
        let conn = state.db.lock().await;
        match list_procs(&conn) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, "supervisor autostart: listing failed");
                return;
            }
        }
    };
    let dir = state.config.paths.supervisor.clone();
    for def in defs {
        if !def.autostart {
            continue;
        }
        let st = {
            let d = dir.clone();
            let name = def.name.clone();
            match tokio::task::spawn_blocking(move || status(&d, &name)).await {
                Ok(st) => st,
                Err(e) => {
                    tracing::warn!(name = %def.name, error = %e, "supervisor autostart: status check failed");
                    continue;
                }
            }
        };
        // running / starting / orphaned: leave to its guard or the user.
        if st.state != "stopped" {
            continue;
        }
        let _guard = state.supervisor_lock.lock().await;
        let d = dir.clone();
        let start_def = def.clone();
        match tokio::task::spawn_blocking(move || start(&d, &start_def)).await {
            Ok(Ok(())) => tracing::info!(name = %def.name, "supervisor autostart: started"),
            Ok(Err(e)) => tracing::warn!(name = %def.name, error = %e, "supervisor autostart failed"),
            Err(e) => tracing::warn!(name = %def.name, error = %e, "supervisor autostart task failed"),
        }
    }
}

// ── Logs ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct LogData {
    pub size: u64,
    pub content: String,
}

/// Tail of the log file (last `lines` lines). Only the final 1 MiB is read
/// so a huge file cannot balloon memory. Missing file → empty content.
pub fn read_log(dir: &Path, def: &ProcDef, lines: usize) -> ApiResult<LogData> {
    let log = logfile(dir, def);
    let meta = match std::fs::metadata(&log) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LogData {
                size: 0,
                content: String::new(),
            })
        }
        Err(e) => return Err(e.into()),
    };
    let size = meta.len();
    let cap = 1024 * 1024u64;
    let start = size.saturating_sub(cap);
    let mut f = std::fs::File::open(&log)?;
    use std::io::{Read, Seek, SeekFrom};
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((size - start) as usize);
    f.read_to_end(&mut buf)?;
    let content = String::from_utf8_lossy(&buf).into_owned();
    let mut tail: Vec<&str> = content.lines().rev().take(lines).collect();
    tail.reverse();
    Ok(LogData {
        size,
        content: tail.join("\n"),
    })
}

/// Clear the log file. Truncating in place would leave the supervisor's fd
/// offset past EOF (sparse NUL hole), so instead: rename the file away,
/// remove it, and SIGHUP the supervisor (`-H`) to reopen a fresh one. When
/// nothing is running, just delete the file.
pub fn clear_log(dir: &Path, def: &ProcDef) -> ApiResult<()> {
    let log = logfile(dir, def);
    if !log.exists() {
        return Ok(());
    }
    let old = log.with_extension("log.old");
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::rename(&log, &old);
    let _ = std::fs::remove_file(&old);
    if let Some(sp) = alive_pid_from(&sup_pidfile(dir, &def.name)) {
        send_signal(sp, libc::SIGHUP);
    }
    Ok(())
}

/// Hourly scheduler job: rotate any log above the size threshold.
/// `<name>.log` → `<name>.log.old` (replacing the previous rotation), then
/// SIGHUP the supervisor (started with `-H`) to reopen the fresh file.
pub fn rotate_logs(dir: &Path, defs: &[ProcDef]) {
    for def in defs {
        let log = logfile(dir, def);
        let Ok(meta) = std::fs::metadata(&log) else {
            continue;
        };
        if !def.logging || !def.log_rotate || meta.len() <= LOG_ROTATE_BYTES {
            continue;
        }
        let old = log.with_extension("log.old");
        if std::fs::rename(&log, &old).is_err() {
            continue;
        }
        if let Some(sp) = alive_pid_from(&sup_pidfile(dir, &def.name)) {
            send_signal(sp, libc::SIGHUP);
        }
        tracing::info!(name = %def.name, bytes = meta.len(), "supervisor log rotated");
    }
}

/// Remove the runtime files of an entry (used on delete / rename). Only the
/// default-location log is touched — a custom `log_file` is left alone.
pub fn remove_runtime_files(dir: &Path, name: &str) {
    let _ = std::fs::remove_file(sup_pidfile(dir, name));
    let _ = std::fs::remove_file(child_pidfile(dir, name));
    let _ = std::fs::remove_file(default_logfile(dir, name));
    let _ = std::fs::remove_file(default_logfile(dir, name).with_extension("log.old"));
}
