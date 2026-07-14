# 15 — RC 配置（rc.conf / sysrc）

## 概述

列出、新增、修改、删除 FreeBSD `rc.conf` 中的启动配置变量。所有操作通过 `src/sysrc.rs` 统一封装模块完成（详见 [26-sysrc.md](26-sysrc.md)），底层调用系统自带的 `/usr/sbin/sysrc`，不直接拼接或写文件。前端提供表格视图，每项可单独编辑或删除，并带新增入口与键/值筛选。

## 实现细节

### 后端 `src/handlers/rcconf.rs`

全部走 `crate::sysrc` 异步 API，用 `Command::new().arg()` 传参，禁止 shell 拼接。

#### 读（列表）

- 调用 `sysrc::list_all_async()`（底层：`sysrc -e -a`）
  - `-a`（小写）：只列出**非默认值**的变量（即用户在 rc.conf 文件里显式设置的，不含 `/etc/defaults/rc.conf` 的几百项默认）。
  - `-e`：导出格式 `KEY="VALUE"`，便于稳定解析。
- 解析由 `sysrc` 模块内部的 `parse_export_lines` / `parse_export_line` / `unescape` 完成。
- 结果按 key 字母序排序。

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

### 前端 `web/js/pages/rcconf.js`

- `renderRcconf` 渲染表格（键 / 值 / 操作），工具栏含筛选输入框与「+ 添加变量」按钮。
- 全量加载后客户端筛选（按 key/value 实时过滤）。
- 操作均经 `window.__fwpRc*` 全局句柄：
  - `__fwpRcAdd` — `formModal`（变量名 + 值）→ `PUT`。
  - `__fwpRcEdit(key)` — `formModal`（仅值，key 固定）→ `PUT`。
  - `__fwpRcDel(key)` — `confirmDialog` → `DELETE`。
- HTML 文本输出经 `esc()` 转义；按钮 `onclick` 属性内传参经 `escAttr()` 转义（key 含引号/尖括号也安全）。

### 菜单与路由

- 菜单：配置 → RC 配置（`nav.rcconf`）。
- 前端路由：`/rcconf`（替换原 `makePlannedPage` 占位）。
- 后端路由：`GET`/`PUT` `/api/rcconf`、`DELETE` `/api/rcconf?key=`（替换原 `mod_stubs::rcconf`，并移除该 stub）。

## API

| 方法 | 路径 | 请求 | 响应 |
|---|---|---|---|
| GET | `/api/rcconf` | — | `[{key, value}, …]`（按 key 排序） |
| PUT | `/api/rcconf` | `{key, value}` | `200 {key, value}`（生效值） |
| DELETE | `/api/rcconf` | `?key=NAME` | `204` |

## 外部依赖

- `/usr/sbin/sysrc`（FreeBSD 自带）
- crate：`regex`（key 校验，复用现有依赖）
- 前端：`formModal`、`confirmDialog`、`toast`

## 配置项

- 无新增 `fwp.toml` 字段。

## 已知限制 / TODO

- `-a` 列出的是**合并后生效的非默认值**，覆盖 `/etc/rc.conf` 与 `/etc/rc.conf.local`，但不区分变量来自哪个文件；删除/写入默认作用于 `rc_conf_files`。
- value 经 sysrc 的 sh 解析：含字面双引号（`"`）的值会被 sysrc 当作引号处理（rc.conf 极少需要字面双引号，常见 YES/NO/路径/IP/flags 不受影响）。
- 不展示 `/etc/defaults/rc.conf` 的默认值（513 项），仅管理用户设置的项。
- 写操作由面板以 root 执行，无额外权限分级。
