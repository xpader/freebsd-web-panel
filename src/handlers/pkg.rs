//! pkg package management — list installed packages, view details (description,
//! dependencies, reverse dependencies, file list).
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

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::LazyLock;

use axum::extract::{Path as AxumPath, Query};
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};

const PKG: &str = "/usr/sbin/pkg";

static RE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_+.{}@-]+$").unwrap());

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

// ---- Query params ----

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub filter: Option<String>,
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

/// Parse `%rn\t%rv` TSV output into DepInfo list.
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

// ---- Handlers ----

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
