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
use std::fs;
use std::path::PathBuf;
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
use crate::cmd;
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
        Some("manual") => cmd::run(PKG, &["query", "-e", "%a = 0", fmt]).await?,
        Some("automatic") => cmd::run(PKG, &["query", "-e", "%a = 1", fmt]).await?,
        _ => cmd::run(PKG, &["query", fmt]).await?,
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
    let raw_json = cmd::run(PKG, &["info", "-R", "--raw-format", "json-compact", &name]).await?;
    let manifests: Vec<RawManifest> = serde_json::from_str(&raw_json).map_err(|e| {
        ApiError::NotFound(format!("package '{name}' not found: {e}"))
    })?;
    let m = manifests.into_iter().next().ok_or_else(|| {
        ApiError::NotFound(format!("package '{name}' not found"))
    })?;

    // 2. Fields absent from raw manifest: automatic, locked, vital, repository.
    let extra = cmd::run(PKG, &["query", "%a\t%k\t%V\t%R", &name]).await?;
    let extra_line = extra.lines().next().unwrap_or("");
    let ef = parse_tsv(extra_line, 4)?;
    let (automatic, locked, vital, repository) = (
        ef[0] == "1",
        ef[1] == "1",
        ef[2] == "1",
        ef[3].to_string(),
    );

    // 3. Reverse dependencies (not available in raw manifest).
    let rdep_out = cmd::run(PKG, &["query", "%rn\t%rv", &name]).await?;
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
    let output = cmd::run(PKG, &["query", fmt, &name]).await?;

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
    let output = cmd::run(PKG, &["rquery", "-g", fmt, &glob]).await?;

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

    let mut full_args: Vec<&str> = vec![&req.action, "-n"];
    if req.action == "delete" {
        full_args.push("-R");
    }
    full_args.extend(req.packages.iter().map(|s| s.as_str()));
    let output = cmd::run_output(PKG, &full_args).await?;

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

// ================================================================
// Repository management
// ================================================================

const SYSTEM_REPO_DIR: &str = "/etc/pkg";
const USER_REPO_DIR: &str = "/usr/local/etc/pkg/repos";

static RE_REPO_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap());

static RE_URL_SCHEME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(pkg\+https?|https?|file|ssh|tcp)://").unwrap()
});

#[derive(Debug, Clone, Serialize)]
pub struct RepoConfig {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub mirror_type: String,
    pub signature_type: String,
    pub fingerprints: Option<String>,
    pub pubkey: Option<String>,
    pub priority: i64,
    pub ip_version: i64,
    /// This repo is originally from /etc/pkg/ (not yet overridden by user).
    pub is_system_origin: bool,
}

#[derive(Debug, Serialize)]
pub struct RepoFile {
    pub path: String,
    pub filename: String,
    pub is_system: bool,
    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Deserialize)]
pub struct RepoInput {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mirror_type: Option<String>,
    #[serde(default)]
    pub signature_type: Option<String>,
    #[serde(default)]
    pub fingerprints: Option<String>,
    #[serde(default)]
    pub pubkey: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub ip_version: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRepoInput {
    pub filename: String,
    #[serde(flatten)]
    pub repo: RepoInput,
}

/// Parse a single UCL `.conf` file into repo configs.
/// Uses a line-based approach: first split into top-level blocks, then parse key-values.
fn parse_repo_file(path: &str) -> Vec<RepoConfig> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Phase 1: extract raw blocks (name → body text).
    // A block starts with `name:` followed by `{` and ends with matching `}`.
    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut depth: i32 = 0;
    let mut current_name = String::new();
    let mut current_body = String::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        if depth == 0 {
            // Looking for a top-level `name: {` pattern.
            // Extract name and check for '{' on same line.
            if let Some(colon_pos) = line.find(':') {
                let name = line[..colon_pos].trim().to_string();
                let rest = &line[colon_pos + 1..].trim();
                if rest.starts_with('{') {
                    current_name = name;
                    current_body.clear();
                    depth = 1;
                    let after_brace = &rest[1..];
                    let brace_count = after_brace.matches('{').count() as i32
                        - after_brace.matches('}').count() as i32;
                    depth += brace_count;
                    if depth <= 0 {
                        // Single-line block like `name: { enabled: no }`
                        current_body = after_brace.to_string();
                        blocks.push((current_name.clone(), current_body.clone()));
                        depth = 0;
                    } else if !after_brace.trim().is_empty() {
                        current_body = after_brace.to_string();
                        current_body.push('\n');
                    }
                }
            }
        } else {
            // Inside a block — accumulate body lines, track brace depth.
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if depth <= 0 {
                // Remove trailing '}' from this line before storing.
                let body_part = line.rfind('}').map(|pos| &line[..pos]).unwrap_or(line);
                current_body.push_str(body_part);
                current_body.push('\n');
                blocks.push((current_name.clone(), current_body.clone()));
                depth = 0;
            } else {
                current_body.push_str(line);
                current_body.push('\n');
            }
        }
    }

    // Phase 2: parse each block's body into RepoConfig fields.
    blocks
        .into_iter()
        .map(|(name, body)| parse_repo_block(&name, &body))
        .collect()
}

/// Parse a single repo block body (key-value lines) into a RepoConfig.
fn parse_repo_block(name: &str, body: &str) -> RepoConfig {
    let mut url = String::new();
    let mut enabled = true;
    let mut mirror_type = "NONE".to_string();
    let mut signature_type = "NONE".to_string();
    let mut fingerprints = None;
    let mut pubkey = None;
    let mut priority = 0i64;
    let mut ip_version = 0i64;

    for raw_line in body.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line == "{" || line == "}" {
            continue;
        }
        // Find the first ':' as key-value separator.
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() != 2 {
            continue;
        }
        let key = parts[0].trim().to_lowercase();
        let raw_val = parts[1].trim();

        // Strip trailing ';' or ','.
        let val = raw_val.trim_end_matches([';', ',']).trim();

        // Strip quotes.
        let val = if (val.starts_with('"') && val.ends_with('"'))
            || (val.starts_with('\'') && val.ends_with('\''))
        {
            val[1..val.len() - 1].to_string()
        } else {
            val.to_string()
        };

        match key.as_str() {
            "url" => url = val,
            "enabled" => enabled = val == "yes" || val == "true" || val == "1",
            "mirror_type" => mirror_type = val.to_uppercase(),
            "signature_type" => signature_type = val.to_uppercase(),
            "fingerprints" => fingerprints = Some(val),
            "pubkey" => pubkey = Some(val),
            "priority" => priority = val.parse().unwrap_or(0),
            "ip_version" => ip_version = val.parse().unwrap_or(0),
            _ => {}
        }
    }

    RepoConfig {
        name: name.to_string(),
        url,
        enabled,
        mirror_type,
        signature_type,
        fingerprints,
        pubkey,
        priority,
        ip_version,
        is_system_origin: false,
    }
}

/// Merge a user override on top of a system repo, field by field.
/// Only fields that differ from the parser's default (i.e. were explicitly
/// set in the override) are applied; the rest are inherited from system.
fn merge_repo(base: &RepoConfig, override_cfg: &RepoConfig) -> RepoConfig {
    let mut merged = base.clone();
    // url: default is "" — if override has a non-empty url, apply it.
    if !override_cfg.url.is_empty() {
        merged.url = override_cfg.url.clone();
    }
    // enabled: default is true — if override is false, it was explicitly set.
    if !override_cfg.enabled {
        merged.enabled = override_cfg.enabled;
    }
    // mirror_type: default is "NONE".
    if override_cfg.mirror_type != "NONE" {
        merged.mirror_type = override_cfg.mirror_type.clone();
    }
    // signature_type: default is "NONE".
    if override_cfg.signature_type != "NONE" {
        merged.signature_type = override_cfg.signature_type.clone();
    }
    // fingerprints: default is None.
    if override_cfg.fingerprints.is_some() {
        merged.fingerprints = override_cfg.fingerprints.clone();
    }
    // pubkey: default is None.
    if override_cfg.pubkey.is_some() {
        merged.pubkey = override_cfg.pubkey.clone();
    }
    // priority: default is 0.
    if override_cfg.priority != 0 {
        merged.priority = override_cfg.priority;
    }
    // ip_version: default is 0.
    if override_cfg.ip_version != 0 {
        merged.ip_version = override_cfg.ip_version;
    }
    merged.is_system_origin = false; // now has user override
    merged
}

/// Scan both system and user directories, return repos grouped by file.
/// When a user file has the same name as a system file, they are merged:
/// the user file wins (override), and the result is shown as custom.
fn read_all_repo_files() -> Vec<RepoFile> {
    use std::collections::BTreeMap;

    // Collect user files: filename → (path, repos).
    let mut user_map: BTreeMap<String, (String, Vec<RepoConfig>)> = BTreeMap::new();
    if let Ok(entries) = fs::read_dir(USER_REPO_DIR) {
        let mut user_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "conf"))
            .collect();
        user_files.sort();
        for f in user_files {
            let path = f.to_string_lossy().to_string();
            let filename = f
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let repos = parse_repo_file(&path);
            user_map.insert(filename, (path, repos));
        }
    }

    // Collect system files: filename → (path, repos).
    let mut sys_map: BTreeMap<String, (String, Vec<RepoConfig>)> = BTreeMap::new();
    if let Ok(entries) = fs::read_dir(SYSTEM_REPO_DIR) {
        let mut sys_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "conf"))
            .collect();
        sys_files.sort();
        for f in sys_files {
            let path = f.to_string_lossy().to_string();
            let filename = f
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut repos = parse_repo_file(&path);
            for r in &mut repos {
                r.is_system_origin = true;
            }
            sys_map.insert(filename, (path, repos));
        }
    }

    let mut files: Vec<RepoFile> = Vec::new();

    // Walk all filenames (sorted). For each, merge system + user.
    let all_filenames: std::collections::BTreeSet<String> = sys_map
        .keys()
        .chain(user_map.keys())
        .cloned()
        .collect();

    for filename in all_filenames {
        let sys_entry = sys_map.get(&filename);
        let user_entry = user_map.get(&filename);

        if let Some((user_path, user_repos)) = user_entry {
            // User file exists → merge with system.
            // Build a map of system repos by name for lookup.
            let sys_repo_map: HashMap<&str, &RepoConfig> = sys_entry
                .map(|(_, sr)| {
                    sr.iter()
                        .map(|r| (r.name.as_str(), r))
                        .collect::<HashMap<_, _>>()
                })
                .unwrap_or_default();

            let user_names: std::collections::HashSet<&str> =
                user_repos.iter().map(|r| r.name.as_str()).collect();

            let mut merged: Vec<RepoConfig> = Vec::new();

            // Add system repos NOT in user override (still effective as-is).
            if let Some((_, sys_repos)) = sys_entry {
                for r in sys_repos {
                    if !user_names.contains(r.name.as_str()) {
                        merged.push(r.clone());
                    }
                }
            }

            // Add user repos — merge onto system base if one exists.
            for ur in user_repos {
                if let Some(base) = sys_repo_map.get(ur.name.as_str()) {
                    merged.push(merge_repo(base, ur));
                } else {
                    merged.push(ur.clone());
                }
            }

            files.push(RepoFile {
                path: user_path.clone(),
                filename: filename.clone(),
                is_system: false,
                repos: merged,
            });
        } else if let Some((sys_path, sys_repos)) = sys_entry {
            // Only system file, no user override.
            files.push(RepoFile {
                path: sys_path.clone(),
                filename: filename.clone(),
                is_system: true,
                repos: sys_repos.clone(),
            });
        }
    }

    files
}

/// Render a single repo config as a UCL block, writing ONLY fields that
/// differ from the original (system) config.  When `orig` is None (a
/// brand-new custom repo with no system counterpart), the comparison
/// baseline is pkg's built-in defaults.
///
/// This produces the minimal override: e.g. if the user only changed
/// `enabled` from yes to no, the block will be just:
///   FreeBSD-ports: {
///     enabled: no;
///   }
fn render_repo_block_diff(r: &RepoConfig, orig: Option<&RepoConfig>) -> String {
    let mut out = String::new();
    out.push_str(&format!("{}: {{\n", r.name));

    // url: no pkg default — always required for new repos.
    if orig.map_or(true, |o| o.url != r.url) {
        out.push_str(&format!("  url: \"{}\";\n", r.url));
    }
    // enabled: pkg default is yes.
    if orig.map_or(!r.enabled, |o| o.enabled != r.enabled) {
        out.push_str(&format!(
            "  enabled: {};\n",
            if r.enabled { "yes" } else { "no" }
        ));
    }
    // mirror_type: pkg default NONE.
    if orig.map_or(r.mirror_type != "NONE", |o| o.mirror_type != r.mirror_type) {
        out.push_str(&format!("  mirror_type: \"{}\";\n", r.mirror_type));
    }
    // signature_type: pkg default NONE.
    if orig.map_or(r.signature_type != "NONE", |o| o.signature_type != r.signature_type) {
        out.push_str(&format!("  signature_type: \"{}\";\n", r.signature_type));
    }
    // fingerprints.
    if orig.map_or(r.fingerprints.is_some(), |o| o.fingerprints != r.fingerprints) {
        if let Some(fp) = &r.fingerprints {
            if !fp.is_empty() {
                out.push_str(&format!("  fingerprints: \"{}\";\n", fp));
            }
        }
    }
    // pubkey.
    if orig.map_or(r.pubkey.is_some(), |o| o.pubkey != r.pubkey) {
        if let Some(pk) = &r.pubkey {
            if !pk.is_empty() {
                out.push_str(&format!("  pubkey: \"{}\";\n", pk));
            }
        }
    }
    // priority: pkg default 0.
    if orig.map_or(r.priority != 0, |o| o.priority != r.priority) {
        out.push_str(&format!("  priority: {};\n", r.priority));
    }
    // ip_version: pkg default 0.
    if orig.map_or(r.ip_version != 0, |o| o.ip_version != r.ip_version) {
        out.push_str(&format!("  ip_version: {};\n", r.ip_version));
    }

    out.push_str("}\n");
    out
}

/// Read the system repos that share the same basename as `target_path`.
/// Used as the diff baseline when writing override files.
fn system_repos_for_target(target_path: &str) -> Vec<RepoConfig> {
    let filename = std::path::Path::new(target_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let sys_path = format!("{}/{}", SYSTEM_REPO_DIR, filename);
    parse_repo_file(&sys_path)
}

/// Write override repos to a file atomically, producing minimal diffs.
/// Each repo is rendered with only the fields that differ from its system
/// original (or pkg defaults for new repos).  Repos that have zero diffing
/// fields are omitted entirely.  If the file ends up empty, it is deleted.
fn write_override_file(path: &str, repos: &[RepoConfig]) -> ApiResult<()> {
    let sys_repos = system_repos_for_target(path);
    let sys_map: HashMap<&str, &RepoConfig> = sys_repos
        .iter()
        .map(|r| (r.name.as_str(), r))
        .collect();

    let mut content = String::from("# Managed by FreeBSD Web Panel\n\n");
    let mut written = 0;

    for r in repos {
        // Never copy system-origin repos into the override file.
        if r.is_system_origin {
            continue;
        }
        let orig = sys_map.get(r.name.as_str()).copied();
        let block = render_repo_block_diff(r, orig);

        // An empty block (only `name: {` + `}`) means zero fields differ — skip.
        if block.lines().count() <= 2 {
            continue;
        }

        content.push_str(&block);
        content.push('\n');
        written += 1;
    }

    if written == 0 {
        // No overrides remain — remove the file if it exists.
        let _ = fs::remove_file(path);
    } else {
        let tmp = format!("{}.tmp", path);
        fs::write(&tmp, &content)?;
        fs::rename(&tmp, path)?;
    }
    Ok(())
}

/// Convert a RepoInput into a RepoConfig.
fn input_to_config(input: &RepoInput) -> RepoConfig {
    RepoConfig {
        name: input.name.clone(),
        url: input.url.clone(),
        enabled: input.enabled,
        mirror_type: input
            .mirror_type
            .as_deref()
            .unwrap_or("NONE")
            .to_uppercase(),
        signature_type: input
            .signature_type
            .as_deref()
            .unwrap_or("NONE")
            .to_uppercase(),
        fingerprints: input.fingerprints.clone(),
        pubkey: input.pubkey.clone(),
        priority: input.priority.unwrap_or(0),
        ip_version: input.ip_version.unwrap_or(0),
        is_system_origin: false,
    }
}

fn validate_repo_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 128 {
        return Err(ApiError::BadRequest("invalid repository name".into()));
    }
    if !RE_REPO_NAME.is_match(name) {
        return Err(ApiError::BadRequest("invalid repository name".into()));
    }
    Ok(())
}

static RE_FILENAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_.-]+\.conf$").unwrap());

fn validate_filename(filename: &str) -> ApiResult<()> {
    if filename.is_empty() || filename.len() > 256 {
        return Err(ApiError::BadRequest("invalid filename".into()));
    }
    if !RE_FILENAME.is_match(filename) {
        return Err(ApiError::BadRequest("invalid filename".into()));
    }
    // Reject path separators.
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(ApiError::BadRequest("invalid filename".into()));
    }
    Ok(())
}

fn validate_url(url: &str) -> ApiResult<()> {
    if url.is_empty() || url.len() > 1024 {
        return Err(ApiError::BadRequest("invalid URL".into()));
    }
    if !RE_URL_SCHEME.is_match(url) {
        return Err(ApiError::BadRequest("invalid URL scheme".into()));
    }
    Ok(())
}

fn validate_mirror_type(mt: &str) -> ApiResult<()> {
    match mt {
        "NONE" | "HTTP" | "SRV" => Ok(()),
        _ => Err(ApiError::BadRequest("invalid mirror_type".into())),
    }
}

fn validate_signature_type(st: &str) -> ApiResult<()> {
    match st {
        "NONE" | "PUBKEY" | "FINGERPRINTS" => Ok(()),
        _ => Err(ApiError::BadRequest("invalid signature_type".into())),
    }
}

/// `GET /api/pkg/repos` — return repos grouped by file.
pub async fn list_repos() -> ApiResult<Json<Vec<RepoFile>>> {
    let files = read_all_repo_files();
    Ok(Json(files))
}

/// `POST /api/pkg/repos` — add a repo to a file in the user dir.
pub async fn create_repo(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateRepoInput>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_filename(&req.filename)?;
    validate_repo_name(&req.repo.name)?;
    validate_url(&req.repo.url)?;
    let mt = req
        .repo
        .mirror_type
        .as_deref()
        .unwrap_or("NONE")
        .to_uppercase();
    validate_mirror_type(&mt)?;
    let st = req
        .repo
        .signature_type
        .as_deref()
        .unwrap_or("NONE")
        .to_uppercase();
    validate_signature_type(&st)?;

    fs::create_dir_all(USER_REPO_DIR)?;
    let path = format!("{}/{}", USER_REPO_DIR, req.filename);

    // Read existing repos from the target file (may not exist yet).
    let mut repos = parse_repo_file(&path);

    // Check for duplicate name within the file.
    if repos.iter().any(|r| r.name == req.repo.name) {
        return Err(ApiError::Conflict(format!(
            "repository '{}' already exists in {}",
            req.repo.name, req.filename
        )));
    }

    repos.push(input_to_config(&req.repo));
    write_override_file(&path, &repos)?;

    audit::record(
        &state,
        Some(&user.username),
        "POST",
        "/api/pkg/repos",
        200,
        Some(format!(
            "Added pkg repo '{}' to {}",
            req.repo.name, req.filename
        )),
    );

    Ok(Json(serde_json::json!({ "name": req.repo.name })))
}

/// `PUT /api/pkg/repos/{name}` — update a repo within its file.
/// Body includes `file` (source_file path) to locate the repo.
pub async fn update_repo(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
    Json(req): Json<UpdateRepoRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_repo_name(&name)?;
    validate_url(&req.url)?;
    let mt = req.mirror_type.as_deref().unwrap_or("NONE").to_uppercase();
    validate_mirror_type(&mt)?;
    let st = req
        .signature_type
        .as_deref()
        .unwrap_or("NONE")
        .to_uppercase();
    validate_signature_type(&st)?;

    let target_path = resolve_target_file(&req.file, &name)?;

    // Read existing repos from target file.
    let mut repos = parse_repo_file(&target_path);

    // Find and replace the repo, or append if not found.
    let new_config = RepoConfig {
        name: name.clone(),
        url: req.url,
        enabled: req.enabled,
        mirror_type: mt,
        signature_type: st,
        fingerprints: req.fingerprints,
        pubkey: req.pubkey,
        priority: req.priority.unwrap_or(0),
        ip_version: req.ip_version.unwrap_or(0),
        is_system_origin: false,
    };

    if let Some(existing) = repos.iter_mut().find(|r| r.name == name) {
        *existing = new_config.clone();
    } else {
        repos.push(new_config);
    }

    write_override_file(&target_path, &repos)?;

    audit::record(
        &state,
        Some(&user.username),
        "PUT",
        "/api/pkg/repos",
        200,
        Some(format!("Updated pkg repo '{}'", name)),
    );

    Ok(Json(serde_json::json!({ "name": name })))
}

/// `DELETE /api/pkg/repos/{name}?file=<path>` — remove a repo from a file.
pub async fn delete_repo(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
    Query(params): Query<DeleteRepoParams>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_repo_name(&name)?;
    let file = params.file.as_deref().ok_or_else(|| {
        ApiError::BadRequest("file parameter is required".into())
    })?;

    let target_path = resolve_target_file(file, &name)?;

    // Only allow deletion from user dir.
    if !target_path.starts_with(USER_REPO_DIR) {
        return Err(ApiError::BadRequest(
            "cannot delete from system files; disable the repo instead".into(),
        ));
    }

    let mut repos = parse_repo_file(&target_path);
    let before = repos.len();
    repos.retain(|r| r.name != name);
    if repos.len() == before {
        return Err(ApiError::NotFound(format!(
            "repository '{}' not found in {}",
            name,
            target_path
        )));
    }

    // write_override_file handles both writing and auto-deleting empty files.
    write_override_file(&target_path, &repos)?;

    audit::record(
        &state,
        Some(&user.username),
        "DELETE",
        "/api/pkg/repos",
        200,
        Some(format!("Deleted pkg repo '{}'", name)),
    );

    Ok(Json(serde_json::json!({ "name": name })))
}

// ---- Request types for update/delete ----

#[derive(Debug, Deserialize)]
pub struct UpdateRepoRequest {
    pub file: String,
    pub url: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mirror_type: Option<String>,
    #[serde(default)]
    pub signature_type: Option<String>,
    #[serde(default)]
    pub fingerprints: Option<String>,
    #[serde(default)]
    pub pubkey: Option<String>,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub ip_version: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteRepoParams {
    pub file: Option<String>,
}

/// Determine the actual file path to operate on.
/// - System file paths redirect to a same-basename override file in user dir.
/// - User file paths are used directly.
fn resolve_target_file(file: &str, repo_name: &str) -> ApiResult<String> {
    let _ = repo_name;

    // System file → override in user dir with same basename.
    if file.starts_with(SYSTEM_REPO_DIR) {
        let basename = std::path::Path::new(file)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .ok_or_else(|| ApiError::BadRequest("invalid file path".into()))?;
        validate_filename(&basename)?;
        fs::create_dir_all(USER_REPO_DIR)?;
        Ok(format!("{}/{}", USER_REPO_DIR, basename))
    } else if file.starts_with(USER_REPO_DIR) {
        Ok(file.to_string())
    } else {
        Err(ApiError::BadRequest("file must be within pkg config directories".into()))
    }
}

/// `POST /api/pkg/repos/update` — run `pkg update -f` in background.
pub async fn repo_update(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<serde_json::Value>> {
    gc_tasks();

    let id = gen_id();
    let task = PkgTask {
        id: id.clone(),
        action: "update".to_string(),
        packages: vec![],
        status: TaskStatus::Running,
        exit_code: None,
        lines: vec![],
        created_at: now_ts(),
    };
    TASKS.lock().insert(id.clone(), task);

    let tid = id.clone();
    let username = user.username.clone();
    tokio::spawn(async move {
        run_update_background(&tid).await;
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
            "/api/pkg/repos/update",
            if ok { 200 } else { 500 },
            Some(format!("pkg update -f: {}", if ok { "ok" } else { "failed" })),
        );
    });

    Ok(Json(serde_json::json!({ "task_id": id })))
}

/// Run `pkg update -f` in the background.
async fn run_update_background(task_id: &str) {
    let mut cmd = tokio::process::Command::new(PKG);
    cmd.arg("update").arg("-f");
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
