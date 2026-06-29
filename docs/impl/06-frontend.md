# 06 — 前端架构

## 概述

原生 ES Modules SPA，无框架、无构建步骤、无 npm 依赖。深色主题自写 CSS。由后端 `ServeDir`/`rust-embed` 托管。

## 实现细节

### 路由 `web/js/router.js`

Hash 路由（`#/dashboard`）。`defineRoute(path, handler)` 注册，`startRouter()` 监听 `hashchange`。

匹配逻辑：
- 以 `/` 结尾的路由（如 `/zfs/pools/`）定义为**纯前缀路由**——只匹配子路径（`startsWith`），不匹配精确路径，用于详情页
- 非斜杠路由只做精确匹配（如 `/zfs/pools` 匹配列表页）
- 优先级：按原始路径长度排序，更长的优先
- 未匹配返回 404 页

认证守卫：无 token 时，路由器惰性查询公开接口 `GET /api/users/bootstrap`（结果缓存）——首启（`needs_setup=true`）重定向到 `#/setup`，否则重定向到 `#/login`，由系统自动区分而非用户手动选择。首启创建管理员后调用 `invalidateSetup()` 令缓存失效，后续未认证访问即走登录页。有 token 时访问登录/初始化页重定向到 `#/dashboard`。`match()` 为 `async`，`hashchange` 触发时不阻塞。

### API 客户端 `web/js/api.js`

```js
api.get(path) / api.post(path, body) / api.put(path, body) / api.del(path)
```

- 自动附加 `Authorization: Bearer <token>`（从 `sessionStorage`）
- 401 响应 → 清除 token + 重定向 `#/login`
### 三级导航 `web/js/ui/layout.js`

顶部主菜单（6 标签）+ 左侧子菜单（随主菜单切换）+ 子子菜单（可折叠）。菜单结构为 `MENU` 常量数组：

```
概览     → [仪表盘]
配置     → [Sysctl, RC 配置, 服务]
网络     → [网络接口, 防火墙]
文件系统 → [概览, 磁盘, ZFS → [Zpool, 数据集, 快照]]
虚拟化   → [Jail 容器, Bhyve 虚拟机]
监控     → [CPU & 负载, 内存, 温度]
```

面板自身管理（用户、审计日志）不在顶部主菜单，而是放在顶栏右侧的设置下拉菜单中：用户名左侧的「⚙ 设置」按钮，点击展开 `SETTINGS` 数组（用户、审计日志）。`groupOfPath` 同时检查 `MENU` 和 `SETTINGS`；当路径属于设置组时，设置按钮高亮，侧边栏显示其子项。

菜单项支持 `children` 数组：
- 有 `children` → 可折叠子组（组头可点击，导航到第一个子项 + 自动展开，`cursor: pointer`，箭头旋转动画）
- 无 `children` → 直接链接

`renderLayout(app, currentPath, pageContent)` 渲染骨架：顶栏 + 侧栏 + 主内容区。`groupOfPath(path)` 计算路径所属的主菜单组（递归检查 `children`）。子项高亮为精确匹配（`currentPath === item.path`）。
```

`renderLayout(app, currentPath, pageContent)` 渲染骨架：顶栏（品牌名 + 导航标签 + 设置下拉 + 用户名下拉）+ 侧栏（当前组子菜单）+ 主内容区。`groupOfPath(path)` 计算路径所属的主菜单组。

顶栏右侧两个下拉：设置菜单（⚙ 设置 → 用户、审计日志）和用户菜单（👤 昵称 → 退出登录）。两者点击切换、点击外部关闭、点击菜单项后关闭。

### 页面模块

| 文件 | 页面 | 说明 |
|---|---|---|
| `auth.js` | 登录 + 初始化向导 | 由路由器守卫保证进入正确的页（首启→初始化，否则→登录）；登录成功存 token，初始化成功后令缓存失效并跳转登录 |
| `dashboard.js` | 仪表盘 | 静态信息卡片 + 3 秒轮询实时指标 + 进度条 |
| `users.js` | 用户 | 列表 + 创建/改密/删除（模态框） |
| `audit.js` | 审计日志 | 表格，按方法/状态着色 |
| `monitor.js` | 监控图表 | Chart.js 折线图 + 时间范围选择 |
| `filesystem.js` | 文件系统概览 | 磁盘/挂载点/ZFS 池概览 |
| `disks.js` | 磁盘 | 各磁盘详情 + 分区表（UUID 悬停提示 + 点击复制） |
| `zfs.js` | ZFS | Zpool 列表/详情、数据集树、快照管理 |
| `planned.js` | 模块占位页 | 工厂函数，生成"计划中"占位页 |
- `confirm.js` — Promise 确认对话框（模态遮罩）；可选 `options` 参数支持复选框（如递归删除勾选），有 options 时返回 `{confirmed, ...checkboxes}`，否则返回布尔值
- `formModal.js` — 可复用表单模态框（替代浏览器 `prompt()`，支持 text/select/textarea/password 字段，自动聚焦）
- `layout.js` — 三级导航骨架
### 仪表盘实时更新 `dashboard.js`

- `renderDashboard` 先调 `/api/system/info` 渲染静态卡片
- 启动 `setInterval(refreshMetrics, 3000)` 每 3 秒调 `/api/system/metrics`
- `refreshMetrics` 更新 CPU/内存/Swap 进度条 + 每核条 + 温度徽章
- 重新进入仪表盘时 `clearInterval` 旧定时器再启动新的（不冗余守卫 `if(pollTimer)`）
- 指标进度条：`.bar` + `.bar-cpu`/`.bar-mem`/`.bar-swap` 渐变色，`transition: width 0.5s`
- 三级子菜单：`.sub-group`（折叠容器）、`.sub-group-header`（`cursor: pointer`，hover 高亮）、`.sub-items`（展开时 `display:block`，左侧 `margin-left: 27px` + 竖线引导线）、`.sidebar-nav .sub-item`（缩进，选中态 accent 色无左边框——选择器带 `.sidebar-nav` 前缀以提升特异性，盖过 `.sidebar-nav a` 的默认 `padding: 8px 20px`）
- pool 卡片：`.pool-card`（hover 高亮 + `cursor: pointer`）、`.vdev-node`（阵列结构树缩进）
- 表格、卡片、徽章、模态框、Toast 等通用样式
- `confirm.js` — Promise 确认对话框（模态遮罩）
- `layout.js` — 两级导航骨架

### 国际化 `web/js/i18n/`

- **框架**：i18next，`i18n/index.js` 初始化并导出 `t(key, params)` / `getLocale()` / `setLocale()`
- **翻译表**：`i18n/translations.js` 导出 `en`（fallback）和 `zh` 两个对象，key 用点分命名空间（`common.name`、`fm.permissions`、`zfs.state`）
- **语言切换**：顶栏右侧语言按钮，切换后整页重渲染；locale 持久化到 `localStorage`

#### 翻译键命名规范（强制）

翻译文件顶部有注释块（必读），核心原则：

**同一含义只用一个 key**——同一个词（如"所有者"/"权限"/"大小"）出现在列表表头、属性弹窗、权限编辑器等不同位置时，必须共用同一个 key，不要按使用场景拆成多个翻译完全相同的 key。

- 新增 key 前先检查是否已有等价词（同义、同翻译），有就直接复用
- 跨页面通用的词（`name`/`size`/`time`/`user`/`delete` 等）放 `common` 命名空间
- 只有语义确实不同时才创建新 key

反面教材（已删除）：`colOwner` + `statOwner` + `permOwner`——三个 key 翻译完全相同（"所有者"），只是出现在列表/属性/权限编辑器不同位置。正确做法：`owner`（一个 key，所有位置共用）。

### CSS `web/css/app.css`

- CSS 变量定义主题色（`--bg`, `--accent`, `--danger` 等）；`:root { color-scheme: dark }` 声明深色主题，让浏览器自动渲染暗色滚动条及表单控件
- 布局：`#app` flex column → `.topbar`（52px sticky）+ `.body-wrap`（flex row：`.sidebar` 240px + `.main`）
- 登录页：`.login-wrap` flex 居中（`width:100%` 修复靠左问题）
- 指标进度条：`.bar` + `.bar-cpu`/`.bar-mem`/`.bar-swap` 渐变色，`transition: width 0.5s`
- 表格行悬停高亮（`tr:hover td` 背景 `--bg-elev2`）、卡片悬停边框变色（`.card:hover` border accent），列表项加 `transition` 过渡；复选框（`input[type=checkbox]`）`width:auto` 避免被全局表单宽度撑满

## 外部依赖

- Chart.js 4.4.7 UMD（`web/vendor/chart.umd.min.js`）— 监控图表
- chartjs-adapter-date-fns 3.0.0（`web/vendor/chartjs-adapter-date-fns.bundle.min.js`）— Chart.js 时间轴

## 已知限制

- 无前端路由历史（hash 变化不支持浏览器前进/后退语义完整）
- 无响应式适配（小屏幕侧边栏不折叠）
- 无前端测试
