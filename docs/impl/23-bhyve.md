# 23 — Bhyve 虚拟机管理

## 概述

Bhyve 模块封装 vm-bhyve（`/usr/local/sbin/vm`）CLI，提供虚拟机列表/详情/创建/启停、串口控制台、VNC 代理，以及镜像/交换机/数据存储/模板/ISO 的只读列表。所有操作通过 spawn `vm` 子进程完成（无 FFI）。

当前阶段已实现：
- VM 列表（`vm list`，含 All/Running 筛选）
- VM 详情（`vm info` + `.conf` 配置文件，含网络接口/磁盘/快照/控制台端口）
- VM 创建（`vm create`，可选模板/数据存储/CPU/内存/磁盘大小 + 安装 ISO）
- VM 启动/停止（`vm start` / `vm stop`）
- VM 串口控制台（WebSocket + PTY + xterm.js，通过 `cu -l` 连接 nmdm 设备）
- VM VNC 代理（WebSocket→TCP 代理 + noVNC 前端）
- vm-bhyve 镜像列表（`vm image list`）
- 虚拟交换列表（`vm switch list`）
- 数据存储列表（`vm datastore list`）
- 数据存储添加/移除（`vm datastore add` / `vm datastore remove`）
- 模板列表（读取 `/vm/.templates/`）
- ISO 列表（读取 `/vm/.iso/`）
- vm-bhyve 初始化检测与引导（检测软件包/rc.conf 配置/init 状态，提供一键初始化）

未实现：VM 删除、配置编辑、快照/克隆/回滚、重启/强制停止/挂起、ISO 下载、交换机创建/删除。

## 实现细节

### 双文件结构

| 文件 | 职责 |
|---|---|
| `src/bhyve.rs` | CLI 封装器：命令执行 + 表格/多段输出解析 + 数据模型 |
| `src/handlers/bhyve.rs` | HTTP handlers（axum），调用 `bhyve::*` 函数，处理校验和审计 |

### CLI 封装器 `src/bhyve.rs`

#### 命令执行助手

- `vm_run(args)` — 捕获 stdout（`:bhyve.rs:15`）。stdin 重定向到 `/dev/null` 防交互等待。
- `start_vm(name)` — **使用 `.status()` 而非 `.output()`**（`:bhyve.rs:490`）。`vm start` 会 fork 长驻 bhyve 进程，管道不关闭导致 `.output()` 永久阻塞（与 `jail -c` 相同的坑）。仅捕获 stderr 以便报错。
- `stop_vm(name)` — 使用 `.output()`，正常终止的命令（`:bhyve.rs:505`）。

#### 表格解析器

vm-bhyve 输出固定列宽表格（`vm list` / `vm switch list` / `vm datastore list` / `vm image list`），列值可能含空格（如 `Running (4272)`）。

解析策略（`:bhyve.rs:36-85`）：
1. `parse_header(header)` — 提取每列名的字节偏移 → `Vec<Column { label, offset }>`
2. `col_value(line, cols, idx)` — 按 `cols[idx].offset` 到 `cols[idx+1].offset` 切片
3. `col_index(cols, label)` — 按列名查找索引（大小写不敏感）

#### `vm list` 解析（`:bhyve.rs:128`）

STATE 列解析（`:bhyve.rs:213`）：
- `Stopped` → `("stopped", None, None)`
- `Running (4272)` → `("running", Some(4272), None)`
- `Locked (ppbsd)` → `("locked", None, Some("ppbsd"))`

AUTO 列解析（`:bhyve.rs:193`）：
- `No` → `(false, None)`
- `Yes` → `(true, None)`
- `Yes [1]` → `(true, Some(1))`

#### `vm info` 解析（`:bhyve.rs:581`）

`vm info` 输出为多段缩进格式：顶层 `key: value` + 子段（`network-interface` / `virtual-disk` / `snapshots` / `console-ports`），子段有标题行后跟缩进的 `key: value`。

解析器（`:bhyve.rs:586`）：
1. 跳过标题块（`-----` + `Virtual Machine: name` + `-----`）
2. 逐行扫描：无冒号的行视为子段标题，后续缩进行为子段内容
3. snapshots 子段特殊处理：制表符分隔（`name<TAB>size<TAB>date`），日期含冒号不可用 `key:value` 分割
4. 读取 `/vm/<name>/<name>.conf` 作为 `config` 字段（`:bhyve.rs:810`）
5. 从 config 中提取 VNC 端口（`graphics="yes"` + `graphics_port`）

### 数据模型

```rust
// 列表项（vm list 解析）
pub struct VmSummary { name, datastore, loader, cpu, memory, vnc: Option, auto_start: bool, auto_order: Option, state: String, pid: Option, locked_by: Option }

// 详情（vm info + .conf 合并）
pub struct VmDetail { name, state, datastore, loader, uuid, cpu, memory, memory_resident: Option, console_port: Option, networks: Vec<VmNetwork>, disks: Vec<VmDisk>, snapshots: Vec<VmSnapshot>, config: BTreeMap, vnc_port: Option<u16> }

pub struct VmNetwork { number, emulation, virtual_switch: Option, mac_address: Option, active_device: Option, bytes_in: Option, bytes_out: Option }
pub struct VmDisk { number, device_type, emulation, system_path, bytes_size: Option, bytes_used: Option }
pub struct VmSnapshot { name, size, date }
pub struct VmImage { uuid, name, created, description }
pub struct VmSwitch { name, typ, iface, address: Option, private: bool, mtu: Option, vlan: Option, ports: Vec }
pub struct VmDatastore { name, typ, path, zfs_dataset: Option }
pub struct IsoImage { name, size: u64 }
```

### HTTP Handlers `src/handlers/bhyve.rs`

所有写操作（create/start/stop）使用 `tokio::task::spawn_blocking` 包装（VM 命令含 ZFS/磁盘 I/O，阻塞）。每步操作记录审计日志。

VM 名称校验（`:handlers/bhyve.rs:176`）：小写字母+数字+`.`/`_`/`-`，首尾必须字母或数字（遵循 vm-bhyve `util::check_name` 规则）。数据存储名称校验使用相同规则（`:handlers/bhyve.rs:248`）。

### 数据存储管理

`add_datastore(name, spec)` / `remove_datastore(name)` 封装 `vm datastore add/remove`。

spec 格式（由前端构造）：
- ZFS：`zfs:pool/dataset`
- 目录：`/path/to/dir`
- ISO：`iso:/path`
- IMG：`img:/path`

`default` 数据存储不可删除（后端 + 前端双重拦截）。移除操作仅删除 vm-bhyve 配置条目，不删除实际数据。

### vm-bhyve 初始化检测与引导

#### 检测逻辑（`bhyve::check_status()`）

返回 `BhyveStatus { installed, enabled, vm_dir, initialized, resolved_path }`：
- `installed` — `/usr/local/sbin/vm` 文件是否存在
- `enabled` — `sysrc -n vm_enable` 是否为 `YES`
- `vm_dir` — `sysrc -n vm_dir` 的值（如 `zfs:zroot/vm` 或 `/home/vm`）
- `initialized` — 解析后的路径下 `.config/` 目录是否存在（`vm init` 创建）
- `resolved_path` — ZFS 类型时查询 `zfs get mountpoint` 得到的实际路径

`resolve_vm_dir()` 辅助函数：`zfs:pool/dataset` → 查询 ZFS mountpoint；纯路径直接返回。

#### 初始化流程（`bhyve::init_bhyve(spec)`）

1. `pkg install -y vm-bhyve bhyve-firmware grub2-bhyve`
2. `sysrc vm_enable=YES` + `sysrc vm_dir=<spec>`
3. 准备存储：ZFS 类型则 `zfs create <dataset>`（若不存在）；目录类型则 `mkdir -p <path>`
4. `vm init` — 加载内核模块（nmdm/if_bridge/if_tuntap），创建 `.config`/`.templates`/`.iso`/`.img`/`null.iso`
5. 复制 `/usr/local/share/examples/vm-bhyve/*` 到 `<resolved_path>/.templates/`

返回步骤描述列表供前端展示进度。

#### 前端交互

- 进入 `/bhyve/vms` 时先调用 `GET /api/bhyve/status`
- 若未初始化，显示警告卡片（列出缺失项）+ 初始化按钮，跳转到 `/bhyve/init`
- `BhyveInitPage` 提供存储类型选择（ZFS/目录）+ 对应输入项，提交后显示步骤进度
- 初始化完成后自动跳转回 VM 列表

### 串口控制台

复用 `src/terminal.rs` 的 WebSocket 终端架构（`:terminal.rs:47` 的 `ws_handler`），新增 `SpawnTarget::Bhyve` 变体（`:terminal.rs:274`）。

连接流程：
1. 浏览器 WS 连接 `/api/term/ws?vm=<name>&token=<token>`
2. 后端读取 `/vm/<name>/console` 文件获取 nmdm 设备路径（如 `com1=/dev/nmdm-alpine.1B`）
3. fork+exec `cu -l /dev/nmdm-alpine.1B`（环境含 `HOME=/root` 消除 tiprc 警告）
4. PTY 双向桥接：浏览器 keystroke → PTY master → `cu` → nmdm → bhyve 串口

### VNC 代理（WebSocket→TCP）

VNC 端点 `/api/bhyve/vms/{name}/vnc`（`:terminal.rs:554`），位于公开路由（与终端相同原因：浏览器无法在 WS 握手中设置 Authorization header）。

代理流程：
1. 验证 token → 读取 VM 的 VNC 端口（`bhyve::get_vnc_port()`）
2. 仅对 `graphics="yes"` 的 VM 生效（否则返回 400）
3. WS 升级时协商 `binary` 子协议（noVNC 要求）
4. 建立 TCP 连接到 `127.0.0.1:<port>`
5. 双向异步管道：TCP→WS（Binary 帧）+ WS→TCP（Binary/Text 帧）

前端使用 noVNC（`@novnc/novnc` npm 包）的 RFB 客户端渲染到 Canvas。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/bhyve/vms` | 列出所有 VM（`?running=true` 仅运行中） |
| POST | `/api/bhyve/vms` | 创建 VM（body: name/template/datastore/size/cpu/memory） |
| GET | `/api/bhyve/vms/{name}` | VM 详情（vm info + .conf） |
| POST | `/api/bhyve/vms/{name}/start` | 启动 VM |
| POST | `/api/bhyve/vms/{name}/stop` | 停止 VM |
| GET | `/api/bhyve/images` | vm-bhyve 镜像列表 |
| GET | `/api/bhyve/switches` | 虚拟交换列表 |
| GET | `/api/bhyve/status` | vm-bhyve 安装/配置状态 |
| POST | `/api/bhyve/init` | 初始化 vm-bhyve（body: spec） |
| GET | `/api/bhyve/datastores` | 数据存储列表 |
| POST | `/api/bhyve/datastores` | 添加数据存储（body: name/spec） |
| DELETE | `/api/bhyve/datastores/{name}` | 移除数据存储 |
| GET | `/api/bhyve/templates` | 可用模板列表 |
| GET | `/api/bhyve/isos` | 可用 ISO 列表 |
| WS | `/api/term/ws?vm=<name>&token=<token>` | 串口控制台（复用终端 WS） |
| WS | `/api/bhyve/vms/{name}/vnc?token=<token>` | VNC 代理（WS→TCP） |

## 外部依赖

- **系统命令**：`/usr/local/sbin/vm`（vm-bhyve 1.7.3）、`/usr/bin/cu`（串口连接）
- **系统文件**：`/vm/<name>/<name>.conf`（VM 配置）、`/vm/<name>/console`（nmdm 设备映射）、`/vm/.templates/*.conf`（模板）、`/vm/.iso/*`（ISO）、`/vm/.config/system.conf`（全局配置）
- **Rust crate**：无额外（复用 axum WS + tokio TCP）
- **前端库**：`@novnc/novnc`（VNC 客户端）、`@xterm/xterm`（串口终端）

## 前端

| 页面 | 路由 | 文件 |
|---|---|---|
| 虚拟机列表 | `/bhyve/vms` | `BhyveVmsPage.vue` |
| 创建虚拟机 | `/bhyve/create` | `BhyveCreatePage.vue` |
| VM 详情 | `/bhyve/detail/:name` | `BhyveDetailPage.vue` |
| 串口控制台 | `/bhyve/console/:name` | `BhyveConsolePage.vue` |
| VNC | `/bhyve/vnc/:name` | `BhyveVncPage.vue` |
| 镜像列表 | `/bhyve/images` | `BhyveImagesPage.vue` |
| 虚拟交换 | `/bhyve/switches` | `BhyveSwitchesPage.vue` |
| 存储池 | `/bhyve/datastores` | `BhyveDatastoresPage.vue` |
| 初始化 | `/bhyve/init` | `BhyveInitPage.vue` |

侧边栏菜单：`virtualization` → `bhyve`（带 4 个子项：虚拟机/镜像/虚拟交换/存储池）。初始化页面不在侧边栏，通过 VM 列表页的引导按钮进入。

VNC 按钮仅在 `vm.vnc`（列表）或 `vm.vnc_port`（详情）存在且 VM 运行时显示。

`vite.config.js` 特殊配置：`build.target: 'es2022'` + `optimizeDeps.esbuildOptions.target: 'es2022'`（noVNC 使用 top-level await）。

## 已知限制 / TODO

- VM 删除（`vm destroy`）、配置编辑、快照/克隆/回滚未实现
- ISO 下载（后台任务）未实现
- 交换机创建/删除未实现
- 数据存储添加时无法自动创建 ZFS dataset 或目录（vm-bhyve 要求用户预先创建）
- `vm start` 使用 `.status()` — 如果 bhyve 启动失败，错误信息可能不够详细（stderr 未捕获到 spawn 端）
- VNC 无密码认证（bhyve fbuf 不支持），依赖面板 token 认证保护 WS 端点
- `vm info` 的 snapshots 段解析依赖制表符分隔格式，未来 vm-bhyve 版本变更可能需适配
