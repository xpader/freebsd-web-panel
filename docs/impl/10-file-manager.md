# 10 — 文件管理器

## 概述

「文件系统」主菜单下的「文件管理器」（`/filesystem/files`）：一个完整的双栏 Web 文件管理器，以 root 身份浏览整个文件系统。

功能：

- **左栏目录树**：懒加载的目录导航，点击 ▸/▾ 展开/折叠，点击目录名切换右侧内容
- **右侧文件列表**：当前目录的子目录与文件，目录在前、按名称排序
- **列表 / 网格视图切换**：☰ 列表（表格）与 ▦ 网格（卡片），切换记忆在 `localStorage`
- **上传**（多文件）、**下载**、**新建文件夹**、**重命名**、**删除**（目录递归）、**属性查看**
- **路径面包屑**：点击任意层级直接跳转

## 安全模型

- 所有路径必须是绝对路径，经词法规范化（`.` / `..` 被解析，`..` 在根处被钳制，**无法逃逸 `/`**）；含 NUL / 换行直接拒绝
- 文件名组件额外校验：非空、≤255 字节、不能是 `.`/`..`、不含 `/` 与 NUL
- 所有端点位于 `require_auth` 中间件之后，需有效 session token
- 以 root 运行，可访问整个文件系统（系统管理面板的预期行为）

## 实现细节

### 后端 `src/handlers/files.rs`

**路径规范化** `normalize(raw)`：遍历 `Path::components()`，丢弃 `RootDir`/`CurDir`，`Normal` 压栈、`ParentDir` 弹栈（钳制在根），重组为绝对路径。非绝对路径 / 含 NUL 换行返回 `BadRequest`。

**权限字符串** `perm_string(type_ch, mode)`：生成 10 字符 `ls` 风格串，含前导类型符（`d`/`l`/`c`/`b`/`p`/`s`/`-`）与 setuid/setgid/sticky 位（`s`/`S`/`t`/`T`）。类型符由 `type_char(file_type, mode)` 根据 `st_mode` 的 `0o170000` 掩码确定。

**目录列表** `list`：`symlink_metadata` 取每个条目（符号链接取链接自身类型），用 `MetadataExt` 取 size/uid/gid/mtime/mode。读取失败的条目被跳过（不致因单个不可读项导致整列失败）。排序：目录在前，组内按名称（小写）排序。

**属性** `stat`：返回完整元数据——路径、父目录、类型、符号链接目标（`read_link`）、大小、mtime/atime/ctime、mode/权限串、uid/gid、nlink、inode、blocks、blksize。

**上传** `upload`：请求体为原始文件字节（`application/octet-stream`），目标目录与文件名经 query 传递，`std::fs::write` 落盘。上传路由在 `app.rs` 中单独拆出并加 `DefaultBodyLimit::disable()`，解除 axum 默认 2 MiB 请求体限制（否则大文件上传会被拒）。

**下载** `download`：`std::fs::read` 读入内存后返回 `Body`，设 `Content-Type: application/octet-stream` + `Content-Disposition: attachment`。目录拒绝下载。

写入操作（mkdir/rename/delete/upload）均经 `crate::audit::record` 写审计日志。

### 前端 `web/js/pages/files.js`

- **状态**：`currentDir`（当前目录）、`viewMode`（list/grid，存 `localStorage`）、`expanded`（展开集合）、`treeChildren`（path→子目录数组，懒加载缓存）
- **目录树** `treeNodeHtml` 递归渲染；`toggleExpand` 首次展开时拉取子目录（仅目录）；`ensureAncestors` 在打开深层目录时加载并展开所有祖先
- **列表/网格**：`listHtml`（表格：名称/大小/权限/修改时间/操作）与 `gridHtml`（卡片网格，操作按钮悬停显示）。每项操作按钮：下载（仅文件）、重命名、属性、删除
- **非 JSON 传输**：上传/下载绕过 `api.js`（仅处理 JSON），直接用 `fetch` + `Authorization: Bearer` 头；下载用 `Blob` + 临时 `<a download>` 触发浏览器保存
- **重命名/新建**：`promptText` 自实现文本输入对话框（Enter 确认 / Esc 取消）；重命名在同一父目录下改名
- **刷新**：增删后 `invalidateTree` + `refreshTree` 重载祖先链子目录，保证目录树与列表一致

### 菜单集成

`layout.js`「文件系统」主菜单含：「概览」+「磁盘」+「文件管理器」+「ZFS」。`main.js` 注册 `/filesystem/files` → `renderFiles`。

## API

路径均以 query 参数传递（文件路径含 `/`，无法用路径参数）。

| 方法 | 路径 | 参数 | 说明 |
|---|---|---|---|
| GET | `/api/files/list` | `?path=` | 目录内容列表（目录在前） |
| GET | `/api/files/stat` | `?path=` | 文件/目录详细属性 |
| POST | `/api/files/mkdir` | `?path=` | 创建目录，已存在返回 409 |
| POST | `/api/files/rename` | `?from=&to=` | 重命名/移动，目标存在返回 409 |
| DELETE | `/api/files` | `?path=` | 删除文件（文件 / 递归目录） |
| POST | `/api/files/upload` | `?path=&filename=`，body=原始字节 | 上传文件 |
| GET | `/api/files/download` | `?path=` | 下载文件（目录返回 400） |

## 外部依赖

- 无系统命令调用，纯 `std::fs` + `std::os::unix::fs::MetadataExt`
- 前端无第三方库

## 已知限制 / TODO

- 上传/下载均将整个文件读入内存（超大文件会占内存），未做流式传输；上传默认无大小限制（已禁用 axum 2 MiB 限制）
- 所有者仅显示 UID/GID 数值，未解析为用户/组名（需 getpwuid/getgrgid FFI）
- 无分页；目录条目极多时全量返回
- 无文件内容预览 / 编辑、无打包下载、无递归上传
- 无配额 / 可访问根限制，可访问整个文件系统（依赖认证保护）
