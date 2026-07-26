# pkg 软件包管理

## 概述

查看系统中通过 `pkg` 安装的所有软件包，包括手动安装包（prime-list）、自动安装包（依赖），以及各个包的详细信息：描述、依赖关系（依赖于什么 / 被什么依赖）、文件列表。支持搜索远程仓库中的可用包，以及安装和删除操作。

## 设计决策

### 使用命令行还是系统 API？

**选择：通过 `Command::new("/usr/sbin/pkg")` 调用 `pkg query` / `pkg info` / `pkg rquery` / `pkg install` / `pkg delete` 命令。**

理由：
1. **与项目约定一致** — AGENTS.md 明确："FreeBSD 管理通过 spawn 系统二进制并传校验过的参数完成"。services、rcconf、crontab 等模块均遵循此模式。
2. **`pkg query` 格式字符串精确控制输出** — 支持 `%n`（名称）、`%v`（版本）、`%dn`（依赖名）等格式符，输出 TSV，解析简单可靠。
3. **`pkg info -R --raw-format json-compact`** 一次获取全部结构化数据（描述、依赖、分类、许可证），天然处理多行字段。
4. **避免 libpkg FFI 的复杂性** — libpkg 不是系统共享库（`/usr/local/lib/libpkg.so` 属于 pkg 自身），引入 FFI 依赖得不偿失。详见 AGENTS.md "系统命令模式" 中的判定准则。

### 模块位置

放在 **Config（配置）** 导航组中，位于 Services 之后。包管理属于系统配置范畴，与服务管理、系统账户同类。

## 实现细节

所有源码位于 `src/handlers/pkg.rs`。

### 数据结构

```rust
// 列表项摘要
PackageSummary { name, version, origin, comment, automatic, size, homepage, maintainer, install_timestamp }

// 详情（含依赖关系）
PackageDetail { name, version, origin, prefix, comment, description, homepage, maintainer,
                automatic, locked, vital, size_bytes, arch, abi, repository,
                install_timestamp, categories, licenses, license_logic,
                dependencies, reverse_dependencies, messages }

DepInfo { name, version }                 // 依赖项（名称 + 版本）
PackageFile { path, owner, group, mode }   // 文件列表项
SearchResult { name, version, origin, comment, size }  // 远程搜索结果

// 后台任务
PkgTask { id, action, packages, status, exit_code, lines, created_at }
TaskStatus: Running | Done | Failed
```

### `pkg` 调用链

#### 1. 列表 `list_packages`

```
pkg query '%n\t%v\t%o\t%c\t%a\t%sh\t%w\t%m\t%t'           # 全部
pkg query -e '%a = 0' '%n\t%v\t%o\t%c\t%a\t%sh\t%w\t%m\t%t'  # 手动（prime-list）
pkg query -e '%a = 1' '%n\t%v\t%o\t%c\t%a\t%sh\t%w\t%m\t%t'  # 自动
```

通过 `?filter=manual|automatic|all` 查询参数控制。

#### 2. 详情 `package_detail`

共 3 次调用：

| 调用 | 命令 | 用途 |
|------|------|------|
| 主信息 | `pkg info -R --raw-format json-compact {name}` | 一次获取全部结构化数据：name/version/origin/prefix/comment/desc/maintainer/www/abi/arch/flatsize/timestamp/licenses/categories/deps/messages |
| 补充字段 | `pkg query '%a\t%k\t%V\t%R' {name}` | raw manifest 不含的 automatic/locked/vital/repository |
| 反向依赖 | `pkg query '%rn\t%rv' {name}` | 被哪些包依赖（raw manifest 不含） |

`serde_json` 反序列化为 `RawManifest` 结构体，字段映射直接。

#### 3. 文件列表 `package_files`

```
pkg query '%Fp\t%Fu\t%Fg\t%Fm' {name}
```

使用 `splitn(4, '\t')` 防止路径中的特殊字符导致解析问题。

#### 4. 搜索 `search`

```
pkg rquery -g '%n\t%v\t%o\t%c\t%sh' '*{pattern}*'
```

`pkg rquery` 查询远程仓库（对应 `pkg query` 查询本地）。`-g` 使用 glob 匹配，`*pattern*` 实现子串搜索。最多返回 100 条结果。

#### 5. 安装 / 删除 `install` / `delete`

安装和删除是长时间操作（下载、解包），采用**通用后台任务（`bgtask` 模块）+ SSE 流式输出**模式：

1. `POST /api/pkg/install` 或 `/api/pkg/delete` → `bgtask::create()` 创建任务，`tokio::spawn` 后台执行
2. 后台任务调用 `bgtask::run_streaming_cmd()` spawn `pkg install -y {packages}` 或 `pkg delete -y [-R] {packages}`，stdout/stderr 逐行推入共享任务存储
3. 前端通过统一 SSE 端点 `GET /api/tasks/{id}/stream` 实时接收增量输出 + 状态
4. 子进程退出后设置 `TaskStatus::Done`（exit 0）或 `Failed`
5. 操作完成自动记录审计日志

`bgtask` 模块（`src/bgtask.rs`）提供通用的后台任务存储、`run_streaming_cmd()` 辅助函数和统一 SSE handler，pkg install/delete、pkg update、bhyve init 等长任务共用同一套基础设施。任务 10 分钟后自动 GC。详见 `docs/impl/30-bgtask.md`。

### 输入验证

- 包名验证正则 `^[a-zA-Z0-9_+.{}@-]+$`（允许 FreeBSD ports 中的合法包名字符），最大 256 字符。
- 搜索模式验证正则 `^[a-zA-Z0-9_+.{}@*?-]+$`（额外允许 glob 元字符）。
- 始终用 `.arg()` 传参，不拼接 shell。

### 前端

`web/js/pages/pkg.js` — 四个功能模块：

1. **列表页** (`/pkg`)：filter-group 切换（全部/手动/自动）+ 本地筛选 + 表格。每行有删除按钮。工具栏的"安装"按钮打开搜索弹窗。
2. **搜索弹窗**：输入关键词 → 350ms 防抖 → 调用 `/api/pkg/search` → 显示结果表格。已安装的包显示"已安装"标签，未安装的显示"安装"按钮。
3. **任务输出弹窗**：安装/删除时弹出，monospace 输出区域实时滚动显示 `pkg` 的 stdout/stderr，自动跟随到底部。完成后启用关闭按钮并 toast 结果。
4. **详情页** (`/pkg/{name}`)：三个 tab：
   - **Info** — kv-table 展示基本信息 + 描述 + messages（底部）
   - **Dependencies** — 双栏：Depends On / Required By，点击包名可跳转
   - **Files** — 文件列表（延迟加载，切 tab 时才请求 API）

## API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/pkg/packages` | 列出已安装包。`?filter=manual\|automatic\|all` |
| GET | `/api/pkg/packages/{name}` | 包详情（含依赖 + 反向依赖 + messages） |
| GET | `/api/pkg/packages/{name}/files` | 包文件列表 |
| GET | `/api/pkg/search?q={pattern}` | 搜索远程仓库中的包（glob 子串匹配） |
| POST | `/api/pkg/install` | 安装包。body: `{"packages": ["vim"]}` → `{"task_id": "..."}` |
| POST | `/api/pkg/delete` | 删除包。body: `{"packages": ["vim"], "recursive": false}` → `{"task_id": "..."}` |
| GET | `/api/pkg/tasks/{id}` | 查询后台任务状态（轮询） |

## 外部依赖

- 系统命令：`/usr/sbin/pkg`（`pkg query`、`pkg info -R`、`pkg rquery`、`pkg install`、`pkg delete`）
- Rust crate：`tokio::process`（异步子进程 + 行读取）、`parking_lot`（任务存储 mutex）、`serde_json`（反序列化 raw manifest）

## 已知限制 / TODO

- **无 `pkg upgrade`**：不支持批量升级已安装的包。
- **无 `pkg autoremove`**：不支持清理不再被需要的自动安装包。
- **无 `pkg lock` / `unlock`**：不支持锁定/解锁包。
- **无更新检查**：不显示 `pkg version` 的可用更新信息。
- **无 audit**：不集成 `pkg audit` 安全漏洞检查。
- **文件列表无分页**：对于文件数极多的包（如 python），一次性返回全部文件。
- **任务无持久化**：服务重启后后台任务状态丢失（仅在内存中）。
