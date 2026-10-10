# 进程守护（Process Supervisor）

## 1. 概述

手动定义受管进程（名称/路径/参数/工作目录/运行用户/环境变量），由 **daemon(8)** 实例做崩溃拉起监护，面板负责编排：存定义（SQLite）、spawn daemon、按 PID 精确发信号、读 pidfile 判状态、读/清/轮转日志。

设计文档：`docs/plan/42-supervisor.md`。

**核心原则落地**：每条目一个 daemon(8) supervisor，PPID=1，完全独立于 fwp——fwp 宕机/重启/升级期间，已启动进程的守护照常工作（已实测验证）。fwp 重启后通过 pidfile 重新识别状态，零收养逻辑。

**本期范围**（按用户指示）：不含开机自启（fwp 启动时扫描 autostart 条目拉起）及 rc.d 相关内容。

## 2. 实现细节

### 2.1 分层

- `src/supervisor.rs` — 核心：模型、校验、DB CRUD、状态判定、启停协议、日志读取/清理/轮转（全部同步函数，由 handler 包 `spawn_blocking`）
- `src/handlers/supervisor.rs` — HTTP 层：反序列化 → 校验（core）→ blocking 线程执行 → 审计 → 响应
- `src/sysinfo.rs::read_proc_start` — `sysctl(3)` 数字 MIB `[CTL_KERN, KERN_PROC, KERN_PROC_PID, pid]` 读 `kinfo_proc.ki_start`（⚠️ `sysctlbyname` 名字形式对 kern.proc 子树返回 ENOENT，必须用数字 MIB；libc crate 提供 `kinfo_proc` 布局，读后校验 `ki_pid == pid` 防布局错配）

### 2.2 数据模型（db.rs migration v5）

```sql
CREATE TABLE supervisor_procs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,        -- ^[a-zA-Z0-9_.-]{1,64}$，用于路径拼接
    path TEXT NOT NULL,               -- 绝对路径，存在且 X_OK（创建/更新时预校验）
    args TEXT NOT NULL DEFAULT '[]',  -- JSON 数组，逐元素传 argv，无 shell
    workdir TEXT,                     -- 可空
    user TEXT,                        -- 可空 = root；getpwnam 校验
    env TEXT NOT NULL DEFAULT '[]',   -- JSON 数组 ["K=V", ...]
    logging INTEGER NOT NULL DEFAULT 1,    -- 0 = 不落盘（daemon 丢弃子进程输出）
    log_file TEXT,                         -- 可空 = 默认 <dir>/<name>.log；自定义路径永不自动删除
    log_rotate INTEGER NOT NULL DEFAULT 1, -- 0 = 调度器跳过该条目
    autostart INTEGER NOT NULL DEFAULT 0,  -- fwp 启动时拉起
    restart INTEGER NOT NULL DEFAULT 1,
    restart_delay INTEGER NOT NULL DEFAULT 1,   -- 1..=3600 秒
    restart_max INTEGER,              -- 可空 = 无限制（-C，不含首次运行）
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
```

启动命令构造（`supervisor.rs::build_daemon_cmd`）：

```
    [-o <log file> -H] -t fwp-guard:<name> [-u <user>] -- <path> <args...>
```

`-o/-H` 仅在 `logging=1` 时传入；关闭时 daemon(8) 仍把子进程 stdio 接到管道，由 supervisor 读取后丢弃（`do_output` 无输出目标即丢，子进程不会因管道满而阻塞——源码验证）。

`-o` 的目标由条目决定：`log_file` 列非空时用该绝对路径（校验：必须绝对路径、父目录存在、文件可不存在——daemon 打开时创建），否则默认 `<supervisor dir>/<name>.log`。**自定义路径的文件在删除/改名条目时永不自动清理**（可能指向 /var/log 下共享文件）；默认路径文件照常随条目删除清理。

- workdir 经 `Command::current_dir()`（daemon 与子进程继承）
- env 经 `Command::envs()` 透传
- supervisor 的 stderr 重定向到临时文件（daemon(3) 派生的长寿命进程继承管道 FD，`.output()` 会永久挂起——同 `handlers/services.rs` 的教训）
- spawn 后轮询 ≤2s 等待 supervisor pidfile 出现且存活；失败时读临时文件 stderr 作为错误信息

### 2.4 状态判定（`supervisor.rs::status`）

读两个 pidfile + `kill(pid, 0)` 判活（EPERM 也算活）：

| 状态 | 判定 | 含义 |
|---|---|---|
| `running` | sup 活 && child 活 | `started_at` = `read_proc_start(child)` |
| `starting` | sup 活 && child 无/死 | 重启延迟窗口（child pidfile 被 daemon 截断） |
| `orphaned` | sup 死 && child 活 | supervisor 被外部误杀；停止操作直接处理 child |
| `stopped` | 均不活 | 顺带清理残留 pidfile（SIGKILL supervisor 会遗留） |

`uptime` = 请求时刻 - `started_at`（handler 计算）。

### 2.5 停止协议（`supervisor.rs::stop`，幂等）

1. sup 活：SIGTERM sup → daemon(8) 转发给 child → 轮询 ≤10s 等 sup 退出；超时 → SIGKILL child（sup 已在 terminating 态，不会重启它）→ 仍不退 → SIGKILL sup
2. orphaned：SIGTERM child → ≤10s → SIGKILL child
3. 清理 pidfile；仍存活则报错（含 PID）

**所有信号只发本条目 pidfile 里的精确 PID，绝无模式匹配/按名 kill。**

### 2.6 并发控制

`AppState.supervisor_lock: Arc<tokio::sync::Mutex<()>>` 串行化 start/stop/restart/update/delete（条目操作低频，全局锁足够）。

- **每条目两个开关**（v5 表列）：`logging`（记不记日志）、`log_rotate`（要不要轮转），创建/编辑表单可改，默认全开；**日志文件路径可配置**（`log_file` 列，空 = 默认 `<dir>/<name>.log`，自定义绝对路径时 `-o`/读取/清空/轮转全部跟随该路径，且文件永不自动删除）

- `logging=0`：daemon 不带 `-o/-H`，子进程输出由 supervisor 读出后丢弃（无文件、不阻塞；日志 API 幂等返回空，前端日志按钮禁用）
- `log_rotate=0`：调度器跳过该条目（日志无限增长，由用户自担——表单帮助文案说明）
- 读取：只读文件末尾 1 MiB，取最后 N 行（默认 200，上限 2000）
- 清理：**rename + 删除 + SIGHUP supervisor**（`-H` 重开新文件）——直接 truncate 会留下守护进程 fd 偏移超 EOF 的稀疏空洞
- 轮转：调度器任务 `supervisor-log-rotate`（每小时，首次延迟 10 分钟），`logging=1 && log_rotate=1` 且单文件 >10 MiB → rename `.log.old`（覆盖旧轮转）→ SIGHUP supervisor 重开

### 2.8 前端

- `SupervisorPage.vue`（列表）：搜索框 + 计数 + 刷新/创建（`.toolbar`，左侧有内容）→ 表格（**整行点击进详情**（`row-clickable`，操作列 `@click.stop` 阻止冒泡）/命令/PID/状态徽标/运行时长/自启/重启策略/操作 btn-group：启动｜停止｜重启｜编辑｜删除）→ 5s 轮询（onUnmounted 清理）。**列表不展示日志**——日志在详情页
- `SupervisorDetailPage.vue`（详情，`GET /api/supervisor/{id}` 单条 + 5s 轮询）：
  - 页头：BackButton + 名称 + 状态徽标 + 操作按钮（启动/停止/重启/编辑/删除，运行中禁用编辑/删除）
  - StatusBar：状态 / PID / Supervisor PID / 运行时长 + 刷新
  - 基础信息卡：命令、工作目录、运行用户、环境变量、重启策略、随面板启动、日志文件（自定义时显示）、日志/轮转开关、创建/更新时间（`kv-table four-col`）
  - 日志卡：自定义路径（mono 小字）+ 尺寸 + 刷新/清空按钮 + `<pre>` 尾部 500 行（随轮询自动刷新）；`logging=0` 时按钮禁用并提示
- 表单共享：`lib/supervisorForm.js` 的 `openSupervisorForm(formModal, t, toast, existing, onSaved)`——列表/详情两处共用同一字段集与提交映射。布局按归属分组：名称/可执行路径（`picker:'file'`）/参数 textarea/工作目录+运行用户（half 一行，`picker:'dir'`）/环境变量 textarea（参数/用户/环境变量的说明收进 label 旁 FieldHelp 问号）→ **记录日志** pill → 日志文件（`picker:'file'`）与 **自动轮转** pill（均 `showIf` 记录日志）→ **随面板启动** pill → **自动重启** pill → 重启延迟+上限（half 一行，`showIf` 自动重启）。开关均为 checkbox-group pill 风格（同 rsync 表单，选项级 FieldHelp），值存 formValues 故 `showIf` 联动不受影响

### 2.9 开机自启

`autostart=1` 的条目在 fwp 启动时自动拉起（`supervisor.rs::autostart`，main 在 listener bind 后 `tokio::spawn` 调用）：

- 只拉 `autostart=1` 且当前 `stopped` 的条目；running/starting/orphaned 不动（守护独立于 fwp）
- 每条目先查状态、再拿 `supervisor_lock` 串行化后 start，与用户操作互斥
- 单条失败只 `tracing::warn`，绝不阻塞面板启动
- 前提：fwp 自身 `service fwp enable`；fwp 不在时条目不会被拉起，但已运行条目的守护照常工作
- 前端：表单「随面板启动」checkbox（默认关）；列表加「随面板启动」列（✓/—）

## 3. API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/supervisor` | 定义 + 运行态（state/sup_pid/pid/started_at/uptime） |
| GET | `/api/supervisor/{id}` | 单条定义 + 运行态（详情页用；404 处理不存在） |
| POST | `/api/supervisor` | 创建（201） |
| PUT | `/api/supervisor/{id}` | 更新（仅 stopped；改名时清理旧名运行时文件） |
| DELETE | `/api/supervisor/{id}` | 删除（仅 stopped；清理 pidfile/日志） |
| POST | `/api/supervisor/{id}/start` `…/stop` `…/restart` | 生命周期 |
| GET | `/api/supervisor/{id}/log?lines=200` | 日志尾部 `{size, content}` |
| DELETE | `/api/supervisor/{id}/log` | 清空日志 |

全部 `require_auth`；写操作 + 启停全部记审计（实测 14 条记录含改名详情）。

## 4. 外部依赖

- `/usr/sbin/daemon`（FreeBSD 基本系统）
- crate：`libc`（kill/sysctl/getpwnam/kinfo_proc）、`rusqlite`、`regex`、`serde_json`

## 5. 配置项

```toml
[paths]
supervisor = "/var/db/fwp/supervisor"   # 运行时目录，启动时自动创建
```

## 6. 已验证行为（2026-10-10 本机实测）

- `kill -9` 子进程 → 1s 后自动拉起（supervisor PID 不变）
- **fwp 停止/重启期间守护进程持续运行**，fwp 回来后状态正确恢复
- `logging=0`：无 `.log` 文件产生；高频输出（300 行突发）不阻塞子进程；更新为 `logging=1` 重启后日志文件正常出现且有内容
- **log_file 自定义路径**：`-o` 写入指定文件、日志 API 读到同路径内容；相对路径 / 父目录不存在 → 400；清空（rename+SIGHUP）作用于自定义文件；删除条目后自定义文件**保留**、默认路径文件照常清理
- **autostart**：`autostart=1` stopped 条目随 fwp 重启自动 running；`autostart=0` stopped 保持不动；运行中条目跨 fwp 重启 PID 不变（daemon(8) 独立守护）
- `-C 1` + 快退进程 → 共运行 2 次（首次+1 重启）后 supervisor 放弃退出 → stopped
- 停止 → supervisor/child 双退、pidfile 消失
- `-u nobody` 降权（实测 uid=65534）+ `env` 传递（实测 `FWP_DEMO=yes`）
- 运行中 PUT → 409；改名更新 → 旧名运行时文件清理；清日志 → size 0；删除 → 列表空
- `cargo test` 75/75 通过（含 `read_proc_start` 单测）；前端 `npm run build` 通过

## 7. 已知限制 / TODO

1. 「失败重启」= 任何退出都重启（daemon(8) 不区分退出码）；UI 帮助文案已说明
2. 子进程退出码/累计重启次数不可见
3. pid 复用极端误判：supervisor 死亡 + pidfile 残留 + pid 被新进程复用时可误报（概率极低，可后续用 proctitle 校验加固）
4. **开机自启**：`autostart=1` 条目由 fwp 启动时拉起，依赖 fwp 自身 `service fwp enable`；fwp 不在时条目不会被拉起，但**已运行条目的守护照常工作**
5. stdout/stderr 合并同一日志；无 syslog 转发
