# 33 — Rsync 同步任务

## 概述

Rsync 模块在"服务 → Rsync 同步"下提供多个用户自定义的 rsync 同步任务，支持手动启动同步（含试运行）、任务 CRUD、源/目标路径的本地与远程目录浏览、可选的定时调度（写入 `/etc/crontab`）、指定执行用户，以及首次使用时的 rsync 安装初始化引导。同步方向由源/目标的位置隐含决定（本地→远程、远程→本地、本地→本地），无需单独的模式字段。

FreeBSD base system 不提供 rsync。模块以 `rsync` pkg 为外部依赖，采用与 SMB / Bhyve 一致的"状态检测 → 初始化引导 → 流式后台任务"流程。所有命令调用均使用参数数组（`Command::new().arg()`），绝不拼接 shell，因此无注入面。

## 实现细节

### 数据存储

任务定义存储在 SQLite 表 `rsync_tasks`（db 迁移 `m3` 建表，`m4` 加 `run_user`/`cron_expr`），字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | INTEGER PK | 自增主键 |
| `description` | TEXT | 任务描述（非空，≤128 字符，无控制字符） |
| `source` | TEXT | 源路径（本地绝对路径或 `host:path` 远程规格） |
| `dest` | TEXT | 目标路径（同上） |
| `archive` | INTEGER | 布尔，`-a` |
| `compress` | INTEGER | 布尔，`-z` |
| `delete` | INTEGER | 布尔，`--delete` |
| `verbose` | INTEGER | 布尔，`-v` |
| `port` | INTEGER | SSH 端口（仅远程时使用，1-65535，可空） |
| `extra_args` | TEXT | 额外 rsync 选项（空格分隔，作为独立 argv 元素） |
| `run_user` | TEXT | 执行 rsync 的系统用户（空=root）。手动运行经 `su` 切换；定时写入 crontab 的 `who` 列 |
| `cron_expr` | TEXT | 5 字段 cron 表达式（分 时 日 月 周）。空=不定时 |
| `last_run_at` | INTEGER | 上次运行 Unix 时间戳 |
| `last_status` | TEXT | `success` / `failed`（上次运行结果） |
| `created_at`, `updated_at` | INTEGER | 时间戳 |

`"delete"` 列名加双引号转义（`delete` 是 SQL 关键字）。

### 状态检测与初始化

`src/handlers/rsync.rs` 的 `check_status()` 返回：

```rust
pub struct RsyncStatus {
    pub installed: bool,
    pub version: Option<String>,
}
```

- `installed`：`/usr/local/bin/rsync` 是否存在。
- `version`：解析 `rsync --version` 首行，跳过 `version` 关键字取版本号（如 `3.4.1`）。

前端任务列表先请求状态。只要 `!installed`，便显示初始化引导（跳转 `/rsync/init`），不展示任务列表。

`RsyncInitPage.vue` 调用初始化接口后，通过统一的 `/api/tasks/{id}/stream` SSE 端点显示输出。

初始化后台任务仅一步：`pkg install -y rsync`。任务结果通过 `bgtask` 保存和推送，完成后写入审计日志。

### argv 构造（`build_rsync_args`）

从任务定义构造 rsync argv 数组，选项逐个作为独立元素：

1. `archive` → `-a`；`verbose` → `-v`；`compress` → `-z`；`delete` → `--delete`；试运行追加 `-n`。
2. **SSH 包装器**：当 `source` 或 `dest` 任一为远程（不以 `/` 开头且含 `:`，匹配 rsync 自身的 `host:path` / `host::module` 判定）时，追加 `-e "ssh -o BatchMode=yes [-p PORT]"`。`BatchMode=yes` 使任务在缺少密钥/口令时**立即失败**而非挂起等待交互输入。
3. `extra_args` 按空白拆分为独立 argv 元素（非 shell 拼接）。
4. 追加 `source`、`dest`。

源/目标的填写顺序直接决定同步方向：本地源→远程目标即"推送"、远程源→本地目标即"拉取"、两者皆本地即本地拷贝。rsync 本身总是按 `[options] source dest` 调用，无需单独的模式字段。

### 手动运行

`POST /api/rsync/tasks/{id}/run` 作为流式后台任务执行：

1. `ensure_installed()` 拦截未安装。
2. 从 DB 读取任务。
3. `build_rsync_args()` 构造 argv。
4. **切用户**：若 `run_user` 非空，用 `su <user> - -c '<shell-quoted rsync>'` 包装执行（root 调 `su` 免密）；否则直接 exec rsync。首行推送实际命令串便于审计。
5. `bgtask::run_streaming_cmd(cmd, &args)` 在 PTY 中执行，逐行捕获输出推入任务存储。
6. 退出后更新 `last_run_at` / `last_status`，设置任务最终状态，记录审计日志。

试运行（`dry_run: true`）追加 `-n`，不实际写入目标，便于预检。

### 定时执行（cron 同步）

当 `cron_expr` 非空（5 字段表达式或 `@daily`/`@hourly` 等特殊别名），任务在 `/etc/crontab` 中物化为一个两行块：

```
# [fwp-managed, rsync=<id>] <description> (managed by FreeBSD-Web-Panel — do not edit manually)
<cron_expr> <who> /usr/local/bin/rsync <shell-quoted args...>
```

**关联机制**：注释行的 `[fwp-managed, rsync=<id>]` 标签用 DB 的 `AUTOINCREMENT` 主键（删除后永不复用）作为锚点。`upsert_cron_block` / `remove_cron_block` 扫描该标签定位块，因此 crontab 行号漂移从不破坏关联。

**fwp-managed 保护**：`[fwp-managed]` 前缀使 crontab 管理模块（`crontab.rs`）识别该条目为面板托管。crontab 的 `update` / `delete` API 遇到带此标记的条目时返回 409（拒绝编辑/删除），源数据只在 rsync 任务本身管理。

- **创建/更新**任务后调用 `sync_cron()`：`cron_expr` 非空则 upsert 块（存在则原地替换、不存在则追加），为空则移除块。
- **删除**任务后调用 `remove_cron_block(id)`：直接从 `/etc/crontab` **删除**整个块（不注释保留——源数据在 DB，无需保留）。
- **执行用户**：`who` 列写 `run_user`（空则为 `root`），cron 自身以该用户运行 rsync——与手动运行的 `su` 语义一致。
- **shell 引号**：`shell_quote_one` 把 argv 元素用单引号包裹（`'` → `'\''`），杜绝 crontab 命令串注入。因 cron 行无 `su` 包装，所有参数都经此转义。

`/etc/crontab` 经 tmp 文件 + rename 原子替换（保持 0644）。cron 在约 1 分钟内按 mtime 检测变更并生效。

### 验证

- 描述：非空，≤128 字符，不含控制字符。
- 路径规格：非空，不含控制字符（`< ' '` 或 `\x7f`）。本地路径必须是绝对路径；远程路径形如 `user@host:path`（语法不过度限制，由 rsync 自身解析）。
- `extra_args`：不含控制字符。
- `port`：1-65535（0 或空表示默认）。
- `run_user`：空（root）或合法用户名（`[a-zA-Z0-9_.-]`，≤32）。
- `cron_expr`：空，或恰好 5 个字段（每字段仅允许 `[0-9*/,-]`），或单个 `@` 特殊别名（`@reboot`/`@yearly`/`@annually`/`@monthly`/`@weekly`/`@daily`/`@midnight`/`@hourly`）。

### 前端结构

路由位于 `frontend/src/router/index.js`：

| 路径 | 页面 | 说明 |
|---|---|---|
| `/rsync` | `RsyncTasksPage.vue` | 初始化引导、任务列表、CRUD、手动运行 |
| `/rsync/init` | `RsyncInitPage.vue` | rsync 安装及 SSE 输出 |

`RsyncTasksPage.vue` 复用 `TaskConsole.vue`（运行日志流）。**创建/编辑使用标准 `formModal`**（`DialogHost`），与全站表单一致：四个选项（归档/压缩/删除多余/详细）用 `checkbox-group` pill 风格，SSH 端口用 `half` 半栏，执行用户用 `half` 半栏。**定时执行**用 `type: 'cron'` 字段——渲染为 `CronScheduleInput.vue` 组件（开关 + schedule 类型下拉 + 5 字段，对外暴露单个 `cron_expr` 字符串）。每行操作按钮包裹在 `.btn-group`，含运行（播放）、试运行（烧瓶）、编辑、删除。已开启定时的任务在描述列显示 cron 表达式徽标。

**远程目录浏览能力下沉到 `DialogHost`**：源/目标路径字段用 `picker: 'dir'` + `portKey: 'port'`。`DialogHost` 的选择按钮按字段当前值自动判定本地/远程——值形如 `user@host`（SSH 连接）时打开 `RemoteFilePicker.vue`（🌐 图标），否则（本地绝对路径或空）打开本地 `FilePicker.vue`（📁 图标）。远程选择器的端口取自 `portKey` 指向的字段（SSH 端口）。该能力对所有 `formModal` 表单通用（现有本地路径表单的值不会误触发远程判定，向后兼容）。

**`RemoteFilePicker.vue`**：与本地 `FilePicker` **完全一致的树形交互与视觉**（复用 `createTreeState` + `FileTreeRow`，仅把目录读取换成 `GET /api/rsync/browse`）。无连接输入框、无面包屑、无地址栏——打开即用字段中的 host 自动连接并以 `host:/` 为树根，点击目录即原地展开（懒加载子目录），双击确认。`createTreeState` 已泛化为可插拔的 `fetchDir` / `ancestorPaths`（本地默认不变，向后兼容 `FilesPage`/`FilePicker`）。若字段含更深的 `host:/path`，打开时自动展开至该目录并选中。

工具栏按钮放在 `.page-header` 行内靠右（左侧为空，遵循布局约定）。

菜单定义在 `frontend/src/lib/menu.js`，位于服务组（"系统服务"和 SMB 共享之后，单页无子菜单，`fa-arrows-rotate`）。

## API

所有接口都需要面板认证。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/rsync/status` | rsync 安装状态与版本 |
| POST | `/api/rsync/init` | 启动 rsync 安装后台任务，返回 `task_id` |
| GET | `/api/rsync/tasks` | 列出全部同步任务 |
| POST | `/api/rsync/tasks` | 创建任务 |
| PUT | `/api/rsync/tasks/{id}` | 更新任务 |
| DELETE | `/api/rsync/tasks/{id}` | 删除任务 |
| POST | `/api/rsync/tasks/{id}/run` | 立即运行任务，返回 `task_id`；`{dry_run: true}` 试运行 |
| GET | `/api/rsync/browse?spec=[user@]host:/path&port=N` | 通过 SSH 列出远程目录子项（目录在前），返回 `[{name,path,is_dir}]`，`path` 为完整可复用 spec |

创建/更新任务请求示例：

```json
{
  "description": "docs-backup",
  "source": "/zroot/data/",
  "dest": "user@nas:/backup/data/",
  "archive": true,
  "compress": true,
  "delete": false,
  "verbose": true,
  "port": 2222,
  "extra_args": "--partial --bwlimit=1000",
  "run_user": "backup",
  "cron_expr": "0 3 * * *"
}
```

运行请求示例：

```json
{ "dry_run": false }
```

任务输出通过统一 SSE 端点 `GET /api/tasks/{id}/stream?token=...` 流式推送。

## 外部依赖

| 依赖 | 用途 |
|---|---|
| `rsync`（pkg） | 同步引擎 `/usr/local/bin/rsync` |
| `/usr/sbin/pkg` | 初始化时安装 rsync |
| `bgtask.rs` | 初始化与手动运行的后台执行与 SSE 输出 |
| `ssh`（客户端） | 远程目录浏览执行 `ls -1Ap`（仅依赖 SSH，不需远程 rsync） |

## 配置项

模块不新增 `fwp.toml` 配置项。任务定义存储在面板 SQLite 数据库（`/var/db/fwp/fwp.db` 的 `rsync_tasks` 表）。

## 已知限制 / TODO

- 不管理 SSH 密钥：远程同步需用户预先配置好免密 SSH 访问（`BatchMode=yes` 会在缺少凭据时直接失败）。
- `extra_args` 不做选项白名单校验，仅按空白拆分为独立 argv 元素（无 shell 拼接，故无注入面，但仍可传入任意 rsync 选项）。
- **真实远程同步需两端都安装 rsync**：rsync 协议要求本地与远程主机都有 rsync 二进制。远程目录浏览只依赖 SSH（执行 `ls`），因此浏览可用不代表同步一定可用——若远程未装 rsync，同步会以 `sh: rsync: not found` 失败。
