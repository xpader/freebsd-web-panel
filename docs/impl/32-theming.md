# 32 — 主题系统（深色 / 浅色 / 跟随系统）

## 概述

前端支持三种颜色主题模式：**跟随系统**（`auto`，默认）、**浅色**（`light`）、**深色**（`dark`）。用户通过顶栏右侧的主题切换按钮选择，偏好持久化在 `localStorage`，页面加载时通过内联防闪烁（anti-FOUC）脚本在 Vue 挂载前应用，避免白屏闪烁。

深色主题为默认主题（GitHub Dark 风格蓝调深色）；浅色主题使用冷灰蓝底色（非纯白），配合同色系蓝紫渐变装饰元素，两个主题保持统一的视觉特色。

## 实现细节

### CSS 变量体系 `src/assets/app.css`

`:root` 定义深色主题（默认值），`:root[data-theme="light"]` 覆盖为浅色变量。切换时仅修改 `<html>` 的 `data-theme` 属性，所有颜色通过 CSS 变量自动响应。

核心变量：

| 变量 | 说明 |
|---|---|
| `--bg` | 页面主背景 |
| `--bg-elev` | 卡片/顶栏/侧栏背景（第一层提升） |
| `--bg-elev2` | 次级背景（hover、代码块、子元素） |
| `--border` | 边框颜色 |
| `--text` / `--text-dim` | 主文字 / 暗淡文字 |
| `--accent` / `--accent-hover` | 主强调色（蓝）及 hover 态 |
| `--accent-2` | 次强调色（紫），用于渐变 |
| `--accent-glow` / `--accent-glow-strong` | 强调色半透明辉光（选中态背景、focus 环） |
| `--success-glow` / `--danger-glow` / `--warn-glow` | 状态色半透明辉光（badge 背景） |
| `--hover-bg` | 统一 hover 背景色 |
| `--topbar-accent` | 顶栏底部渐变条 + 品牌名渐变文字 |
| `--shadow` / `--shadow-lg` | 阴影（深色更浓，浅色更淡） |
| `--toast-bg` / `--toast-border` / `--toast-shadow` | Toast 通知毛玻璃效果 |
| `--modal-overlay` / `--modal-busy-bg` | 模态遮罩 |
| `--github-icon-filter` | GitHub 图标滤镜（深色反色，浅色无） |

### 视觉特色

两个主题共享以下特色设计：

- **品牌标识** — 顶栏左侧渐变方块 logo（闪电图标 + `--topbar-accent` 渐变背景）配 "fwp" 粗体缩写，点击跳转仪表盘
- **选中态辉光** — 导航标签、侧边栏选中项使用 `--accent-glow` 半透明背景而非纯色填充
- **卡片 hover 光圈** — 可点击卡片 hover 时出现 `box-shadow: 0 0 0 3px var(--accent-glow)` 光圈
- **输入框 focus 辉光** — `box-shadow: 0 0 0 3px var(--accent-glow-strong)`
- **指标条渐变** — CPU/内存进度条使用 `--accent` → `--accent-2` 渐变

### 主题 Store `src/stores/theme.js`

非 Pinia store（纯 Vue ref + watch），在模块导入时立即初始化。

| 导出 | 说明 |
|---|---|
| `preference` | `ref<'auto' \| 'light' \| 'dark'>`，读写 `localStorage['fwp_theme']` |
| `effective` | `ref<'light' \| 'dark'>`，解析后的实际主题 |
| `setTheme(pref)` | 设置偏好（写入 ref → 触发 watch → 写 localStorage + 应用 data-theme） |

内部逻辑：
1. **`readStored()`** — 读 `localStorage`，验证合法值，默认 `auto`
2. **`resolve(pref)`** — `auto` → 查询 `matchMedia('(prefers-color-scheme: dark)')`；否则直接返回
3. **`apply(effective)`** — 设置 `document.documentElement.dataset.theme`
4. **`watch(preference, ...)`** — 写 localStorage + 重新 resolve
5. **`watch(effective, ...)`** — 重新 apply（`immediate: true` 保证初始化）
6. **`setupMqListener()`** — 监听 `matchMedia` `change` 事件，`auto` 模式下跟随系统切换

### Anti-FOUC `index.html`

页面 `<head>` 内联脚本在 CSS 和 JS 加载前同步执行：

```js
var pref = localStorage.getItem('fwp_theme') || 'auto';
var dark = pref === 'dark' || (pref === 'auto' && matchMedia('(prefers-color-scheme: dark)').matches);
document.documentElement.dataset.theme = dark ? 'dark' : 'light';
```

确保首次渲染即为正确主题，无白屏闪烁。

### 顶栏切换 UI `components/layout/TopBar.vue`

位于语言切换与设置按钮之间。按钮图标随当前偏好变化：

| 偏好 | 图标 |
|---|---|
| `auto` | `fa-circle-half-stroke`（半圆） |
| `light` | `fa-sun`（太阳） |
| `dark` | `fa-moon`（月亮） |

下拉项使用 `topbar.themeSystem` / `topbar.themeLight` / `topbar.themeDark` 翻译键，点击调用 `setTheme()`。

### Chart.js 主题适配 `src/lib/chart.js`

图表的网格线、刻度、标签颜色改为在运行时动态读取 CSS 变量，而非硬编码：

| 函数 | 读取变量 | 替代旧常量 |
|---|---|---|
| `gridColor()` | `--border` | `GRID_COLOR` |
| `tickColor()` | `--text-dim` | `TICK_COLOR` |
| `labelColor()` | `--text` | `LABEL_COLOR` |

三个监控页面（`MonitorCpuPage`、`MonitorMemoryPage`、`MonitorNetworkPage`）通过 `watch(themeEff, ...)` 监听主题变化，在主题切换后重新绘制图表。

### 终端主题适配 `src/lib/term-theme.js`

xterm.js 终端配色通过 `termTheme(effective)` 返回对应主题的完整配色对象（含 16 色 ANSI 调色板）。三个终端页面（ShellPage / JailTerminalPage / BhyveConsolePage）初始化时读取当前主题，并通过 `watch(themeEff, ...)` 在切换时实时更新 `term.options.theme`。终端容器背景由 `--term-bg` CSS 变量控制（深色 `#0b0e14`、浅色 `#fafbfc`）。

## 涉及源码

| 文件 | 变更 |
|---|---|
| `frontend/src/assets/app.css` | 新增 `:root[data-theme="light"]` 变量块；新增 `--accent-glow` 等辉光变量、`--topbar-accent` 渐变、`--hover-bg` 统一 hover；品牌 logo（渐变方块 + fwp 文字）、顶栏彩条、选中态辉光、卡片 hover 光圈；所有硬编码 `rgba()` 阴影替换为 CSS 变量 |
| `frontend/src/stores/theme.js` | **新增**：主题偏好 ref + localStorage 持久化 + matchMedia 监听 + data-theme 应用 |
| `frontend/src/components/layout/TopBar.vue` | 新增主题切换下拉（语言与设置之间）；引入 `preference` / `setTheme` |
| `frontend/src/lib/chart.js` | `GRID_COLOR` / `TICK_COLOR` / `LABEL_COLOR` 常量替换为 `gridColor()` / `tickColor()` / `labelColor()` 动态函数 |
| `frontend/src/lib/term-theme.js` | **新增**：终端深/浅两套配色（含 16 色 ANSI 调色板），`termTheme(effective)` 按主题返回 |
| `frontend/src/pages/ShellPage.vue` | 导入 `termTheme` + `effective`，初始化和 `watch` 切换终端配色 |
| `frontend/src/pages/JailTerminalPage.vue` | 同上 |
| `frontend/src/pages/BhyveConsolePage.vue` | 同上 |
| `frontend/src/pages/MonitorCpuPage.vue` | 导入 `effective` ref，`watch(themeEff)` 触发 `drawAll()` |
| `frontend/src/pages/MonitorMemoryPage.vue` | 同上 |
| `frontend/src/pages/MonitorNetworkPage.vue` | 同上；`ifaceOptions()` 改用动态颜色函数 |
| `frontend/src/main.js` | 导入 `stores/theme.js` 以在应用启动时初始化监听 |
| `frontend/index.html` | `<head>` 新增 anti-FOUC 内联脚本 |
| `frontend/src/i18n/translations.js` | 新增 `topbar.theme` / `topbar.themeSystem` / `topbar.themeLight` / `topbar.themeDark`（en + zh） |
| `frontend/src/components/ui/ComboBox.vue` | hover 硬编码 `rgba` 替换为 `--hover-bg`、`--shadow` |
| `frontend/src/components/ui/FileTreeRow.vue` | hover `--bg-elev2` 替换为 `--hover-bg` |
| `frontend/src/pages/MailPage.vue` | modal overlay `rgba(0,0,0,0.6)` 替换为 `--modal-overlay` |

## 外部依赖

无新增依赖。仅使用浏览器原生 API：
- `localStorage` — 偏好持久化
- `window.matchMedia('(prefers-color-scheme: dark)')` — 系统主题检测
- CSS `color-scheme` 属性 — 浏览器原生表单控件（滚动条等）主题适配

## 配置项

无 `fwp.toml` 配置项。主题偏好纯前端存储（`localStorage['fwp_theme']`）。

## 已知限制

- 终端页面（ShellPage / BhyveConsolePage / JailTerminalPage）的 xterm.js 主题通过 `lib/term-theme.js` 提供深/浅两套配色，监听 `themeEff` 切换时实时更新；`.term-host` 容器背景使用 `--term-bg` 变量
- VNC 页面（BhyveVncPage）始终使用黑色背景
- Dashboard 内存分段颜色（Active/Wired/Free 等）为硬编码 hex 值，不随主题变化
- Chart.js 数据系列颜色（蓝/紫/琥珀等）为固定调色板，不随主题变化（仅网格/刻度/标签适配）
