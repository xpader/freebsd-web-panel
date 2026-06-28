//! SQLite database access with a connection pool and schema bootstrap.

use std::path::Path;
use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use tokio::sync::Mutex;

use crate::error::{ApiError, ApiResult};

pub type Db = Arc<Mutex<Connection>>;

/// A user record.
#[derive(Debug, Clone, serde::Serialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub role: String,
    pub created_at: i64,
    pub last_login: Option<i64>,
}

/// A live session bound to a user.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Session {
    pub id: i64,
    pub user_id: i64,
    pub token_hash: String,
    pub created_at: i64,
    pub expires_at: i64,
}

pub fn open(path: &Path) -> ApiResult<Db> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let conn = Connection::open(path)
        .map_err(|e| ApiError::Internal(format!("open db: {e}")))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    migrate(&conn)?;
    Ok(Arc::new(Mutex::new(conn)))
}

fn migrate(conn: &Connection) -> ApiResult<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            username      TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            role          TEXT NOT NULL DEFAULT 'admin',
            created_at    INTEGER NOT NULL,
            last_login    INTEGER
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            token_hash  TEXT NOT NULL UNIQUE,
            created_at  INTEGER NOT NULL,
            expires_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
        CREATE INDEX IF NOT EXISTS idx_sessions_expires ON sessions(expires_at);

        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

pub fn user_count(conn: &Connection) -> ApiResult<i64> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
        .map_err(ApiError::Database)?;
    Ok(n)
}

pub fn create_user(
    conn: &Connection,
    username: &str,
    password_hash: &str,
    role: &str,
    now: i64,
) -> ApiResult<i64> {
    conn.execute(
        "INSERT INTO users (username, password_hash, role, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![username, password_hash, role, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_user_by_username(
    conn: &Connection,
    username: &str,
) -> ApiResult<Option<(User, String)>> {
    let row = conn
        .query_row(
            "SELECT id, username, password_hash, role, created_at, last_login \
             FROM users WHERE username = ?1",
            params![username],
            |r| {
                let pw: String = r.get(2)?;
                Ok((
                    User {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        role: r.get(3)?,
                        created_at: r.get(4)?,
                        last_login: r.get(5)?,
                    },
                    pw,
                ))
            },
        )
        .optional()?;
    Ok(row)
}

pub fn list_users(conn: &Connection) -> ApiResult<Vec<User>> {
    let mut stmt =
        conn.prepare("SELECT id, username, role, created_at, last_login FROM users ORDER BY id")?;
    let users = stmt
        .query_map([], |r| {
            Ok(User {
                id: r.get(0)?,
                username: r.get(1)?,
                role: r.get(2)?,
                created_at: r.get(3)?,
                last_login: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(users)
}

pub fn get_user(conn: &Connection, id: i64) -> ApiResult<Option<User>> {
    let u = conn
        .query_row(
            "SELECT id, username, role, created_at, last_login FROM users WHERE id = ?1",
            params![id],
            |r| {
                Ok(User {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    role: r.get(2)?,
                    created_at: r.get(3)?,
                    last_login: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(u)
}

pub fn update_user_password(conn: &Connection, id: i64, password_hash: &str) -> ApiResult<()> {
    conn.execute(
        "UPDATE users SET password_hash = ?1 WHERE id = ?2",
        params![password_hash, id],
    )?;
    Ok(())
}

pub fn delete_user(conn: &Connection, id: i64) -> ApiResult<()> {
    conn.execute("DELETE FROM users WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn touch_last_login(conn: &Connection, id: i64, now: i64) -> ApiResult<()> {
    conn.execute(
        "UPDATE users SET last_login = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    Ok(())
}

pub fn create_session(
    conn: &Connection,
    user_id: i64,
    token_hash: &str,
    now: i64,
    expires_at: i64,
) -> ApiResult<i64> {
    conn.execute(
        "INSERT INTO sessions (user_id, token_hash, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4)",
        params![user_id, token_hash, now, expires_at],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_session_by_hash(
    conn: &Connection,
    token_hash: &str,
    now: i64,
) -> ApiResult<Option<Session>> {
    let s = conn
        .query_row(
            "SELECT id, user_id, token_hash, created_at, expires_at \
             FROM sessions WHERE token_hash = ?1 AND expires_at > ?2",
            params![token_hash, now],
            |r| {
                Ok(Session {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    token_hash: r.get(2)?,
                    created_at: r.get(3)?,
                    expires_at: r.get(4)?,
                })
            },
        )
        .optional()?;
    Ok(s)
}

pub fn delete_session(conn: &Connection, token_hash: &str) -> ApiResult<()> {
    conn.execute("DELETE FROM sessions WHERE token_hash = ?1", params![token_hash])?;
    Ok(())
}

pub fn purge_expired_sessions(conn: &Connection, now: i64) -> ApiResult<()> {
    conn.execute("DELETE FROM sessions WHERE expires_at <= ?1", params![now])?;
    Ok(())
}
