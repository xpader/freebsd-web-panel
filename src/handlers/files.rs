//! File manager — browse, upload, download, rename, delete, file properties.
//!
//! Runs as root and can access the whole filesystem (intended for a system
//! admin panel). All paths must be absolute and are lexically normalized:
//! `.` / `..` are resolved and cannot escape the filesystem root. Paths are
//! passed as query parameters (file paths contain `/`, so path params won't
//! work — same approach as the ZFS handlers).

use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct RenameQuery {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    /// Destination directory (must already exist).
    pub path: String,
    pub filename: String,
}

/// Lexically normalize an absolute path: resolve `.` and `..`, reject
/// non-absolute input and embedded NUL/newline bytes. `..` is clamped at the
/// filesystem root (cannot escape `/`).
fn normalize(raw: &str) -> ApiResult<PathBuf> {
    if raw.is_empty() || !raw.starts_with('/') {
        return Err(ApiError::BadRequest("path must be absolute".into()));
    }
    if raw.contains('\0') || raw.contains('\n') {
        return Err(ApiError::BadRequest("path contains invalid characters".into()));
    }
    let mut parts: Vec<OsString> = Vec::new();
    for comp in Path::new(raw).components() {
        use std::path::Component;
        match comp {
            Component::RootDir | Component::Prefix(_) => {}
            Component::Normal(c) => parts.push(c.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
        }
    }
    let mut out = PathBuf::from("/");
    for p in parts {
        out.push(p);
    }
    Ok(out)
}

/// Validate a single filename component (new file/folder/rename basename).
fn validate_name_component(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 255 {
        return Err(ApiError::BadRequest("invalid name length".into()));
    }
    if name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(ApiError::BadRequest("invalid name".into()));
    }
    Ok(())
}

/// Build a 10-char `ls`-style permission string including the leading type
/// char and setuid/setgid/sticky bits.
fn perm_string(type_ch: char, mode: u32) -> String {
    let mut s = String::with_capacity(10);
    s.push(type_ch);
    for shift in [6usize, 3, 0] {
        let m = (mode >> shift) & 0o7;
        s.push(if m & 4 != 0 { 'r' } else { '-' });
        s.push(if m & 2 != 0 { 'w' } else { '-' });
        let exec = m & 1 != 0;
        match shift {
            6 => match (mode & 0o4000 != 0, exec) {
                (true, true) => s.push('s'),
                (true, false) => s.push('S'),
                _ => s.push(if exec { 'x' } else { '-' }),
            },
            3 => match (mode & 0o2000 != 0, exec) {
                (true, true) => s.push('s'),
                (true, false) => s.push('S'),
                _ => s.push(if exec { 'x' } else { '-' }),
            },
            _ => match (mode & 0o1000 != 0, exec) {
                (true, true) => s.push('t'),
                (true, false) => s.push('T'),
                _ => s.push(if exec { 'x' } else { '-' }),
            },
        }
    }
    s
}

/// Leading type char from file type + mode.
fn type_char(ft: &std::fs::FileType, mode: u32) -> char {
    if ft.is_dir() {
        'd'
    } else if ft.is_symlink() {
        'l'
    } else {
        match mode & 0o170000 {
            0o020000 => 'c', // character device
            0o060000 => 'b', // block device
            0o010000 => 'p', // fifo
            0o140000 => 's', // socket
            _ => '-',
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: i64,
    pub mode: u32,
    pub permissions: String,
    pub uid: u32,
    pub gid: u32,
}

fn build_entry(full: &Path) -> Option<DirEntry> {
    let name = full.file_name()?.to_string_lossy().to_string();
    let meta = std::fs::symlink_metadata(full).ok()?;
    let ft = meta.file_type();
    let mode = meta.mode();
    Some(DirEntry {
        name,
        path: full.to_string_lossy().to_string(),
        is_dir: ft.is_dir(),
        is_file: ft.is_file(),
        is_symlink: ft.is_symlink(),
        size: meta.len(),
        modified: meta.mtime(),
        mode,
        permissions: perm_string(type_char(&ft, mode), mode),
        uid: meta.uid(),
        gid: meta.gid(),
    })
}

/// List the contents of a directory. Directories are sorted before files;
/// within each group entries are sorted case-insensitively by name.
pub async fn list(Query(q): Query<PathQuery>) -> ApiResult<Json<Vec<DirEntry>>> {
    let dir = normalize(&q.path)?;
    let meta = std::fs::symlink_metadata(&dir)
        .map_err(|_| ApiError::NotFound(format!("path not found: {}", dir.display())))?;
    if !meta.is_dir() {
        return Err(ApiError::BadRequest(format!("not a directory: {}", dir.display())));
    }

    let mut entries: Vec<DirEntry> = Vec::new();
    for item in std::fs::read_dir(&dir)? {
        let Ok(item) = item else {
            continue; // skip unreadable entries (e.g. permission denied)
        };
        if let Some(e) = build_entry(&item.path()) {
            entries.push(e);
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Json(entries))
}

#[derive(Debug, Serialize)]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub parent: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub size: u64,
    pub modified: i64,
    pub accessed: i64,
    pub changed: i64,
    pub mode: u32,
    pub permissions: String,
    pub uid: u32,
    pub gid: u32,
    pub nlink: u64,
    pub inode: u64,
    pub blocks: u64,
    pub blksize: u64,
}

/// Detailed file/directory properties for the properties dialog.
pub async fn stat(Query(q): Query<PathQuery>) -> ApiResult<Json<FileInfo>> {
    let path = normalize(&q.path)?;
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|_| ApiError::NotFound(format!("path not found: {}", path.display())))?;
    let ft = meta.file_type();
    let mode = meta.mode();
    let symlink_target = if ft.is_symlink() {
        std::fs::read_link(&path).ok().map(|p| p.to_string_lossy().to_string())
    } else {
        None
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let parent = path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());

    Ok(Json(FileInfo {
        path: path.to_string_lossy().to_string(),
        name,
        parent,
        is_dir: ft.is_dir(),
        is_file: ft.is_file(),
        is_symlink: ft.is_symlink(),
        symlink_target,
        size: meta.len(),
        modified: meta.mtime(),
        accessed: meta.atime(),
        changed: meta.ctime(),
        mode,
        permissions: perm_string(type_char(&ft, mode), mode),
        uid: meta.uid(),
        gid: meta.gid(),
        nlink: meta.nlink(),
        inode: meta.ino(),
        blocks: meta.blocks(),
        blksize: meta.blksize(),
    }))
}

pub async fn mkdir(State(state): State<AppState>, Query(q): Query<PathQuery>) -> ApiResult<StatusCode> {
    let path = normalize(&q.path)?;
    if path.exists() {
        return Err(ApiError::Conflict(format!("already exists: {}", path.display())));
    }
    std::fs::create_dir(&path)?;
    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/files/mkdir",
        201,
        Some(format!("mkdir {}", path.display())),
    );
    Ok(StatusCode::CREATED)
}

pub async fn rename(
    State(state): State<AppState>,
    Query(q): Query<RenameQuery>,
) -> ApiResult<StatusCode> {
    let from = normalize(&q.from)?;
    let to = normalize(&q.to)?;
    if !from.exists() {
        return Err(ApiError::NotFound(format!("not found: {}", from.display())));
    }
    if to.exists() {
        return Err(ApiError::Conflict(format!("target exists: {}", to.display())));
    }
    std::fs::rename(&from, &to)?;
    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/files/rename",
        200,
        Some(format!("rename {} -> {}", from.display(), to.display())),
    );
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(State(state): State<AppState>, Query(q): Query<PathQuery>) -> ApiResult<StatusCode> {
    let path = normalize(&q.path)?;
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|_| ApiError::NotFound(format!("not found: {}", path.display())))?;
    if meta.is_dir() {
        std::fs::remove_dir_all(&path)?;
    } else {
        std::fs::remove_file(&path)?;
    }
    crate::audit::record(
        &state,
        None,
        "DELETE",
        "/api/files",
        200,
        Some(format!("delete {}", path.display())),
    );
    Ok(StatusCode::NO_CONTENT)
}

/// Upload a file: the request body is the raw file bytes, destination dir and
/// filename come via query parameters.
pub async fn upload(
    State(state): State<AppState>,
    Query(q): Query<UploadQuery>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    let dir = normalize(&q.path)?;
    validate_name_component(&q.filename)?;
    let dest_dir_meta = std::fs::metadata(&dir)
        .map_err(|_| ApiError::NotFound(format!("directory not found: {}", dir.display())))?;
    if !dest_dir_meta.is_dir() {
        return Err(ApiError::BadRequest(format!("not a directory: {}", dir.display())));
    }
    let dest = dir.join(&q.filename);
    std::fs::write(&dest, &body)?;
    crate::audit::record(
        &state,
        None,
        "POST",
        "/api/files/upload",
        201,
        Some(format!("upload {} ({} bytes)", dest.display(), body.len())),
    );
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({"path": dest.to_string_lossy(), "size": body.len()})),
    ))
}

/// Download a single file as an attachment. Directories are rejected.
pub async fn download(Query(q): Query<PathQuery>) -> ApiResult<Response> {
    let path = normalize(&q.path)?;
    let meta = std::fs::symlink_metadata(&path)
        .map_err(|_| ApiError::NotFound(format!("not found: {}", path.display())))?;
    if meta.is_dir() {
        return Err(ApiError::BadRequest("cannot download a directory".into()));
    }
    let bytes = std::fs::read(&path)?;
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let mut resp = Response::new(axum::body::Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}
