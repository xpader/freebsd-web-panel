# 06 — 前端架构

## 概述

Vue 3 SPA（Composition API + `<script setup>`），使用 Vite 构建。支持深色/浅色/跟随系统三种主题模式（详见 [32-theming.md](32-theming.md)）。Vite 构建输出到 `web/`，由后端 `rust-embed` 内嵌为单二进制资源。

## 构建流程

```sh
cd frontend && npm install && npm run build   # 输出到 ../web/
cargo build                                    # 内嵌 web/ 到二进制
```

开发模式：`cd frontend && npm run dev`（Vite 代理 `/api` 到 `127.0.0.1:18080`），同时 `cargo run`。

### 技术栈

| 库 | 用途 |
|---|---|
| Vue 3 | 响应式 UI 框架 |
| Vue Router 4 | Hash 路由（`createWebHashHistory`） |
| Pinia | 状态管理（auth、UI dialogs） |
| vue-i18n 10 | 国际化（中文/英文） |
| Chart.js 4 | 监控图表（npm 包，不再用 UMD vendor） |
| @xterm/xterm 5 | Web 终端（npm 包） |
| Vite 6 | 构建工具 |

## 目录结构

```
frontend/
├── package.json
├── vite.config.js          # outDir=../web, 分块策略
├── index.html              # Vite 入口
├── public/                 # 静态资源（原样复制到 web/）
│   ├── img/                # 国旗 SVG
│   └── vendor/fontawesome/ # FontAwesome CSS + woff2
└── src/
    ├── main.js             # Vue 应用启动（Pinia + Router + i18n）
    ├── App.vue             # 根组件（router-view + ToastContainer + DialogHost）
    ├── assets/app.css      # 原始 CSS（迁移自 web/css/app.css）
    ├── lib/
    │   ├── api.js          # fetch 封装（token + 401 重定向）
    │   ├── format.js       # fmtBytes/fmtRate/fmtUptime/fmtDate 等工具函数
    │   ├── menu.js         # 导航菜单配置（MENU + SETTINGS 常量）
    │   └── chart.js        # Chart.js 注册 + 共享图表工具函数
    ├── i18n/
    │   ├── index.js        # vue-i18n 初始化 + {{}} → {} 语法转换
    │   └── translations.js # 翻译资源（en/zh，从原项目迁移）
    ├── router/index.js     # Vue Router 路由表 + 认证守卫
    ├── stores/
    │   ├── auth.js         # token、用户信息、首启状态
    │   ├── ui.js           # Toast 队列 + 命令式对话框队列
    │   └── theme.js        # 主题偏好（auto/light/dark）+ 系统媒体查询监听
    ├── composables/
    │   └── useDialog.js    # useToast/useConfirm/useAlert/useFormModal
    ├── components/
    │   ├── layout/
    │   │   ├── AppLayout.vue   # 骨架：topbar + sidebar + router-view
    │   │   ├── TopBar.vue      # 顶栏：logo 标识 + 导航 + 语言 + 主题 + 设置 + 用户
    │   │   └── SideBar.vue     # 侧栏：菜单项（含可折叠子组）
    │   └── ui/
    │       ├── ToastContainer.vue  # Toast 渲染
    │       ├── DialogHost.vue       # confirm/alert/formModal 渲染
    │       ├── SearchInput.vue     # 搜索输入框（v-model + 清除按钮）
    │       ├── TaskConsole.vue     # 后台任务输出（SSE 流式 + 自动滚动）
    │       └── ProgressBar.vue    # 百分比进度条（统一封装 metric bar）
    └── pages/              # 各功能页面（35 个 .vue 文件）
```

## 实现细节

### 路由 `src/router/index.js`

Vue Router hash 模式（`createWebHashHistory`）。路由表使用嵌套结构：认证页面（`/login`、`/setup`）为顶级路由，其余页面在 `AppLayout` 的 `children` 中。

认证守卫（`router.beforeEach`）：
- 认证页（`meta.auth = false`）：已登录 → 重定向到 `/dashboard`
- 受保护页：无 token 时惰性查询 `GET /api/users/bootstrap`——首启重定向 `/setup`，否则 `/login`
- token 存 `sessionStorage`，401 由 `api.js` 统一处理（forever-pending 模式，详见 `01-auth.md`）

### API 客户端 `src/lib/api.js`

```js
api.get(path) / api.post(path, body) / api.put(path, body) / api.del(path)
```

- 自动附加 `Authorization: Bearer <token>`
- 401 处理（非登录页）：登出 + 跳转 `/login` + 弹"登录已失效"框 + `return new Promise(() => {})`（forever-pending，页面的 catch 不会执行，无需各处判断 401）
- 401 处理（登录页）：正常 throw，LoginPage 显示密码错误
- 429 区分：`err.data.error === 'account_locked'` / `'ip_banned'`，LoginPage 分别显示不同提示
- `authFetch(url, opts)` — 带 token 的原始 fetch（用于文件上传/下载），401 同样走 forever-pending

### 状态管理

**`stores/auth.js`** — Pinia store：
- `token` — 会话令牌（持久化到 sessionStorage）
- `user` — 当前用户信息（`GET /api/auth/me`）
- `needsSetup` — 首启状态缓存
- `resolveNeedsSetup()` — 惰性查询 bootstrap 接口

**`stores/ui.js`** — Pinia store：
- `toasts` — Toast 通知队列（自动过期移除）
- `dialog` — 当前活动对话框（confirm/alert/form）
- `showDialog(cfg)` → Promise，`resolveDialog(value)` 解决

### 命令式对话框 `src/composables/useDialog.js`

保持原有的 Promise-based API：
- `useToast().toast(message, type)` — 显示通知
- `useConfirm()(title, message, options)` → `Promise<boolean | {confirmed, ...}>`
- `useAlert()(title, message)` → `Promise<void>`
- `useFormModal()(title, fields, opts)` → `Promise<Object | null>`
- `useCodePreview()(title, content)` → `Promise<void>`
- `useCountdown()(title, message, expiresAt, timeoutSeconds, opts)` → `Promise<'confirm' | 'rollback'>`

对话框通过 `stores/ui.js` 的响应式状态驱动 `DialogHost.vue` 组件渲染。

#### 对话框类型

| 类型 | 渲染 | 说明 |
|---|---|---|
| `toast` | ToastContainer | 右下角通知，自动消失 |
| `confirm` | modal | 确认对话框，可选 checkbox 选项 |
| `alert` | modal | 警告对话框，仅一个 OK 按钮 |
| `form` | modal-wide | 表单对话框，支持多种字段类型 |
| `code` | modal | 代码/文本预览（等宽字体） |
| `countdown` | modal | 带倒计时进度条的确认/回滚对话框（防火墙 apply 用） |

#### form 字段 schema

`useFormModal()(title, fields, opts)` 的 `fields` 数组中每个字段对象支持以下属性：

| 属性 | 类型 | 说明 |
|---|---|---|
| `key` | string | 字段标识，结果对象中的键名 |
| `label` | string | 字段标题 |
| `type` | string | `text`（默认）、`password`、`select`、`radio`、`checkbox`、`checkbox-group`、`textarea` |
| `value` | any | 初始值 |
| `inputType` | string | 当 type 为通用 input 时，设置 `<input type>`（如 `password`） |
| `placeholder` | string | 占位提示 |
| `required` | bool | 是否必填 |
| `disabled` | bool | 是否禁用 |
| `help` | string | Tooltip 文本（渲染 FieldHelp 图标） |
| `half` | bool | 半宽字段，与相邻 `half` 字段两两并排 |
| `row` | number/string | 将同 `row` 值的 `half` 字段强制放入同一行 |
| `showIf` | `{key: val}` | 条件显示：当另一字段等于 `val`（或数组中任一值）时显示 |
| `requiredIf` | `{key: val}` | 条件必填 |
| `options` | array | `select`/`radio`：`{value, label}`；`checkbox-group`：`{key, label, value, help?}` |
| `picker` | `'dir'`/`'file'` | 路径选择器：渲染 `.input-with-btn` + 按钮。按钮按字段值自动判定本地/远程——值形如 `user@host`（SSH 连接）时打开 `RemoteFilePicker`，否则打开本地 `FilePicker` |

`opts` 支持 `submitLabel`（提交按钮文字）和 `submitHandler`（异步提交函数，抛错时内联显示错误不关闭对话框）。

#### 字段类型渲染细节

- **`radio`** — pill 样式横排，选中高亮
- **`checkbox`** — 带描述文字的确认选项样式
- **`checkbox-group`** — 多个 pill 样式 checkbox 内联排列，共用 `label` 作为组标题，每个 option 的值直接写入 `formValues[opt.key]`。每个 option 可选 `help` 属性，渲染为 pill 内的 FieldHelp 工具提示
- **`select`** — 下拉框
- **`textarea`** — 多行文本
- **`picker`** — 输入框 + 单按钮，按钮行为按字段当前值自动判定：本地路径或空 → 打开 `FilePicker`（📁）；SSH 规格（`user@host`）→ 打开 `RemoteFilePicker`（🌐）。可选 `portKey` 属性指定另一字段名，远程选择器从中读取 SSH 端口
- **`half` + `row` 布局** — `groupedFields()` 将连续 `half` 字段两两配对为 flex 行；同 `row` 值的字段强制同组

#### 示例

```js
// 两栏布局 + 路径选择器 + pill 多选组
const result = await formModal('创建共享', [
  { key: 'name', label: '名称', required: true },
  { key: 'path', label: '路径', required: true, picker: 'dir' },
  { key: 'create_mask', label: '文件掩码', value: '0664', half: true },
  { key: 'directory_mask', label: '目录掩码', value: '0775', half: true },
  {
    key: '_flags', label: '选项', type: 'checkbox-group',
    options: [
      { key: 'browseable', label: '可浏览', value: true },
      { key: 'writable', label: '可写', value: false },
      { key: 'time_machine', label: 'Time Machine', value: false, help: '作为 macOS Time Machine 备份目标' },
    ],
  },
  { key: 'valid_users', label: '授权用户', help: '空格分隔的用户名' },
]);

// 异步提交 + 内联错误
const result = await formModal('添加仓库', fields, {
  submitLabel: '创建',
  submitHandler: async (values) => {
    await api.post('/api/repos', values);  // 抛错时对话框不关闭
  },
});
```

#### 参考页面

`DialogDemoPage.vue`（路由 `/dialog-demo`，仅开发模式）提供所有字段类型和交互模式的可交互演示。

### 导航 `src/lib/menu.js` + `components/layout/`

`MENU` 常量定义 7 个顶级组（概览/系统/服务/网络/存储/虚拟化/监控），每组含 `items`（侧栏菜单项），菜单项可选 `children`（可折叠子组）。`groupOfPath(path)` 计算路径所属组。顶栏主菜单标签支持**鼠标悬停直接展开子菜单下拉列表**（由 `openKey` ref 驱动：wrapper 的 `@mouseenter` 置为当前组 key、`@mouseleave` 置空；`.topnav-submenu` 容器 `@click` 置空——点击任一子项后立即收起），无需先点击主菜单再经过默认子项。下拉内容与侧栏一致（直接项为链接，带 `children` 的项显示为分组小标题 + 缩进子链接）；wrapper 用 `.topnav-submenu::before` 透明伪元素桥接 tab 与下拉间的间隙，保证鼠标斜向移动不脱出（submenu 属 wrapper DOM 子树，移动其间不触发 `mouseleave`）。

- `AppLayout.vue` — 骨架：topbar + sidebar + `<router-view />`。侧栏可收缩（`.sidebar-toggle` 竖条按钮附于侧栏右边缘，点击切换；收起状态持久化到 `localStorage` key `fwp_sidebar_collapsed`）
- `TopBar.vue` — logo 标识（闪电方块 + fwp）+ 导航标签（悬停展开子菜单下拉）+ 语言切换（国旗）+ 主题切换（太阳/月亮图标）+ 设置下拉 + 用户下拉
- `SideBar.vue` — 当前组子菜单，支持可折叠子组

### 国际化 `src/i18n/`

- **框架**：vue-i18n 10（替换原来的 i18next）
- **翻译表**：`translations.js` 从原项目迁移，保留 `en`/`zh` 两个对象
- **语法转换**：`index.js` 中 `convertMsg()` 在运行时将 i18next 的 `{{key}}` 转换为 vue-i18n 的 `{key}`，无需修改翻译文件
- **语言切换**：`setLang(code)` 修改 vue-i18n locale + 持久化到 `localStorage`，Vue 响应式自动更新所有页面（无需手动重渲染）

### CSS `src/assets/app.css`

- 支持深色（默认）与浅色两套 CSS 变量主题，通过 `:root[data-theme="light"]` 覆盖 `:root` 默认变量实现切换（详见 [32-theming.md](32-theming.md)）
- 品牌标识：渐变方块 logo（闪电图标）+ “fwp” 缩写文字，点击跳转仪表盘
- 所有 hover 态统一使用 `--hover-bg` 变量；选中态使用 `--accent-glow` 半透明辉光
- 布局：`#app` flex column → `.topbar`（52px sticky）+ `.body-wrap`（flex row：`.sidebar` 240px + `.sidebar-toggle` 16px 收缩条 + `.main`）。侧栏收缩时 `.sidebar` 宽度动画到 0（`overflow: hidden` 裁切内容），收缩条箭头方向切换
- 表格横向滚动：在 `.card` 内的 `<table>` 外包一层 `<div class="table-wrap">`，容器设置 `overflow-x: auto`，内部 `th`/`td` 默认 `white-space: nowrap`（不挤压、不换行），仅 `.cell-wrap` 列允许换行吸收空间。视口缩窄时出现横向滚动条而非挤压内容。
- 按钮组：相邻的多个按钮包裹在 `<div class="btn-group">`（`display: flex; flex-wrap: nowrap`），防止换行且消除按钮间多余间距。
- 搜索输入框：使用 `SearchInput.vue` 组件（`v-model` + `placeholder`），有内容时右侧显示清除按钮。底层由 `.search-input`（`position: relative`）+ `.search-clear` 定位实现。
- 后台任务输出：使用 `TaskConsole.vue` 组件（prop: `task-id`，event: `done`），封装 SSE 连接管理、输出累积、自动滚动到底部、`[完成]` 后缀、错误回退轮询。PackagesPage、JailBasesPage、SmbInitPage、BhyveInitPage 共用。
- 进度条：使用 `ProgressBar.vue` 组件（prop: `pct`/`variant`/`size`/`threshold`），封装 `.bar-wrap`/`.bar` 三层结构与阈值着色。`variant` 取 `cpu`/`mem`/`swap` 固定配色，或 `auto`（默认，超 `threshold` 转橙色警告，否则紫色）。百分比内部 clamp 到 [0,100]。Dashboard（CPU/内存/Swap/核心）、Disks、FilesystemOverview、ZfsPools、ZfsPoolDetail 共用。
- Vite 将 CSS 打包到 `web/assets/*.css`

## 页面模块

| 组件 | 页面 | 说明 |
|---|---|---|
| `LoginPage.vue` | 登录 | 表单提交 → 存 token → 跳转仪表盘 |
| `SetupPage.vue` | 首启初始化 | 创建首个管理员 |
| `DashboardPage.vue` | 仪表盘 | 静态信息卡片 + 3 秒轮询实时指标 |
| `ShellPage.vue` | Web 终端 | xterm.js + WebSocket ↔ PTY |
| `UsersPage.vue` | 面板用户 | 列表 + 创建/改密/删除 |
| `AuditPage.vue` | 审计日志 | 表格，按方法/状态着色 |
| `MonitorCpu/Memory/NetworkPage.vue` | 监控图表 | Chart.js 折线图 + 时间范围 |
| `FilesystemOverviewPage.vue` | 文件系统概览 | 磁盘/挂载点/ZFS 池 |
| `DisksPage.vue` | 磁盘 | 各磁盘详情 + 分区表 |
| `FilesPage.vue` | 文件管理器 | 目录树 + 列表/网格视图 + 上传/下载/重命名/删除/属性 |
| `SysctlPage.vue` | sysctl | 浏览/搜索/分页/编辑/重置 |
| `RcconfPage.vue` | RC 配置 | 列表/新增/编辑/删除 rc.conf |
| `CronPage.vue` | 定时任务 | 分区列表/新增/编辑/启停/删除 |
| `NetworkPage.vue` | 网络接口 | 接口卡片 + 路由表 + 默认网关 |
| `DnsPage.vue` | DNS | 域名服务器编辑 + 验证 |
| `ServicesPage.vue` | 服务 | 列表 + start/stop/restart |
| `AccountsUsers/GroupsPage.vue` | 系统用户/组 | 只读列表 + 搜索 |
| `PfPage/BhyvePage.vue` | 占位页 | 使用 `_PlannedPage.vue` 工厂组件 |
| `ZfsPools/PoolDetail/Datasets/SnapshotsPage.vue` | ZFS | Zpool/数据集/快照管理 |
| `PackagesPage/PackageDetailPage.vue` | 软件包 | 列表/搜索/安装/删除/升级/清理/锁定/详情 |
| `PkgReposPage.vue` | 软件源 | 仓库 CRUD + 预设 + `pkg update -f` SSE |
| `JailsList/Create/Detail/BasesPage.vue` | Jail 容器 | 列表/创建/详情/基础系统 |

## 外部依赖

- Chart.js 4 + chartjs-adapter-date-fns（npm 包）— 监控图表
- @xterm/xterm 5 + @xterm/addon-fit（npm 包）— Web 终端
- Font Awesome 6.7.2 Free（`frontend/public/vendor/fontawesome/`）— UI 图标
- vue / vue-router / pinia / vue-i18n（npm 包）— Vue 生态

## 已知限制

- 无前端测试
- 无响应式适配（小屏幕侧边栏不折叠）
- 构建产物（`web/assets/`、`web/index.html`）需 `npm run build` 生成，不在 git 中跟踪
