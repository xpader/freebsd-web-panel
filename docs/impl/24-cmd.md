# 24 — 命令执行封装（cmd 模块）

## 概述

`src/cmd.rs` 是所有系统命令执行的统一入口。它解决两个问题：

1. **并发安全**：同步的 `std::process::Command` 会阻塞 tokio async worker 线程。所有命令必须通过 `spawn_blocking` 在阻塞线程池上执行，否则长命令（如 `zfs scrub`、`fetch`）会卡死其他并发请求。
2. **消除重复**：各 handler 文件原先各自维护 `run()` 辅助函数，逻辑重复且行为不一致。统一后错误处理、stdin 重定向、输出提取全部一致。

## 函数一览

| 函数 | async/sync | 返回类型 | 非零退出 | stdin | 用途 |
|------|-----------|---------|---------|-------|------|
| `run` | async | `ApiResult<String>` | 报错 | null | handler 中单个命令，自带 spawn_blocking |
| `run_output` | async | `ApiResult<Output>` | 不报错 | null | 需自行解析退出码/输出（如 dry-run） |
| `run_sync` | sync | `ApiResult<String>` | 报错 | null | spawn_blocking 闭包内部用 |
| `run_sync_str` | sync | `Result<String, String>` | 报错 | null | 同上，调用方错误类型是 `String`（bhyve 模块） |
| `run_forget_sync` | sync | `()` | 忽略 | null | fire-and-forget（清理操作） |
| `status_sync` | sync | `bool` | 忽略 | null | 只关心成功/失败 |
| `output_ok` | sync | `ApiResult<String>` | 报错 | — | 检查已有的 `Output`（配合 `run_output`） |

所有函数统一设置 `stdin(Stdio::null())`，避免子进程等待 stdin。

## 两种调用模式

### 模式 A：单命令 — 直接用 async 版本

handler 中只调一次命令时，用 `cmd::run()` 最省事：

```rust
pub async fn pool_list() -> ApiResult<Json<...>> {
    let raw = cmd::run(ZPOOL, &["list", "-H", "-p"]).await?;
    // parse raw...
}
```

`cmd::run` 内部自带 `spawn_blocking`，调用方无需关心线程切换。

### 模式 B：多操作混合 — 手动包 spawn_blocking

handler 需要连续执行多个命令 + 文件 I/O + FFI 调用时（如 `jail_create`、`interface_rcconf_save`），手动包一个 `spawn_blocking`，内部全用 sync 函数：

```rust
pub async fn interface_apply(...) -> ApiResult<...> {
    let name = name.clone();
    let result = tokio::task::spawn_blocking(move || -> ApiResult<ApplyResult> {
        let cfg = parse_iface_rcconf(&name);          // 文件 I/O
        apply_ifconfig(&name, &cfg)                   // → run_ifconfig → cmd::run_sync_str
            .map_err(ApiError::Command)?;
        Ok(result)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;
    Ok(Json(result))
}
```

**为什么不用多个 `cmd::run().await`？** 因为一个 `spawn_blocking` 在同一线程上跑完全部操作，避免了多次线程池调度开销。

### 选择标准

| 场景 | 用什么 |
|------|--------|
| handler 内单条命令 | `cmd::run().await` |
| handler 内多条命令 + I/O 混合 | `spawn_blocking` + `cmd::run_sync()` |
| bhyve 模块内部（错误类型是 `String`） | `cmd::run_sync_str()` |
| 清理操作（失败也无所谓） | `cmd::run_forget_sync()` |
| 只需知道成功/失败 | `cmd::status_sync()` |
| 需自行解析退出码和输出 | `cmd::run_output().await` |

## 错误处理

- `ApiError` 版本（`run`、`run_sync`、`output_ok`）：非零退出 → `ApiError::Command(stderr)` → HTTP 422。stderr 为空时 fallback 到 `"{cmd} failed"`。
- `String` 版本（`run_sync_str`）：非零退出 → `Err(stderr)`，stderr 为空时 fallback 到 stdout，再为空时 `"{cmd} failed"`。错误格式略有不同（多一级 stdout fallback），因为 bhyve 的 `vm` 命令经常把错误写到 stdout。

## 不使用 cmd 模块的特殊场景

以下场景因技术限制不能走 `cmd::` 通用封装，仍保留直接使用 `Command`：

| 场景 | 文件 | 原因 |
|------|------|------|
| `vm start` / `vm install` | `bhyve.rs` | 用 `.status()` + `Stdio::null()` 避免 fork 出的长生命周期 bhyve 进程继承管道 FD 导致 `.output()` 永久阻塞 |
| `service start/stop` | `handlers/services.rs` `control()` | 同上——daemon 包装的服务会 fork 出长生命周期子进程，用临时文件重定向 stdout/stderr |
| `jail -c` / `jail -r` | `jail.rs` `run_jail_cmd()` | 同上——`exec.start` 脚本可能 spawn 长生命周期进程，stderr 重定向到临时文件 |
| `crontab -u name -` | `handlers/crontab.rs` `run_crontab_install()` | 需要 piped stdin 写入 crontab 内容，无法用通用封装 |

### 管道 FD 死锁问题

`Command::output()` 会等待 stdout/stderr 管道关闭。如果子进程 fork 出的长生命周期子进程（如 bhyve 虚拟机、daemon 服务）继承了这些管道 FD，`.output()` 会永久阻塞。解决方案是用 `.status()`（不捕获管道）或将输出重定向到文件。

## 涉及源码

| 文件 | 说明 |
|------|------|
| `src/cmd.rs` | 7 个封装函数 |
| `src/main.rs:3` | `mod cmd;` 声明 |
| 所有 `handlers/*.rs` | 通过 `cmd::run` 或 `cmd::run_sync` 调用 |
| `src/bhyve.rs` | 通过 `cmd::run_sync_str` 调用（`vm_run` 封装） |
