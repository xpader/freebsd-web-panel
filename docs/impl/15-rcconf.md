# 15 — RC 配置（rc.conf / sysrc）

## 概述

列出、新增、修改、删除 FreeBSD `rc.conf` 中的启动配置变量。读取操作通过 `src/sysrc.rs` 的直接文件解析（`read_rcconf_files`）完成，不 spawn 子进程；写入和删除通过 `/usr/sbin/sysrc` 完成（正确处理文件锁定与格式化）。前端提供表格视图，每项可单独编辑或删除，并带新增入口与键/值筛选。

## 实现细节

### 后端 `src/handlers/rcconf.rs`

用 `Command::new().arg()` 传参，禁止 shell 拼接。

#### 读（列表）

- 调用 `sysrc::list_all_async()`，底层调用 `read_rcconf_files()`（直接 `std::fs::read_to_string` 读取 rc.conf 文件，不经子进程）。
- 读取顺序匹配 FreeBSD `source_rc_confs()` 语义：`/etc/defaults/rc.conf` 解析 `rc_conf_files` → 依次读取 `/etc/rc.conf`、`/etc/rc.conf.local` → `/etc/rc.conf.d/*`。
- 只展示**非默认值**的变量（即用户在 rc.conf 文件里显式设置的，不含 `/etc/defaults/rc.conf` 的几百项默认）。
- 解析由 `sysrc` 模块内部的 `parse_export_line` / `unescape` 完成。
- 结果按 key 字母序排序。
- 延迟 <1ms（对比之前用 `sysrc -e -a` 子进程约 500ms）。

#### 写（新增/修改）

- 调用 `sysrc::set_async(key, value)`（底层：`sysrc KEY=VALUE`）。
- sysrc 默认写入 `rc_conf_files` 的第一个可写文件，即 `/etc/rc.conf`。
- 写入后用 `sysrc::get_async(key)` 回读实际生效值，作为响应返回（sysrc 可能对值做规范化）。
- 新增与修改是同一操作（sysrc 语义即 create-or-update），故共用 `PUT`。

#### 删除

- 调用 `sysrc::delete_async(key)`（底层：`sysrc -x KEY`），从 rc.conf 文件中移除该变量。

#### 输入校验

- `validate_key`：`^[a-zA-Z_][a-zA-Z0-9_]*$`，长度 1–128。
- `validate_value`：禁止 `\0` / `\n` / `\r`（防止破坏 rc.conf 单行结构）。
- 因使用 `Command::new().arg()` 而非 shell，value 内容无注入风险；校验仅保证文件完整性。

### 数据结构

```rust
struct RcVar { key: String, value: String }

struct SetRequest { key: String, value: String }   // PUT body
struct KeyQuery { key: String }                     // DELETE ?key=
```

### 前端 `frontend/src/pages/RcconfPage.vue`

- 表格视图（键 / 值 / 操作），工具栏含筛选输入框与「+ 添加变量」按钮。
- 全量加载后客户端筛选（按 key/value 实时过滤）。
- 操作均经 `useFormModal`/`useConfirm`/`useToast` composables。

### 菜单与路由

- 菜单：配置 → RC 配置（`nav.rcconf`）。
- 前端路由：`/rcconf`。
- 后端路由：`GET`/`PUT` `/api/rcconf`、`DELETE` `/api/rcconf?key=`。

## API

| 方法 | 路径 | 请求 | 响应 |
|---|---|---|---|
| GET | `/api/rcconf` | — | `[{key, value}, …]`（按 key 排序） |
| PUT | `/api/rcconf` | `{key, value}` | `200 {key, value}`（生效值） |
| DELETE | `/api/rcconf` | `?key=NAME` | `204` |

## 外部依赖

- `/usr/sbin/sysrc`（FreeBSD 自带，仅用于写/删）
- crate：`regex`（key 校验，复用现有依赖）
- 前端：`formModal`、`confirmDialog`、`toast`

## 配置项

- 无新增 `fwp.toml` 字段。

## 已知限制 / TODO

- 列表展示的是**合并后生效的非默认值**，覆盖 `/etc/rc.conf` 与 `/etc/rc.conf.local`，但不区分变量来自哪个文件；删除/写入默认作用于 `rc_conf_files`。
- value 经 sysrc 的 sh 解析：含字面双引号（`"`）的值会被 sysrc 当作引号处理（rc.conf 极少需要字面双引号，常见 YES/NO/路径/IP/flags 不受影响）。
- 不展示 `/etc/defaults/rc.conf` 的默认值（513 项），仅管理用户设置的项。
- 写操作由面板以 root 执行，无额外权限分级。
- `read_rcconf_files` 不执行 shell source（即不展开 `${var}` 引用），仅做文本级 `KEY="VALUE"` 解析。绝大多数 rc.conf 变量是字面值，不受影响。
