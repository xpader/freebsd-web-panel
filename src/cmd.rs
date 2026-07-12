//! Shared helpers for spawning system commands off the async executor.
//!
//! Synchronous `std::process::Command` calls block the calling thread. If run
//! directly on a tokio async worker thread, a slow binary (e.g. `zfs scrub`,
//! `fetch`) stalls every other concurrent request sharing that worker. This
//! module provides wrappers that execute commands via `spawn_blocking` on the
//! dedicated blocking thread pool, so async workers stay free.
//!
//! ## Functions split into two groups
//!
//! **Async** (call directly from `async fn` handlers):
//! - [`run`] — single-command shortcut, spawns one blocking task internally.
//! - [`run_output`] — same, but returns raw [`Output`] without erroring on
//!   non-zero exit.
//!
//! **Sync** (call from inside a `spawn_blocking` closure, or from a sync helper
//! that is itself wrapped by one):
//! - [`run_sync`] — `ApiResult<String>`, errors on non-zero exit.
//! - [`run_sync_str`] — same but `Result<String, String>` for modules whose
//!   error type is `String` (e.g. `bhyve`).
//! - [`run_forget_sync`] — fire-and-forget, ignores all output and errors.
//! - [`status_sync`] — returns `true`/`false`, discards output.
//! - [`output_ok`] — checks an already-obtained [`Output`].
//!
//! ## Choosing a function
//!
//! | Situation | Use |
//! |-----------|-----|
//! | One command in an async handler | [`run`] |
//! | Need raw exit code + output | [`run_output`] + [`output_ok`] |
//! | Multiple commands / I/O in one handler | `spawn_blocking` + [`run_sync`] |
//! | Caller error type is `String` | [`run_sync_str`] |
//! | Cleanup, failure is OK | [`run_forget_sync`] |
//! | Only care about success/failure | [`status_sync`] |
//!
//! All functions set `stdin(Stdio::null())`.

use std::process::{Command, Output, Stdio};

use crate::error::{ApiError, ApiResult};

/// Run a command synchronously and return its stdout on success.
///
/// **Context**: sync — call from inside a `spawn_blocking` closure, or from a
/// sync helper that is itself wrapped by `spawn_blocking`.
///
/// **On failure**: non-zero exit → `ApiError::Command` (HTTP 422) with trimmed
/// stderr (or `"{cmd} failed"` if stderr is empty). Spawn failure →
/// `ApiError::Io` (HTTP 500).
///
/// For the async equivalent (self-contained `spawn_blocking`), use [`run`].
pub fn run_sync(cmd: &str, args: &[&str]) -> ApiResult<String> {
    let output = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    output_ok(cmd, &output)
}

/// Run a command via `spawn_blocking`, returning stdout on success.
///
/// **Context**: async — call directly from an `async fn` handler. Wraps
/// [`run_sync`] in a `spawn_blocking` task internally, so the caller does not
/// need to manage blocking threads.
///
/// **On failure**: non-zero exit → `ApiError::Command` (HTTP 422) with trimmed
/// stderr. Spawn failure → `ApiError::Io` (HTTP 500). Join error →
/// `ApiError::Internal`.
///
/// Best for handlers that execute a **single** command. For handlers that mix
/// multiple commands with file I/O or FFI, prefer wrapping everything in one
/// `spawn_blocking` and calling [`run_sync`] inside.
pub async fn run(cmd: &str, args: &[&str]) -> ApiResult<String> {
    let cmd = cmd.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        run_sync(&cmd, &arg_refs)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
}

/// Run a command via `spawn_blocking`, returning the raw [`Output`].
///
/// **Context**: async — call directly from an `async fn` handler.
///
/// Unlike [`run`], this does **not** error on non-zero exit codes. Use when the
/// exit status or combined stdout+stderr is meaningful (e.g. `pkg install -n`
/// dry-run may exit non-zero with useful output). Pair with [`output_ok`] to
/// convert to the usual error semantics when needed.
pub async fn run_output(cmd: &str, args: &[&str]) -> ApiResult<Output> {
    let cmd = cmd.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        Command::new(&cmd)
            .args(&arg_refs)
            .stdin(Stdio::null())
            .output()
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
    .map_err(Into::into)
}

// ── sync helpers (call from within spawn_blocking) ──────────────────

/// Run a command synchronously, returning `Result<String, String>`.
///
/// **Context**: sync — call from inside a `spawn_blocking` closure.
///
/// **Error fallback chain** differs from [`run_sync`]: non-zero exit produces
/// `Err(stderr)` → if stderr empty, `Err(stdout)` → if both empty,
/// `Err("{cmd} failed")`. The extra stdout fallback exists because some
/// utilities (notably `vm-bhyve`) write errors to stdout, not stderr.
///
/// Use in modules whose internal error type is `String` (e.g. `bhyve`).
/// Everywhere else prefer [`run_sync`] which returns `ApiResult`.
pub fn run_sync_str(cmd: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{cmd} exec failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("{cmd} failed")
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Fire-and-forget: run a command, discarding stdout, stderr, and exit code.
///
/// **Context**: sync — call from inside a `spawn_blocking` closure.
///
/// Use for best-effort cleanup operations where failure is acceptable and
/// should not propagate (e.g. `sysrc -x` to delete a key that may not exist).
/// Uses `.status()` (not `.output()`) so no pipe buffers are allocated.
pub fn run_forget_sync(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Run a command, returning `true` if it exited successfully, `false` otherwise.
///
/// **Context**: sync — call from inside a `spawn_blocking` closure.
///
/// stdout and stderr are redirected to `/dev/null`. Use when only the boolean
/// success/failure matters (e.g. `service <name> status` to check if a service
/// is running).
pub fn status_sync(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check a command's [`Output`] and return stdout, or [`ApiError::Command`] on
/// non-zero exit.
///
/// The error message is the trimmed stderr, or `"{cmd} failed"` when stderr is
/// empty. Use to post-process an [`Output`] obtained from [`run_output`] when
/// the standard error-on-non-zero semantics are desired.
pub fn output_ok(cmd: &str, output: &Output) -> ApiResult<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::Command(if stderr.is_empty() {
            format!("{cmd} failed")
        } else {
            stderr
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
