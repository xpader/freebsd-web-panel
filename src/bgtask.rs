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
use tokio::io::{AsyncBufReadExt, BufReader};

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

/// Append a line of output to the task.
pub fn push_line(task_id: &str, line: &str) {
    let mut tasks = TASKS.lock();
    if let Some(task) = tasks.get_mut(task_id) {
        task.lines.push(line.to_string());
    }
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

/// Remove tasks older than TASK_TTL_SECS.
pub fn gc() {
    let cutoff = now_ts() - TASK_TTL_SECS;
    let mut tasks = TASKS.lock();
    tasks.retain(|_, t| t.created_at > cutoff);
}

/// Spawn a command with piped stdout/stderr, streaming lines to the task store.
/// Returns the exit code.
pub async fn run_streaming_cmd(task_id: &str, cmd: &str, args: &[&str]) -> i32 {
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
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            push_line(&tid_out, &line);
        }
    });

    let tid_err = task_id.to_string();
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            push_line(&tid_err, &line);
        }
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
