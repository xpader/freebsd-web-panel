//! pkg package management — list, search, install, delete, and view details.
//!
//! ## Strategy
//!
//! - **List**: `pkg query` with TSV format (lightweight, no multiline fields).
//! - **Detail**: `pkg info -R --raw-format json-compact` for the bulk of data
//!   (description, deps, categories, licenses — all in one structured payload),
//!   supplemented by `pkg query '%a\t%k\t%V\t%R'` for fields absent from the
//!   raw manifest (automatic/locked/vital/repository), and
//!   `pkg query '%rn\t%rv'` for reverse dependencies.
//! - **Files**: `pkg query '%Fp\t%Fu\t%Fg\t%Fm'` (lazy-loaded).
//! - **Search**: `pkg rquery -g '%n\t%v\t%o\t%c\t%sh' '*pattern*'` (remote repo).
//! - **Install/Delete**: background `tokio::spawn` running `pkg install -y` /
//!   `pkg delete -y`, with stdout/stderr captured line-by-line into a shared
//!   task store polled by the frontend.

use std::collections::{BTreeMap, HashMap};
use std::process::Command;
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Path as AxumPath, Query, State};
use axum::response::{sse::{Event, KeepAlive, Sse}, IntoResponse, Response};
use axum::Json;
use futures_util::stream::{self, StreamExt};
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::audit;
use crate::auth::{validate_token, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;

const PKG: &str = "/usr/sbin/pkg";

static RE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_+.{}@-]+$").unwrap());

static RE_SEARCH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_+.{}@*?-]+$").unwrap());

// ---- Public data models ----

#[derive(Debug, Serialize)]
pub struct PackageSummary {
    pub name: String,
    pub version: String,
    pub origin: String,
    pub comment: String,
    pub automatic: bool,
    pub size: String,
    pub homepage: String,
    pub maintainer: String,
    pub install_timestamp: i64,
}

#[derive(Debug, Serialize)]
pub struct PackageDetail {
    pub name: String,
    pub version: String,
    pub origin: String,
    pub prefix: String,
    pub comment: String,
    pub description: String,
    pub homepage: String,
    pub maintainer: String,
    pub automatic: bool,
    pub locked: bool,
    pub vital: bool,
    pub size_bytes: i64,
    pub arch: String,
    pub abi: String,
    pub repository: String,
    pub install_timestamp: i64,
    pub categories: Vec<String>,
    pub licenses: Vec<String>,
    pub license_logic: String,
    pub dependencies: Vec<DepInfo>,
    pub reverse_dependencies: Vec<DepInfo>,
    pub messages: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DepInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct PackageFile {
    pub path: String,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub name: String,
    pub version: String,
    pub origin: String,
    pub comment: String,
    pub size: String,
}

// ---- Internal: raw manifest deserialized from `pkg info -R` JSON ----

#[derive(Debug, Deserialize)]
struct RawManifest {
    name: String,
    version: String,
    origin: String,
    comment: String,
    maintainer: String,
    www: String,
    abi: String,
    arch: String,
    prefix: String,
    flatsize: i64,
    timestamp: i64,
    licenselogic: String,
    #[serde(default)]
    licenses: Vec<String>,
    desc: String,
    #[serde(default)]
    deps: BTreeMap<String, RawDep>,
    #[serde(default)]
    categories: Vec<String>,
    /// pkg-message entries; each has a message body and a type (install/remove/upgrade).
    #[serde(default)]
    messages: Vec<RawMessage>,
}

#[derive(Debug, Deserialize)]
struct RawDep {
    #[allow(dead_code)]
    origin: String,
    version: String,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    message: String,
    #[serde(default)]
    #[allow(dead_code)]
    r#type: String,
}

// ---- Background task infrastructure ----

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct PkgTask {
    pub id: String,
    pub action: String,
    pub packages: Vec<String>,
    pub status: TaskStatus,
    pub exit_code: Option<i32>,
    pub lines: Vec<String>,
    pub created_at: i64,
}

static TASKS: LazyLock<Mutex<HashMap<String, PkgTask>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const MAX_SEARCH_RESULTS: usize = 100;
const TASK_TTL_SECS: i64 = 600;

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
    // Simple entropy from SystemTime nanos.
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

fn push_line(task_id: &str, line: &str) {
    let mut tasks = TASKS.lock();
    if let Some(task) = tasks.get_mut(task_id) {
        task.lines.push(line.to_string());
    }
}

fn set_status(task_id: &str, status: TaskStatus, exit_code: Option<i32>) {
    let mut tasks = TASKS.lock();
    if let Some(task) = tasks.get_mut(task_id) {
        task.status = status;
        task.exit_code = exit_code;
    }
}

/// Remove tasks older than TASK_TTL_SECS.
fn gc_tasks() {
    let cutoff = now_ts() - TASK_TTL_SECS;
    let mut tasks = TASKS.lock();
    tasks.retain(|_, t| t.created_at > cutoff);
}

// ---- Query params ----

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub filter: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
}

#[derive(Debug, Deserialize)]
pub struct InstallRequest {
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteRequest {
    pub packages: Vec<String>,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Deserialize)]
pub struct PreviewRequest {
    /// "install" or "delete".
    pub action: String,
    pub packages: Vec<String>,
}

// ---- Helpers ----

fn run(args: &[&str]) -> ApiResult<String> {
    let output = Command::new(PKG).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ApiError::Command(if stderr.is_empty() {
            "pkg failed".to_string()
        } else {
            stderr
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn validate_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 256 {
        return Err(ApiError::BadRequest("invalid package name".into()));
    }
    if !RE_NAME.is_match(name) {
        return Err(ApiError::BadRequest("invalid package name".into()));
    }
    Ok(())
}

fn validate_names(names: &[String]) -> ApiResult<()> {
    if names.is_empty() {
        return Err(ApiError::BadRequest("no packages specified".into()));
    }
    for n in names {
        validate_name(n)?;
    }
    Ok(())
}

fn validate_search(pattern: &str) -> ApiResult<()> {
    if pattern.is_empty() || pattern.len() > 256 {
        return Err(ApiError::BadRequest("invalid search pattern".into()));
    }
    // Allow glob metacharacters for rquery.
    if !RE_SEARCH.is_match(pattern) {
        return Err(ApiError::BadRequest("invalid search pattern".into()));
    }
    Ok(())
}

fn parse_tsv(line: &str, expected: usize) -> ApiResult<Vec<&str>> {
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != expected {
        return Err(ApiError::Internal(format!(
            "expected {expected} fields, got {}",
            fields.len()
        )));
    }
    Ok(fields)
}

fn parse_dep_list(output: &str) -> Vec<DepInfo> {
    output
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let parts: Vec<&str> = l.split('\t').collect();
            if parts.len() == 2 {
                Some(DepInfo {
                    name: parts[0].to_string(),
                    version: parts[1].to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

// ---- Handlers: read-only ----

/// `GET /api/pkg/packages?filter=all|manual|automatic`
pub async fn list_packages(Query(q): Query<ListQuery>) -> ApiResult<Json<Vec<PackageSummary>>> {
    let fmt = "%n\t%v\t%o\t%c\t%a\t%sh\t%w\t%m\t%t";
    let output = match q.filter.as_deref() {
        Some("manual") => run(&["query", "-e", "%a = 0", fmt])?,
        Some("automatic") => run(&["query", "-e", "%a = 1", fmt])?,
        _ => run(&["query", fmt])?,
    };

    let mut packages = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let f = parse_tsv(line, 9)?;
        packages.push(PackageSummary {
            name: f[0].to_string(),
            version: f[1].to_string(),
            origin: f[2].to_string(),
            comment: f[3].to_string(),
            automatic: f[4] == "1",
            size: f[5].to_string(),
            homepage: f[6].to_string(),
            maintainer: f[7].to_string(),
            install_timestamp: f[8].parse().unwrap_or(0),
        });
    }
    Ok(Json(packages))
}

/// `GET /api/pkg/packages/{name}`
pub async fn package_detail(AxumPath(name): AxumPath<String>) -> ApiResult<Json<PackageDetail>> {
    validate_name(&name)?;

    // 1. Raw manifest via JSON — contains desc, deps, categories, licenses, etc.
    let raw_json = run(&["info", "-R", "--raw-format", "json-compact", &name])?;
    let manifests: Vec<RawManifest> = serde_json::from_str(&raw_json).map_err(|e| {
        ApiError::NotFound(format!("package '{name}' not found: {e}"))
    })?;
    let m = manifests.into_iter().next().ok_or_else(|| {
        ApiError::NotFound(format!("package '{name}' not found"))
    })?;

    // 2. Fields absent from raw manifest: automatic, locked, vital, repository.
    let extra = run(&["query", "%a\t%k\t%V\t%R", &name])?;
    let extra_line = extra.lines().next().unwrap_or("");
    let ef = parse_tsv(extra_line, 4)?;
    let (automatic, locked, vital, repository) = (
        ef[0] == "1",
        ef[1] == "1",
        ef[2] == "1",
        ef[3].to_string(),
    );

    // 3. Reverse dependencies (not available in raw manifest).
    let rdep_out = run(&["query", "%rn\t%rv", &name])?;
    let reverse_dependencies = parse_dep_list(&rdep_out);

    // 4. Dependencies from raw manifest (map → sorted vector).
    let dependencies: Vec<DepInfo> = m
        .deps
        .iter()
        .map(|(k, v)| DepInfo {
            name: k.clone(),
            version: v.version.clone(),
        })
        .collect();

    Ok(Json(PackageDetail {
        name: m.name,
        version: m.version,
        origin: m.origin,
        prefix: m.prefix,
        comment: m.comment,
        description: m.desc,
        homepage: m.www,
        maintainer: m.maintainer,
        automatic,
        locked,
        vital,
        size_bytes: m.flatsize,
        arch: m.arch,
        abi: m.abi,
        repository,
        install_timestamp: m.timestamp,
        categories: m.categories,
        licenses: m.licenses,
        license_logic: m.licenselogic,
        dependencies,
        reverse_dependencies,
        messages: m.messages.into_iter().map(|m| m.message).collect(),
    }))
}

/// `GET /api/pkg/packages/{name}/files`
pub async fn package_files(AxumPath(name): AxumPath<String>) -> ApiResult<Json<Vec<PackageFile>>> {
    validate_name(&name)?;

    let fmt = "%Fp\t%Fu\t%Fg\t%Fm";
    let output = run(&["query", fmt, &name])?;

    let mut files = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, '\t').collect();
        files.push(PackageFile {
            path: parts[0].to_string(),
            owner: parts.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string()),
            group: parts.get(2).filter(|s| !s.is_empty()).map(|s| s.to_string()),
            mode: parts.get(3).filter(|s| !s.is_empty()).map(|s| s.to_string()),
        });
    }
    Ok(Json(files))
}

/// `GET /api/pkg/search?q=pattern`
pub async fn search(Query(q): Query<SearchQuery>) -> ApiResult<Json<Vec<SearchResult>>> {
    let pattern = q.q.trim();
    validate_search(pattern)?;

    // Use glob matching with wildcards on both sides for substring search.
    let glob = format!("*{pattern}*");
    let fmt = "%n\t%v\t%o\t%c\t%sh";
    let output = run(&["rquery", "-g", fmt, &glob])?;

    let mut results = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let f = parse_tsv(line, 5)?;
        results.push(SearchResult {
            name: f[0].to_string(),
            version: f[1].to_string(),
            origin: f[2].to_string(),
            comment: f[3].to_string(),
            size: f[4].to_string(),
        });
        if results.len() >= MAX_SEARCH_RESULTS {
            break;
        }
    }
    Ok(Json(results))
}

// ---- Handlers: preview (dry-run) ----

#[derive(Debug, Serialize)]
pub struct PreviewResult {
    pub install: Vec<String>,
    pub delete: Vec<String>,
}

/// `POST /api/pkg/preview`
/// Runs `pkg install -n` or `pkg delete -nR` in dry-run mode to determine
/// which packages will be affected. Returns the package names to be installed
/// or removed.
pub async fn preview(Json(req): Json<PreviewRequest>) -> ApiResult<Json<PreviewResult>> {
    validate_names(&req.packages)?;

    let mut args = vec!["-n".to_string()];
    if req.action == "delete" {
        args.push("-R".to_string());
    }
    for p in &req.packages {
        args.push(p.clone());
    }

    let output = Command::new(PKG)
        .arg(&req.action)
        .args(&args)
        .output()?;

    // dry-run may exit non-zero (e.g. "already installed"); parse stdout/stderr
    // regardless.
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mut install = Vec::new();
    let mut delete = Vec::new();
    let mut section: Option<&str> = None;

    for line in combined.lines() {
        let trimmed = line.trim();
        if trimmed.contains("New packages to be INSTALLED") {
            section = Some("install");
            continue;
        }
        if trimmed.contains("Installed packages to be REMOVED") {
            section = Some("delete");
            continue;
        }
        if trimmed.starts_with("Number of packages") || trimmed.is_empty() {
            section = None;
            continue;
        }
        if let Some(s) = section {
            // Lines like: "\tvim: 9.2.0277 [FreeBSD-ports]"
            if let Some(name) = trimmed.split(':').next() {
                let name = name.trim();
                if !name.is_empty() && !name.starts_with("Number") {
                    if s == "install" {
                        install.push(name.to_string());
                    } else {
                        delete.push(name.to_string());
                    }
                }
            }
        }
    }

    Ok(Json(PreviewResult { install, delete }))
}

// ---- Handlers: install / delete ----

/// `POST /api/pkg/install`
pub async fn install(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<InstallRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_names(&req.packages)?;

    gc_tasks();

    let id = gen_id();
    let task = PkgTask {
        id: id.clone(),
        action: "install".to_string(),
        packages: req.packages.clone(),
        status: TaskStatus::Running,
        exit_code: None,
        lines: Vec::new(),
        created_at: now_ts(),
    };
    TASKS.lock().insert(id.clone(), task);

    let pkgs = req.packages.clone();
    let tid = id.clone();
    let username = user.username.clone();
    tokio::spawn(async move {
        run_pkg_background(&tid, "install", &pkgs, false).await;
        let (status, _exit_code) = {
            let tasks = TASKS.lock();
            let t = tasks.get(&tid);
            (t.map(|t| t.status.clone()), t.and_then(|t| t.exit_code))
        };
        let ok = status == Some(TaskStatus::Done);
        audit::record(
            &state,
            Some(&username),
            "POST",
            "/api/pkg/install",
            if ok { 200 } else { 500 },
            Some(format!(
                "pkg install {}: {}",
                pkgs.join(", "),
                if ok { "ok" } else { "failed" }
            )),
        );
    });

    Ok(Json(serde_json::json!({ "task_id": id })))
}

/// `POST /api/pkg/delete`
pub async fn delete(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<DeleteRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_names(&req.packages)?;

    gc_tasks();

    let id = gen_id();
    let task = PkgTask {
        id: id.clone(),
        action: "delete".to_string(),
        packages: req.packages.clone(),
        status: TaskStatus::Running,
        exit_code: None,
        lines: Vec::new(),
        created_at: now_ts(),
    };
    TASKS.lock().insert(id.clone(), task);

    let pkgs = req.packages.clone();
    let recursive = req.recursive;
    let tid = id.clone();
    let username = user.username.clone();
    tokio::spawn(async move {
        run_pkg_background(&tid, "delete", &pkgs, recursive).await;
        let (status, _exit_code) = {
            let tasks = TASKS.lock();
            let t = tasks.get(&tid);
            (t.map(|t| t.status.clone()), t.and_then(|t| t.exit_code))
        };
        let ok = status == Some(TaskStatus::Done);
        audit::record(
            &state,
            Some(&username),
            "POST",
            "/api/pkg/delete",
            if ok { 200 } else { 500 },
            Some(format!(
                "pkg delete {}: {}",
                pkgs.join(", "),
                if ok { "ok" } else { "failed" }
            )),
        );
    });

    Ok(Json(serde_json::json!({ "task_id": id })))
}

/// `GET /api/pkg/tasks/{id}`
pub async fn task_status(AxumPath(id): AxumPath<String>) -> ApiResult<Json<PkgTask>> {
    let tasks = TASKS.lock();
    let task = tasks
        .get(&id)
        .ok_or_else(|| ApiError::NotFound("task not found".into()))?;
    Ok(Json(task.clone()))
}

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    token: String,
}

/// `GET /api/pkg/tasks/{id}/stream` — SSE stream of task output.
/// Public route (token validated via query param, like WebSocket terminal).
pub async fn task_stream(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(params): Query<StreamParams>,
) -> Response {
    if validate_token(&state, &params.token).await.is_err() {
        return ApiError::NotAuthenticated.into_response();
    }

    let stream = stream::unfold(0usize, move |last_len| {
        let id = id.clone();
        async move {
            let task = {
                let tasks = TASKS.lock();
                tasks.get(&id).cloned()
            };
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
                "packages": task.packages,
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
        // Signal completion so the browser EventSource gets a clean close.
        Event::default().event("done").data("[\"done\"]")
    }))
    .map(Ok::<_, Infallible>);

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

/// Run `pkg install -y` or `pkg delete -y` in the background, streaming
/// stdout/stderr line by line into the shared task store.
async fn run_pkg_background(task_id: &str, action: &str, packages: &[String], recursive: bool) {
    let mut cmd = tokio::process::Command::new(PKG);
    cmd.arg(action).arg("-y");
    if action == "delete" && recursive {
        cmd.arg("-R");
    }
    for p in packages {
        cmd.arg(p);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            push_line(task_id, &format!("Failed to spawn pkg: {e}"));
            set_status(task_id, TaskStatus::Failed, Some(-1));
            return;
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

    let exit_code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1);
    let status = if exit_code == 0 {
        TaskStatus::Done
    } else {
        TaskStatus::Failed
    };
    set_status(task_id, status, Some(exit_code));
}
