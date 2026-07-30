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
    // ── Base schema ───────────────────────────────────────────────────
    // Core tables that every database must have.  Always idempotent.
    // Feature-specific tables are created by versioned migrations.
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

        CREATE TABLE IF NOT EXISTS metric_samples (
            ts       INTEGER NOT NULL,
            category TEXT NOT NULL,
            name     TEXT NOT NULL,
            value    REAL NOT NULL,
            PRIMARY KEY (ts, category, name)
        );
        CREATE INDEX IF NOT EXISTS idx_samples_query
            ON metric_samples(category, name, ts);

        CREATE TABLE IF NOT EXISTS schema_version (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );
        "#,
    )?;

    // ── Versioned migrations ─────────────────────────────────────────
    //
    // Each migration is a standalone function.  To add a new one:
    //   1. Write a `fn mN(conn) -> ApiResult<()>` in the migrations module.
    //   2. Append it to MIGRATIONS with the next sequential version.
    // Rules:
    //   - Versions are sequential: 1, 2, 3, …  Never reuse or skip.
    //   - Never edit a published migration — add a new one instead.
    //   - Migrations run inside a transaction and are recorded in
    //     schema_version, so each runs at most once.

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let current: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    for m in migrations::MIGRATIONS {
        if current >= m.version {
            continue;
        }
        tracing::info!("db migration {}: {}", m.version, m.desc);
        let tx = conn.unchecked_transaction()?;
        (m.func)(&tx)?;
        tx.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
            params![m.version, now],
        )?;
        tx.commit()?;
    }

    Ok(())
}

// ── migrations ─────────────────────────────────────────────────────

mod migrations {
    use crate::error::ApiResult;
    use rusqlite::Connection;

    pub struct Migration {
        pub version: i64,
        pub desc: &'static str,
        pub func: fn(&Connection) -> ApiResult<()>,
    }

    pub const MIGRATIONS: &[Migration] = &[
        Migration {
            version: 1,
            desc: "firewall: create firewall tables in current form",
            func: m1,
        },
        Migration {
            version: 2,
            desc: "firewall: create firewall_nat_rules table",
            func: m2,
        },
        Migration {
            version: 3,
            desc: "rsync: create rsync_tasks table",
            func: m3,
        },
    ];

    /// v1: Create firewall tables (rules, state, tables, table entries).
    fn m1(conn: &Connection) -> ApiResult<()> {
        // ── firewall_state ──
        conn.execute(
            "CREATE TABLE IF NOT EXISTS firewall_state (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            [],
        )?;

        // ── firewall_tables ──
        conn.execute(
            "CREATE TABLE IF NOT EXISTS firewall_tables (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL UNIQUE,
                description TEXT,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            )",
            [],
        )?;

        // ── firewall_table_entries ──
        conn.execute(
            "CREATE TABLE IF NOT EXISTS firewall_table_entries (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                table_id   INTEGER NOT NULL,
                address    TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (table_id) REFERENCES firewall_tables(id) ON DELETE CASCADE
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_fw_table_entries
                ON firewall_table_entries(table_id)",
            [],
        )?;

        // ── firewall_rules ──
        conn.execute(
            "CREATE TABLE IF NOT EXISTS firewall_rules (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                position    INTEGER NOT NULL DEFAULT 0,
                enabled     INTEGER NOT NULL DEFAULT 1,
                action      TEXT    NOT NULL,
                direction   TEXT    NOT NULL,
                protocol    TEXT    NOT NULL,
                src_kind    TEXT    NOT NULL,
                src_value   TEXT    NOT NULL DEFAULT '',
                src_port    TEXT,
                dst_kind    TEXT    NOT NULL,
                dst_value   TEXT    NOT NULL DEFAULT '',
                dst_port    TEXT,
                interface   TEXT,
                log         INTEGER NOT NULL DEFAULT 0,
                icmp_type   TEXT,
                description TEXT,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_firewall_rules_position
                ON firewall_rules(position)",
            [],
        )?;

        Ok(())
    }

    /// v2: Create firewall_nat_rules table (NAT / port-forward rules).
    fn m2(conn: &Connection) -> ApiResult<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS firewall_nat_rules (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                position    INTEGER NOT NULL DEFAULT 0,
                enabled     INTEGER NOT NULL DEFAULT 1,
                kind        TEXT    NOT NULL,
                family      TEXT    NOT NULL,
                interface   TEXT    NOT NULL,
                src_addr    TEXT    NOT NULL,
                dst_addr    TEXT,
                src_port    TEXT,
                dst_port    TEXT,
                protocol    TEXT    NOT NULL,
                description TEXT,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_firewall_nat_position
                ON firewall_nat_rules(position)",
            [],
        )?;
        Ok(())
    }

    /// v3: Create rsync_tasks table (Rsync sync task definitions).
    fn m3(conn: &Connection) -> ApiResult<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS rsync_tasks (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                description TEXT NOT NULL DEFAULT '',
                source      TEXT NOT NULL,
                dest        TEXT NOT NULL,
                archive     INTEGER NOT NULL DEFAULT 1,
                compress    INTEGER NOT NULL DEFAULT 0,
                \"delete\"   INTEGER NOT NULL DEFAULT 0,
                verbose     INTEGER NOT NULL DEFAULT 1,
                port        INTEGER,
                extra_args  TEXT NOT NULL DEFAULT '',
                run_user    TEXT NOT NULL DEFAULT '',
                cron_enabled INTEGER NOT NULL DEFAULT 0,
                cron_expr   TEXT NOT NULL DEFAULT '',
                last_run_at INTEGER,
                last_status TEXT,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

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

/// Query instantaneous-value data aggregated into time buckets by the
/// specified SQL aggregate function (MIN, AVG, or MAX).  Used for
/// downsampling CPU/memory/net-rate series when the raw point count is
/// too high for smooth rendering.
pub fn query_series_grouped(
    conn: &Connection,
    category: &str,
    name: &str,
    from_ts: i64,
    to_ts: i64,
    bucket_sec: i64,
    agg: &str,
) -> ApiResult<Vec<(i64, f64)>> {
    let sql_func = match agg {
        "min" => "MIN",
        "max" => "MAX",
        _ => "AVG",
    };
    let sql = format!(
        "SELECT (ts / ?5) * ?5 AS bucket_ts, {sql_func}(value) \
         FROM metric_samples \
         WHERE category = ?1 AND name = ?2 AND ts >= ?3 AND ts <= ?4 \
         GROUP BY bucket_ts \
         ORDER BY bucket_ts ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
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
