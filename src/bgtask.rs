//! Generic background task infrastructure.
//!
//! Provides a shared store for long-running operations (package install,
//! vm-bhyve init, jail base import, etc.) with streaming stdout/stderr
//! output delivered to the frontend via a single unified SSE endpoint.
//!
//! ## Usage
//!
//! 1. `let id = bgtask::create("pkg-install", "pkg install vim");`
//! 2. Inside a `tokio::spawn` block, call `bgtask::run_streaming_cmd()` for
//!    each command, or manually `push_line()` / `set_status()`.
//! 3. Frontend opens `EventSource("/api/tasks/{id}/stream?token=...")`.
//!
//! The SSE endpoint (`stream_handler`) is registered once in `app.rs` as a
//! public route — no per-module SSE handlers needed.

use std::collections::HashMap;
use std::convert::Infallible;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Path, Query, State};
use axum::response::{sse::{Event, KeepAlive, Sse}, IntoResponse, Response};
use futures_util::stream::{self, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::auth::validate_token;
use crate::error::ApiError;
use crate::state::AppState;

const TASK_TTL_SECS: i64 = 600;

// ── Types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct BgTask {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub status: TaskStatus,
    pub exit_code: Option<i32>,
    pub lines: Vec<String>,
    pub created_at: i64,
}

// ── Store ──────────────────────────────────────────────────────────

static TASKS: LazyLock<Mutex<HashMap<String, BgTask>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn gen_id() -> String {
    use std::fmt::Write;
    let ts = now_ts();
    let pid = std::process::id();
    let mut buf = [0u8; 8];
    if let Ok(nanos) = SystemTime::now().duration_since(UNIX_EPOCH) {
        let n = nanos.subsec_nanos() as u64;
        buf.copy_from_slice(&n.to_le_bytes());
    }
    let mut s = String::new();
    let _ = write!(&mut s, "{ts:x}{pid:x}");
    for b in &buf {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

// ── Public API ─────────────────────────────────────────────────────

/// Create a new background task and insert it into the store.
/// Returns the task id.
pub fn create(kind: &str, label: &str) -> String {
    gc();
    let id = gen_id();
    let task = BgTask {
        id: id.clone(),
        kind: kind.to_string(),
        label: label.to_string(),
        status: TaskStatus::Running,
        exit_code: None,
        lines: Vec::new(),
        created_at: now_ts(),
    };
    TASKS.lock().insert(id.clone(), task);
    id
}

/// Append a line of output to the task, with ANSI escape sequences stripped.
pub fn push_line(task_id: &str, line: &str) {
    let cleaned = strip_ansi(line);
    if cleaned.is_empty() {
        return;
    }
    let mut tasks = TASKS.lock();
    if let Some(task) = tasks.get_mut(task_id) {
        task.lines.push(cleaned);
    }
}

/// Strip ANSI escape sequences (colors, cursor movement, etc.) from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            // CSI sequence: ESC [ ... terminated by a byte in 0x40..=0x7e
            i += 2;
            while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                i += 1;
            }
            i += 1; // skip the terminator
        } else if i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b']' {
            // OSC sequence: ESC ] ... terminated by BEL (0x07) or ST (ESC \)
            i += 2;
            while i < bytes.len() {
                if bytes[i] == 0x07 {
                    i += 1;
                    break;
                }
                if i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'\\' {
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else if i + 2 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] != b'[' && bytes[i + 1] != b']' {
            // Other escape: ESC + one byte (e.g. ESC =, ESC >)
            i += 2;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out.trim().to_string()
}

/// Set the final status and exit code of the task.
pub fn set_status(task_id: &str, status: TaskStatus, exit_code: Option<i32>) {
    let mut tasks = TASKS.lock();
    if let Some(task) = tasks.get_mut(task_id) {
        task.status = status;
        task.exit_code = exit_code;
    }
}

/// Get a snapshot of a task (cloned).
pub fn get(task_id: &str) -> Option<BgTask> {
    TASKS.lock().get(task_id).cloned()
}

/// Remove a task from the store.
fn remove(task_id: &str) {
    TASKS.lock().remove(task_id);
}

/// Remove tasks older than TASK_TTL_SECS.
pub fn gc() {
    let cutoff = now_ts() - TASK_TTL_SECS;
    let mut tasks = TASKS.lock();
    tasks.retain(|_, t| t.created_at > cutoff);
}

/// Spawn a command inside a pseudo-terminal so that commands which use `\r`
/// for in-place progress updates (e.g. `pkg install`, `fetch`) actually emit
/// those updates. Each `\r`-delimited segment is pushed as a separate line.
///
/// If PTY allocation fails, falls back to piped stdout/stderr.
/// Returns the exit code.
pub async fn run_streaming_cmd(task_id: &str, cmd: &str, args: &[&str]) -> i32 {
    match run_with_pty(task_id, cmd, args).await {
        Some(code) => code,
        None => run_with_pipe(task_id, cmd, args).await,
    }
}

/// Run a command in a PTY. Returns `Some(exit_code)` on success, or `None`
/// if PTY allocation or fork failed.
async fn run_with_pty(task_id: &str, cmd: &str, args: &[&str]) -> Option<i32> {
    use std::ffi::CString;

    // Open a PTY master/slave pair.
    let master = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
    if master < 0 {
        return None;
    }
    if unsafe { libc::grantpt(master) } < 0 {
        unsafe { libc::close(master) };
        return None;
    }
    if unsafe { libc::unlockpt(master) } < 0 {
        unsafe { libc::close(master) };
        return None;
    }
    let mut name_buf = [0u8; 256];
    if unsafe {
        libc::ptsname_r(
            master,
            name_buf.as_mut_ptr() as *mut libc::c_char,
            name_buf.len(),
        )
    } != 0
    {
        unsafe { libc::close(master) };
        return None;
    }
    let slave_cstr = CString::new(&name_buf[..name_buf.iter().position(|&b| b == 0).unwrap_or(0)]).unwrap();
    let cmd_cstr = CString::new(cmd).unwrap();
    let argv_cstrings: Vec<CString> = std::iter::once(cmd_cstr.clone())
        .chain(args.iter().map(|a| CString::new(a.as_bytes()).unwrap()))
        .collect();

    // Build argv and env pointer arrays (used only during fork, not across await).
    let pid = {
        let mut argv_ptrs: Vec<*const libc::c_char> =
            argv_cstrings.iter().map(|s| s.as_ptr()).collect();
        argv_ptrs.push(std::ptr::null());

        let env: Vec<CString> = {
            let mut v = vec![CString::new("TERM=xterm").unwrap()];
            for (k, val) in std::env::vars() {
                if k == "TERM" || k == "COLUMNS" || k == "LINES" {
                    continue;
                }
                if let Ok(s) = CString::new(format!("{k}={val}")) {
                    v.push(s);
                }
            }
            v
        };
        let mut env_ptrs: Vec<*const libc::c_char> = env.iter().map(|s| s.as_ptr()).collect();
        env_ptrs.push(std::ptr::null());

        let pid = unsafe { libc::fork() };
        if pid == 0 {
            // === child ===
            unsafe {
                libc::setsid();
                let slave = libc::open(slave_cstr.as_ptr(), libc::O_RDWR);
                if slave < 0 {
                    libc::_exit(127);
                }
                libc::ioctl(slave, libc::TIOCSCTTY, 0);
                libc::dup2(slave, 0);
                libc::dup2(slave, 1);
                libc::dup2(slave, 2);
                if slave > 2 {
                    libc::close(slave);
                }
                libc::close(master);
                libc::execve(cmd_cstr.as_ptr(), argv_ptrs.as_ptr(), env_ptrs.as_ptr());
                libc::_exit(127);
            }
        }
        // env/env_ptrs/argv_ptrs dropped here (before any await).
        pid
    };

    if pid < 0 {
        unsafe { libc::close(master) };
        return None;
    }

    // Set window size so progress bars don't wrap.
    let win_size = libc::winsize {
        ws_row: 50,
        ws_col: 200,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        libc::ioctl(master, libc::TIOCSWINSZ, &win_size);
    }

    // Read from master in a blocking thread, split on \r and \n.
    let tid = task_id.to_string();
    let reader = tokio::task::spawn_blocking(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = unsafe { libc::read(master, chunk.as_mut_ptr() as *mut _, chunk.len()) };
            if n <= 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n as usize]);
            while let Some(pos) = buf.iter().position(|&b| b == b'\r' || b == b'\n') {
                let line = String::from_utf8_lossy(&buf[..pos]).trim().to_string();
                if !line.is_empty() {
                    push_line(&tid, &line);
                }
                buf = buf[pos + 1..].to_vec();
            }
        }
        if !buf.is_empty() {
            let line = String::from_utf8_lossy(&buf).trim().to_string();
            if !line.is_empty() {
                push_line(&tid, &line);
            }
        }
        unsafe { libc::close(master) };
    });

    let _ = reader.await;

    // Reap the child.
    let mut status = 0;
    unsafe { libc::waitpid(pid, &mut status, 0) };
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        -1
    };
    Some(code)
}

/// Fallback: run a command with piped stdout/stderr (no PTY).
async fn run_with_pipe(task_id: &str, cmd: &str, args: &[&str]) -> i32 {
    let mut child = match tokio::process::Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            push_line(task_id, &format!("Failed to spawn {cmd}: {e}"));
            return -1;
        }
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let tid_out = task_id.to_string();
    let stdout_task = tokio::spawn(async move {
        stream_split_cr_ln(stdout, |line| {
            push_line(&tid_out, line);
        })
        .await;
    });

    let tid_err = task_id.to_string();
    let stderr_task = tokio::spawn(async move {
        stream_split_cr_ln(stderr, |line| {
            push_line(&tid_err, line);
        })
        .await;
    });

    let _ = stdout_task.await;
    let _ = stderr_task.await;

    child
        .wait()
        .await
        .ok()
        .and_then(|s| s.code())
        .unwrap_or(-1)
}

/// Read a stream, splitting on both `\r` and `\n`, calling `on_line` for each
/// non-empty trimmed segment.
async fn stream_split_cr_ln<R, F>(reader: R, mut on_line: F)
where
    R: tokio::io::AsyncRead + Unpin,
    F: FnMut(&str),
{
    use tokio::io::AsyncReadExt;
    let mut reader = reader;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                while let Some(pos) = buf.iter().position(|&b| b == b'\r' || b == b'\n') {
                    let line = String::from_utf8_lossy(&buf[..pos]).trim().to_string();
                    if !line.is_empty() {
                        on_line(&line);
                    }
                    buf = buf[pos + 1..].to_vec();
                }
            }
            Err(_) => break,
        }
    }
    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf).trim().to_string();
        if !line.is_empty() {
            on_line(&line);
        }
    }
}

// ── SSE Handler ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    token: String,
}

/// `GET /api/tasks/{id}/stream` — unified SSE stream for any background task.
///
/// Public route (token validated via query param, same reason as WebSocket
/// terminal — EventSource cannot set Authorization headers).
pub async fn stream_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<StreamParams>,
) -> Response {
    if validate_token(&state, &params.token).await.is_err() {
        return ApiError::NotAuthenticated.into_response();
    }

    let stream = stream::unfold(0usize, move |last_len| {
        let id = id.clone();
        async move {
            let task = get(&id);
            let task = match task {
                None => return None,
                Some(t) => t,
            };

            let new_lines = &task.lines[last_len..];
            let new_len = task.lines.len();
            let is_done = task.status != TaskStatus::Running;

            let data = serde_json::json!({
                "status": task.status,
                "lines": new_lines,
                "exit_code": task.exit_code,
                "kind": task.kind,
                "label": task.label,
            });
            let event = Event::default().data(data.to_string());

            if is_done {
                // Task is complete — remove it from the store so it doesn't
                // linger. The SSE stream will end right after this event.
                remove(&id);
                Some((event, new_len))
            } else {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Some((event, new_len))
            }
        }
    })
    .chain(stream::once(async {
        Event::default().event("done").data("[\"done\"]")
    }))
    .map(Ok::<_, Infallible>);

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}
