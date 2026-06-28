//! Static web asset serving.
//!
//! Assets are embedded into the binary at compile time via `rust-embed`, and
//! also optionally read from disk. At runtime each request is resolved by
//! first checking the on-disk `web_root` (if it exists) and falling back to
//! the embedded copy. This lets the binary run from any working directory.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/"]
struct WebAsset;
use crate::state::AppState;

/// Handler that resolves a static asset request.
pub async fn serve_asset(State(state): State<AppState>, req: Request<Body>) -> Response {
    let path = req.uri().path();
    let disk = state.web_root.as_ref().filter(|p| p.is_dir());

    // Try on-disk first (dev mode / overrides).
    if let Some(root) = disk {
        let rel = path.trim_start_matches('/');
        let candidate = if rel.is_empty() { "index.html" } else { rel };
        let full = root.join(candidate);
        if full.is_file() {
            if let Ok(bytes) = std::fs::read(&full) {
                let mime = mime_guess_for(candidate);
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, mime)],
                    bytes,
                )
                    .into_response();
            }
        }
    }

    // Embedded fallback.
    if let Some((bytes, mime)) = embedded(path) {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime)],
            bytes,
        )
            .into_response();
    }

    // SPA fallback: serve index.html for client-side hash routing.
    if !has_extension(path.trim_start_matches('/')) {
        if let Some((bytes, _)) = embedded("index.html") {
            return (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                bytes,
            )
                .into_response();
        }
    }

    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn embedded(path: &str) -> Option<(Vec<u8>, &'static str)> {
    let key = path.trim_start_matches('/');
    let key = if key.is_empty() { "index.html" } else { key };
    let file = WebAsset::get(key)?;
    let mime = mime_guess_for(key);
    Some((file.data.to_vec(), mime))
}

fn mime_guess_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        _ => "application/octet-stream",
    }
}

fn has_extension(name: &str) -> bool {
    name.contains('.') && !name.starts_with('.')
}
