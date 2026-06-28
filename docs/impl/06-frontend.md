# 06 — 前端架构

## 概述

原生 ES Modules SPA，无框架、无构建步骤、无 npm 依赖。深色主题自写 CSS。由后端 `ServeDir`/`rust-embed` 托管。

## 实现细节

### 路由 `web/js/router.js`

Hash 路由（`#/dashboard`）。`defineRoute(path, handler)` 注册，`startRouter()` 监听 `hashchange`。

匹配逻辑：精确匹配或前缀匹配（最长匹配优先）。未匹配返回 404 页。

认证守卫：无 token 时重定向到 `#/login`；有 token 时访问登录/初始化页重定向到 `#/dashboard`。

### API 客户端 `web/js/api.js`

```js
api.get(path) / api.post(path, body) / api.put(path, body) / api.del(path)
```

- 自动附加 `Authorization: Bearer <token>`（从 `sessionStorage`）
- 401 响应 → 清除 token + 重定向 `#/login`
- 错误抛出 `{status, message}` 对象，调用方 try/catch

### 两级导航 `web/js/ui/layout.js`

顶部主菜单（6+1 标签）+ 左侧子菜单（随主菜单切换）。菜单结构为 `MENU` 常量数组：

```
概览 → [仪表盘]
配置 → [Sysctl, RC 配置, 服务管理]
网络 → [网络接口, 防火墙]
文件系统 → [ZFS]
虚拟化 → [Jail 容器, Bhyve 虚拟机]
监控 → [CPU & 负载, 内存, 温度]
系统 → [用户管理, 审计日志]
```

`renderLayout(app, currentPath, pageContent)` 渲染骨架：顶栏（品牌名 + 导航标签 + 用户名 + 退出）+ 侧栏（当前组子菜单）+ 主内容区。`groupOfPath(path)` 计算路径所属的主菜单组。

### 页面模块

| 文件 | 页面 | 说明 |
|---|---|---|
| `auth.js` | 登录 + 初始化向导 | bootstrap 检查 → 显示对应表单 |
| `dashboard.js` | 仪表盘 | 静态信息卡片 + 3 秒轮询实时指标 + 进度条 |
| `users.js` | 用户管理 | 列表 + 创建/改密/删除（模态框） |
| `audit.js` | 审计日志 | 表格，按方法/状态着色 |
| `monitor.js` | 监控图表 | Chart.js 折线图 + 时间范围选择 |
| `planned.js` | 模块占位页 | 工厂函数，生成"计划中"占位页 |

### 仪表盘实时更新 `dashboard.js`

- `renderDashboard` 先调 `/api/system/info` 渲染静态卡片
- 启动 `setInterval(refreshMetrics, 3000)` 每 3 秒调 `/api/system/metrics`
- `refreshMetrics` 更新 CPU/内存/Swap 进度条 + 每核条 + 温度徽章
- 重新进入仪表盘时 `clearInterval` 旧定时器再启动新的（不冗余守卫 `if(pollTimer)`）

### UI 组件

- `toast.js` — 右上角通知（成功/错误），3 秒自动消失
- `confirm.js` — Promise 确认对话框（模态遮罩）
- `layout.js` — 两级导航骨架

### CSS `web/css/app.css`

- CSS 变量定义主题色（`--bg`, `--accent`, `--danger` 等）
- 布局：`#app` flex column → `.topbar`（52px sticky）+ `.body-wrap`（flex row：`.sidebar` 240px + `.main`）
- 登录页：`.login-wrap` flex 居中（`width:100%` 修复靠左问题）
- 指标进度条：`.bar` + `.bar-cpu`/`.bar-mem`/`.bar-swap` 渐变色，`transition: width 0.5s`
- 表格、卡片、徽章、模态框、Toast 等通用样式

## 外部依赖

- Chart.js 4.4.7 UMD（`web/vendor/chart.umd.min.js`）— 监控图表
- chartjs-adapter-date-fns 3.0.0（`web/vendor/chartjs-adapter-date-fns.bundle.min.js`）— Chart.js 时间轴

## 已知限制

- 无前端路由历史（hash 变化不支持浏览器前进/后退语义完整）
- 无响应式适配（小屏幕侧边栏不折叠）
- 无国际化框架（硬编码中文）
- 无前端测试
