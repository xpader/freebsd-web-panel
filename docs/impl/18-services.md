# 18 — rc.d 服务管理

## 概述

列出 FreeBSD rc.d 服务脚本（`/etc/rc.d/` 和 `/usr/local/etc/rc.d/`），显示每个服务的启用状态（rc.conf `*_enable=YES`）和运行状态（`service <name> status`），并支持 start / stop / restart 操作。

## 实现细节

### 后端（`src/handlers/services.rs`）

所有操作通过 `/usr/sbin/service` 完成，不拼接 shell 字符串，参数经校验后以 `Command::new().arg()` 传递。

**数据采集流程**（`list` handler）：

1. `service -l` — 获取所有 rc.d 脚本名称（纯名称，不含路径）。
2. 过滤伪服务（`DAEMON`、`FILESYSTEMS`、`LOGIN`、`NETWORKING`、`SERVERS`）——这些是依赖标记，不支持 start/stop/status。
3. 直接读取 `/etc/defaults/rc.conf` + `/etc/rc.conf` 解析所有变量（无子进程），合并为有效值 map。
4. 单次 `ps -ax` 获取进程表快照（`HashMap<PID, command>`）。
5. 对每个服务：
   - **描述 + 变量**：读取 rc.d 脚本，用正则提取 `desc=`、`rcvar=`、`pidfile=`、`procname=`、`command=`、`name=` 等 shell 变量。
   - **启用状态**：从脚本解析 `rcvar`（如 `rcvar="sshd_enable"`），展开 `${name}` 变量引用后，在 rc.conf map 中检查值是否为 `YES`。若脚本未设 rcvar，默认 `${name}_enable`。
   - **运行状态**（快速路径）：优先用 pidfile（读 PID → 查进程表），其次用 procname/command（匹配进程表命令列）。仅对已启用服务检查。
   - **运行状态**（回退）：pidfile/procname 无法解析的服务（如 one-shot 类型的 netif、zfs），回退到 `service <name> status`，通过 `std::thread::scope` 并行执行。

**服务控制**（`control` handler）：

- 路径参数 `{name}/{action}`，action 仅允许 `start` | `stop` | `restart`。
- 服务名校验：正则 `^[a-zA-Z0-9_.-]+$`，1–128 字符。
- 成功后记录审计日志。
- 返回 `ServiceActionResponse { name, action, output }`（output 为命令 stdout）。

### 前端（`web/js/pages/services.js`）

- 表格列：名称（mono）、描述、启用状态（badge）、运行状态（badge）、操作按钮。
- 状态徽章配色：
  - 已启用 → `badge-success`（绿）
  - 已禁用 → `badge-dim`（灰）
  - 运行中 → `badge-success`（绿）
  - 已停止（已启用）→ `badge-warn`（黄）
  - 已停止（已禁用）→ `badge-dim`（灰）
- 操作按钮根据运行状态禁用：运行中时禁用 Start，未运行时禁用 Stop。
- 筛选框：按名称或描述模糊匹配。
- 刷新按钮：重新加载服务列表。
- 操作完成后自动刷新列表。

### 路由

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/services` | 列出所有服务（含状态） |
| POST | `/api/services/{name}/{action}` | start/stop/restart |

前端路由：`/services`（配置 → 服务）。

## API

### GET /api/services

**响应**：`200` `ServiceInfo[]`

```json
[
  {
    "name": "sshd",
    "description": "Secure Shell Daemon",
    "enabled": true,
    "running": true
  }
]
```

### POST /api/services/{name}/{action}

**路径参数**：
- `name` — 服务名（如 `sshd`）
- `action` — `start` | `stop` | `restart`

**响应**：`200` `ServiceActionResponse`

```json
{
  "name": "sshd",
  "action": "restart",
  "output": "Performing sanity check on sshd configuration.\nStopping sshd.\nStarting sshd."
}
```

**错误**：
- `400` — 无效的服务名或 action
- `422` — service 命令执行失败（返回 stderr）

## 外部依赖

- `/usr/sbin/service` — FreeBSD 服务管理命令（列表 + 控制）
- `/bin/ps` — 进程表快照（运行状态快速检查）
- `/etc/defaults/rc.conf`、`/etc/rc.conf` — 直接读取启用状态（无子进程）
- rc.d 脚本目录：`/etc/rc.d/`、`/usr/local/etc/rc.d/`
- crate：`regex`、`serde`、`axum`

## 已知限制 / TODO

- `cleartmp` 和 `sendmail` 的 enabled 判断可能与服务 `service -e` 略有差异（rcvar 格式非标准），不影响主要使用。
- 约一半已启用服务为 one-shot 类型（netif、zfs、cleanvar 等），无 pidfile/procname，运行状态检查走 `service status` 回退（并行执行）。
- 不支持 `enable` / `disable` 操作（修改 rc.conf `*_enable` 变量）——可由 RC 配置页面完成。
