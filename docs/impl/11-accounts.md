# 11 — 系统用户与用户组

## 概述

查看 FreeBSD 系统用户与用户组，通过直接解析 `/etc/passwd` 和 `/etc/group` 实现。目前为只读列表展示（含客户端筛选），后续可扩展增删改、`pw` 命令操作等。

## 实现细节

### 后端 `src/handlers/accounts.rs`

直接读取 `/etc/passwd` 和 `/etc/group` 文本文件并逐行解析（面板以 root 运行，有读取权限）。

#### `/etc/passwd` 格式（7 字段）

```
name:passwd:uid:gid:gecos:home:shell
```

- 跳过空行和 `#` 注释行
- `splitn(7, ':')` 分割
- 用户列表按 uid 升序排列

#### `/etc/group` 格式（4 字段）

```
group_name:passwd:gid:member_list
```

- `member_list` 为逗号分隔的用户名，可能为空
- 组列表按 gid 升序排列

#### 主组名解析

`read_group_map()` 构建 `gid → group_name` 的 HashMap，在返回用户列表时为每个用户附加 `group_name` 字段（主组名），避免前端再查一次。

### 数据结构

```rust
struct SystemUser {
    name: String,
    uid: u32,
    gid: u32,
    gecos: String,
    home: String,
    shell: String,
    group_name: Option<String>,  // 通过 /etc/group 解析的主组名
}

struct SystemGroup {
    name: String,
    gid: u32,
    members: Vec<String>,
}
```

### 前端 `web/js/pages/accounts.js`

两个页面函数：

- `renderSysUsers` — 系统用户表格（用户名 / UID / 主组 / 描述 / 家目录 / Shell）
- `renderSysGroups` — 系统用户组表格（组名 / GID / 成员标签列表）

均带客户端筛选输入框（按用户名/UID、组名/GID/成员实时过滤）。

### 菜单与路由

- 菜单：配置 → 用户（可折叠组）→ 用户 / 用户组（三级菜单，与 ZFS 同模式）
- 前端路由：`/accounts/users`、`/accounts/groups`

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/accounts/users` | 系统用户列表（按 uid 排序） |
| GET | `/api/accounts/groups` | 系统用户组列表（按 gid 排序） |

## 外部依赖

- 无外部命令；直接文件读取
- 无额外 crate

## 已知限制 / TODO

- 只读：目前不支持增删改系统用户/组
- 未使用 `/etc/master.passwd`（有更多字段如 class/expire），仅用公开的 `/etc/passwd`
- 成员列表仅来自 `/etc/group` 的附加成员，不含以该组为主组的用户
- 后续可集成 `pw` 命令实现用户/组管理
