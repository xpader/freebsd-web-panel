# AGENTS.md

AI 编码代理在 FreeBSD Web Panel（`fwp`）项目上工作时的指引。

## 项目

一个基于 Web 的 FreeBSD 系统管理面板。管理 sysctl、rc.conf、网络、服务、PF 防火墙、Jail 容器（通过原生 libjail FFI，不依赖任何第三方 jail 工具）、Bhyve 虚拟机（通过 vm-bhyve）、ZFS 文件系统。自带用户体系（非系统用户），HTTP API + SPA 前端，单二进制部署。

目标平台：FreeBSD 15.x amd64。以 root 运行（系统管理需要）。

## 技术栈

- **后端**：Rust 2021 edition（MSRV 1.74）、Axum 0.8、tokio、rusqlite（bundled SQLite）、argon2（密码哈希）、rust-embed（通过 `embed-web` 默认 feature 将 Web 资源嵌入二进制）。
- **前端**：Vue 3（Composition API + `<script setup>`）+ Vite 构建。Vue Router（hash 路由）、Pinia（状态管理）、vue-i18n（国际化）。手写深色主题 CSS 保持不变。Vite 构建输出到 `web/`，由 rust-embed 内嵌。
- **配置**：TOML 格式，位于 `/usr/local/etc/fwp.toml`（首次运行自动生成默认配置）。数据位于 `/var/db/fwp/`。

## 构建与运行

```sh
# 前端构建（首次或前端改动后）
cd frontend && npm install && npm run build   # 输出到 ../web/

# 后端构建
cargo build                  # debug 构建
cargo build --release        # release 构建（LTO、strip）
cargo run -- --config /path/to/fwp.toml   # 用指定配置运行
cargo run -- --config fwp.toml            # 开发配置（见下）

# 前端开发模式（热更新，代理 /api 到后端）
cd frontend && npm run dev   # Vite dev server on :5173
```

### 开发配置（本地测试用）

```toml
# fwp.toml — web_root 指向仓库目录，以便实时反映文件改动
[server]
listen = "127.0.0.1:18080"
web_root = "web"

[paths]
db = "/tmp/fwp-test/fwp.db"
audit = "/tmp/fwp-test/audit.log"

[auth]
session_ttl = 28800
```

服务器解析静态资源时先尝试磁盘 `web_root`，再回退到内嵌资源。开发时可用 `npm run dev` 启动 Vite 开发服务器（热更新），或构建后用 `web_root` 指向 `web/`。生产环境内嵌资源可从任意工作目录运行。

### 前端检查

```sh
cd frontend && npm run build   # Vite 构建（含类型检查 + 打包）
```

前端源码在 `frontend/src/`，使用 Vue 3 SFC（`.vue` 文件）。详见 `docs/impl/06-frontend.md`。

## 代码结构

```
src/
├── main.rs           # 入口：CLI（clap）、配置加载、db/audit 打开、服务绑定
├── state.rs          # AppState（共享状态，抽出独立模块以避免循环依赖）
├── app.rs            # 路由组装 + 回退到 web_assets
├── config.rs         # Config 结构体 + TOML 加载/创建
├── error.rs          # ApiError → HTTP 响应；ApiResult<T> = Result<T, ApiError>
├── db.rs             # SQLite 打开 + 版本化迁移系统 + 自由函数（user_count、get_user 等）
├── auth.rs           # 密码哈希（argon2）、session token、require_auth 中间件、
│                     # AuthUser 提取器（FromRequestParts）
├── audit.rs          # 追加式 JSON 审计日志（parking_lot::Mutex<File>）
├── monitor.rs        # 监控采集器（后台 tokio 任务）+ 时序查询 API
├── web_assets.rs     # rust-embed + 磁盘回退的资源 handler
└── handlers/
    ├── auth.rs       # login / logout / me
    ├── users.rs      # 用户 CRUD + bootstrap（首启创建管理员）
    ├── system.rs     # 系统信息 + 实时指标（CPU/内存/温度，通过 sysctl）
    ├── audit.rs      # 审计日志读取
    └── mod_stubs.rs  # 未实现模块的占位 handler（返回 "planned"）

frontend/                   # Vue 3 + Vite 前端源码
├── src/
│   ├── main.js           # Vue 应用启动（Pinia + Router + i18n）
│   ├── App.vue           # 根组件
│   ├── assets/app.css    # 全部样式（深色主题，迁移自原项目）
│   ├── lib/              # API 客户端、格式化工具、菜单配置、Chart.js 工具
│   ├── i18n/             # vue-i18n 初始化 + 翻译资源
│   ├── router/           # Vue Router 配置 + 认证守卫
│   ├── stores/           # Pinia stores（auth、ui dialogs）
│   ├── composables/      # useToast/useConfirm/useAlert/useFormModal
│   ├── components/       # 布局组件（TopBar、SideBar）+ UI 组件（Toast、Dialog）
│   └── pages/            # 各功能页面（35 个 .vue 组件）
├── public/               # 静态资源（img、fontawesome）
└── vite.config.js        # Vite 配置（outDir=../web）

web/                       # Vite 构建输出（rust-embed 内嵌目标）
├── index.html            # 自动生成
├── assets/               # 打包后的 JS/CSS

docs/plan/                # 设计计划文档（功能要做什么）
docs/impl/                # 实现文档（功能怎么做的，开发/变更时必须维护）
rc.d/fwp                  # FreeBSD rc.d 启动脚本

## 编码约定

### Rust

- **错误处理**：`ApiError`（thiserror）→ `IntoResponse` 映射到 HTTP 状态码。Handler 返回 `ApiResult<T>`，其中 `T: IntoResponse`。不要对可失败操作 `unwrap()`——通过 `?` 传播为 `ApiError`。
- **数据库访问**：`db.rs` 中的 SQLite 自由函数接收 `&Connection`。`Db` 类型为 `Arc<tokio::sync::Mutex<Connection>>`（异步 mutex——guard 在 handler 的 `.await` 间持有）。用 `state.db.lock().await` 加锁，调用自由函数：`db::get_user(&conn, id)`。
- **Mutex 选择**：不跨 `.await` 的同步代码用 `parking_lot::Mutex`（如 `LAST_CP_TIMES`、审计日志文件）。仅当 guard 必须在 `.await` 间存活时才用 `tokio::sync::Mutex`。禁止用 `std::sync::Mutex`。
- **静态变量**：初始化在编译期已知的用 `std::sync::LazyLock`（不用 `once_cell`、不用 `OnceLock`，除非需要运行时输入）。
- **路由**：axum 0.8 捕获参数语法是 `{name}`（不是 `:name`）。
- **FFI**：Jail 模块使用 libjail（`jailparam_*`）——所有 `unsafe` 集中在 `sys` 子模块，配安全封装。详见"系统命令模式"中的判定准则。
- **匹配模式**：用 match ergonomics（`match &value`），不在模式里写显式 `ref`/`ref mut`。

### 前端

- **Vue 3 + Vite**：使用 Composition API + `<script setup>` 语法。源码在 `frontend/src/`，Vite 构建输出到 `web/`。
- **资源服务**：默认内嵌（rust-embed）；开发时可用 `npm run dev`（热更新）或磁盘覆盖。
- **API 调用**：用 `lib/api.js` 的 `api.get/post/put/del`（处理 auth header + token + 401 重定向）。token 存 `sessionStorage`。
- **导航**：Vue Router hash 路由（`#/dashboard`）。布局 = 顶部栏主标签 + 侧边栏子项。菜单结构在 `lib/menu.js`。
- **状态管理**：Pinia stores——`stores/auth.js`（认证）、`stores/ui.js`（Toast + 对话框）。
- **命令式对话框**：通过 composables（`useConfirm`/`useAlert`/`useFormModal`/`useToast`）调用，返回 Promise。底层由 `stores/ui.js` + `DialogHost.vue` 驱动。
- **定时器清理**：在 `onUnmounted` 中 `clearInterval`/`clearTimeout`。
- **Chart.js**：npm 包，在 `lib/chart.js` 中注册。页面组件在 `onUnmounted` 中销毁 Chart 实例。
- **xterm.js**：npm 包（`@xterm/xterm` + `@xterm/addon-fit`），在 `onUnmounted` 中 dispose + close WebSocket。
- **消息反馈（强制）**：判断标准是——**用户漏看该消息是否会造成误解或困惑**。
  - **重要消息用弹窗**：操作失败、错误、校验异常等。用户如果没有到错误及其原因，会误以为操作成功，造成重大误解——必须用模态弹窗（`useConfirm()` 或 `useAlert()`）强制展示，确保用户看到。
  - **非重要消息用 Toast**：操作成功（创建成功、删除成功、保存成功等）。用户漏看也不会造成误解（操作已经生效）——用 `useToast().toast()` 弱提示即可，不打断用户流程。
  - 简记：成功 → toast，失败 → 弹窗。
- **i18n 翻译键命名（强制）**：新增或复用 `frontend/src/i18n/translations.js` 中的翻译键前，**必须先读该文件顶部的命名规范注释**并严格遵守。核心规则：
  1. 同一含义只用一个 key（跨页面同义的词如 name/value/edit/delete 等一律放 `common` 命名空间复用，不按 nav/zfs/rcconf 等场景拆分）
  2. 只有语义确实不同时才建新 key
  3. **如果一个词在各语言中完全相同**（如 MAC/MTU/Metric/IPv4），**不要建 key**，直接在模板中写原文，避免无意义的 key 膨胀
  4. English 是 fallback，每个 key 必须在 `en` 中存在
- **按钮组（强制）**：同一单元格/区域内的多个操作按钮**必须**包裹在 `<div class="btn-group">` 中，否则按钮间间距过大。不论在表格操作列、弹窗、工具栏，只要是相邻的按钮组都要加。
- **工具栏布局（强制）**：页面操作按钮（创建、刷新等）的放置位置取决于左侧是否有内容：
  - **左侧为空**（如 Zpool 列表、数据集列表）→ 按钮直接放 `.page-header` 行内靠右（`style="margin-left:auto;"`），不单独使用 `.toolbar`，避免按钮左边完全是空白。
  - **左侧有内容**（如快照列表、Jail/虚拟机列表有过滤/搜索框）→ 按钮放独立的 `.toolbar` 行，与左侧内容保持在一行。

## 系统命令模式

FreeBSD 管理通过 spawn 系统二进制并传校验过的参数完成。**禁止字符串拼接 shell**——始终用 `Command::new().arg()` 防注入。传给命令前先校验输入（如 jailname 匹配 `^[a-zA-Z0-9_.-]+$`）。

本机已确认存在的关键工具：`/sbin/sysctl`、`/usr/sbin/sysrc`、`/sbin/ifconfig`、`/sbin/pfctl`、`/sbin/zfs`、`/sbin/zpool`、`/usr/sbin/jail`、`/usr/sbin/pkg`、`/usr/local/sbin/vm`（vm-bhyve 1.7.3）。

**系统源码**：FreeBSD 完整源码树位于 `/usr/src`（只读）。需要了解内核结构体定义、ioctl/sysctl 调用方式、系统命令（如 `ifconfig`、`jail`）的内部实现逻辑、API 调用链、如何查询或操作系统资源时，均可参照源码。

### 何时用 C API / FFI，何时用 spawn 命令？

默认用 **spawn 命令**。仅当满足以下条件之一时，才用 C API（libc syscall）或 FFI（共享库）：

1. **无合适的命令行替代**——如 `jail.rs` 用 libjail FFI（`jls` 输出固定列，无法获取完整 jail 参数）；`network.rs` 用 `getifaddrs(3)` + `sysctl(NET_RT_DUMP)`（路由表是二进制 `rt_msghdr` 消息，`netstat -r` 文本格式不稳定）。
2. **高频调用下 fork/exec 开销不可忽视**——如 `sysinfo.rs`/`sysctl.rs` 供后台监控采集器每 5 秒采样，用 `sysctl(3)` syscall 替代 spawn `/sbin/sysctl`。
3. **数据是二进制结构，文本解析反而更脆弱**——如 `cp_times` 数组、路由表套接字消息。

当成熟的命令行工具存在、调用频率低（用户交互时才请求）、且有机器可读输出（`-H -p`、TSV、JSON 等）时，用 spawn 命令。示例：`pkg`（`pkg query`/`pkg info --raw-format json-compact`）、`zfs`/`zpool`（`-H -p`）、`service`、`sysrc`、`crontab`、`df`/`geom`/`mount`。

## 文档维护（强制）

项目维护两套文档，**开发或变更功能时必须同步维护**：

- `docs/plan/` — 设计计划：功能的目标、架构决策、接口设计（实现前写的，也实现后的前瞻规划）
- `docs/impl/` — 实现文档：功能实际怎么做的，含数据结构、算法、调用链、API、配置项、已知限制

### 规则

1. **开发新功能前**：先读 `docs/impl/` 中相关的已有实现文档，复用已有模式和约定。`docs/impl/README.md` 有索引。
2. **实现新功能后**：在 `docs/impl/` 创建对应实现文档（编号续接），遵循 README 中的格式规范。
3. **变更已有功能时**：更新对应的 `docs/impl/` 文档，保持与代码一致。
4. **删除功能时**：删除或归档对应文档。
5. **设计阶段**（未实现的功能规划）：写 `docs/plan/`；**实现完成后**：写 `docs/impl/`。

不要让文档与代码脱节——过时的文档比没有文档更危险。

## 架构决策

- **纯 HTTP**（无 TLS）——远程访问请前置反向代理。
- **自带认证**：SQLite users 表、Argon2id 哈希、session token（DB 中存 SHA-256 哈希）。不用 PAM/系统用户。
- **首启引导**：无用户时 `/api/users/bootstrap` 创建首个管理员（无需认证，仅一次）。
- **Jail 走 libjail FFI**（不用 iocage/ezjail 等）——项目要求。
- **Bhyve 走 vm-bhyve**——项目要求。

## 待办

框架（认证、布局、仪表盘、用户管理、审计）+ 监控采集（CPU/内存/负载/温度）已完成。未实现模块返回 "planned" 占位。阶段计划见 `docs/plan/80-roadmap.md`。
