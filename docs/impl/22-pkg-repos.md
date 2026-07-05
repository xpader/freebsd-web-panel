# 22 — PKG 软件源配置管理

> 对应计划文档：`docs/plan/22-pkg-repos.md`

## 概述

通过 Web 面板管理 FreeBSD pkg 软件包仓库源配置。支持查看所有仓库（系统默认 + 用户自定义）、添加/编辑/删除自定义源、一键启用/禁用源、以及执行 `pkg update -f` 刷新目录。

## 设计决策

### 1. 按文件分组，而非每仓库一个文件

与系统配置（`/etc/pkg/FreeBSD.conf` 含多个 repo）保持一致。面板管理时，用户指定目标文件名，多个 repo 可以共存于同一文件中。添加/修改/删除操作在文件内部进行，不产生文件碎片。

### 2. 自研 UCL 解析器（行式两阶段）

不使用 `pkg -vv`（输出格式不稳定）也不引入 C 依赖。采用两阶段行式解析器：

1. **Phase 1**：按行扫描，用花括号深度跟踪将文件拆分为 `name → body` 块
2. **Phase 2**：对每个块解析 `key: value` 对，支持引号/无引号/注释

### 3. 同名文件合并

当用户目录 `/usr/local/etc/pkg/repos/FreeBSD.conf` 与系统 `/etc/pkg/FreeBSD.conf` 同名时，两者**合并为一个文件展示**（标记为 Custom）。合并逻辑：

- 用户文件中的 repo 按名称覆盖系统文件中的同名 repo
- 未被覆盖的系统 repo 仍保留展示（其 `is_system_origin: true`）
- 被覆盖的 repo 通过 `merge_repo()` 字段级合并：以系统 repo 为 base，仅叠加用户覆盖中与解析默认值不同的字段
- 合并后的 `path` 指向用户文件（编辑操作写入此处）

前端根据 `is_system_origin` 标志：
- 禁止修改仓库名（readonly）
- 禁止删除（仅可禁用）

### 4. 最小差异写入

覆盖文件只写入与系统原始配置不同的字段（`render_repo_block_diff`）。例如系统 repo 有 `url/mirror_type/signature_type/fingerprints/enabled`，用户只改了 `enabled`，覆盖文件只有：

```ucl
# Managed by FreeBSD Web Panel

FreeBSD-ports: {
  enabled: no;
}
```

pkg 自动继承系统文件中的其他字段。当所有差异被还原（覆盖为空）时，自动删除覆盖文件。

### 5. 原子写入

写入临时文件 → `rename`，避免写入中断导致配置损坏。

## 数据结构

```rust
/// 一个仓库的配置字段。
#[derive(Debug, Clone, Serialize)]
struct RepoConfig {
    name: String,
    url: String,
    enabled: bool,
    mirror_type: String,       // NONE | HTTP | SRV
    signature_type: String,    // NONE | PUBKEY | FINGERPRINTS
    fingerprints: Option<String>,
    pubkey: Option<String>,
    priority: i64,
    ip_version: i64,
    is_system_origin: bool,    // 该 repo 来自系统文件（未被用户覆盖）
}

/// 按文件分组的仓库列表。
struct RepoFile {
    path: String,              // 完整路径，如 /etc/pkg/FreeBSD.conf
    filename: String,          // 文件名，如 FreeBSD.conf
    is_system: bool,           // 仅系统文件（无用户同名覆盖）= true
    repos: Vec<RepoConfig>,    // 该文件中所有仓库（合并后）
}

/// 创建仓库的请求体（含目标文件名）。
struct CreateRepoInput {
    filename: String,          // 如 "FreeBSD.conf" 或 "myrepo.conf"
    repo: RepoInput,           // 仓库字段（name, url, enabled, ...）
}

/// 修改仓库的请求体（含来源文件路径）。
struct UpdateRepoRequest {
    file: String,              // 来源文件完整路径
    url: String,
    enabled: bool,
    mirror_type: Option<String>,
    signature_type: Option<String>,
    fingerprints: Option<String>,
    pubkey: Option<String>,
    priority: Option<i64>,
    ip_version: Option<i64>,
}
```

## API

| 方法 | 路径 | 认证 | 说明 |
|---|---|---|---|
| GET | `/api/pkg/repos` | 是 | 返回 `Vec<RepoFile>`，按文件分组列出所有仓库（合并后） |
| POST | `/api/pkg/repos` | 是 | 向指定文件添加仓库（body: `{ filename, name, url, ... }`） |
| PUT | `/api/pkg/repos/{name}` | 是 | 修改仓库（body: `{ file, url, enabled, ... }`，file 为来源路径） |
| DELETE | `/api/pkg/repos/{name}?file=<path>` | 是 | 从文件中移除仓库（仅用户文件中的非系统来源仓库） |
| POST | `/api/pkg/repos/update` | 是 | 执行 `pkg update -f`（后台任务，返回 task_id） |

### 文件级操作逻辑

- **添加**：指定目标文件名（如 `FreeBSD.conf`），仓库追加到 `/usr/local/etc/pkg/repos/{filename}`。如果文件已含其他仓库，保留原有仓库，仅追加新块。
- **修改**：body 中 `file` 指明来源文件。系统文件 → `resolve_target_file()` 自动重定向到用户目录同名覆盖文件（pkg 原生覆盖机制）。写入时通过 `render_repo_block_diff()` 只写差异字段。
- **删除**：仅允许删除非 `is_system_origin` 的仓库。删除后如果覆盖文件中无有效 repo，自动删除空文件。
- **原子写入**：每次写入先写 `.tmp` 再 `rename`，保持文件完整性。

### 写入函数链

```
update_repo / create_repo / delete_repo
  → parse_repo_file()       // 读取目标用户文件已有 repos
  → render_repo_block_diff() // 逐字段对比系统原始值，只输出差异
  → write_override_file()    // 渲染 + 原子写入；全空则删除文件
```

### 读取函数链

```
list_repos
  → read_all_repo_files()
    → parse_repo_file()      // 分别解析系统/用户目录的 .conf
    → merge_repo()           // 同名 repo 字段级合并（系统 base + 用户 overlay）
```

## 输入校验

- 仓库名：`^[a-zA-Z0-9_-]+$`，长度 1-128
- 文件名：`^[a-zA-Z0-9_.-]+\.conf$`，禁止路径穿越（`/`, `\`, `..`）
- URL：必须以合法 scheme 开头（`pkg+http`, `pkg+https`, `http`, `https`, `file`, `ssh`, `tcp`）
- mirror_type：`NONE` | `HTTP` | `SRV`
- signature_type：`NONE` | `PUBKEY` | `FINGERPRINTS`

## 前端

### 页面结构（`/pkg/repos`）

- 按**文件分组**展示：每个 `.conf` 文件为一个 card，header 显示文件名、路径、来源标记（System/Custom）、快捷「添加源」按钮
- 文件内的仓库表格：名称（系统来源 repo 显示 System badge）、URL、启用状态（badge）、优先级
- 操作列：启用/禁用 toggle、编辑、删除（仅非系统来源 repo）
- 顶部：「添加源」+「更新目录」按钮

### 添加/编辑模态框

- **目标文件选择**（仅添加模式）：下拉选择已有用户文件，或输入新文件名；从文件卡片快捷添加时自动预选
- 预设模板按钮（仅添加模式）：FreeBSD 官方 latest/quarterly、中科大（ustc.conf）、清华（tuna.conf）——一键填充名称、URL、文件名
- 表单使用 div 布局（`.repo-form`），每个字段：标签（130px）+ 输入区，部分字段下方有灰色提示说明
- 签名类型联动显示 fingerprints/pubkey 字段
- 系统来源 repo 编辑时仓库名为 readonly

### 更新目录模态框

- SSE 实时输出（复用 pkg 任务流 `/api/pkg/tasks/{id}/stream`）
- 完成后显示成功/失败

### 导航

两级菜单（`web/js/ui/layout.js`）：
- 软件包（`fa-box`）
  - 包列表（`/pkg`）
  - 源配置（`/pkg/repos`）

## 文件清单

| 文件 | 说明 |
|---|---|
| `src/handlers/pkg.rs` | 后端：repo CRUD + update handler + UCL 解析器 + 差异写入 |
| `src/app.rs` | 路由注册 |
| `web/js/pages/pkg-repos.js` | 前端：源配置页面 |
| `web/css/app.css` | `.repo-form` 表单布局样式 |
| `web/js/main.js` | 路由注册 |
| `web/js/ui/layout.js` | 两级菜单 |
| `web/js/i18n/translations.js` | i18n（en + zh） |

## 已知限制

- 不管理 `/usr/local/etc/pkg.conf` 全局配置（REPOS_DIR、ABI 等）
- 不管理 fingerprints/trusted 密钥文件内容
- 不支持 `env` 字段的编辑
- `pkg update -f` 需要网络访问
- 后台任务不持久化（重启丢失）
