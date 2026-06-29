# 14 — Web 终端

## 概述

在「概览 → 终端」提供一个基于浏览器的交互式 Shell，等价于一个 root 登录会话。浏览器通过 WebSocket 与服务端通信，服务端为每个会话分配一个真实的 FreeBSD 伪终端（PTY），fork+exec 一个登录 shell，并在 WebSocket 与 PTY master 之间双向搬运字节。因为是真 PTY，全屏程序（top、vi、tmux 等）、ANSI 颜色、Ctrl-C、窗口尺寸变化都正常工作。

## 实现细节

### 认证模型（WebSocket 专用）

浏览器无法在 WebSocket 握手时设置自定义请求头（如 `Authorization: Bearer`），因此终端端点**不能**复用全局的 `require_auth` 中间件。终端路由注册在 `app.rs` 的 *public* 路由组中，由 handler 自行鉴权：

- 客户端以查询参数携带会话 token：`/api/term/ws?token=<token>`
- handler（`ws_handler`）复用 `auth::hash_token` + `db::get_session_by_hash` / `db::get_user` 验证 token，失败返回 `401`
- 鉴权通过后 `audit::record(...)` 记录一条 "terminal session opened"

### PTY 分配与进程派生 `src/terminal.rs`

所有 `unsafe` 集中在本模块，对外提供安全封装：

1. **`open_pty()`** — `posix_openpt(O_RDWR|O_NOCTTY)` 创建 master，`grantpt` / `unlockpt` 授权，`ptsname_r`（线程安全版）取 slave 设备路径。返回 `(master_fd, slave_path)`。
2. **`spawn_shell()`** — `fork()`，子进程中**仅调用 async-signal-safe 函数**：
   - `setsid()` 成为新会话首
   - `open(slave, O_RDWR)` 打开 slave
   - `ioctl(slave, TIOCSCTTY, 0)` 将 slave 设为控制终端
   - `dup2(slave, 0/1/2)` 接管标准 IO
   - 关闭多余 fd，`close(master)`
   - `chdir(home)` —— 进入用户主目录，等同 ssh 登录
   - `execve(shell, argv, envp)` 替换为 shell
   - 失败则 `_exit(127)`
3. **shell / 主目录 / 用户名选择** —— `current_user_info()` 用 `getuid()` + `getpwuid()` 从 passwd 数据库解析**当前进程的有效用户**（而非硬编码 root），一次性读取 `pw_name`、`pw_shell`（登录 shell，缺失回退 `/bin/sh`）、`pw_dir`（主目录，缺失回退进程当前目录）。这样 fwp 以什么用户运行，终端就以什么用户身份开启会话。
4. **argv[0]** 设为 `-<basename>`，请求登录 shell 行为（读取 `/etc/profile`、`~/.profile` 等）。
5. **环境** —— `build_env(user)` 继承当前进程环境，强制 `HOME`/`USER`/`LOGNAME` 为解析出的用户身份、`TERM=xterm-256color`，补齐 `PATH` 默认值。

> 设计要点：fork 后到 exec 之间只调用 async-signal-safe 函数（`chdir` 也是安全的），因此在多线程 tokio 进程中 fork 是安全的（其余 tokio 线程的状态在子进程中无关紧要，因为立即 exec）。子进程显式 `chdir(home)` 而非依赖 shell 自身 cd，保证无论 fwp 进程的工作目录在哪，终端都从该用户的主目录启动（与 ssh 直接登录一致）。用户身份取自当前进程的 `geteuid`，即「fwp 以谁运行，终端就是谁」。

### 会话驱动 `run_session`

升级为 WebSocket 后，建立两个 `tokio::task::spawn_blocking` 阻塞任务 + 一个异步 select 主循环：

```
                 ┌─────────────┐
 PTY master ───► │ reader 任务 │ ──blocking_send──► out_rx ─┐
 (libc::read)    └─────────────┘                           │
                                                           ▼
                                                    ┌─────────────┐
                                                    │  select 循环 │ ── ws.send(output)
                                                    └─────────────┘
                                                    │  select 循环 │ ◄── ws.recv(input)
                                                    └──────┬──────┘
 PTY master ◄─── libc::write ── in_rx ◄─blocking_recv─ │ writer 任务 │
                                  ▲                    └─────────────┘
                                  └──────── in_tx ─────┘
```

- **reader**：`spawn_blocking` 循环 `libc::read(master, buf)`，收到字节经 `out_tx.blocking_send` 送入主循环；read 返回 ≤0（shell 退出 / master 关闭）时结束。
- **writer**：`spawn_blocking` 循环 `in_rx.blocking_recv()`，将输入 `write_all_fd()` 写入 master；输入通道关闭时结束。
- **主循环**：`tokio::select!` 在 `out_rx.recv()` 与 `ws_receiver.next()` 间多路复用。
- **默认窗口**：会话起始 `set_winsize(master, 80, 24)`。

### 窗口尺寸

`set_winsize()` 对 master 执行 `ioctl(TIOCSWINSZ, &winsize)`，内核据此向 shell 发送 `SIGWINCH`，全屏程序随之重排。

### 清理（关键）

任一端断开 / shell 退出时，主循环跳出后按序清理，确保**无僵尸进程、无 fd 泄漏**：

1. `drop(in_tx)` —— 关闭输入通道，writer 任务收到 `None` 后退出
2. `kill(pid, SIGHUP)` —— 终止 shell（此时 reader 的 `read` 返回 EOF 自然退出）
3. `waitpid(pid)`（在 `spawn_blocking` 中）—— 回收子进程
4. `close(master)` —— 关闭 PTY master
5. `await writer / reader` —— 确保两个任务结束

> PTY master 写入在 slave 关闭后返回 `EIO`（非 SIGPIPE），`write_all_fd()` 据此返回 `false` 让 writer 安全退出，不会误杀服务进程。

### 协议（JSON over WebSocket 文本帧）

客户端 → 服务端：

| type | 字段 | 说明 |
|---|---|---|
| `input` | `data: string` | 发往 shell 的按键 / 粘贴内容 |
| `resize` | `cols, rows: number` | 更新 PTY 窗口尺寸 |

（也接受二进制帧作为原始输入，便于低延迟按键直传。）

服务端 → 客户端：

| type | 字段 | 说明 |
|---|---|---|
| `output` | `data: string` | shell 的输出（`String::from_utf8_lossy` 转码） |
| `exit` | — | shell 已退出 |
| `error` | `data: string` | 会话初始化失败原因 |

### 前端 `web/js/pages/terminal.js`

- 仅在 `/shell` 路由**按需加载** xterm.js UMD 包及其 CSS（`loadScript` / `loadCss` 幂等，重复进入不重载）。
- 使用 `window.Terminal`（核心）+ `window.FitAddon.FitAddon`（自适应尺寸）。
- `term.onData` → 发 `{type:input}`；`ws.onmessage` output → `term.write`。
- `ResizeObserver` 监听容器尺寸 → `fitAddon.fit()` → `term.onResize` 发 `{type:resize}`。
- 工具栏：连接状态徽标 + 「重新连接」按钮（断开后启用）。
- **生命周期清理**：模块级 `hashchange` 监听器在离开 `/shell` 时调用 `cleanup()`（关闭 WS、`term.dispose()`、断开 observer），触发服务端清理。

### 第三方库（vendor）

`web/vendor/xterm/` 下本地副本（无构建步骤，随二进制内嵌）：

| 文件 | 来源 |
|---|---|
| `xterm.js` | `@xterm/xterm@5.5.0`（UMD，暴露 `window.Terminal`） |
| `xterm.css` | `@xterm/xterm@5.5.0` |
| `xterm-addon-fit.js` | `@xterm/addon-fit@0.10.0`（暴露 `window.FitAddon.FitAddon`） |

### 菜单与路由

- 菜单：概览 → 终端（`fa-solid fa-terminal`）
- 前端路由：`/shell` → `renderTerminal`

## API

| 方法 | 路径 | 鉴权 | 说明 |
|---|---|---|---|
| GET（Upgrade） | `/api/term/ws?token=<token>` | 查询参数 token | 升级为 WebSocket，开始终端会话 |

## 外部依赖

- 无新增系统命令；直接使用 FreeBSD 内核 PTY 接口（`posix_openpt`、`grantpt`、`unlockpt`、`ptsname_r`）
- 无新增 Rust crate（`libc`、`tokio` 已在依赖中）；前端新增 `@xterm/xterm` + `@xterm/addon-fit`（vendor 本地副本）

## 配置项

无独立配置项。终端始终可用，用户身份、登录 shell 与默认工作目录均取自当前进程的有效用户（`geteuid` + `getpwuid`），与 ssh 直接登录一致。

## 已知限制 / 安全

- **继承进程身份**：终端以 fwp 进程的有效用户身份运行（通常为 root，但若以普通用户启动 fwp 则为该用户）。任何通过认证的面板用户都能获得此身份的终端。当前无按角色限制（与其它模块一致的「已登录即可用」模型）。如需收紧，可在 `ws_handler` 中按 `user.role` 拦截。
- **输出编码**：PTY 字节流用 `from_utf8_lossy` 转为字符串，遇到非 UTF-8 字节会被替换字符吞掉（对正常 shell 使用无影响）。
- **会话数无上限**：未限制并发终端会话数；大量会话会占阻塞线程池（每会话 2 个 `spawn_blocking`）与内存。
- **无录屏/回放**：会话内容不落盘（仅审计日志记录「会话开启」事件）。
