# 00 — 项目骨架

## 概述

单二进制 HTTP 服务，clap 解析配置路径，加载 TOML 配置 → 打开 SQLite → 打开审计日志 → 构建路由 → 绑定监听。纯 HTTP（无 TLS）。

## 实现细节

### 入口 `src/main.rs`

```
CLI(clap) → Config::load_or_create() → db::open() → AuditLog::open()
  → AppState 构造 → build(state) → monitor::spawn_collector(state)
  → TcpListener::bind → axum::serve
```

- `--config <path>` 默认 `/usr/local/etc/fwp.toml`
- 配置文件不存在时自动写入默认值
- 用户数为 0 时打印警告（首启引导提示）
- 监控采集器在 `axum::serve` 之前 spawn，与 HTTP 服务并行运行

### 路由组装 `src/app.rs`

`build(state: AppState) -> Router` 组装三层：

1. **public**（无需认证）：`/api/users/bootstrap`（GET+POST）、`/api/auth/login`
2. **api**（需认证）：系统/用户/审计/模块占位/监控，挂 `require_auth` 中间件
3. **fallback**：`web_assets::serve_asset` 处理所有非 API 路径（静态资源）

路由用 axum 0.8 的 `{name}` 捕获语法（非 `:name`）。

### 共享状态 `src/state.rs`

```rust
pub struct AppState {
    pub db: Db,                           // Arc<tokio::sync::Mutex<Connection>>
    pub config: Arc<Config>,
    pub audit: Option<Arc<AuditLog>>,
    pub web_root: Option<PathBuf>,
}
```

抽出独立模块避免 `app.rs` ↔ `web_assets.rs` 循环依赖。`now_ts()` 返回 Unix 秒。

### 配置 `src/config.rs`

TOML 反序列化，三段 + monitor：

```toml
[server]   listen, web_root
[paths]    db, audit
[auth]     session_ttl
[monitor]  enabled, interval_sec, retention_days
```

所有字段带 `#[serde(default = "fn")]`，缺失时用默认值。`load_or_create()` 不存在则写默认配置。

### 错误处理 `src/error.rs`

```rust
pub type ApiResult<T> = Result<T, ApiError>;

pub enum ApiError {
    Unauthorized, NotAuthenticated, Forbidden,
    NotFound(String), BadRequest(String), Conflict(String),
    Database(rusqlite::Error), Hash(String),
    Io(std::io::Error), Command(String), Internal(String),
}
```
`IntoResponse` 实现将错误映射为 HTTP 状态码 + JSON `{error, message}`。内部错误（DB/Hash/Io/Internal）不泄露详情，仅记录 tracing 日志。`Command` 用于系统命令执行失败（非零退出码），映射到 `422 Unprocessable Entity`（kind=`command_failed`），**透传 stderr 原文**给用户以便排查（如 ZFS 报"dataset already exists"）。

## 外部依赖

- `clap` 4（CLI）、`toml` 0.8（配置）、`tracing` + `tracing-subscriber`（日志）

## 配置项

| 字段 | 默认值 | 说明 |
|---|---|---|
| `server.listen` | `127.0.0.1:8080` | 监听地址 |
| `server.web_root` | `/usr/local/share/fwp/web` | 磁盘资源目录（开发覆盖用） |
| `paths.db` | `/var/db/fwp/fwp.db` | SQLite 路径 |
| `paths.audit` | `/var/db/fwp/audit.log` | 审计日志路径 |
