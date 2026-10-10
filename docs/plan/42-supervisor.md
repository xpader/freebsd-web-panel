# 进程守护（Process Supervisor）设计方案

> 状态：**已实现**（2026-10-10）——见 `docs/impl/37-supervisor.md`。实现差异：`read_proc_start` 用 sysctl(3) 数字 MIB（`sysctlbyname` 名字形式对 kern.proc 子树无效）；清空日志用 rename+SIGHUP 重开（避免 truncate 稀疏空洞）；自启动与自定义日志路径均收在 v5 建表迁移内（开发期未发布，不单列迁移版本）。

## 1. 目标与边界

在「服务」菜单组新增「进程守护」页面：

- 手动添加受管进程：名称、可执行文件路径、参数、工作目录、运行用户、环境变量
- 策略配置：开机自启（fwp 启动时拉起）、失败是否重启、重启延迟、最大重启次数
- 界面操作：启动 / 停止 / 重启 / 查看日志 / 编辑 / 删除
- 等价 supervisord 的核心能力

**非目标**：资源限制（rlimits/cgroups）、进程优先级、进程组管理、输出流分离（stdout/stderr 合并写同一日志）。

**硬约束（项目核心原则）**：面板不是运行时依赖——fwp 宕机、重启、升级期间，已启动进程的守护（崩溃拉起）必须继续工作。这直接决定了技术选型。

## 2. 技术选型

### 候选方案

| | A. fwp 内建 supervisor | B. 每条目一个 daemon(8) 实例（推荐） | C. 生成 rc.d 脚本 |
|---|---|---|---|
| 实现方式 | fwp 直接 fork/wait 子进程，自写状态机 | spawn `/usr/sbin/daemon -r ...` 托管 | 每条目写 /etc/rc.d 脚本 |
| fwp 宕机时守护 | ❌ 中断（违背核心原则） | ✅ 不中断（supervisor PPID=1 独立运行，已实测） | ✅ 不中断 |
| 代码量 | 大（状态机、孤儿回收、pid 复用竞态） | 小（拼参数 + pidfile 读写 + 信号） | 中，但污染系统配置 |
| FreeBSD 原生 | ❌ fwp 自己变成 supervisor 守护进程 | ✅ 基本系统自带 | ✅ |
| 语义丰富度 | 高（退出码、重启计数、仅失败重启） | 低（见「已知限制」） | 低 |
| 误杀 supervisor | N/A | 子进程成孤儿，界面可识别并清理 | 同 B |

**决策：方案 B。** daemon(8) 是 FreeBSD 基本系统自带的进程监护器，fwp 只做编排（存定义、spawn、发信号、读 pidfile/日志），监护循环本身完全脱离 fwp 生命周期。fwp 重启后通过 pidfile 重新识别运行状态，零收养逻辑。

### daemon(8) 实测语义（FreeBSD 15.1-RELEASE-p3 本机验证，2026-10）

| 能力 | flag | 实测结论 |
|---|---|---|
| 守护重启 | `-r` | 任何退出（含 exit 0、SIGKILL）都重启，无限次；supervisor PID 稳定 |
| 重启延迟 | `-R N` | 子进程退出后等 N 秒再拉起（1–31536000） |
| 最多重启 N 次 | `-C N` | **不含首次运行**：`-C 3` 实测共运行 4 次后 supervisor 退出；默认无限制 |
| 不重启仅托管 | 省略 `-r` | 仍写 pidfile + 日志，子进程退出后 supervisor 一并退出 |
| 停止 | SIGTERM → supervisor | supervisor 转发 SIGTERM 给子进程 → 等待 → 双双退出 → **pidfile 自动删除**（pidfile(3) 锁语义） |
| 顽固子进程 | — | daemon(8) 无强杀定时器（源码 TODO）：fwp 停止流程需在超时后直接 SIGKILL 子进程（supervisor 已进 terminating 态，不会因 SIGKILL 重启） |
| 日志 | `-o file`（追加，stdout+stderr 合并）、`-H` | `-H`：SIGHUP 时重开日志文件 → 支持 rename+HUP 轮转 |
| 降权 | `-u user` | setusercontext，自动设 HOME/USER/SHELL |
| 进程标识 | `-t title` | setproctitle，ps 中显示 `daemon: <cmd>[pid] (title)` |
| exec 失败 | — | ⚠️ 原父进程 **exit 0**，错误只在 stderr（`daemon: /x: No such file...`）→ fwp 必须预校验 + 启动后复查 |
| supervisor 独立性 | — | PPID=1（daemon(3) 脱离会话），fwp 崩溃零影响 |

重启延迟窗口内 child pidfile 被截断（`pidfile_truncate`），用于区分「运行中」与「待重启」状态。

## 3. 数据模型

### SQLite（`db.rs` migration v5）

```sql
CREATE TABLE IF NOT EXISTS supervisor_procs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    name          TEXT NOT NULL UNIQUE,          -- ^[a-zA-Z0-9_.-]+$
    path          TEXT NOT NULL,                 -- 绝对路径，存在且可执行
    args          TEXT NOT NULL DEFAULT '[]',    -- JSON 数组，逐元素传 argv，无 shell
    workdir       TEXT,                          -- 可空 = 不切换
    user          TEXT,                          -- 可空 = root；-u 传给 daemon
    env           TEXT NOT NULL DEFAULT '[]',    -- JSON 数组 ["K=V", ...]
    autostart     INTEGER NOT NULL DEFAULT 0,    -- fwp 启动时拉起
    logging       INTEGER NOT NULL DEFAULT 1,    -- 0 = 不落盘（daemon 丢弃输出）
    log_file      TEXT,                          -- 可空 = 默认 <dir>/<name>.log；自定义路径永不自动删除
    log_rotate    INTEGER NOT NULL DEFAULT 1,    -- 0 = 调度器跳过轮转
    restart       INTEGER NOT NULL DEFAULT 1,    -- 0=不重启 1=守护
    restart_delay INTEGER NOT NULL DEFAULT 1,    -- 秒，>=1
    restart_max   INTEGER,                       -- 可空 = 无限制（-C）
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
```

### 运行时目录（可配置，默认 `/var/db/fwp/supervisor/`）

```
/var/db/fwp/supervisor/
├── <name>.sup.pid      # daemon supervisor PID（-P）
├── <name>.child.pid    # 受管进程 PID（-p）
└── <name>.log          # 子进程 stdout+stderr（-o，追加）
```

`config.rs` 的 `PathsConfig` 增加 `supervisor: PathBuf`（默认 `/var/db/fwp/supervisor`），目录启动时自动创建。

### 启动命令构造

```
/usr/sbin/daemon
  [-r] [-R <restart_delay>] [-C <restart_max>]     # restart=1 时
  -P <dir>/<name>.sup.pid
  -p <dir>/<name>.child.pid
  -o <dir>/<name>.log -H
  -t fwp-guard:<name>
  [-u <user>]
  -- <path> [args...]                               # argv 直传，无 shell
```

- `Command::current_dir(workdir)`（daemon 默认继承调用者 cwd，不传 `-c`）
- `Command::envs(env)`：环境变量经 daemon → execvp 透传给子进程
- `spawn_blocking` 内 `.output()` 等待原父进程退出（daemon(3) 立即返回）；**stderr 非空且 supervisor 未存活 → 启动失败，stderr 作为错误信息**

## 4. 状态判定与生命周期

每条目状态（读 pidfile + `kill(pid, 0)` 判活）：

| 状态 | 判定 | 说明 |
|---|---|---|
| `running` | sup 活 && child 活 | 正常 |
| `starting` | sup 活 && child pidfile 空/child 死 | 重启延迟窗口或刚启动 |
| `orphaned` | sup 死 && child 活 | supervisor 被外部误杀；停止操作直接处理 child |
| `stopped` | 均不活 / pidfile 不存在 | |

- 运行时长：`sysinfo.rs` 新增 `read_proc_start(pid)`（sysctl `KERN_PROC_PID` → `libc::kinfo_proc.ki_start`，二进制结构 → 符合 FFI 使用准则）
- 显示命令行：可选读 `KERN_PROC_ARGS`（v1 可省，DB 里已有定义）

**停止协议**（stop/restart/delete 前置）：

1. SIGTERM supervisor（自动转发给子进程）
2. 轮询 ≤10s 等 supervisor 退出
3. 超时 → SIGKILL child → supervisor（terminating 态）随后退出；仍不退 → SIGKILL supervisor
4. orphaned 态：直接 SIGTERM child，超时 SIGKILL

**并发控制**：`AppState` 新增 `supervisor_lock: Arc<tokio::sync::Mutex<()>>`，所有启停操作串行化（条目操作低频，全局锁足够，避免 per-entry map 复杂度）。

**开机自启**：`main.rs` 在 listener bind 后 `tokio::spawn` 扫描 `autostart=1` 且未运行的条目逐个启动（失败仅 warn 日志，不阻塞启动）。前提 fwp 服务已 enable——见「已知限制」。

**日志轮转**：daemon 恒带 `-H`；scheduler 注册 `register_interval!("supervisor_log_rotate", 3600s)`：单文件 >10MB → rename 为 `.log.old`（覆盖旧轮转文件）→ SIGHUP supervisor 重开新文件。

## 5. API 设计

全部走 `require_auth`，全部写操作记审计日志。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/supervisor` | 列表 = 定义 + 运行时状态（state/pid/uptime） |
| GET | `/api/supervisor/{id}` | 单条定义 + 运行时状态（详情页） |
| PUT | `/api/supervisor/{id}` | 更新（仅 `stopped` 状态允许） |
| DELETE | `/api/supervisor/{id}` | 删除（仅 `stopped` 允许；同时清理 pidfile/日志文件。前端编排「先停后删」） |
| POST | `/api/supervisor/{id}/start` | 启动 |
| POST | `/api/supervisor/{id}/stop` | 停止（上述停止协议） |
| POST | `/api/supervisor/{id}/restart` | 停止 + 启动 |
| GET | `/api/supervisor/{id}/log?lines=200` | 读日志尾部 N 行（默认 200，上限 2000） |
| DELETE | `/api/supervisor/{id}/log` | 清空日志 |

后端文件布局：核心逻辑 `src/supervisor.rs`（spawn/stop/status/校验，参照 `bhyve.rs`/`jail.rs` 的 core+handler 分层），HTTP 层 `src/handlers/supervisor.rs`。

## 6. 前端设计

- **菜单**：`services` 组新增 `{ path: '/supervisor', labelKey: 'nav.processGuard', icon: 'fa-solid fa-heart-pulse' }`（与 /services、/rsync 平级）
- **页面** `SupervisorPage.vue`：
  - `.page-header`（标题 + 副标题）
  - `.toolbar`：SearchInput + 计数 + 刷新 + 「添加进程」（左侧有搜索框 → 按 toolbar 规范）
  - 表格列：名称 / 命令（path + 参数摘要，mono）/ 状态徽标（running 绿、starting 黄、orphaned 红、stopped 灰）/ PID / 运行时长 / 自启 / 重启策略 / 操作
  - 操作列 `btn-group`（`@click.stop` 阻止冒泡）：启动｜停止｜重启｜编辑｜删除（运行中禁用编辑/删除；删除用 `useConfirm` 二次确认）；**整行点击进详情页**（`row-clickable`，同 Jail/Bhyve 列表），**列表不展示日志**
- **详情页** `SupervisorDetailPage.vue`（路由 `supervisor/:id`，`GET /api/supervisor/{id}` 单条 + 5s 轮询）：
  - 上方：页头（BackButton + 名称 + 状态徽标 + 操作按钮组）→ StatusBar（状态/PID/Supervisor PID/运行时长）→ 基础信息卡（命令/工作目录/运行用户/环境变量/重启策略/随面板启动/日志开关/创建更新时间）
  - 下方：日志卡（尺寸 + 刷新/清空 + `<pre>` 尾部 500 行，随轮询自动刷新；`logging=0` 禁用）
- **创建/编辑**：`useFormModal`（`lib/supervisorForm.js` 共享，列表/详情两处复用），字段 = 名称、路径、参数（textarea **每行一个参数**，避免引号解析）、工作目录、运行用户、环境变量（textarea 每行 `K=V`）、记录日志（checkbox，关 = 输出丢弃）、日志文件（文本框，`showIf` 日志开启；空 = 默认路径，自定义须为绝对路径且父目录存在）、自动轮转（checkbox，`showIf` 日志开启）、随面板启动（checkbox）、失败后自动重启（checkbox）、重启延迟秒、最大重启次数（空 = 无限）
- **反馈**：成功 → toast；失败 → `useAlert` 弹窗（含后端错误详情）（遵循项目消息反馈规范）
- **i18n**：新增 `supv.*` 命名空间 + `nav.processGuard`；`common.start/stop/restart/running/stopped/name/status/actions` 等全部复用现有 key；遵守 translations.js 顶部命名规范

## 7. 安全

- name 匹配 `^[a-zA-Z0-9_.-]+$`（用于路径拼接，杜绝穿越）
- path 必须绝对路径且 `X_OK`；args 以 JSON 数组逐元素传 argv——**无 shell 拼接**
- user 经 `getpwnam` 校验存在
- 所有写操作 + 启停操作进审计日志
- 提示性文案：受管进程默认以 root 运行（面板即 root），需要降权时填「运行用户」

## 8. 已知限制

1. **「失败重启」= 任何退出都重启**（daemon(8) 语义，不区分退出码）。对常驻服务场景等价；若进程正常 exit 0 也会被拉起——UI 帮助文案说明。
2. **重启次数与退出码不可见**：daemon(8) 不暴露子进程退出状态；界面只展示当前 PID/运行时长/状态。
3. **pid 复用极端场景**：supervisor 死亡与 pidfile 残留（SIGKILL supervisor 时可能发生）→ 判活用 `kill(pid,0)` + 失败即视为 stopped 并清理残留 pidfile；pid 被系统复用为新进程的误判概率极低，v1 接受（可后续用 proctitle 校验加固）。
4. **开机自启依赖 fwp 服务启用**：条目由 fwp 启动时拉起（rc.d `fwp_enable=YES` 即覆盖开机场景）；若 fwp 未设自启，条目也不会被拉起，但**已运行条目的守护照常工作**。
5. stdout/stderr 合并写同一日志文件；无日志转发（syslog 可后续加）。

## 9. 验收标准

- 添加 `/bin/sleep` 条目 → 启动 → `ps` 可见 supervisor + child；`kill -9 child` → 延迟后自动拉起，状态徽标经历 starting→running
- 停止 → 两进程退出、pidfile 消失；重启按钮 = stop+start
- `service fwp restart` → 受管进程不中断、fwp 起来后状态正确显示
- `-C` 上限条目反复退出后 supervisor 放弃退出，界面显示 stopped
- 日志实时追加可见；>10MB 自动轮转出 `.log.old`
- autostart 条目随 fwp 启动拉起
- `cargo check` 与 `cd frontend && npm run build` 通过

## 10. 涉及文件

| 层 | 文件 | 变更 |
|---|---|---|
| 后端 | `src/supervisor.rs` | 新增：核心逻辑（构造命令/启停/状态/校验/日志读取/轮转） |
| 后端 | `src/handlers/supervisor.rs` | 新增：HTTP handler |
| 后端 | `src/db.rs` | migration v5 + CRUD 自由函数 |
| 后端 | `src/app.rs`、`src/handlers/mod.rs` | 路由注册 |
| 后端 | `src/config.rs` | `paths.supervisor` 默认值 |
| 后端 | `src/state.rs` | `supervisor_lock` |
| 后端 | `src/main.rs` | autostart 拉起任务 |
| 后端 | `src/scheduler.rs` | 日志轮转任务 |
| 后端 | `src/sysinfo.rs` | `read_proc_start`（KERN_PROC_PID） |
| 前端 | `lib/menu.js`、`router` | 菜单项 + 路由（列表 + `supervisor/:id` 详情） |
| 前端 | `pages/SupervisorPage.vue` | 列表页 |
| 前端 | `pages/SupervisorDetailPage.vue` | 详情页（信息 + 操作 + 日志） |
| 前端 | `lib/supervisorForm.js` | 创建/编辑表单（列表/详情共享） |
| 前端 | `i18n/translations.js` | `supv.*` + `nav.processGuard` |
| 文档 | `docs/impl/37-supervisor.md` | 实现文档（实现完成后） |
