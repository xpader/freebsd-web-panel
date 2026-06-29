//! Interactive web terminal — bridges a browser WebSocket to a PTY-backed shell.
//!
//! A browser cannot set custom headers (e.g. Authorization) on a WebSocket
//! handshake, so the session token is passed as a `?token=` query parameter and
//! verified here before the upgrade. The PTY child is a real FreeBSD pseudo
//! terminal allocated via posix_openpt(2); fork(2)+execve(2) gives the shell a
//! controlling tty, so full-screen programs (top, vi, …) work. All `unsafe` is
//! confined to this module behind safe wrappers.

use std::ffi::{CStr, CString};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::auth::{hash_token, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

#[derive(Deserialize)]
pub struct TermParams {
    token: String,
}

/// WebSocket upgrade handler. Validates the query-param token, records an audit
/// entry, then hands the upgraded socket to the session driver.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(params): Query<TermParams>,
) -> ApiResult<impl IntoResponse> {
    let user = authenticate(&state, &params.token).await?;
    crate::audit::record(
        &state,
        Some(&user.username),
        "GET",
        "/api/term/ws",
        200,
        Some("terminal session opened".to_string()),
    );
    Ok(ws.on_upgrade(move |socket| run_session(socket, user)))
}

async fn authenticate(state: &AppState, token: &str) -> ApiResult<AuthUser> {
    let hash = hash_token(token);
    let now = state.now_ts();
    let session = {
        let conn = state.db.lock().await;
        crate::db::get_session_by_hash(&conn, &hash, now)?
    };
    let session = session.ok_or(ApiError::NotAuthenticated)?;
    let user = {
        let conn = state.db.lock().await;
        crate::db::get_user(&conn, session.user_id)?
    };
    let user = user.ok_or(ApiError::NotAuthenticated)?;
    Ok(AuthUser {
        user_id: user.id,
        username: user.username.clone(),
        role: user.role.clone(),
    })
}

/// Drive one terminal session: spawn the PTY shell, then pump bytes between the
/// WebSocket and the PTY master until either side closes.
async fn run_session(mut socket: WebSocket, user: AuthUser) {
    let (master, _slave_path, shell) = match setup_pty_shell() {
        Ok(v) => v,
        Err(e) => {
            send_text(&mut socket, "error", &e).await;
            return;
        }
    };
    let _ = set_winsize(master, DEFAULT_COLS, DEFAULT_ROWS);

    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(64);
    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(64);

    // Reader: blocking loop draining the PTY master → channel → websocket.
    let master_read = master;
    let reader = tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 8192];
        loop {
            let n = unsafe { libc::read(master_read, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            if out_tx.blocking_send(buf[..n as usize].to_vec()).is_err() {
                break;
            }
        }
    });

    // Writer: blocking loop draining input channel → PTY master.
    let master_write = master;
    let writer = tokio::task::spawn_blocking(move || {
        while let Some(data) = in_rx.blocking_recv() {
            if !write_all_fd(master_write, &data) {
                return;
            }
        }
    });

    let (mut ws_sender, mut ws_receiver) = socket.split();

    loop {
        tokio::select! {
            msg = out_rx.recv() => match msg {
                Some(data) => {
                    let text = String::from_utf8_lossy(&data).into_owned();
                    let payload = serde_json::json!({ "type": "output", "data": text }).to_string();
                    if ws_sender.send(Message::Text(payload.into())).await.is_err() {
                        break;
                    }
                }
                None => {
                    // PTY reader ended → shell exited.
                    let payload = serde_json::json!({ "type": "exit" }).to_string();
                    let _ = ws_sender.send(Message::Text(payload.into())).await;
                    break;
                }
            },
            msg = ws_receiver.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    if let Some(action) = parse_control(t.as_str()) {
                        match action {
                            Action::Input(bytes) => {
                                if in_tx.send(bytes).await.is_err() {
                                    break;
                                }
                            }
                            Action::Resize(cols, rows) => {
                                let _ = set_winsize(master, cols, rows);
                            }
                        }
                    }
                }
                Some(Ok(Message::Binary(b))) => {
                    // Allow raw binary input for low-latency keystrokes.
                    if in_tx.send(b.to_vec()).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                _ => {}
            }
        }
    }

    // Cleanup: stop accepting input, terminate the shell, reap it, close the PTY.
    drop(in_tx);
    unsafe { libc::kill(shell.pid, libc::SIGHUP); }
    let pid = shell.pid;
    let _ = tokio::task::spawn_blocking(move || {
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0); }
    })
    .await;
    unsafe { libc::close(master); }
    let _ = writer.await;
    let _ = reader.await;
    let _ = user; // retained so the session is bound to an identity
}

enum Action {
    Input(Vec<u8>),
    Resize(u16, u16),
}

fn parse_control(text: &str) -> Option<Action> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    match v.get("type")?.as_str()? {
        "input" => {
            let s = v.get("data")?.as_str()?;
            Some(Action::Input(s.as_bytes().to_vec()))
        }
        "resize" => {
            let cols = v.get("cols").and_then(|x| x.as_u64()).unwrap_or(DEFAULT_COLS as u64) as u16;
            let rows = v.get("rows").and_then(|x| x.as_u64()).unwrap_or(DEFAULT_ROWS as u64) as u16;
            Some(Action::Resize(cols, rows))
        }
        _ => None,
    }
}

async fn send_text(socket: &mut WebSocket, kind: &str, msg: &str) {
    let payload = serde_json::json!({ "type": kind, "data": msg }).to_string();
    let _ = socket.send(Message::Text(payload.into())).await;
    let _ = socket.send(Message::Close(None)).await;
}

/// All the per-session data tied to a spawned shell process.
struct Shell {
    pid: libc::pid_t,
    _shell: CString, // kept to avoid early drop of the path string
}

/// Allocate a PTY, fork+exec a login shell, and return the master fd + child pid.
fn setup_pty_shell() -> Result<(libc::c_int, CString, Shell), String> {
    let (master, slave_path) = open_pty().map_err(|e| format!("openpty: {e}"))?;
    let user_info = current_user_info();
    let shell_cstr = user_info.shell.clone();
    let home_cstr = user_info.home.clone();
    let basename = shell_cstr
        .to_str()
        .unwrap_or("/bin/sh")
        .rsplit('/')
        .next()
        .unwrap_or("sh");
    // argv[0] with a leading '-' requests login-shell behaviour.
    let argv0 = CString::new(format!("-{basename}")).unwrap();
    let envp = build_env(&user_info);

    let pid = spawn_shell(master, &slave_path, &shell_cstr, &home_cstr, &argv0, &envp)
        .map_err(|e| format!("spawn: {e}"))?;

    let shell = Shell { pid, _shell: shell_cstr };
    Ok((master, slave_path, shell))
}

/// Open a master/slave pseudo-terminal pair and return (master fd, slave path).
fn open_pty() -> std::io::Result<(libc::c_int, CString)> {
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        if master < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::grantpt(master) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(master);
            return Err(e);
        }
        if libc::unlockpt(master) < 0 {
            let e = std::io::Error::last_os_error();
            libc::close(master);
            return Err(e);
        }
        let mut buf = [0u8; 256];
        if libc::ptsname_r(master, buf.as_mut_ptr() as *mut libc::c_char, buf.len()) != 0 {
            let e = std::io::Error::last_os_error();
            libc::close(master);
            return Err(e);
        }
        let slave = CStr::from_ptr(buf.as_ptr() as *const libc::c_char).to_owned();
        Ok((master, slave))
    }
}

/// fork+exec: the child becomes a session leader, opens the slave as its
/// controlling tty, wires stdio to it, then execs the shell. The parent returns
/// the child pid. Only async-signal-safe calls happen between fork and exec.
fn spawn_shell(
    master: libc::c_int,
    slave_path: &CStr,
    shell: &CStr,
    home: &CStr,
    argv0: &CStr,
    envp: &[CString],
) -> std::io::Result<libc::pid_t> {
    // Pre-build the NULL-terminated argv/envp pointer arrays before forking.
    let argv: [*const libc::c_char; 2] = [argv0.as_ptr(), std::ptr::null()];
    let mut env_ptrs: Vec<*const libc::c_char> = envp.iter().map(|s| s.as_ptr()).collect();
    env_ptrs.push(std::ptr::null());

    unsafe {
        let pid = libc::fork();
        if pid < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if pid == 0 {
            // === child ===
            libc::setsid();
            let slave = libc::open(slave_path.as_ptr(), libc::O_RDWR);
            if slave < 0 {
                libc::_exit(127);
            }
            // Acquire the slave as the controlling terminal of the new session.
            libc::ioctl(slave, libc::TIOCSCTTY, 0);
            libc::dup2(slave, libc::STDIN_FILENO);
            libc::dup2(slave, libc::STDOUT_FILENO);
            libc::dup2(slave, libc::STDERR_FILENO);
            if slave > libc::STDERR_FILENO {
                libc::close(slave);
            }
            libc::close(master);
            // Start in the user's home directory, like an ssh login.
            libc::chdir(home.as_ptr());
            libc::execve(shell.as_ptr(), argv.as_ptr(), env_ptrs.as_ptr());
            // exec failed
            libc::_exit(127);
        }
        // === parent ===
        Ok(pid)
    }
}

/// The identity a terminal session runs as: the current process's effective
/// user, resolved from the passwd database so the shell, home dir and login
/// name all match a real login session for that user.
struct UserInfo {
    name: String,
    shell: CString,
    home: CString,
}

/// Resolve the current process user's login shell, home directory and name from
/// the passwd database (getuid + getpwuid), falling back to /bin/sh and the
/// current directory when the lookup fails.
fn current_user_info() -> UserInfo {
    let default_shell = CString::new("/bin/sh").unwrap();
    let default_home = std::env::current_dir()
        .ok()
        .and_then(|p| CString::new(p.to_string_lossy().into_owned()).ok())
        .unwrap_or_else(|| CString::new("/").unwrap());
    let euid = unsafe { libc::geteuid() };
    unsafe {
        let pw = libc::getpwuid(euid);
        if !pw.is_null() {
            let name = if (*pw).pw_name.is_null() {
                String::new()
            } else {
                CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned()
            };
            let shell = if (*pw).pw_shell.is_null() {
                None
            } else {
                CStr::from_ptr((*pw).pw_shell).to_str().ok()
            };
            let home = if (*pw).pw_dir.is_null() {
                None
            } else {
                CStr::from_ptr((*pw).pw_dir).to_str().ok()
            };
            let shell = shell
                .filter(|s| !s.is_empty())
                .map(|s| CString::new(s).unwrap())
                .unwrap_or(default_shell);
            let home = home
                .filter(|s| !s.is_empty())
                .map(|s| CString::new(s).unwrap())
                .unwrap_or(default_home);
            let name = if name.is_empty() {
                euid.to_string()
            } else {
                name
            };
            return UserInfo { name, shell, home };
        }
    }
    UserInfo {
        name: euid.to_string(),
        shell: default_shell,
        home: default_home,
    }
}

/// Build the child environment: inherit the current env, force a known TERM,
/// HOME/USER/PATH so the shell behaves predictably. HOME/USER/LOGNAME are set
/// explicitly to the resolved user's identity to match a real login session.
fn build_env(user: &UserInfo) -> Vec<CString> {
    let mut map: std::collections::HashMap<String, String> = std::env::vars().collect();
    let home_str = user.home.to_str().unwrap_or("/");
    map.insert("TERM".to_string(), "xterm-256color".to_string());
    map.insert("HOME".to_string(), home_str.to_string());
    map.insert("USER".to_string(), user.name.clone());
    map.insert("LOGNAME".to_string(), user.name.clone());
    map.entry("PATH".to_string()).or_insert_with(|| {
        "/sbin:/bin:/usr/sbin:/usr/bin:/usr/local/sbin:/usr/local/bin".to_string()
    });
    let mut out = Vec::new();
    for (k, v) in map {
        if let Ok(s) = CString::new(format!("{k}={v}")) {
            out.push(s);
        }
    }
    out
}

/// Update the PTY window size; the kernel forwards SIGWINCH to the shell.
fn set_winsize(master: libc::c_int, cols: u16, rows: u16) -> std::io::Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let r = unsafe { libc::ioctl(master, libc::TIOCSWINSZ, &ws) };
    if r < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Write the full buffer to an fd, retrying around partial writes / EINTR.
/// Returns false if the fd is no longer writable (PTY closed).
fn write_all_fd(fd: libc::c_int, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        let n = unsafe { libc::write(fd, data.as_ptr() as *const _, data.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        data = &data[n as usize..];
    }
    true
}
