//! rc.d service management — list available/enabled services and run
//! start/stop/restart.
//!
//! ## Status checking strategy
//!
//! 1. **Enabled status**: determined by reading `/etc/defaults/rc.conf` and
//!    `/etc/rc.conf` directly (no subprocess). Each service's `rcvar` is
//!    parsed from its rc.d script and looked up in the merged rc.conf map.
//! 2. **Running status** (fast path): one `ps -ax` snapshot + rc.d script
//!    parsing (pidfile / procname) — no per-service subprocess.
//! 3. **Running status** (fallback): services whose pidfile/procname cannot
//!    be resolved fall back to `service <name> status`, run in parallel.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use axum::extract::{Path as AxumPath, State};
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::AppState;

const SERVICE: &str = "/usr/sbin/service";
const PS: &str = "/bin/ps";

/// Pseudo-services that are pure dependency markers (no start/stop/status).
const PSEUDO_SERVICES: &[&str] = &[
    "DAEMON",
    "FILESYSTEMS",
    "LOGIN",
    "NETWORKING",
    "SERVERS",
];

static RE_DESC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*desc="([^"]*)""#).unwrap());
/// Double-quoted assignment: `var="value"`
static RE_ASSIGN_DQ: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*([a-zA-Z_][a-zA-Z0-9_]*)="([^"]*)""#).unwrap());
/// Unquoted assignment: `var=value`
static RE_ASSIGN_UQ: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r##"^\s*([a-zA-Z_][a-zA-Z0-9_]*)=([^\s"#]+)"##).unwrap());
/// Default assignment: `: ${var:="value"}`
static RE_DEFAULT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*:\s*\$\{([a-zA-Z_][a-zA-Z0-9_]*):="([^"]*)"\}"#).unwrap()
});
static RE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_.-]+$").unwrap());

#[derive(Debug, Serialize)]
pub struct ServiceInfo {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub running: bool,
    /// "system" (/etc/rc.d) or "local" (/usr/local/etc/rc.d).
    pub source: String,
}

#[derive(Debug, Serialize)]
pub struct ServiceActionResponse {
    pub name: String,
    pub action: String,
    pub output: String,
}

/// Run a command and return its stdout, or an ApiError on failure.
fn run(cmd: &str, args: &[&str]) -> ApiResult<String> {
    let output = Command::new(cmd).args(args).output()?;
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

/// Validate a service name: must match `[a-zA-Z0-9_.-]+`, 1–128 chars.
fn validate_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 128 {
        return Err(ApiError::BadRequest("invalid service name length".into()));
    }
    if !RE_NAME.is_match(name) {
        return Err(ApiError::BadRequest("invalid service name".into()));
    }
    Ok(())
}

/// Parse a single shell variable assignment line (double-quoted, unquoted,
/// or `: ${var:="default"}` form). Inserts into `vars`, later files override
/// earlier ones.
fn parse_var_line(line: &str, vars: &mut HashMap<String, String>) {
    if let Some(cap) = RE_ASSIGN_DQ.captures(line) {
        vars.insert(cap[1].to_string(), cap[2].to_string());
    } else if let Some(cap) = RE_ASSIGN_UQ.captures(line) {
        vars.insert(cap[1].to_string(), cap[2].to_string());
    } else if let Some(cap) = RE_DEFAULT.captures(line) {
        vars.entry(cap[1].to_string()).or_insert_with(|| cap[2].to_string());
    }
}

/// Read all shell variables from rc.conf files (defaults + user overrides).
/// Returns a merged map where user values override defaults.
fn read_rcconf() -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for path in &[
        "/etc/defaults/rc.conf",
        "/etc/rc.conf",
        "/etc/rc.conf.local",
    ] {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                parse_var_line(line, &mut vars);
            }
        }
    }
    vars
}

/// Locate the rc.d script path for a service name.
fn find_script(name: &str) -> Option<String> {
    for dir in &["/etc/rc.d", "/usr/local/etc/rc.d"] {
        let path = format!("{dir}/{name}");
        if Path::new(&path).exists() {
            return Some(path);
        }
    }
    None
}

/// Parse description and all shell variables from an rc.d script.
/// Returns `(description, variable_map)`.
fn parse_script(path: &str) -> Option<(Option<String>, HashMap<String, String>)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut vars = HashMap::new();
    let mut description = None;

    for line in content.lines() {
        if description.is_none() {
            if let Some(cap) = RE_DESC.captures(line) {
                description = Some(cap[1].to_string());
            }
        }
        parse_var_line(line, &mut vars);
    }

    Some((description, vars))
}

/// Expand `${var}` references using a variable map.
/// Returns None if any references remain unresolved after 5 iterations.
fn expand_vars(value: &str, vars: &HashMap<String, String>) -> Option<String> {
    let mut result = value.to_string();
    for _ in 0..5 {
        let before = result.clone();
        for (k, v) in vars {
            let pattern = format!("${{{}}}", k);
            result = result.replace(&pattern, v);
        }
        if result == before {
            break;
        }
    }
    if result.contains("${") {
        None
    } else {
        Some(result)
    }
}

/// Get a snapshot of all running processes via a single `ps` call.
fn process_table() -> HashMap<i32, String> {
    let raw = run(PS, &["-ax", "-o", "pid=", "-o", "command="]).unwrap_or_default();
    let mut procs = HashMap::new();
    for line in raw.lines() {
        let line = line.trim_start();
        let Some(ws) = line.find(char::is_whitespace) else {
            continue;
        };
        let Ok(pid) = line[..ws].parse::<i32>() else {
            continue;
        };
        let cmd = line[ws..].trim().to_string();
        if pid > 0 && !cmd.is_empty() {
            procs.insert(pid, cmd);
        }
    }
    procs
}

/// Check if a service is running by reading its pidfile and looking up the PID.
/// Returns `None` if the pidfile doesn't exist or can't be parsed.
fn check_pidfile(pidfile: &str, procs: &HashMap<i32, String>) -> Option<bool> {
    let content = std::fs::read_to_string(pidfile).ok()?;
    let pid_str = content.trim().split_whitespace().next()?;
    let pid: i32 = pid_str.parse().ok()?;
    Some(procs.contains_key(&pid))
}

/// Check if a service is running by matching procname against the process table.
fn check_procname(procname: &str, procs: &HashMap<i32, String>) -> bool {
    let pn = procname.trim();
    procs.values().any(|cmd| {
        let exe = cmd.split_whitespace().next().unwrap_or("");
        if exe == pn {
            return true;
        }
        // If procname is a basename (no /), match against exe's basename too.
        if !pn.contains('/') {
            return Path::new(exe).file_name().map_or(false, |f| f == pn);
        }
        false
    })
}

/// Fallback: check via `service <name> status` (spawns a subprocess).
fn check_via_service(name: &str) -> bool {
    Command::new(SERVICE)
        .arg(name)
        .arg("status")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Try to resolve running status from parsed script variables + process table.
/// Returns `Some(bool)` on success, `None` if it cannot be determined.
fn try_fast_check(
    vars: &HashMap<String, String>,
    procs: &HashMap<i32, String>,
) -> Option<bool> {
    // 1. pidfile path → read PID → look up in process table.
    if let Some(raw) = vars.get("pidfile") {
        if let Some(pf) = expand_vars(raw, vars) {
            if let Some(running) = check_pidfile(&pf, procs) {
                return Some(running);
            }
        }
    }

    // 2. procname (or command) → match against process table.
    let pn_raw = vars.get("procname").or_else(|| vars.get("command"));
    if let Some(pn) = pn_raw.and_then(|v| expand_vars(v, vars)) {
        // Only use for matching if it's a clean executable path (no args).
        if !pn.contains(char::is_whitespace) {
            return Some(check_procname(&pn, procs));
        }
    }

    None
}

/// Determine the rcvar for a service from its script variables.
/// Falls back to `${name}_enable` (using the script's `name` or the filename).
fn resolve_rcvar(filename: &str, vars: &HashMap<String, String>) -> String {
    if let Some(raw) = vars.get("rcvar") {
        if let Some(expanded) = expand_vars(raw, vars) {
            return expanded;
        }
    }
    let svc_name = vars
        .get("name")
        .and_then(|n| expand_vars(n, vars))
        .unwrap_or_else(|| filename.to_string());
    format!("{svc_name}_enable")
}

/// GET /api/services — list all rc.d services with enabled/running status,
/// sorted by name. Pseudo-services (DAEMON, FILESYSTEMS, …) are excluded.
pub async fn list() -> ApiResult<Json<Vec<ServiceInfo>>> {
    let infos = tokio::task::spawn_blocking(collect_services)
        .await
        .map_err(|e| ApiError::Internal(format!("task join error: {e}")))??;
    Ok(Json(infos))
}

/// Synchronous implementation — runs on a blocking thread.
fn collect_services() -> ApiResult<Vec<ServiceInfo>> {
    // All available service scripts (names only).
    let available_raw = run(SERVICE, &["-l"])?;
    let mut names: Vec<String> = available_raw
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .filter(|l| !PSEUDO_SERVICES.contains(&l.as_str()))
        .collect();
    names.sort();
    names.dedup();

    // Enabled status from rc.conf files (fast, no subprocess).
    let rcconf = read_rcconf();

    // Single process table snapshot (one subprocess for all services).
    let procs = process_table();

    // Phase 1 — fast path: resolve enabled + running status.
    let mut infos: Vec<ServiceInfo> = Vec::with_capacity(names.len());
    let mut fallback_indices: Vec<usize> = Vec::new();

    for (i, name) in names.iter().enumerate() {
        let script_path = find_script(name);
        let source = script_path
            .as_deref()
            .map(|p| if p.starts_with("/etc/rc.d/") { "system" } else { "local" })
            .unwrap_or("unknown");
        let (desc, vars) = script_path
            .as_deref()
            .and_then(|p| parse_script(p))
            .unwrap_or((None, HashMap::new()));

        // Enabled: check rcvar value in rc.conf.
        let rcvar = resolve_rcvar(name, &vars);
        let en = rcconf
            .get(&rcvar)
            .map(|v| v.eq_ignore_ascii_case("YES"))
            .unwrap_or(false);

        let mut running = false;
        if en {
            match try_fast_check(&vars, &procs) {
                Some(r) => running = r,
                None => fallback_indices.push(i),
            }
        }

        infos.push(ServiceInfo {
            name: name.clone(),
            description: desc,
            enabled: en,
            running,
            source: source.to_string(),
        });
    }

    // Phase 2 — fallback: run `service status` in parallel for unresolved
    // enabled services (pidfile/procname could not be resolved from script).
    if !fallback_indices.is_empty() {
        let fb_results: Vec<(usize, bool)> = std::thread::scope(|s| {
            fallback_indices
                .iter()
                .map(|&i| {
                    let name = names[i].clone();
                    s.spawn(move || (i, check_via_service(&name)))
                })
                .map(|h| h.join().unwrap_or((0, false)))
                .collect()
        });
        for (i, running) in fb_results {
            if i < infos.len() {
                infos[i].running = running;
            }
        }
    }

    // Sort: enabled first, then by name.
    infos.sort_by(|a, b| b.enabled.cmp(&a.enabled).then(a.name.cmp(&b.name)));

    Ok(infos)
}

#[derive(Debug, Deserialize)]
pub struct ActionPath {
    pub name: String,
    pub action: String,
}

/// POST /api/services/{name}/{action} — start, stop, or restart a service.
/// Returns the command stdout on success.
pub async fn control(
    State(state): State<AppState>,
    auth: AuthUser,
    AxumPath(p): AxumPath<ActionPath>,
) -> ApiResult<Json<ServiceActionResponse>> {
    validate_name(&p.name)?;
    let action = p.action.clone();
    if !matches!(action.as_str(), "start" | "stop" | "restart") {
        return Err(ApiError::BadRequest(
            "action must be start, stop, or restart".into(),
        ));
    }

    let svc_name = p.name.clone();
    let action_clone = action.clone();

    let task = tokio::task::spawn_blocking(move || {
        // Redirect stdout/stderr to a temp file instead of pipes.
        // `daemon`-wrapped services (e.g. numa, frpc) fork long-lived child
        // processes that inherit pipe FDs; `.output()` then waits for EOF
        // forever. Writing to a file avoids this: the service script exits,
        // its FDs close, and the file content is read back separately.
        let tmp = std::env::temp_dir().join(format!("fwp-svc-{}-{}.log", svc_name, std::process::id()));
        let status = Command::new(SERVICE)
            .arg(&svc_name)
            .arg(&action_clone)
            .stdout(std::fs::File::create(&tmp)?)
            .stderr(std::fs::OpenOptions::new().append(true).open(&tmp)?)
            .status()?;
        let output = std::fs::read_to_string(&tmp).unwrap_or_default();
        let _ = std::fs::remove_file(&tmp);
        let output = output.trim().to_string();
        Ok::<_, std::io::Error>((output, status.success()))
    });

    let (output, success) = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        task,
    )
    .await
    {
        Ok(Ok(Ok(result))) => result,
        Ok(Ok(Err(e))) => return Err(ApiError::Io(e)),
        Ok(Err(e)) => return Err(ApiError::Internal(format!("task join error: {e}"))),
        Err(_) => {
            return Err(ApiError::Command(format!(
                "service {action} {} timed out (>30s)",
                p.name
            )))
        }
    };

    if !success {
        return Err(ApiError::Command(if output.is_empty() {
            format!("service {action} {} failed", p.name)
        } else {
            output
        }));
    }

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        &format!("/api/services/{}/{}", p.name, action),
        200,
        Some(format!("service {} {}", p.name, action)),
    );

    Ok(Json(ServiceActionResponse {
        name: p.name,
        action: action.to_string(),
        output,
    }))
}
