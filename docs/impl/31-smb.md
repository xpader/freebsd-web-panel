# 31 — SMB 文件共享

## 概述

SMB 模块在"服务 → 文件共享"下提供 Samba 服务的初始化、运行状态控制、共享目录 CRUD、Samba 用户管理和全局配置管理。

FreeBSD base system 不提供现代 SMB 服务器。模块以 `samba416` pkg 为外部依赖，采用与 Bhyve 一致的"状态检测 → 初始化引导 → 流式后台任务"流程。面板不创建系统账户：Samba 用户必须是已有系统用户，模块只通过 `smbpasswd` 写入 Samba 密码数据库。

## 实现细节

### 状态检测与初始化

`src/handlers/smb.rs` 的 `check_status()` 返回以下状态：

```rust
pub struct SmbStatus {
    pub installed: bool,
    pub enabled: bool,
    pub initialized: bool,
    pub service_running: bool,
    pub version: Option<String>,
}
```

- `installed`：`/usr/local/sbin/smbd` 是否存在。
- `enabled`：`rc.conf` 的 `samba_server_enable` 是否为 `YES`。
- `initialized`：`/usr/local/etc/smb4.conf` 是否存在。
- `service_running`：读取 `/var/run/samba4/smbd.pid` 和 `nmbd.pid`，用 `kill(pid, 0)` 验证进程是否存在。不依赖 `service samba_server status`（rc.d 在 `enable=NO` 时会跳过 status 检查）。
- `version`：解析 `smbd --version` 的版本号。

前端共享列表先请求状态。只要未安装或缺少配置文件（`!installed || !initialized`），便显示初始化引导，不请求共享列表。`enabled` 状态不再影响初始化判定——因为 start/stop 会同时切换 `samba_server_enable`，停止服务不应被视为"未初始化"。

`SmbInitPage.vue` 调用初始化接口后，通过统一的 `/api/tasks/{id}/stream` SSE 端点显示输出。

初始化后台任务 `run_init_streaming()` 依序执行：

1. `pkg install -y samba416`。
2. 通过 `sysrc` 写入 `samba_server_enable=YES`。
3. 生成默认 `/usr/local/etc/smb4.conf`，启动 `samba_server`。

任务结果通过 `bgtask` 保存和推送，完成后写入审计日志。

### 服务控制

`POST /api/smb/service/{action}` 的 start 和 stop 操作与防火墙一致，同时管理 rc.conf 和运行状态：

- **start**：先 `sysrc samba_server_enable=YES`，再 `service samba_server start`。
- **stop**：先检查 `samba_server_enable` 当前值。如果为 `YES`，用 `service samba_server stop`；如果已为 `NO`（rc.d 会跳过 stop），改用 `service samba_server onestop` 强制停止。停止成功后再 `sysrc samba_server_enable=NO`。
- **restart**：只执行 `service samba_server restart`，不改 rc.conf。
- **reload**：如果服务正在运行，执行 `service samba_server reload`。

start/stop 操作后通过 `is_samba_running()` 验证服务确实到达了目标状态，未达预期时返回错误。

### smb4.conf 管理

配置文件路径为 `/usr/local/etc/smb4.conf`。`parse_conf()` 逐行解析 INI 段：`[global]` 映射为 `GlobalConfig`，其余段映射为 `SmbShare`。空行、`#` 和 `;` 注释被忽略，键名转小写后匹配。

`generate_conf()` 以固定字段顺序生成面板管理的配置。`write_conf()` 先写入同目录临时文件，再 `rename` 覆盖目标，避免部分写入。更新全局配置或共享后执行 `service samba_server reload`，使配置生效而不中断已有连接。

默认全局配置：

```ini
[global]
    workgroup = WORKGROUP
    server string = FreeBSD Samba Server
    server role = standalone
    map to guest = Bad User
    passdb backend = tdbsam
    server min protocol = SMB2
    dns proxy = no
    load printers = no
    log level = 1
```

每个共享管理 `name`、`comment`、`path`、`browseable`、`writable`、`guest ok`、`valid users`、`create mask`、`directory mask`、`time_machine` 和 `time_machine_max_size`。

**macOS 兼容性**：设置页的全局开关 `fruit_enabled` 启用后，`generate_conf()` 在 `[global]` 段写入 `vfs objects = fruit streams_xattr`、`fruit:aapl = yes`、`fruit:metadata = stream`、`fruit:encoding = native`，所有共享自动获得 AAPL 协议支持和扩展属性流（消除 `._` 文件、提升 Finder 性能）。`parse_conf()` 通过检测 `[global]` 的 `vfs objects` 是否含 `fruit` 来回填该开关。

**Time Machine**：共享级 `time_machine` 布尔字段启用后，`generate_conf()` 在对应共享段写入 `fruit:time machine = yes`；若全局未启用 macOS 兼容性，则同时在该共享段写入 `vfs objects = fruit streams_xattr`。可选的 `time_machine_max_size`（如 `1T`、`500G`）写入 `fruit:time machine max size`，为 Time Machine 备份设置容量上限。

共享名称只允许 `[a-zA-Z0-9_.$-]`，最长 64 个字符。共享路径必须是绝对路径且不能含 NUL。所有命令调用均使用参数数组，不拼接 shell。

### Samba 用户

Samba 用户数据库独立于 `/etc/passwd`，但 Samba 用户必须先是系统用户。

- `pdbedit -L` 列出 Samba 用户。
- `GET /api/smb/sysusers` 读取 `/etc/passwd`，排除 UID 小于 1000、`nologin`/`false` shell 和已加入 Samba 的账户。
- 创建用户前 `validate_system_user()` 检查系统账户存在。
- `smbpasswd -a -s <username>` 从 stdin 读取两次密码，新增或更新 Samba 密码。
- `pdbedit -x -u <username>` 删除 Samba 用户，不删除系统账户。

`src/cmd.rs` 新增 `run_sync_stdin()` 和异步包装 `run_stdin()`，以 piped stdin 执行需要交互输入的命令。密码仅在请求处理和子进程 stdin 中短暂存在，不写入日志或审计详情。

### 前端结构

路由位于 `frontend/src/router/index.js`：

| 路径 | 页面 | 说明 |
|---|---|---|
| `/shares/smb` | `SmbSharesPage.vue` | 初始化引导、共享列表和共享 CRUD |
| `/shares/smb/init` | `SmbInitPage.vue` | Samba 安装及 SSE 输出 |
| `/shares/smb/users` | `SmbUsersPage.vue` | Samba 用户和密码管理 |
| `/shares/smb/settings` | `SmbSettingsPage.vue` | 全局配置 |

`SmbStatusBar.vue` 封装 SMB 专有状态业务，并复用通用 `StatusBar.vue` 布局。它显示运行状态和 Samba 版本，提供启动、停止和重启操作。共享列表页和设置页都复用该组件，组件操作完成后通过 `refresh` 事件要求父页面重新读取状态。

共享创建/编辑弹窗使用 `formModal` 的 `half: true` 实现两栏布局，路径字段通过 `picker: 'dir'` 接入目录选择器（`FilePicker.vue`）。四个布尔选项（可浏览/可写/允许访客/Time Machine）使用 `checkbox-group` 类型以 pill 风格内联排列，其中 Time Machine pill 带有 FieldHelp 工具提示说明用途。Time Machine 容量上限为独立的文本字段，带 FieldHelp 提示。

设置页采用 `form-row`（`180px 1fr` 网格）布局，与 Jail/Bhyve 编辑页一致。除工作组、服务器描述、最小协议、访客映射和日志级别外，还提供 macOS 兼容性复选框（对应 `fruit_enabled`），使用 `checkbox-label` + `param-desc-inline` 内联描述样式。

菜单定义在 `frontend/src/lib/menu.js`，位于服务组（紧跟"系统服务"rc.d 项之后，父菜单标签为「SMB 共享」）：共享目录（`fa-folder-tree`）、共享用户（`fa-user-lock`）和 Samba 设置（`fa-gear`）。

## API

所有接口都需要面板认证。

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/smb/status` | Samba 安装、服务、配置和版本状态 |
| POST | `/api/smb/init` | 启动 Samba 初始化后台任务，返回 `task_id` |
| GET | `/api/smb/config` | 读取 `[global]` 配置 |
| PUT | `/api/smb/config` | 更新 `[global]` 配置并 reload 服务 |
| GET | `/api/smb/shares` | 列出共享 |
| POST | `/api/smb/shares` | 创建共享 |
| PUT | `/api/smb/shares/{name}` | 更新共享 |
| DELETE | `/api/smb/shares/{name}` | 删除共享 |
| GET | `/api/smb/users` | 列出 Samba 用户 |
| GET | `/api/smb/sysusers` | 列出可添加的系统用户 |
| POST | `/api/smb/users` | 为已有系统用户设置 Samba 密码 |
| PUT | `/api/smb/users/{name}/password` | 更新 Samba 密码 |
| DELETE | `/api/smb/users/{name}` | 删除 Samba 用户数据库记录 |
| POST | `/api/smb/service/{action}` | `start`（同时 enable）、`stop`（同时 disable）、`restart` 或 `reload` 服务 |

创建共享请求示例：

```json
{
  "name": "public",
  "comment": "Public files",
  "path": "/zroot/data/public",
  "browseable": true,
  "writable": true,
  "guest_ok": false,
  "valid_users": ["alice", "bob"],
  "create_mask": "0664",
  "directory_mask": "0775",
  "time_machine": false,
  "time_machine_max_size": ""
}
```

创建或更新 Samba 密码请求示例：

```json
{
  "username": "alice",
  "password": "password"
}
```

## 外部依赖

| 依赖 | 用途 |
|---|---|
| `samba416` | 提供 `smbd`、`pdbedit`、`smbpasswd` 和 `samba_server` rc.d 脚本 |
| `/usr/sbin/pkg` | 初始化时安装 Samba |
| `/usr/sbin/sysrc` | 管理 `samba_server_enable` |
| `/usr/sbin/service` | 管理 `samba_server`（含 `onestop` 强制停止） |
| `bgtask.rs` | 初始化任务的后台执行与 SSE 输出 |
| `libc` (`kill(pid, 0)`) | 检测 smbd/nmbd 进程是否存活 |

## 配置项

模块不新增 `fwp.toml` 配置项。

| 文件或变量 | 用途 |
|---|---|
| `/usr/local/etc/smb4.conf` | Samba 全局和共享配置 |
| `samba_server_enable` | Samba 开机自启开关（start/stop 联动） |
| `/var/run/samba4/smbd.pid`, `nmbd.pid` | 进程状态检测 |
| `/var/db/samba4/` | Samba 密码数据库等运行数据，由 Samba 管理 |

## 已知限制 / TODO

- 仅支持 standalone Samba server，不支持 Active Directory 域加入或 Kerberos。
- 不提供 Windows ACL、NT ACL 或打印机共享管理。
- 配置解析器仅保留面板支持的字段，手工写入的不支持字段会在下次保存时丢失。
- 不显示实时连接、锁和会话信息；后续可接入 `smbstatus`。
- 面板 API 传递 Samba 密码时为 HTTP 请求体；生产环境必须通过 TLS 反向代理访问面板。
