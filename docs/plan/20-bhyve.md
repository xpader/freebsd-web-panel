# 模块设计：Bhyve 虚拟机管理

> 依赖：`vm-bhyve` 1.7.3（已安装在 `/usr/local/sbin/vm`）。封装其 CLI，解析表格输出。

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
| 数据存储 | `vm datastore list/add/remove` | |
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

## 4. API 设计

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/vms` | 列出所有 VM |
| GET | `/api/vms/:name` | VM 详情 |
| POST | `/api/vms` | 创建 VM（参数：name/template/cpu/mem/size/datastore） |
| PUT | `/api/vms/:name` | 修改 VM 配置（停机时改 cpu/mem/disk） |
| DELETE | `/api/vms/:name` | 删除 VM（含磁盘，需二次确认） |
| POST | `/api/vms/:name/start` | 启动 |
| POST | `/api/vms/:name/stop` | 优雅停止 |
| POST | `/api/vms/:name/destroy` | 强制断电 |
| POST | `/api/vms/:name/restart` | 重启 |
| POST | `/api/vms/:name/install` | 挂载 ISO 安装（body: iso 名） |
| GET | `/api/vms/:name/console` | WebSocket VNC 代理 或 nmdm 文本控制台 |
| POST | `/api/vms/:name/snapshot` | 创建快照（body: snap 名称） |
| POST | `/api/vms/:name/rollback` | 回滚快照 |
| POST | `/api/vms/:name/clone` | 克隆（body: newname） |
| GET | `/api/vms/:name/snapshots` | 列出快照 |
| GET | `/api/isos` | 列出可用 ISO |
| POST | `/api/isos` | 下载 ISO（body: url） |
| DELETE | `/api/isos/:name` | 删除 ISO |
| GET | `/api/vm-switches` | 列出虚拟交换 |
| POST | `/api/vm-switches` | 创建交换 |
| DELETE | `/api/vm-switches/:name` | 删除交换 |
| GET | `/api/vm-datastores` | 列出数据存储 |

## 5. 控制台访问

vm-bhyve 控制台通过 `vm console <name>` 进入 nmdm 伪终端。
- WebSocket 端点 `/api/vms/:name/console` 分配 nmdm 设备（`/dev/nmdm<N>A`），面板 fork 进程 attach B 端
- VNC：若 VM 配置了 `vnc=0.0.0.0:port`，前端直接连该端口（或面板做 WebSocket→TCP 代理）

## 6. 实现里程碑

1. **M1 — 命令封装器**（`VmCmd::run()` 带超时和错误捕获）+ `vm list`/`info` 解析
2. **M2 — VM CRUD + 生命周期 API**（start/stop/destroy/restart/install）
3. **M3 — 快照/克隆/ISO API**
4. **M4 — 交换机/数据存储 API**
5. **M5 — 控制台 WebSocket**

## 7. 风险与缓解

| 风险 | 缓解 |
|---|---|
| `vm list` 输出格式跨版本不稳定 | 解析时校验表头列名，不匹配则降级为按空格分割 + 警告日志 |
| `vm` 命令可能交互式等待（console/install） | 非交互命令统一加 `-f` 或重定向 stdin 到 `/dev/null`；交互场景隔离到 WebSocket 通道 |
| VM 配置文件手动编辑冲突 | 读写用原子替换；编辑前展示当前内容 diff |
| 长时间命令（iso 下载） | 转为后台任务 + 任务 ID 查询进度（见 `70-task-queue.md`） |
