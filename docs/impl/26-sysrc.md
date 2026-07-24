# 26 — sysrc 统一封装

## 概述

`src/sysrc.rs` 是全项目对 rc.conf 读写的唯一入口。采用两层设计：

1. **读取**：直接解析 rc.conf 文件（`std::fs`，<1ms，无子进程）
2. **写入**：委托 `/usr/sbin/sysrc`（正确的引号处理、语法校验、文件选择）

## 设计动机

### 为什么读不用 sysrc？

`sysrc` 是一个 941 行的 `/bin/sh` 脚本，内部 source 了 bsdconfig 框架（1808 行 shell）。

`sysrc -e -a`（读取全部非默认变量）对每个变量都调用 `f_sysrc_get`，而 `f_sysrc_get` 每次都会 fork 新子 shell → clean_env → source defaults(800 行) → source rc.conf → eval 取值。最终导致 **~180 次 fork/exec**，总耗时 **~500ms**。

同理，`sysrc -n <key>`（读取单个 key）约 **~100ms**。

对比直接 `std::fs::read_to_string`：<1ms。差了两个数量级。

### 为什么写保留 sysrc？

1. **写入前语法校验** — sysrc 写入前执行 `/bin/sh -n` 检查，有语法错误时拒绝写入
2. **正确的 shell 引号处理** — sysrc 用 shell 赋值语义确保值被正确引用
3. **写入正确的文件** — sysrc 按 `rc_conf_files` 顺序找第一个可写文件
4. **写入频率低** — 都是用户手动操作，~100ms 可接受

## API

### 读取（直接文件解析，无子进程）

| 函数 | 签名 | 说明 |
|---|---|---|
| `get` | `(key: &str) -> Option<String>` | 读取单个 key，`None` 当未设置/空/`"NO"` |
| `get_list` | `(key: &str) -> Vec<String>` | 读取空格分隔的列表（如 `jail_list`、`vm_list`、`cloned_interfaces`） |
| `is_yes` | `(key: &str) -> bool` | 判断 key 是否为 `"YES"` |
| `read_rcconf_files` | `() -> HashMap<String, String>` | 读取全部非默认变量，匹配 `source_rc_confs()` 解析顺序 |

#### `read_rcconf_files` 解析顺序

1. 从 `/etc/defaults/rc.conf` 解析 `rc_conf_files`（默认：`/etc/rc.conf /etc/rc.conf.local`）
2. 依次读取；若某文件重定义了 `rc_conf_files`，第二趟扫描新列表
3. 最后应用 `/etc/rc.conf.d/*`（最低优先级）

### 写入（通过 sysrc 子进程）

| 函数 | 签名 | 说明 |
|---|---|---|
| `set` | `(key: &str, value: &str) -> Result<(), String>` | 设置单个 key=value |
| `set_multi` | `(items: &[(&str, &str)]) -> Result<(), String>` | 批量设置（一次 sysrc 调用，减少 fork 次数） |
| `set_forget` | `(key: &str, value: &str)` | 设置，忽略错误 |
| `delete` | `(key: &str)` | 删除 key |
| `ensure_yes` | `(key: &str)` | 确保 key=`"YES"`（已是 YES 时不 spawn sysrc） |
| `ensure_no` | `(key: &str)` | 确保 key=`"NO"`（已是 NO 或未设置时不 spawn sysrc） |

### 列表操作（读文件 + 幂等写入）

| 函数 | 签名 | 说明 |
|---|---|---|
| `list_add` | `(key: &str, item: &str) -> Result<(), String>` | 添加元素到空格列表（已存在则 no-op） |
| `list_remove` | `(key: &str, item: &str) -> Result<(), String>` | 从列表移除元素（列表为空时删除 key） |

用于 `cloned_interfaces`、`jail_list`、`vm_list` 等空格分隔列表的管理。

### 异步变体（用于 axum handler）

| 函数 | 签名 | 说明 |
|---|---|---|
| `get_async` | `(key: &str) -> ApiResult<String>` | 异步读取单个 key（文件读取 + spawn_blocking） |
| `set_async` | `(key: &str, value: &str) -> ApiResult<()>` | 异步写入 |
| `delete_async` | `(key: &str) -> ApiResult<()>` | 异步删除 |
| `list_all_async` | `() -> ApiResult<HashMap<String, String>>` | 异步读取全部（文件读取 + spawn_blocking） |

### 内部函数

| 函数 | 说明 |
|---|---|
| `get_raw` | 读取单个 key，不过滤 `"NO"`（用于 `get_async`） |
| `resolve_rc_conf_files` | 从 `/etc/defaults/rc.conf` 解析 `rc_conf_files` |
| `merge_rcconf_lines` | 将文件内容逐行解析合并到 HashMap |
| `parse_export_line` | 解析单行 `KEY="VALUE"` 或 `KEY=VALUE` |
| `unescape` | 反转义 shell 风格转义 |

## 关键设计决策

### 读路径选择

| 场景 | 方式 | 延迟 |
|---|---|---|
| 读取单个/多个 key | `read_rcconf_files().get(...)` | <1ms |
| 批量读取 | `read_rcconf_files()` | <1ms |
| 写入 | `sysrc KEY=VALUE` | ~100ms |
| 批量写入 | `set_multi`（一次 sysrc 多个参数） | ~100ms（不随数量增长） |
| 删除 | `sysrc -x KEY` | ~100ms |

### `"NO"` 过滤

`get()` 过滤 `"NO"`（rc.conf 语义：未设置的变量解析为 `"NO"`）。`get_async()` 不过滤（rcconf CRUD 需要看到用户显式设置的 `"NO"` 值）。

### `set_multi` 批量写入

sysrc 支持一次调用设置多个变量：`sysrc firewall_enable=YES firewall_quiet=YES`。`set_multi` 将 N 次子进程减少为 1 次。

使用示例（`firewall_gen.rs::init_ipfw`）：

```rust
sysrc::set_multi(&[
    ("firewall_enable", "YES"),
    ("firewall_type", IPFW_RULES_PATH),
    ("firewall_quiet", "YES"),
    ("firewall_logging", "YES"),
]).map_err(|e| ApiError::Command(e))?;
```

### `ensure_yes` / `ensure_no` 优化

读取走文件（<1ms），仅在值需要改变时才 spawn sysrc 写入。常见情况（已是目标值）零子进程开销。`ensure_no` 在变量未设置时也为 no-op（`get()` 返回 `None`，无需写入）。

### `list_add` / `list_remove` 幂等性

读取列表走文件（<1ms），仅在列表实际变化时才 spawn sysrc 写入。元素已存在/不存在时直接返回，无子进程开销。

## 调用方

| 调用方 | 使用的函数 | 用途 |
|---|---|---|
| `handlers/zfs.rs` | `ensure_yes("zfs_enable")` | pool create/import 后确保 ZFS 启动 |
| `handlers/jails.rs` | `list_add("jail_list", ...)`, `list_remove`, `ensure_yes("jail_enable")`, `get_list` | jail 自动启动管理 |
| `bhyve.rs` | `read_rcconf_files`, `set_multi`（`vm_enable`+`vm_dir`）, `list_add`/`list_remove("vm_list", ...)`, `get_list` | vm-bhyve 状态检测与自动启动 |
| `handlers/rcconf.rs` | `list_all_async`, `set_async`, `get_async`, `delete_async` | rc.conf CRUD |
| `handlers/network.rs` | `read_rcconf_files`, `list_add`/`list_remove("cloned_interfaces", ...)`, `set`, `delete`, `set_async`, `delete_async` | 接口配置 + 默认网关 + cloned_interfaces |
| `handlers/firewall.rs` | `set_multi`, `ensure_no` | 防火墙启用/禁用 |
| `firewall_gen.rs` | `set_multi`, `ensure_no`, `delete` | 防火墙初始化（ipfw/pf） |

## 性能对比

| 操作 | 旧方式（sysrc 子进程） | 新方式（文件读取） |
|---|---|---|
| 读取全部变量 | ~500ms（180 次 fork/exec） | <1ms |
| 读取单个 key | ~100ms | <1ms |
| 读取 2 个 key | ~200ms（串行） | <1ms（一次文件读取） |
| 写入 1 个 key | ~100ms | ~100ms（不变，走 sysrc） |
| 写入 4 个 key | ~400ms（串行） | ~100ms（`set_multi`） |
| `ensure_yes`/`ensure_no`（已是目标值） | ~100ms | <1ms |
| `list_add`（已存在） | ~100ms | <1ms |

## 文件清单

| 文件 | 说明 |
|---|---|
| `src/sysrc.rs` | 统一 sysrc 封装模块（文件读取 + sysrc 子进程写入） |
| `src/main.rs` | `mod sysrc;` 注册 |
