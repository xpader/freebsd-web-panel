# 11 — 系统用户与用户组

## 概述

管理 FreeBSD 系统用户与用户组。读取列表通过直接解析 `/etc/passwd`、`/etc/group`、`/etc/master.passwd` 实现；增删改通过 `pw(8)` 命令完成（参数全部经校验后以 `.arg()` 传递，绝不字符串拼接 shell）。

## 实现细节

### 后端 `src/handlers/accounts.rs`

#### 读取（列表）

- `list_users` — 解析 `/etc/passwd`（7 字段 `name:passwd:uid:gid:gecos:home:shell`），按 uid 升序。每条记录附加：
  - `group_name` — 主组名，由 `read_group_map()`（`/etc/group` 的 `gid → name`）解析。
  - `groups` — 附加组成员关系，由 `read_supp_group_map()`（扫描 `/etc/group` 第 4 字段的成员列表，构建 `username → [组名]`）解析。
  - `locked` — 账户是否锁定，由 `read_locked_map()`（读 `/etc/master.passwd`，密码字段以 `*LOCKED*` 开头则锁定）解析。读 `master.passwd` 失败时返回空 map，不报错（锁定状态仅作展示）。
- `list_groups` — 解析 `/etc/group`（4 字段 `name:passwd:gid:member_list`），按 gid 升序；`member_list` 为逗号分隔。
- `list_shells` — 解析 `/etc/shells`（跳过空行与 `#` 注释），返回合法 shell 列表，供前端下拉选择。

#### 写入（`pw(8)` 命令）

所有写操作经 `tokio::task::spawn_blocking` 在阻塞线程池执行（命令为 fork+exec，不能占用 async worker）。`pw` 位于 `/usr/sbin/pw`。

| 操作 | 命令 |
|---|---|
| 创建用户 | `pw useradd -n <name> [-u uid] [-g 主组] [-G 附加组1,附加组2] [-c gecos] [-d home] [-s shell] [-m] [-h 0]` |
| 修改用户 | `pw usermod -n <name> [-l newname] [-u uid] [-g 主组] [-G 附加组] [-c gecos] [-d home] [-s shell] [-h 0]` |
| 锁定/解锁 | `pw lock <name>` / `pw unlock <name>` |
| 删除用户 | `pw userdel -n <name> [-r]`（`-r` 连带删除主目录） |
| 创建组 | `pw groupadd -n <name> [-g gid] [-M member1,member2]` |
| 修改组 | `pw groupmod -n <name> [-l newname] [-g gid] [-M members]` |
| 删除组 | `pw groupdel -n <name>` |

- **密码**：`-h 0` 表示从 stdin（fd 0）读取明文密码，`pw` 自行加密（`-H fd` 才是已加密）。密码经 `cmd::run_sync_stdin` 管道传入。
- **附加组/成员替换语义**：`pw usermod -G` 与 `pw groupmod -M` 都是**整体替换**（非追加）。传入空串 `""` 即清空。前端编辑表单预填当前值，保存时整体提交，符合替换语义。
- **重命名**：`usermod -l` / `groupmod -l`。重命名后若还需锁定/解锁，使用新名称。
- **主目录**：创建时 `-m` 从 skel 目录（`/usr/share/skel`）复制；修改时 `-d` 仅改记录不搬移文件。

#### 校验（`validate_*`）

- `validate_name` — 用户名/组名正则 `^[a-zA-Z0-9_][a-zA-Z0-9_.-]{0,31}$`。首字符限制同时防止以 `-` 开头（否则被 `pw` 当作选项）。
- `validate_group_ref` — 主组/成员引用，接受组名或数字 gid。
- `validate_shell` — 必须在 `/etc/shells` 中。
- `validate_path_str` — 必须绝对路径，禁 NUL / 换行。
- `validate_gecos` — 禁 `:` / 换行 / NUL，长度 ≤ 256。
- `validate_uid` — 禁 uid 0（root）。

#### 安全护栏

- 拒绝创建名为 `root` 的用户；拒绝删除 uid 0（root）；拒绝重命名为 `root`。
- 拒绝锁定 root 账户。
- 拒绝删除/重命名 gid 0（wheel）；拒绝创建/改为 gid 0。
- 重复名称返回 `409 Conflict`；不存在返回 `404 NotFound`。

### 数据结构

```rust
struct SystemUser {
    name: String,
    uid: u32,
    gid: u32,
    gecos: String,
    home: String,
    shell: String,
    group_name: Option<String>,   // 主组名
    groups: Vec<String>,          // 附加组成员关系
    locked: bool,                 // 是否锁定（来自 master.passwd）
}

struct SystemGroup {
    name: String,
    gid: u32,
    members: Vec<String>,
}
```

### 前端

- `AccountsUsersPage.vue` — 用户表格（用户名/UID/主组/附加组/描述/主目录/Shell/操作）+ 创建/编辑/删除。
- `AccountsGroupsPage.vue` — 组表格（组名/GID/成员/操作）+ 创建/编辑/删除。
- 创建/编辑用 `useFormModal`；主组用 `<select>`（来自组列表），Shell 用 `<select>`（来自 `/api/accounts/shells`），附加组/成员用逗号分隔文本输入。
- 删除用户时 `useConfirm` 提供复选框「同时删除主目录」（`?remove_home=true`）。
- 消息：成功 → toast，失败 → alert 弹窗。

### 菜单与路由

- 菜单：配置 → 用户与组（可折叠）→ 用户 / 用户组
- 前端路由：`/accounts/users`、`/accounts/groups`

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/accounts/users` | 系统用户列表（按 uid 排序，含附加组与锁定状态） |
| POST | `/api/accounts/users` | 创建用户（`pw useradd`） |
| PUT | `/api/accounts/users/{name}` | 修改用户（`pw usermod` / `pw lock` / `pw unlock`） |
| DELETE | `/api/accounts/users/{name}` | 删除用户（`?remove_home=true` 连带删除主目录） |
| GET | `/api/accounts/groups` | 系统组列表（按 gid 排序） |
| POST | `/api/accounts/groups` | 创建组（`pw groupadd`） |
| PUT | `/api/accounts/groups/{name}` | 修改组（`pw groupmod`） |
| DELETE | `/api/accounts/groups/{name}` | 删除组（`pw groupdel`） |
| GET | `/api/accounts/shells` | 合法 shell 列表（来自 /etc/shells） |

### 请求体示例

创建用户 `POST /api/accounts/users`：
```json
{ "name": "alice", "uid": 2001, "gid": "alice", "groups": ["wheel"],
  "gecos": "Alice", "home": "/home/alice", "shell": "/bin/sh",
  "password": "secret", "create_home": true }
```
修改用户 `PUT /api/accounts/users/alice`（字段均可选，`groups` 为整体替换）：
```json
{ "new_name": "alice2", "shell": "/usr/local/bin/bash",
  "groups": ["wheel","operator"], "password": "newsecret", "locked": true }
```
创建组 `POST /api/accounts/groups`：
```json
{ "name": "devs", "gid": 5000, "members": ["alice","bob"] }
```

## 外部依赖

- `/usr/sbin/pw`（FreeBSD base）—— 用户/组增删改
- 直接读取 `/etc/passwd`、`/etc/group`、`/etc/master.passwd`、`/etc/shells`
- 无额外 crate（复用 `regex`、`serde`）

## 已知限制

- 仅用 `/etc/passwd` 公开字段；`master.passwd` 的 class/expire 等字段未暴露（仅读取锁定状态）。
- 附加组成员列表仅来自 `/etc/group` 的成员字段，不含以该组为主组的用户。
- `pw usermod -G` / `groupmod -M` 为整体替换语义（编辑表单已据此预填并整体提交）。
