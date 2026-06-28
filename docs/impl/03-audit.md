# 03 — 审计日志

## 概述

追加式 JSON Lines 审计日志，记录所有写操作（POST/PUT/DELETE）。用 `parking_lot::Mutex<File>` 保护写入（同步锁，不跨 `.await`）。

## 实现细节

### AuditLog `src/audit.rs`

```rust
pub struct AuditLog {
    file: Mutex<std::fs::File>,    // parking_lot::Mutex
    path: Box<Path>,
}
```

- `open(path)` — 以 append 模式打开文件，自动创建父目录
- `write(entry)` — JSON 序列化后 `writeln!`，失败仅 tracing warn（不影响业务）
- `read_recent(limit)` — 全量读取、反序、截断前 N 条

### 记录时机

`handlers/audit.rs::record(state, user, method, path, status, detail)` 在每个写操作的 handler 中显式调用。不使用全局中间件拦截（只记录有意义的关键操作，避免噪音）。

### 日志格式

每行一个 JSON 对象：

```json
{"ts":1782474496,"user":"admin","method":"POST","path":"/api/auth/login","status":200,"detail":null}
{"ts":1782474500,"user":"admin","method":"DELETE","path":"/api/users/2","status":200,"detail":"user deleted"}
```

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/audit?limit=200` | 返回最近 N 条审计记录（最多 2000） |

## 配置项

| 字段 | 默认值 | 说明 |
|---|---|---|
| `paths.audit` | `/var/db/fwp/audit.log` | 日志文件路径 |

## 已知限制

- 无日志轮转（文件无限增长）
- `read_recent` 全量读取后截断，大文件时性能下降（实际场景单机面板可接受）
- 审计日志路径不可写时静默降级（`state.audit = None`），不影响服务运行
