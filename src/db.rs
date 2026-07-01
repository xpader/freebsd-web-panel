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

        CREATE TABLE IF NOT EXISTS metric_samples (
            ts       INTEGER NOT NULL,
            category TEXT NOT NULL,
            name     TEXT NOT NULL,
            value    REAL NOT NULL,
            PRIMARY KEY (ts, category, name)
        );
        CREATE INDEX IF NOT EXISTS idx_samples_query
            ON metric_samples(category, name, ts);
        "#,
    )?;

    // One-time purge: legacy net samples whose interface name contains '*'
    // (e.g. "bge0*.rx"). The '*' suffix in `netstat -i` output marks
    // interfaces without the UP flag; `read_net_counters()` now strips it
    // before writing, so these rows are stale leftovers from before that fix.
    let purged: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'migration_net_star_purged'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if purged.is_none() {
        conn.execute(
            "DELETE FROM metric_samples WHERE category = 'net' AND name LIKE '%*%'",
            [],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('migration_net_star_purged', '1')",
            [],
        )?;
    }

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

// ---- Metric samples (monitoring) ----

/// A single time-series data point.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricSample {
    pub ts: i64,
    pub category: String,
    pub name: String,
    pub value: f64,
}

/// Insert a batch of samples in a single transaction.
pub fn insert_samples(conn: &Connection, samples: &[MetricSample]) -> ApiResult<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO metric_samples (ts, category, name, value) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for s in samples {
            stmt.execute(params![s.ts, s.category, s.name, s.value])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Query a time series for a given category/name within [from_ts, to_ts].
pub fn query_series(
    conn: &Connection,
    category: &str,
    name: &str,
    from_ts: i64,
    to_ts: i64,
) -> ApiResult<Vec<MetricSample>> {
    let mut stmt = conn.prepare(
        "SELECT ts, category, name, value FROM metric_samples \
         WHERE category = ?1 AND name = ?2 AND ts >= ?3 AND ts <= ?4 \
         ORDER BY ts ASC",
    )?;
    let rows = stmt
        .query_map(params![category, name, from_ts, to_ts], |r| {
            Ok(MetricSample {
                ts: r.get(0)?,
                category: r.get(1)?,
                name: r.get(2)?,
                value: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Query a time series aggregated into fixed-size time buckets.
/// Query delta-value data aggregated into time buckets by SUM.  Each stored
/// sample is already the bytes transferred in one interval, so summing them
/// yields exact total bytes per bucket.
pub fn query_counter_aggregate(
    conn: &Connection,
    category: &str,
    name: &str,
    from_ts: i64,
    to_ts: i64,
    bucket_sec: i64,
) -> ApiResult<Vec<(i64, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT (ts / ?5) * ?5 AS bucket_ts, SUM(value) \
         FROM metric_samples \
         WHERE category = ?1 AND name = ?2 AND ts >= ?3 AND ts <= ?4 \
         GROUP BY bucket_ts \
         ORDER BY bucket_ts ASC",
    )?;
    let rows = stmt
        .query_map(params![category, name, from_ts, to_ts, bucket_sec], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Get the most recent sample for each (category, name) in a category.
pub fn latest_in_category(conn: &Connection, category: &str) -> ApiResult<Vec<MetricSample>> {
    let mut stmt = conn.prepare(
        "SELECT m.ts, m.category, m.name, m.value FROM metric_samples m \
         INNER JOIN ( \
             SELECT name, MAX(ts) AS max_ts FROM metric_samples \
             WHERE category = ?1 GROUP BY name \
         ) latest ON m.name = latest.name AND m.ts = latest.max_ts \
         WHERE m.category = ?1 \
         ORDER BY m.name",
    )?;
    let rows = stmt
        .query_map(params![category], |r| {
            Ok(MetricSample {
                ts: r.get(0)?,
                category: r.get(1)?,
                name: r.get(2)?,
                value: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Delete samples older than the given timestamp (data retention).
pub fn purge_old_samples(conn: &Connection, before_ts: i64) -> ApiResult<usize> {
    let n = conn.execute(
        "DELETE FROM metric_samples WHERE ts < ?1",
        params![before_ts],
    )?;
    Ok(n)
}
