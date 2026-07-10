# 模块设计：Bhyve 虚拟机管理

> 依赖：`vm-bhyve` 1.7.3（已安装在 `/usr/local/sbin/vm`）。封装其 CLI，解析表格输出。
>
> **实现状态**：M1（命令封装 + 列表/详情解析）、M2（VM CRUD + 生命周期 start/stop）、M5（控制台 + VNC）已完成。
> 实现文档见 `docs/impl/23-bhyve.md`。

## 1. 调用契约

所有操作通过子进程执行 `vm <command>`，`vm-bhyve` 自身负责 bhyve/nmdm/grub-bhyve 等底层调用。
面板职责：命令封装 + 输出解析 + 状态查询 + 配置文件编辑。

### 1.1 vm-bhyve 命令映射（基于实际 `vm help` 输出）

| 功能 | 命令 | 备注 |
|---|---|---|
| 列表 | `vm list` | 表格：NAME/DATASTORE/LOADER/CPU/MEMORY/VNC/AUTO/STATE |
| 详情 | `vm info <name>` | 多段 key:value |
| 创建 | `vm create [-d ds] [-t template] [-s size] [-m mem] [-c cpu] [-i image] <name>` | |
| 安装 | `vm install [-fi] <name> <iso>` | `-f` 强制重装 |
| 启动 | `vm start [-fi] <name>` | `-f` 强制（已运行时重启） |
| 停止 | `vm stop <name>` | 优雅关机 |
| 强制停 | `vm destroy [-f] <name>` | 直接断电 |
| 重启 | `vm restart <name>` | |
| 挂起 | `vm suspend <name>` | |
| 控制台 | `vm console [-w] <name> [com1\|com2]` | `-w` 等待 |
| 编辑配置 | `vm edit <name>` | 编辑 `/vm/<name>/<name>.conf` |
| 重命名 | `vm rename <old> <new>` | |
| 加盘/网卡 | `vm add [-d device] [-t type] [-s size\|switch] <name>` | |
| 快照 | `vm snapshot [-f] <name@snap>` | |
| 回滚 | `vm rollback [-r] <name@snap>` | `-r` 删除当前磁盘 |
| 克隆 | `vm clone <name@snap> <new>` | |
| ISO 管理 | `vm iso [url]` | 下载/列出 |
| 开机自启 | `vm list` 中 AUTO 列 + `/vm/.config/system.conf` | |
| 全部启动 | `vm startall` | rc.d 调用 |
| 全部停止 | `vm stopall [-f]` | |
| 虚拟交换 | `vm switch list/create/destroy/add/remove/vlan/nat` | |
| 数据存储 | `vm datastore list/add/remove` | ✅ list + add + remove |
| 直通 | `vm passthru` | PCI 设备直通 |

## 2. 输出解析

### 2.1 `vm list` 解析

实际输出格式（已采样）：
```
NAME      DATASTORE  LOADER     CPU  MEMORY  VNC           AUTO     STATE
alpine    default    uefi       2    1G      -             No       Locked (ppbsd)
ubuntu    default    grub       4    4G      -             Yes [1]  Running (4272)
```

解析策略：
- 跳过表头行
- 按多空格分列（STATE 列含空格，需按表头列宽固定切分）
- 更稳健方式：**使用列起始位置**（表头每列名首字符位置即为数据列起始）
- STATE 提取状态枚举：`Stopped | Running (pid) | Locked (host) | Suspended`

### 2.2 `vm info <name>` 解析

```
Virtual Machine: ubuntu
  state: running
  cpu: 4
  memory: 4G
  network-interface: interface=vmx0,bridge=public
  disk: disk0
  ...
```
逐行 `key: value`，部分值含逗号分隔子字段。

### 2.3 VM 配置文件（`/vm/<name>/<name>.conf`）

```ini
loader="grub"
cpu=4
memory=4G
network0_type="virtio-net"
network0_switch="public"
disk0_type="nvme"
disk0_name="disk0.img"
```
普通 INI/key=value，直接复用通用 config 解析器。

## 3. 数据模型

```rust
struct Vm {
    name: String,
    datastore: String,
    loader: VmLoader,           // bhyveload | grub | uefi
    cpu: u32,
    memory: String,             // "4G" | "512M"
    vnc: Option<String>,        // "0.0.0.0:8010" 或 None
    auto_start: bool,
    state: VmState,
    pid: Option<u32>,
    locked_by: Option<String>,
}

enum VmState { Stopped, Running(u32), Suspended, Locked(String) }
enum VmLoader { Bhyveload, Grub, Uefi }

struct VmSwitch {
    name: String,
    typ: String,                // standard | manual
    ports: Vec<String>,         // 物理接口 / vlan
    address: Option<String>,
    nat: bool,
}

struct IsoImage { name: String, size: u64 }
```

## 4. API 设计（✅ = 已实现）

| 方法 | 路径 | 说明 | 状态 |
|---|---|---|---|
| GET | `/api/bhyve/vms` | 列出所有 VM（`?running=true` 仅运行中） | ✅ |
| GET | `/api/bhyve/vms/{name}` | VM 详情（vm info + .conf） | ✅ |
| POST | `/api/bhyve/vms` | 创建 VM（body: name/template/cpu/mem/size/datastore） | ✅ |
| POST | `/api/bhyve/vms/{name}/start` | 启动 | ✅ |
| POST | `/api/bhyve/vms/{name}/stop` | 优雅停止 | ✅ |
| DELETE | `/api/bhyve/vms/{name}` | 删除 VM | |
| POST | `/api/bhyve/vms/{name}/destroy` | 强制断电 | |
| POST | `/api/bhyve/vms/{name}/restart` | 重启 | |
| POST | `/api/bhyve/vms/{name}/install` | 挂载 ISO 安装 | |
| PUT | `/api/bhyve/vms/{name}` | 修改 VM 配置 | |
| WS | `/api/term/ws?vm=<name>` | 串口控制台（复用终端 WS） | ✅ |
| WS | `/api/bhyve/vms/{name}/vnc` | VNC 代理（WS→TCP） | ✅ |
| GET | `/api/bhyve/images` | 镜像列表 | ✅ |
| GET | `/api/bhyve/switches` | 虚拟交换列表 | ✅ |
| GET | `/api/bhyve/datastores` | 数据存储列表 | ✅ |
| GET | `/api/bhyve/templates` | 模板列表 | ✅ |
| GET | `/api/bhyve/isos` | ISO 列表 | ✅ |

## 5. 控制台访问

### 串口控制台（✅ 已实现）

复用 `/api/term/ws`（WebSocket 终端），新增 `SpawnTarget::Bhyve` 变体。
- 浏览器 WS 连接 `/api/term/ws?vm=<name>&token=<token>`
- 后端从 `/vm/<name>/console` 读取 nmdm 设备路径（如 `com1=/dev/nmdm-{name}.1B`）
- fork+exec `cu -l <device>`（环境含 `HOME=/root` 消除 tiprc 警告）
- PTY 双向桥接到 bhyve 串口

### VNC（✅ 已实现）

WebSocket→TCP 代理（Rust 内建），无外部依赖。
- 端点 `/api/bhyve/vms/{name}/vnc?token=<token>`（公开路由）
- 从 `.conf` 读取 `graphics="yes"` + `graphics_port` → 代理到 `127.0.0.1:<port>`
- WS 握手协商 `binary` 子协议（noVNC 要求）
- 前端使用 noVNC（`@novnc/novnc`）RFB 客户端渲染到 Canvas
- VNC 按钮仅在 VM 启用了 graphics 且运行中时显示

## 6. 实现里程碑

1. **M1 ✅** — 命令封装器 + 列表/详情解析（`vm list`/`info`/`switch list`/`image list`/`datastore list`/templates/ISO）
2. **M2 ✅** — VM CRUD + 生命周期 API（create/start/stop）
3. **M3** — 快照/克隆/ISO 管理 API（未开始）
4. **M4** — 交换机/数据存储管理 API（未开始，只读列表已完成）
5. **M5 ✅** — 控制台 + VNC（串口控制台 + VNC 代理）
6. **M6** — 配置编辑 + VM 删除（未开始）

## 7. 风险与缓解

| 风险 | 缓解 |
|---|---|
| `vm list` 输出格式跨版本不稳定 | 解析时校验表头列名，不匹配则报错 |
| `vm start` fork 长驻进程致 `.output()` 阻塞 | 使用 `.status()` 而非 `.output()`（同 jail.rs 模式） |
| `vm info` snapshots 段日期含冒号 | snapshots 子段使用制表符分割，不使用 `key:value` 解析 |
| `cu` 启动时 `$HOME not set` 警告 | 在子进程环境设置 `HOME=/root` |
| noVNC 需 `binary` WS 子协议 | WS 握手时 `.protocols(["binary"])` |
| noVNC 使用 top-level await | `vite.config.js` 设 `target: 'es2022'`（build + optimizeDeps） |
| VNC 无密码认证 | bhyve fbuf 仅监听 127.0.0.1，面板 token 认证保护 WS 端点 |
| VM 配置文件手动编辑冲突 | 读写用原子替换；编辑前展示当前内容 diff（计划中） |
| 长时间命令（iso 下载） | 转为后台任务 + 任务 ID 查询进度（见 `70-task-queue.md`） |
