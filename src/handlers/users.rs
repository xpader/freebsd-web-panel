//! Panel user management: bootstrap, list, create, update, delete.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::{hash_password, AuthUser};
use crate::audit;
use crate::db::{self, User};
use crate::error::{ApiError, ApiResult};
use crate::AppState;

/// GET /api/users/bootstrap — returns whether an initial admin still needs to
/// be created. The frontend uses this to decide between the login form and the
/// first-run setup wizard.
pub async fn bootstrap_status(State(state): State<AppState>) -> ApiResult<Json<BootstrapStatus>> {
    let count = {
        let conn = state.db.lock().await;
        db::user_count(&conn)?
    };
    Ok(Json(BootstrapStatus {
        needs_setup: count == 0,
        user_count: count,
    }))
}

#[derive(Debug, Serialize)]
pub struct BootstrapStatus {
    pub needs_setup: bool,
    pub user_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct BootstrapRequest {
    pub username: String,
    pub password: String,
}

/// POST /api/users/bootstrap — create the first admin. Allowed only when no
/// users exist yet. Unauthenticated by design (one-time).
pub async fn bootstrap(
    State(state): State<AppState>,
    Json(body): Json<BootstrapRequest>,
) -> ApiResult<(StatusCode, Json<User>)> {
    validate_username(&body.username)?;
    validate_password(&body.password)?;

    let now = state.now_ts();
    let phc = hash_password(&body.password)?;

    let user = {
        let conn = state.db.lock().await;
        if db::user_count(&conn)? > 0 {
            return Err(ApiError::Conflict("setup already completed".into()));
        }
        let id = match db::create_user(&conn, &body.username, &phc, "admin", now) {
            Ok(id) => id,
            Err(ApiError::Database(de)) if de.to_string().contains("UNIQUE") => {
                return Err(ApiError::Conflict("username already exists".into()));
            }
            Err(e) => return Err(e),
        };
        db::get_user(&conn, id)?.unwrap()
    };

    audit::record(
        &state,
        Some(&body.username),
        "POST",
        "/api/users/bootstrap",
        201,
        Some("initial admin created".into()),
    );

    Ok((StatusCode::CREATED, Json(user)))
}

pub async fn list_users(State(state): State<AppState>) -> ApiResult<Json<Vec<User>>> {
    let users = {
        let conn = state.db.lock().await;
        db::list_users(&conn)?
    };
    Ok(Json(users))
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: Option<String>,
}

pub async fn create_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateUserRequest>,
) -> ApiResult<(StatusCode, Json<User>)> {
    validate_username(&body.username)?;
    validate_password(&body.password)?;
    let role = body.role.as_deref().unwrap_or("admin");
    if role != "admin" {
        return Err(ApiError::BadRequest("only 'admin' role is supported".into()));
    }

    let now = state.now_ts();
    let phc = hash_password(&body.password)?;

    let user = {
        let conn = state.db.lock().await;
        let id = match db::create_user(&conn, &body.username, &phc, role, now) {
            Ok(id) => id,
            Err(ApiError::Database(de)) if de.to_string().contains("UNIQUE") => {
                return Err(ApiError::Conflict("username already exists".into()));
            }
            Err(e) => return Err(e),
        };
        db::get_user(&conn, id)?.unwrap()
    };

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/users",
        201,
        Some(format!("created user '{}'", body.username)),
    );

    Ok((StatusCode::CREATED, Json(user)))
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub password: Option<String>,
}

pub async fn update_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
    Json(body): Json<UpdateUserRequest>,
) -> ApiResult<StatusCode> {
    if let Some(pw) = body.password {
        validate_password(&pw)?;
        let phc = hash_password(&pw)?;
        let conn = state.db.lock().await;
        if db::get_user(&conn, id)?.is_none() {
            return Err(ApiError::NotFound("user".into()));
        }
        db::update_user_password(&conn, id, &phc)?;
    }

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        &format!("/api/users/{}", id),
        200,
        Some("password updated".into()),
    );

    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    if id == auth.user_id {
        return Err(ApiError::BadRequest("cannot delete yourself".into()));
    }
    let conn = state.db.lock().await;
    if db::get_user(&conn, id)?.is_none() {
        return Err(ApiError::NotFound("user".into()));
    }
    db::delete_user(&conn, id)?;

    audit::record(
        &state,
        Some(&auth.username),
        "DELETE",
        &format!("/api/users/{}", id),
        200,
        Some("user deleted".into()),
    );

    Ok(StatusCode::NO_CONTENT)
}

fn validate_username(name: &str) -> ApiResult<()> {
    let re = regex::Regex::new(r"^[a-zA-Z0-9_.-]{2,32}$").unwrap();
    if !re.is_match(name) {
        return Err(ApiError::BadRequest(
            "username must be 2-32 chars of [a-zA-Z0-9_.-]".into(),
        ));
    }
    Ok(())
}

fn validate_password(pw: &str) -> ApiResult<()> {
    if pw.len() < 6 {
        return Err(ApiError::BadRequest(
            "password must be at least 6 characters".into(),
        ));
    }
    Ok(())
}
