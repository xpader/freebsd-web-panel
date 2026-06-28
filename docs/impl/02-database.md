# 02 — SQLite 数据库

## 概述

rusqlite（bundled SQLite，编译自带 libsqlite3，无需系统安装）。单个 `Arc<tokio::sync::Mutex<Connection>>` 实例，异步 mutex（guard 跨 `.await` 持有）。WAL 模式 + 外键约束。

## 实现细节

### 连接管理 `src/db.rs`

```rust
pub type Db = Arc<tokio::sync::Mutex<Connection>>;

pub fn open(path: &Path) -> ApiResult<Db>
```

- 自动创建父目录
- `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`
- `migrate()` 建表（`CREATE TABLE IF NOT EXISTS`，幂等）

### Mutex 选择

`tokio::sync::Mutex`：handler 中 `state.db.lock().await` 持有锁跨 `.await` 调用。同步代码（如 CPU delta 计算）用 `parking_lot::Mutex`。

### 数据访问层

所有操作为**自由函数**（非 `impl Connection`，因 Rust 不允许对外部类型 inherent impl），签名 `fn(conn: &Connection, ...) -> ApiResult<T>`。

### 表结构

**users** — 面板用户

| 列 | 类型 | 说明 |
|---|---|---|
| id | INTEGER PK AUTOINCREMENT | |
| username | TEXT UNIQUE | 2-32 位 |
| password_hash | TEXT | Argon2id PHC |
| role | TEXT | 固定 'admin' |
| created_at | INTEGER | Unix 秒 |
| last_login | INTEGER | 可空 |

**sessions** — 会话

| 列 | 类型 | 说明 |
|---|---|---|
| id | INTEGER PK | |
| user_id | INTEGER FK → users | ON DELETE CASCADE |
| token_hash | TEXT UNIQUE | SHA-256 hex |
| created_at | INTEGER | |
| expires_at | INTEGER | |

索引：`idx_sessions_user`、`idx_sessions_expires`

**metric_samples** — 监控时序数据

| 列 | 类型 | 说明 |
|---|---|---|
| ts | INTEGER | Unix 秒 |
| category | TEXT | cpu/memory/load/temp |
| name | TEXT | total/core0/usage/cpu0... |
| value | REAL | |
| PRIMARY KEY | (ts, category, name) | 幂等写入 |

索引：`idx_samples_query` ON (category, name, ts)

**meta** — 键值元数据（预留）

### 关键函数

| 函数 | 说明 |
|---|---|
| `user_count` | 用户总数 |
| `create_user` | 插入用户 |
| `get_user_by_username` | 登录时查找（返回含 password_hash） |
| `get_user` | 按 ID 查找 |
| `list_users` | 全部用户（不含 hash） |
| `create_session` / `get_session_by_hash` / `delete_session` | 会话 CRUD |
| `purge_expired_sessions` | 清理过期会话 |
| `insert_samples` | 批量插入时序数据（事务） |
| `query_series` | 按分类+名称+时间范围查询 |
| `latest_in_category` | 每个名称的最新值 |
| `purge_old_samples` | 清理过期样本（保留策略） |

## 外部依赖

- `rusqlite` 0.32（features: bundled — 编译时静态链接 SQLite）

## 已知限制

- 单连接（`Mutex<Connection>`），无连接池；单机面板负载低，足够
- 无数据库备份机制（需手动复制 `.db` 文件）
