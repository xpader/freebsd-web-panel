//! Authentication endpoints: login, logout, current-user.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::{extract_client_ip, hash_token, mint_token, verify_password, AuthUser};
use crate::audit;
use crate::error::{ApiError, ApiResult};
use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub expires_at: i64,
    pub user: UserInfo,
}

#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub role: String,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Json(body): Json<LoginRequest>,
) -> ApiResult<(StatusCode, Json<LoginResponse>)> {
    let now = state.now_ts();
    let ip = extract_client_ip(&headers, peer);

    // Layer 2 — IP ban check (strongest): reject any request from a banned IP.
    if let Err(remaining) = state.login_guard.check_ip(&ip, now) {
        audit::record(
            &state,
            Some(&body.username),
            "POST",
            "/api/auth/login",
            429,
            Some(format!("IP banned ({ip}), {remaining}s remaining")),
        );
        return Err(ApiError::IpBanned(format!(
            "this IP address has been banned; try again in {} seconds",
            remaining
        )));
    }

    // Layer 1 — username lockout check.
    if let Err(remaining) = state.login_guard.check_user(&body.username, now) {
        audit::record(
            &state,
            Some(&body.username),
            "POST",
            "/api/auth/login",
            429,
            Some("account locked".into()),
        );
        return Err(ApiError::AccountLocked(format!(
            "too many failed attempts; try again in {} seconds",
            remaining
        )));
    }

    // Lookup the user.
    let (user, phc) = {
        let conn = state.db.lock().await;
        match crate::db::get_user_by_username(&conn, &body.username)? {
            Some(v) => v,
            None => {
                state.login_guard.record_user_failure(
                    &body.username,
                    now,
                    state.config.auth.max_login_attempts,
                    state.config.auth.lockout_sec,
                );
                state.login_guard.record_ip_failure(
                    &ip,
                    now,
                    state.config.auth.max_ip_login_attempts,
                    state.config.auth.ip_ban_sec,
                );
                audit::record(
                    &state,
                    Some(&body.username),
                    "POST",
                    "/api/auth/login",
                    401,
                    Some(format!("unknown user (from {ip})")),
                );
                return Err(ApiError::Unauthorized);
            }
        }
    };

    // Constant-time-ish verify; on failure audit + 401.
    if let Err(e) = verify_password(&body.password, &phc) {
        state.login_guard.record_user_failure(
            &body.username,
            now,
            state.config.auth.max_login_attempts,
            state.config.auth.lockout_sec,
        );
        state.login_guard.record_ip_failure(
            &ip,
            now,
            state.config.auth.max_ip_login_attempts,
            state.config.auth.ip_ban_sec,
        );
        audit::record(
            &state,
            Some(&body.username),
            "POST",
            "/api/auth/login",
            401,
            Some(format!("bad password (from {ip})")),
        );
        return Err(e);
    }

    // Successful authentication — clear both username and IP failed-attempt history.
    state.login_guard.record_success(&body.username, &ip);

    let (token, hash) = mint_token();
    let expires_at = now + (state.config.auth.session_ttl as i64);

    {
        let conn = state.db.lock().await;
        crate::db::purge_expired_sessions(&conn, now)?;
        crate::db::create_session(&conn, user.id, &hash, now, expires_at)?;
        crate::db::touch_last_login(&conn, user.id, now)?;
    }

    audit::record(
        &state,
        Some(&body.username),
        "POST",
        "/api/auth/login",
        200,
        None,
    );

    Ok((
        StatusCode::OK,
        Json(LoginResponse {
            token,
            expires_at,
            user: UserInfo {
                id: user.id,
                username: user.username,
                role: user.role,
            },
        }),
    ))
}

pub async fn logout(
    State(state): State<AppState>,
    req: Request,
) -> ApiResult<StatusCode> {
    if let Some(token) = crate::auth::extract_bearer(&req) {
        let hash = hash_token(token);
        let conn = state.db.lock().await;
        crate::db::delete_session(&conn, &hash)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(user: AuthUser) -> ApiResult<Json<UserInfo>> {
    Ok(Json(UserInfo {
        id: user.user_id,
        username: user.username,
        role: user.role,
    }))
}
