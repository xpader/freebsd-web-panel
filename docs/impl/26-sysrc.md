# 26 — sysrc 统一封装

## 概述

`src/sysrc.rs` 是全项目对 rc.conf 读写的唯一入口。封装 `/usr/sbin/sysrc` 命令，提供同步和异步两套 API，以及 `sysrc -e -a` 输出解析器。消除了此前分散在 5 个文件中的重复 SYSRC 常量、`sysrc_get`/`sysrc_set` 助手函数和 `parse_export_line`/`unescape` 解析器。

## 设计动机

重构前，以下文件各自维护了重复的 sysrc 常量和助手函数：

| 文件 | 原有重复内容 |
|---|---|
| `handlers/zfs.rs` | `SYSRC` const, `ensure_zfs_enabled()` |
| `handlers/jails.rs` | `SYSRC` const, 内联 sysrc 调用 |
| `bhyve.rs` | `SYSRC` const, `sysrc_get()`, `sysrc_set()` |
| `handlers/rcconf.rs` | `SYSRC` const, `parse_export_line()`, `unescape()`, `use crate::cmd` |
| `handlers/network.rs` | `SYSRC` const, `parse_sysrc_export_line()`, `unescape_sysrc()`, `HashMap` import |

提取为统一模块后，所有文件只需 `use crate::sysrc` 或调用 `crate::sysrc::*`。

## API

### 同步变体（用于 `spawn_blocking` 上下文）

| 函数 | 签名 | 说明 |
|---|---|---|
| `get` | `(key: &str) -> Option<String>` | 读取单个 key。返回 `None` 当值为空或 sysrc 输出 `"NO"`（未设置的变量 sysrc 返回 `NO`） |
| `set` | `(key: &str, value: &str) -> Result<(), String>` | 设置 key=value，失败返回错误字符串 |
| `set_forget` | `(key: &str, value: &str)` | 设置 key=value，忽略错误（fire-and-forget） |
| `delete` | `(key: &str)` | 删除 key（fire-and-forget，`sysrc -x`） |
| `ensure_yes` | `(key: &str)` | 确保 key=`"YES"`，已是 YES 时为空操作 |
| `get_list` | `(key: &str) -> Vec<String>` | 读取空格分隔的列表值（如 `jail_list`、`vm_list`） |
| `list_all` | `() -> HashMap<String, String>` | 读取全部非默认 rc.conf 变量 |

### 异步变体（用于 axum handler 直接调用）

| 函数 | 签名 | 说明 |
|---|---|---|
| `get_async` | `(key: &str) -> ApiResult<String>` | 异步读取单个 key |
| `set_async` | `(key: &str, value: &str) -> ApiResult<()>` | 异步设置 key=value |
| `delete_async` | `(key: &str) -> ApiResult<()>` | 异步删除 key |
| `list_all_async` | `() -> ApiResult<HashMap<String, String>>` | 异步读取全部非默认变量 |

### 内部解析器

| 函数 | 说明 |
|---|---|
| `parse_export_lines(raw) -> HashMap` | 将 `sysrc -e -a` 输出解析为 HashMap |
| `parse_export_line(line) -> Option<(String, String)>` | 解析单行 `KEY="VALUE"` 或 `KEY=VALUE` |
| `unescape(s) -> String` | 反转义 sysrc 的 shell 风格转义（`\"`→`"`、`\\`→`\`） |

## 关键设计决策

### `get()` 的 NO 过滤

`sysrc -n <key>` 对未设置的变量返回 `"NO"`（而非空字符串或错误）。`get()` 函数过滤此情况：

```rust
pub fn get(key: &str) -> Option<String> {
    let s = cmd::run_sync(SYSRC, &["-n", key]).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() || s == "NO" {
        None
    } else {
        Some(s)
    }
}
```

注意：`get_async()` 不做此过滤（直接返回 sysrc 原始输出），因为异步调用者（如 rcconf handler）需要看到原始值。只有同步 `get()` 用于条件判断场景（如 `ensure_yes`），需要区分"未设置"和"值为 NO"。

### sync vs async 选择

- **同步**（`spawn_blocking` 内）：`get`、`set`、`set_forget`、`delete`、`ensure_yes`、`get_list`、`list_all`。用于 bhyve CLI 封装器（`bhyve.rs`）、jail handler 中的条件判断（`jail_create`/`jail_update`）、zfs handler（`pool_create`/`pool_import`）。
- **异步**（axum handler 直接调用）：`get_async`、`set_async`、`delete_async`、`list_all_async`。用于 rcconf handler 和 network handler。

### 底层命令执行

- 同步变体通过 `crate::cmd` 的 `run_sync`/`run_sync_str`/`run_forget_sync` 执行。
- 异步变体通过 `crate::cmd::run` 执行。
- 所有调用使用 `Command::new().arg()` 传参，禁止 shell 拼接。

## 调用方

| 调用方 | 使用的函数 | 用途 |
|---|---|---|
| `handlers/zfs.rs` | `ensure_yes("zfs_enable")` | pool create/import 后确保 ZFS 服务启动 |
| `handlers/jails.rs` | `get_list("jail_list")`, `set_forget("jail_list")`, `ensure_yes("jail_enable")` | jail 自动启动管理 |
| `bhyve.rs` | `get("vm_enable")`, `get("vm_dir")`, `set("vm_enable")`, `set("vm_dir")`, `get_list("vm_list")`, `set("vm_list")` | vm-bhyve 初始化检测与自动启动 |
| `handlers/rcconf.rs` | `list_all_async`, `set_async`, `get_async`, `delete_async` | rc.conf CRUD |
| `handlers/network.rs` | `get`, `set`, `delete`, `set_forget`, `get_list`, `list_all` | 接口配置 + 默认网关 |

## 文件清单

| 文件 | 说明 |
|---|---|
| `src/sysrc.rs` | 统一 sysrc 封装模块 |
| `src/main.rs` | `mod sysrc;` 注册（`mod sysinfo;` 之后） |
