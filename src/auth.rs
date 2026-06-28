//! Password hashing, session-token minting/verification, and auth middleware.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use base64::Engine;
use sha2::{Digest, Sha256};

use crate::error::{ApiError, ApiResult};
use crate::AppState;

/// Hash a plaintext password using Argon2id.
pub fn hash_password(plain: &str) -> ApiResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| ApiError::Hash(e.to_string()))
}

/// Verify a plaintext password against a stored PHC string.
pub fn verify_password(plain: &str, phc: &str) -> ApiResult<()> {
    let parsed = PasswordHash::new(phc).map_err(|e| ApiError::Hash(e.to_string()))?;
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .map_err(|_| ApiError::Unauthorized)
}

/// Generate a cryptographically random session token and its SHA-256 hash
/// (only the hash is stored in the DB).
pub fn mint_token() -> (String, String) {
    let raw = uuid::Uuid::new_v4().to_string();
    let secret: [u8; 32] = rand::random();
    let token = format!(
        "{}.{}",
        hex::encode(&raw.as_bytes()[..]),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret)
    );
    let hash = hash_token(&token);
    (token, hash)
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Extract the session token from an Authorization: Bearer header.
pub fn extract_bearer(req: &Request) -> Option<&str> {
    let header = req.headers().get(axum::http::header::AUTHORIZATION)?;
    let value = header.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::trim)
}
/// User identity injected into request extensions by the auth middleware.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: i64,
    pub username: String,
    pub role: String,
}
impl axum::extract::FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .cloned()
            .ok_or(ApiError::NotAuthenticated)
    }
}
/// Require authentication on all routes layered under it.
pub async fn require_auth(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = extract_bearer(&req).ok_or(ApiError::NotAuthenticated)?;
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

    req.extensions_mut().insert(AuthUser {
        user_id: user.id,
        username: user.username.clone(),
        role: user.role.clone(),
    });

    Ok(next.run(req).await)
}
