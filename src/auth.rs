//! Password hashing, session-token minting/verification, and auth middleware.

use std::collections::HashMap;
use std::sync::Arc;

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

// ── Login brute-force protection ─────────────────────────────────────────

/// In-memory tracker for failed login attempts per username **and** per IP.
///
/// Two layers of protection:
/// 1. Per-username: after `max_login_attempts` failures for the same
///    username, that username is locked for `lockout_sec` seconds.
/// 2. Per-IP: after `max_ip_login_attempts` failures from the same IP
///    (across all usernames), the IP is banned for `ip_ban_sec` seconds.
///
/// Successful login clears both the username and the IP record.
/// The guard is cheap to clone (inner state behind `Arc<Mutex>`).
#[derive(Clone)]
pub struct LoginGuard {
    inner: Arc<parking_lot::Mutex<GuardState>>,
}

#[derive(Default)]
struct GuardState {
    by_user: HashMap<String, AttemptRecord>,
    by_ip: HashMap<String, AttemptRecord>,
}

#[derive(Clone, Copy)]
struct AttemptRecord {
    fail_count: u32,
    locked_until: i64,
}

impl LoginGuard {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(parking_lot::Mutex::new(GuardState::default())),
        }
    }

    /// Returns `Err(remaining_secs)` if the username is currently locked.
    pub fn check_user(&self, username: &str, now: i64) -> Result<(), i64> {
        let mut st = self.inner.lock();
        if let Some(rec) = st.by_user.get(username) {
            if rec.locked_until > now {
                return Err(rec.locked_until - now);
            }
            // Locked but expired — remove so fail_count resets to zero.
            if rec.locked_until > 0 {
                st.by_user.remove(username);
            }
        }
        Ok(())
    }

    /// Returns `Err(remaining_secs)` if the IP is currently banned.
    pub fn check_ip(&self, ip: &str, now: i64) -> Result<(), i64> {
        let mut st = self.inner.lock();
        if let Some(rec) = st.by_ip.get(ip) {
            if rec.locked_until > now {
                return Err(rec.locked_until - now);
            }
            if rec.locked_until > 0 {
                st.by_ip.remove(ip);
            }
        }
        Ok(())
    }

    /// Record a per-username failed attempt. If the count reaches
    /// `max_attempts`, the username is locked for `lockout_sec` seconds.
    pub fn record_user_failure(
        &self,
        username: &str,
        now: i64,
        max_attempts: u32,
        lockout_sec: u64,
    ) {
        let mut st = self.inner.lock();
        let rec = st.by_user.entry(username.to_string()).or_insert(AttemptRecord {
            fail_count: 0,
            locked_until: 0,
        });
        rec.fail_count += 1;
        if rec.fail_count >= max_attempts {
            rec.locked_until = now + lockout_sec as i64;
        }
    }

    /// Record a per-IP failed attempt. If the count reaches `max_attempts`,
    /// the IP is banned for `ban_sec` seconds.
    pub fn record_ip_failure(
        &self,
        ip: &str,
        now: i64,
        max_attempts: u32,
        ban_sec: u64,
    ) {
        let mut st = self.inner.lock();
        let rec = st.by_ip.entry(ip.to_string()).or_insert(AttemptRecord {
            fail_count: 0,
            locked_until: 0,
        });
        rec.fail_count += 1;
        if rec.fail_count >= max_attempts {
            rec.locked_until = now + ban_sec as i64;
        }
    }

    /// Clear both the username and IP records on successful login.
    pub fn record_success(&self, username: &str, ip: &str) {
        let mut st = self.inner.lock();
        st.by_user.remove(username);
        st.by_ip.remove(ip);
    }
}

/// Extract the client IP from request headers or connection info.
///
/// Checks `X-Forwarded-For` (leftmost entry) first, then `X-Real-IP`,
/// falling back to the raw connection peer address.
pub fn extract_client_ip(
    headers: &axum::http::HeaderMap,
    peer: std::net::SocketAddr,
) -> String {
    if let Some(hv) = headers.get("x-forwarded-for") {
        if let Ok(s) = hv.to_str() {
            if let Some(first) = s.split(',').next() {
                let ip = first.trim();
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }
    }
    if let Some(hv) = headers.get("x-real-ip") {
        if let Ok(s) = hv.to_str() {
            let ip = s.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }
    peer.ip().to_string()
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
/// Validate a raw session token (e.g. from a query param for WS/SSE endpoints
/// that cannot set Authorization headers). Returns the authenticated user on
/// success.
pub async fn validate_token(state: &AppState, token: &str) -> ApiResult<AuthUser> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    #[test]
    fn user_lock_after_max_failures() {
        let guard = LoginGuard::new();
        for _ in 0..4 {
            guard.record_user_failure("alice", 1000, 5, 300);
            assert!(guard.check_user("alice", 1000).is_ok());
        }
        // 5th failure triggers lock
        guard.record_user_failure("alice", 1000, 5, 300);
        assert!(guard.check_user("alice", 1000).is_err());
        // Still locked at t=1299
        assert!(guard.check_user("alice", 1299).is_err());
        // Unlocked at t=1300 (1000 + 300)
        assert!(guard.check_user("alice", 1300).is_ok());
    }

    #[test]
    fn user_lock_clears_on_success() {
        let guard = LoginGuard::new();
        for _ in 0..5 {
            guard.record_user_failure("bob", 1000, 5, 300);
        }
        assert!(guard.check_user("bob", 1000).is_err());
        guard.record_success("bob", "1.2.3.4");
        assert!(guard.check_user("bob", 1000).is_ok());
    }

    #[test]
    fn user_lock_independent_users() {
        let guard = LoginGuard::new();
        for _ in 0..5 {
            guard.record_user_failure("carol", 1000, 5, 300);
        }
        assert!(guard.check_user("carol", 1000).is_err());
        assert!(guard.check_user("dave", 1000).is_ok());
    }

    #[test]
    fn ip_ban_after_max_failures() {
        let guard = LoginGuard::new();
        for _ in 0..19 {
            guard.record_ip_failure("10.0.0.1", 2000, 20, 1800);
            assert!(guard.check_ip("10.0.0.1", 2000).is_ok());
        }
        // 20th failure triggers ban
        guard.record_ip_failure("10.0.0.1", 2000, 20, 1800);
        assert!(guard.check_ip("10.0.0.1", 2000).is_err());
        // Still banned at t=3799
        assert!(guard.check_ip("10.0.0.1", 3799).is_err());
        // Ban expires at t=3800 (2000 + 1800)
        assert!(guard.check_ip("10.0.0.1", 3800).is_ok());
    }

    #[test]
    fn ip_ban_independent_ips() {
        let guard = LoginGuard::new();
        for _ in 0..20 {
            guard.record_ip_failure("10.0.0.1", 2000, 20, 1800);
        }
        assert!(guard.check_ip("10.0.0.1", 2000).is_err());
        // A different IP is not affected
        assert!(guard.check_ip("10.0.0.2", 2000).is_ok());
    }

    #[test]
    fn ip_ban_clears_on_success() {
        let guard = LoginGuard::new();
        for _ in 0..20 {
            guard.record_ip_failure("10.0.0.1", 2000, 20, 1800);
        }
        assert!(guard.check_ip("10.0.0.1", 2000).is_err());
        // Successful login from this IP clears the record
        guard.record_success("someone", "10.0.0.1");
        assert!(guard.check_ip("10.0.0.1", 2000).is_ok());
    }

    #[test]
    fn ip_failures_accumulate_across_usernames() {
        let guard = LoginGuard::new();
        // 10 failures with username "alice", 10 with "bob" — same IP
        for _ in 0..10 {
            guard.record_ip_failure("10.0.0.1", 2000, 20, 1800);
        }
        assert!(guard.check_ip("10.0.0.1", 2000).is_ok());
        for _ in 0..10 {
            guard.record_ip_failure("10.0.0.1", 2000, 20, 1800);
        }
        // 20 total from same IP → banned
        assert!(guard.check_ip("10.0.0.1", 2000).is_err());
    }

    #[test]
    fn extract_ip_from_xff_header() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.5, 10.0.0.1".parse().unwrap(),
        );
        let peer: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        assert_eq!(extract_client_ip(&headers, peer), "203.0.113.5");
    }

    #[test]
    fn extract_ip_fallback_to_peer() {
        let headers = axum::http::HeaderMap::new();
        let peer: SocketAddr = "192.168.1.55:8080".parse().unwrap();
        assert_eq!(extract_client_ip(&headers, peer), "192.168.1.55");
    }
}
