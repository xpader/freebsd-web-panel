# 10 — 文件管理器

## 概述

「存储」主菜单下的「文件管理器」（`/filesystem/files`）：一个完整的双栏 Web 文件管理器，以 root 身份浏览整个文件系统。

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

**上传** `upload`：请求体为原始文件字节（`application/octet-stream`），目标目录与文件名经 query 传递。上传路由在 `app.rs` 中单独拆出并加 `DefaultBodyLimit::disable()`，解除 axum 默认 2 MiB 请求体限制（否则大文件上传会被拒）。

流式写入 + 原子提交（避免大文件占满内存 + 中断时留下残文件；背景与模式总结见 [25-memory.md](25-memory.md)）：

1. 提取器用 `axum::body::Body`（不是 `Bytes`），不在聚合阶段就把整个 body 读进内存。
2. 在目标目录内创建临时文件 `.upload-tmp.<filename>`（与目标文件同目录、同 basename、固定前缀）。同一目标路径的并发上传会竞争同一临时文件，后完成者覆盖前者，但目标文件始终保持完整版本。
3. `body.into_data_stream()` 得到 chunk 流（hyper 默认约 64 KB/chunk），循环 `file.write_all(chunk).await`，累加 `total_size`，chunk drop 后即释放——峰值内存恒定 ≈ chunk 大小。
4. 写完后 `file.sync_all().await`：flush 用户态缓冲 + `fsync` 确保数据落到稳定存储。
5. `tokio::fs::rename(tmp, dest).await` 原子替换（POSIX rename 在同一文件系统上是原子的，目标路径只会观察到"旧文件"或"新文件"，无半新半旧窗口）。
6. 任何阶段失败（chunk 网络中断 / write 失败 / sync 失败 / rename 失败）都会 `tokio::fs::remove_file(tmp)` 清理临时文件，目标路径保持不变（原文件不被破坏）。

审计记录 `upload <path> (<size> bytes)` 仅在 rename 成功后写入。

**下载** `download`：`tokio::fs::File::open` + `tokio_util::io::ReaderStream` 包装为 `axum::body::Body::from_stream`，分块（默认约 8 KB/chunk）流出，**不把整个文件读进内存**。响应头：`Content-Type: application/octet-stream` + `Content-Disposition: attachment; filename="<name>"`。目录拒绝下载。

写入操作（mkdir/rename/delete/upload）均经 `crate::audit::record` 写审计日志。

### 前端 `web/js/pages/files.js`

- **状态**：`currentDir`（当前目录）、`viewMode`（list/grid，存 `localStorage`）、`expanded`（展开集合）、`treeChildren`（path→子目录数组，懒加载缓存）
- **目录树** `treeNodeHtml` 递归渲染；`toggleExpand` 首次展开时拉取子目录（仅目录）；`ensureAncestors` 在打开深层目录时加载并展开所有祖先
- **列表/网格**：`listHtml`（表格：名称/大小/权限/修改时间/操作）与 `gridHtml`（卡片网格，操作按钮悬停显示）。每项操作按钮：下载（仅文件）、重命名、属性、删除
- **非 JSON 传输**：上传/下载绕过 `api.js`（仅处理 JSON），直接用 `fetch` + `Authorization: Bearer` 头；下载用 `Blob` + 临时 `<a download>` 触发浏览器保存
- **重命名/新建**：`promptText` 自实现文本输入对话框（Enter 确认 / Esc 取消）；重命名在同一父目录下改名
- **刷新**：增删后 `invalidateTree` + `refreshTree` 重载祖先链子目录，保证目录树与列表一致

### 菜单集成

`menu.js`「存储」主菜单含：「概览」+「磁盘」+「文件管理器」+「ZFS」。`main.js` 注册 `/filesystem/files` → `renderFiles`。

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
| GET | `/api/files/accounts` | — | 系统用户/组列表（供 chown 下拉选择） |
| PUT | `/api/files/chmod` | `?path=`，body=`{mode}` | 修改权限（八进制 mode，含 setuid/sticky） |
| PUT | `/api/files/chown` | `?path=`，body=`{uid?, gid?}` | 修改所有者/组，未提供的字段保持不变 |

### chmod / chown

**chmod** `chmod`：请求体 `{mode: u32}`（八进制，经 `& 0o7777` 截断）。通过 `fchmodat(AT_FDCWD, path, mode, AT_SYMLINK_NOFOLLOW)` FFI 调用——不跟随符号链接，直接修改链接自身的权限（等价 `chmod -h`）。std::fs 不提供 lchmod，故用裸 FFI 声明 `extern "C" fn fchmodat`。

**chown** `chown`：请求体 `{uid?: u32, gid?: u32}`。通过 `lchown(path, uid, gid)` FFI 调用——不跟随符号链接。未提供的字段从当前 `symlink_metadata` 读取保持不变。

**accounts** `accounts`：解析 `/etc/passwd`（去重 UID）和 `/etc/group`（去重 GID），返回 `{users: [{name,id}], groups: [{name,id}]}`，按名称排序。供前端 chown 下拉框使用。

## 外部依赖

- chmod/chown 通过裸 FFI（`fchmodat` + `lchown`，FreeBSD libc），无系统命令调用
- 上传/下载使用 `tokio::fs::File` + `tokio_util::io::ReaderStream`（`tokio-util = "0.7"`，io feature）做流式 I/O
- 其余纯 `std::fs` + `std::os::unix::fs::MetadataExt`
- 前端无第三方库

## 已知限制 / TODO

- 上传临时文件用 `.` 开头（常规 `ls` 隐藏）；服务被 `SIGKILL` 强杀时可能留下 `.upload-tmp.*` 残留，启动时未自动清理
- 上传默认无大小限制（已禁用 axum 2 MiB 限制）
- 所有者/组解析自 `/etc/passwd`、`/etc/group`（首次访问 `LazyLock` 缓存），非系统 UID/GID 回退为数字
- chmod/chown 不递归（不支持 `-R`）
- 无分页；目录条目极多时全量返回
- 无文件内容预览 / 编辑、无打包下载、无递归上传
- 无配额 / 可访问根限制，可访问整个文件系统（依赖认证保护）
