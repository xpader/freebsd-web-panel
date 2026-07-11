# 23 — Bhyve 虚拟机管理

## 概述

Bhyve 模块封装 vm-bhyve（`/usr/local/sbin/vm`）CLI，提供虚拟机列表/详情/创建/启停、串口控制台、VNC 代理，以及镜像、交换机、数据存储、模板和 ISO 管理。所有操作通过 spawn `vm` 子进程完成（无 FFI）。

当前阶段已实现：
- VM 列表（`vm list`，含 All/Running 筛选）
- VM 详情（`vm info` + `.conf` 配置文件，含网络接口/磁盘/快照/控制台端口）与完整 VM 配置编辑
- VM 创建（`vm create`，可选模板/数据存储/CPU/内存/磁盘大小 + 安装 ISO）
- VM 启动/停止（`vm start` / `vm stop`）
- VM 串口控制台（WebSocket + PTY + xterm.js，通过 `cu -l` 连接 nmdm 设备）
- VM VNC 代理（WebSocket→TCP 代理 + noVNC 前端）
- vm-bhyve 镜像列表（`vm image list`）
- 交换机列表/详情/创建/删除（`vm switch list` / `vm switch info` / `vm switch create` / `vm switch destroy`）
- 数据存储列表（`vm datastore list`）
- 数据存储添加/移除（`vm datastore add` / `vm datastore remove`）
- 模板列表（读取默认数据存储的 `.templates/`）
- ISO 列表（读取默认数据存储的 `.iso/`）
- vm-bhyve 初始化检测与引导（检测软件包/rc.conf 配置/init 状态，提供一键初始化）

未实现：VM 删除、快照/克隆/回滚、重启/强制停止/挂起、ISO 下载。

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
4. 遍历 `vm datastore list` 的存储路径，定位 `<datastore-path>/<name>/<name>.conf` 作为 `config` 字段
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

### VM 配置编辑

`PUT /api/bhyve/vms/{name}` 接收完整 `config` 键值映射，独立编辑页支持修改、增加和删除任意 vm-bhyve 配置项。键仅允许小写字母、数字和下划线；值拒绝换行和 NUL，避免生成额外配置项。

编辑页 `/bhyve/edit/:name` 复用 Jail 创建页的 `SectionCard` 组件，以 Tab 模式按基础设置、磁盘设备、网络设备、VNC/图形、其他设备分区。磁盘与网络区按 `disk<N>_*` / `network<N>_*` 识别配置项，支持新增和删除完整设备组；高级参数不作为独立 Tab，而是作为单独 card 放在 SectionCard 下方，保留未分类参数的完整编辑能力。页面顶部使用 `WarnBanner` 组件提示“修改将在虚拟机下次启动时生效”，底部使用 `form-actions-bar` 放置取消和保存按钮。

参数控件依据 vm-bhyve 1.7.3 的 `config.sample`：
- 枚举选择：loader（bhyveload/grub/uefi/uefi-csm）、磁盘模拟类型（virtio-blk/ahci-hd/ahci-cd/nvme/virtio-9p）、磁盘后端（file/zvol/sparse-zvol/custom/iscsi）、网卡模拟类型（virtio-net/e1000）、VNC 分辨率、等待策略和 VGA 模式。
- `virtio-9p` 使用专用字段：共享名与主机目录，通过目录选择器组合为 bhyve 所需的 `disk<N>_name="共享名=/绝对路径"`，并强制 `disk<N>_dev=custom`；`disk<N>_opts` 可填 `ro` 以只读导出共享目录。
- 勾选：wired_memory、uefi_vars、ignore_msr、utctime、debug、virt_random、network span、graphics、xhci_mouse、sound。勾选项使用 `checkbox-label` 布局，勾选框靠左、描述文字跟在后面（与 Jail 编辑页一致）。
- 文件选择器：bhyveload_loader、自定义磁盘 `disk<N>_name`（仅 dev=custom）与 sound_play/sound_rec。
- 字段标签：专用字段显示简短 EN/ZH 名称，详细说明通过 `FieldHelp` 组件以可点击信息图标 tooltip 呈现；高级区将未知键显示为“高级参数（原始键名）”。
- 其他设备：按编号管理 `passthru<N>` PCI 直通和 `virt_console<N>` virtio 控制台。

后端按 VM 实际所在 datastore 定位配置文件，先创建同目录的 `.conf.fwp.bak` 备份，再写入 `.conf.fwp.tmp` 并以 `rename` 原子替换。写入前先读取原配置文件，将前端未提交的键（即原配置中存在但 UI 未展示的字段，如 `network0_span`、`network0_device`、`hostbridge`、`comports`、`cpu_sockets` 等）保留合并到新配置中，避免编辑保存导致这些字段丢失。合并后的配置按固定优先级排序输出：先 loader 与引导相关（loader、bhyveload_loader、bhyveload_args、loader_timeout、grub_install0、grub_run0），然后 CPU（cpu、cpu_sockets、cpu_cores、cpu_threads），再内存（memory、wired_memory），其余按键名字典序跟在后面。所有值统一写为双引号包裹。运行中 VM 的配置在下次启动生效。

### 交换机创建

`create_switch()` 按参数数组调用 `vm switch create`：`-t type`、可选 `-i interface`、`-n vlan`、`-b bridge`、`-a address/prefix`、`-m mtu` 与 `-p`。

前端根据交换机类型显示有效字段：standard 支持接口、VLAN、CIDR 地址、MTU 和隔离；manual 强制现有 bridge；VXLAN 强制接口和 VLAN；netgraph 与 VALE 不接受创建参数。后端重复校验类型、VXLAN/manual 的必填组合、VLAN 范围（0-4094）、MTU 范围（100-9000）和 IPv4 CIDR 格式。

`get_switch_info()` 调用 `vm switch info <name>`，解析缩进的 `key: value` 字段为有序键值映射，详情页展示类型、接口标识、VLAN、物理端口和收发字节等实际可用字段。

`destroy_switch()` 调用 `vm switch destroy <name>`；前端删除前强制确认，警告已连接 VM 会失去网络连接，成功后刷新列表。

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
| PUT | `/api/bhyve/vms/{name}` | 替换 VM 配置（body: config 键值映射） |
| GET | `/api/bhyve/vms/{name}/disk-resources` | 磁盘可选资源（VM 目录文件 + 数据集 ZVOL） |
| POST | `/api/bhyve/vms/{name}/disks` | 创建并附加新磁盘（body: disk_type/size，调用 `vm add`） |
| DELETE | `/api/bhyve/vms/{name}/disks/{index}` | 从配置中移除磁盘（不删除物理数据） |
| DELETE | `/api/bhyve/vms/{name}/networks/{index}` | 从配置中移除网络适配器 |
| POST | `/api/bhyve/vms/{name}/start` | 启动 VM |
| POST | `/api/bhyve/vms/{name}/stop` | 停止 VM |
| GET | `/api/bhyve/images` | vm-bhyve 镜像列表 |
| GET | `/api/bhyve/switches` | 交换机列表 |
| GET | `/api/bhyve/switches/{name}` | 交换机详情 |
| POST | `/api/bhyve/switches` | 创建交换机（body: name/type/iface/vlan/bridge/address/mtu/private） |
| DELETE | `/api/bhyve/switches/{name}` | 删除交换机 |
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
- **系统文件**：默认数据存储 PATH 下的 `<name>/<name>.conf`（VM 配置）、`<name>/console`（nmdm 设备映射）、`.templates/*.conf`（模板）、`.iso/*`（ISO）与 `.config/system.conf`（全局配置）。默认路径由 `vm datastore list` 中名称为 `default` 的条目确定，不固定为 `/vm`。
- **Rust crate**：无额外（复用 axum WS + tokio TCP）
- **前端库**：`@novnc/novnc`（VNC 客户端）、`@xterm/xterm`（串口终端）

## 前端

| 页面 | 路由 | 文件 |
|---|---|---|
| 虚拟机列表 | `/bhyve/vms` | `BhyveVmsPage.vue` |
| 创建虚拟机 | `/bhyve/create` | `BhyveCreatePage.vue` |
| VM 详情 | `/bhyve/detail/:name` | `BhyveDetailPage.vue` |
| VM 配置 | `/bhyve/edit/:name` | `BhyveEditPage.vue` |
| 串口控制台 | `/bhyve/console/:name` | `BhyveConsolePage.vue` |
| VNC | `/bhyve/vnc/:name` | `BhyveVncPage.vue` |
| 镜像列表 | `/bhyve/images` | `BhyveImagesPage.vue` |
| 虚拟交换机 | `/bhyve/switches` | `BhyveSwitchesPage.vue` |
| 交换机详情 | `/bhyve/switches/:name` | `BhyveSwitchDetailPage.vue` |
| 存储池 | `/bhyve/datastores` | `BhyveDatastoresPage.vue` |
| 初始化 | `/bhyve/init` | `BhyveInitPage.vue` |

侧边栏菜单：`virtualization` → `bhyve`（带 4 个子项：虚拟机/镜像/虚拟交换机/存储池）。初始化页面不在侧边栏，通过 VM 列表页的引导按钮进入。

VNC 按钮仅在 `vm.vnc`（列表）或 `vm.vnc_port`（详情）存在且 VM 运行时显示。

`vite.config.js` 特殊配置：`build.target: 'es2022'` + `optimizeDeps.esbuildOptions.target: 'es2022'`（noVNC 使用 top-level await）。

## 已知限制 / TODO

- VM 删除（`vm destroy`）、快照/克隆/回滚未实现
- ISO 下载（后台任务）未实现
- 数据存储添加时无法自动创建 ZFS dataset 或目录（vm-bhyve 要求用户预先创建）
- `vm start` 使用 `.status()` — 如果 bhyve 启动失败，错误信息可能不够详细（stderr 未捕获到 spawn 端）
- VNC 无密码认证（bhyve fbuf 不支持），依赖面板 token 认证保护 WS 端点
- `vm info` 的 snapshots 段解析依赖制表符分隔格式，未来 vm-bhyve 版本变更可能需适配

## 磁盘配置：diskX_dev 与 diskX_name 的关系

vm-bhyve 中 `diskX_dev`（存储后端类型）和 `diskX_name`（磁盘名称/路径）配合使用，`name` 的含义随 `dev` 类型而变：

| `dev` | `name` 含义 | 示例 |
|---|---|---|
| `file` | guest 目录下的文件名 | `disk0.img` |
| `zvol` | guest 数据集下创建的 ZVOL 名称（仅名称，非完整路径） | `disk1` → `<dataset>/<vm>/disk1` |
| `custom` | 任意完整路径，包括 `/dev/zvol/...` 设备路径 | `/dev/zvol/zroot/disks/disk1` |
| `iscsi` | iSCSI 会话目标 `session[/lun]` | `1/0` |

关键点：
- `zvol` 的 `name` 只能是相对名称，vm-bhyve 会自动拼接为 `<VM_DS_ZFS_DATASET>/<vm_name>/<name>`。
- 如需使用 guest 数据集之外的 ZVOL，必须使用 `custom` 类型并在 `name` 中填写完整的 `/dev/zvol/...` 路径。
- 编辑界面已移除 `sparse-zvol` 选项（`zvol` 和 `sparse-zvol` 在运行时完全等价，区别仅在 `vm create` 创建磁盘时是否使用 `zfs create -s`）。如果配置文件中已有 `sparse-zvol`，加载时自动归一化为 `zvol`。
- `diskX_dev` 值为 `file` 时不写入配置文件（`file` 是 vm-bhyve 的默认值），加载时缺少该字段的磁盘自动补为 `file`。
- `diskX_opts` 为空时不写入配置文件，避免产生无意义的空配置项。

### 磁盘管理架构（独立即时操作）

磁盘与其它配置（基础设置、网络、图形等）完全解耦，采用独立的即时操作模式：

- **磁盘列表视图**：磁盘以只读卡片列表展示（索引、模拟设备、数据类型、名称/路径、选项），每项右侧有编辑和删除按钮。
- **编辑磁盘**：点击编辑弹出模态框，修改后即时 PUT 保存该磁盘的配置键（不影响其它未保存的配置）。
- **导入磁盘**：点击导入弹出模态框（与编辑相同的表单），选择已有文件/ZVOL 进行关联，保存后即时生效。
- **创建磁盘**：弹出表单（类型：ZVol/稀疏 ZVol/文件 + 大小），调用 `POST /api/bhyve/vms/{name}/disks`（`vm add -d disk -t <type> -s <size> <name>`），物理创建磁盘镜像并附加到配置。
- **删除磁盘**：确认后调用 `DELETE /api/bhyve/vms/{name}/disks/{index}`，从配置文件中移除该磁盘的所有键（`disk{N}_*`），不删除物理磁盘文件或 ZVol。
- 所有磁盘操作后自动刷新磁盘列表和可选资源，不影响其它标签页的未保存配置。

磁盘名称字段根据 `dev` 类型约束：
  - `file`：下拉选择 VM 目录下已有的磁盘文件（`.img`/`.iso`/`.raw`/`.qcow2`/`.vmdk`/`.vhd`），已被其它 file 磁盘选中的文件不可重复选择。
  - `zvol`：下拉选择 VM 数据集下的 ZVOL（通过 `zfs list -t volume -r <dataset>/<vm>` 查询），已被其它 zvol 磁盘选中的不可重复选择。
  - `custom`：文件选择器，可选任意路径。
  - `iscsi`：文本输入，保持不变。

### 网络管理架构（独立即时操作）

网络与磁盘一样，采用独立的即时操作模式，与基础设置等标签页完全解耦：

- **网络列表视图**：表格展示（编号、适配器类型、交换机、MAC 地址），右侧编辑/删除按钮。
- **编辑/添加网络**：弹出模态框修改（编号可编辑、适配器类型、交换机、MAC），保存后即时 PUT。
- **删除网络**：确认后调用 `DELETE /api/bhyve/vms/{name}/networks/{index}`，即时生效。
- 所有操作后自动刷新网络列表，不影响其它标签页。

### 配置保存架构

- 基础设置/图形/其它设备标签页各有独立的保存按钮，调用 `PUT /api/bhyve/vms/{name}` 保存所有非磁盘、非网络配置键。
- 磁盘和网络操作均为即时保存，不依赖保存按钮。
- 后端 `update_vm_config` 采用合并写入：读取现有配置 → 覆盖提交的键（空值删除）→ 原子写入（先写临时文件再 rename，写入前自动备份 `.conf.fwp.bak`）。
- `delete_device`（通用函数）读取完整配置 → 移除指定 `{prefix}{N}_*` 键（支持 `disk` 和 `network` 前缀）→ 原子写入。
