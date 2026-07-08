# 20 — Jail 容器管理

## 概述

Jail 模块提供完整的 jail 生命周期管理：容器列表（运行中/全部）、详情查看、创建（从基础系统或目录）、启动/停止/删除，以及基础系统管理（三种创建方式/编辑/镜像创建）。运行时查询通过 libjail C API（`jailparam_*`）直接调用，配置管理通过解析/写回 `/etc/jail.conf` 实现。

当前阶段已实现：
- libjail FFI 绑定 + RAII 安全封装
- 容器列表页（运行中 Tab / 全部 Tab，全部 Tab 解析 jail.conf）
- Jail 详情页（分区展示，翻译标签，含启动/停止/删除按钮）
- Jail 创建（从基础系统 ZFS Clone / SharedFS，或直接指定目录）
- Jail 编辑（参数编辑、meta.* 网络配置、VNET 自动配置、fstab 管理）
- Jail 启动 / 停止 / 删除（`jail -c` / `jail -r`）
- jail.conf 解析器（变量替换、注释处理、全局参数继承）
- jail.conf 块写入（备份 + 原子替换，智能省略与全局默认相同的参数）
- 基础系统管理（三种创建方式/编辑/镜像创建）
- VNET 自动配置（epair/bridge 生命周期、DHCP/静态 IP）
- Jail 终端（WebSocket + PTY + xterm.js）
- Jail 初始化（检测 /etc/jail.conf 不存在 → 428 状态码 → 初始化按钮 → 创建默认配置文件）
- 默认配置管理（jail.conf 全局参数 / devfs.rules / 默认 resolv.conf 的查看与编辑）

未实现：restart、快照/回滚 UI。

## 实现细节

### libjail FFI 模块 `src/jail.rs`

所有 `unsafe` 集中在 `sys` 子模块，配安全封装。

#### FFI 声明（`sys` mod）

```rust
mod sys {
    pub const JAIL_DYING: c_int   = 0x08;
    pub const JAIL_CREATE: c_int  = 0x01;
    pub const JAIL_UPDATE: c_int  = 0x02;
    pub const JAIL_ATTACH: c_int  = 0x04;

    #[repr(C)]
    #[derive(Clone)]
    pub struct Jailparam { /* jp_name, jp_value, jp_valuelen, ... */ }

    extern "C" {
        pub fn jailparam_all(jpp: *mut *mut Jailparam) -> c_int;
        pub fn jailparam_init(jp: *mut Jailparam, name: *const c_char) -> c_int;
        pub fn jailparam_import(jp: *mut Jailparam, value: *const c_char) -> c_int;
        pub fn jailparam_set(jp: *mut Jailparam, njp: c_uint, flags: c_int) -> c_int;
        pub fn jailparam_get(jp: *mut Jailparam, njp: c_uint, flags: c_int) -> c_int;
        pub fn jailparam_export(jp: *const Jailparam) -> *mut c_char;
        pub fn jailparam_free(jp: *mut Jailparam, njp: c_uint);
        pub fn jail_remove(jid: c_int) -> c_int;
        pub static jail_errmsg: [c_char; 1024];
    }
}
```

#### RAII 封装 `JailParams`

- `JailParams::all()` — 调用 `jailparam_all()` 获取系统全部已知参数名
- `JailParams::from_names(&[&str])` — 从指定参数名列表创建
- `set(name, value)` — 调用 `jailparam_import()` 导入字符串值
- `query(flags)` — 调用 `jailparam_get()`，返回 JID 或 -1
- `export_all()` — 对每个参数调用 `jailparam_export()`，返回 `HashMap<String, String>`
- `Drop` 实现调用 `jailparam_free()` 释放内存

#### 高层 API

```rust
pub fn list_jails() -> Result<Vec<HashMap<String, String>>, String>
pub fn get_jail(name: &str) -> Result<Option<HashMap<String, String>>, String>
pub fn start_jail(name: &str) -> Result<(), String>     // jail -c
pub fn stop_jail(name: &str) -> Result<(), String>      // jail -r
pub fn is_jail_running(name: &str) -> bool
```

`start_jail` / `stop_jail` 使用 `jail(8)` 命令而非 `jailparam_set`，因为 `jail -c` 能自动处理 fstab 挂载、exec.start、mount.devfs 等全局默认参数。

**列出 jail 的迭代机制**：使用 `lastjid` 参数迭代。初始 `lastjid=0`，每次 `jailparam_get()` 返回下一个 JID，设为新的 `lastjid` 继续，直到返回 -1。使用 `JAIL_DYING` 标志包含正在关闭的 jail。

### 链接 `build.rs`

```rust
fn main() {
    println!("cargo:rustc-link-lib=jail");
}
```

### jail.conf 解析器

`parse_jail_conf()` / `parse_jail_conf_from_str()` 解析 `/etc/jail.conf`：
- 处理 `#` 行注释和 `/* */` 块注释
- 区分全局参数（文件顶部）和 jail 块参数（jail 块继承全局默认值）
- 变量替换：`${name}`、`${path}`、`${host.hostname}`、`$name`
- 解析每个 jail 块的参数（path, hostname, interface, ip4, ip4.addr 等）

### jail.conf 写入

- `backup_jail_conf(state)` — 备份到 `/var/db/fwp/backup/jail.conf.<timestamp>`
- `write_jail_conf_atomic(content)` — 写入临时文件 + rename（原子操作）
- `generate_jail_block(...)` — 生成 jail 块，**智能省略与全局默认相同的参数**（如 `path="/jails/${name}"` 匹配默认值时不写入）
- `remove_jail_block(conf, name)` — 从内容中移除指定 jail 块

### Handler 模块 `src/handlers/jails.rs`

#### 容器列表与详情

统一数据结构 `JailInfo`，列表与详情共用，通过 `Option` 字段区分：

```rust
pub struct JailInfo {
    pub name: String,
    pub jid: i32,                              // >0 = 运行中, 0 = 已停止
    pub hostname: String,
    pub path: String,
    pub ip4_addr: String,                      // 逗号分隔，非数组
    pub ip6_addr: String,
    pub params: Option<HashMap<String, String>>,   // 仅详情：jail.conf 参数
    pub runtime: Option<JailRuntime>,              // 仅详情 + 运行中
}

pub struct JailRuntime {
    pub jid: i32,
    pub state: String,                        // "running" | "dying"
    pub params: HashMap<String, String>,      // libjail 全部运行时参数
}
```

- `list(q)` → `GET /api/jails` — 默认返回全部 jail（解析 jail.conf + 交叉比对 libjail 运行状态）；`?running=true` 时走 libjail 快速路径，仅返回运行中 jail。列表视图不填充 `params`/`runtime`。
- `detail(name)` → `GET /api/jails/{name}` — 始终返回 jail.conf 配置参数（`params`），运行中时额外填充 `runtime`。前端将 conf 参数与 libjail 参数合并展示。

#### Jail 创建

`jail_create()` → `POST /api/jails/create`

流程：
1. 校验名称 + 检查 jail.conf 重名
2. 准备文件系统：
   - `directory` 类型：`mkdir -p` 目标路径
   - `base` + ZFS 类型：`zfs clone <snapshot> <dataset>` + `zfs set mountpoint=<target>`
   - `base` + SharedFS 类型：`cp -R template/. target/` + 写 fstab 文件
3. 备份 jail.conf
4. 生成 jail.conf 块（智能省略全局默认参数）
5. 原子写入 jail.conf

jail.conf 块生成示例（hostname 与 name 不同时才输出）：
```
testjail {
    host.hostname = "testjail.example.com";
    interface = "bge1";
    ip4 = "inherit";
}
```

#### Jail 生命周期控制

- `jail_start(name)` → `POST /api/jails/{name}/start` — `jail -c`
- `jail_stop(name)` → `POST /api/jails/{name}/stop` — `jail -r`
- `jail_delete(name)` → `DELETE /api/jails/{name}?remove_files=true|false`

**删除流程**：
1. 停止 jail（如运行中）
2. **在修改 jail.conf 之前**提取 jail 的 path 和 mount.fstab
3. 备份 jail.conf → 移除 jail 块 → 原子写回
4. `remove_files=true` 时：
   - 用 `zfs list -H -o name,mountpoint <path>` 检测是否为独立 ZFS 数据集
   - **关键安全校验**：只有 `mountpoint == path` 精确匹配时才执行 `zfs destroy`，否则按普通目录 `rm -rf`（避免误删父数据集）
   - 删除 jail.conf 中引用的 fstab 文件

### 基础系统存储架构

#### 核心模型：两种基础系统类型

基础系统有**两种类型**，每种对应一种镜像创建方式：

| 基础系统类型 | source_path 含义 | sharedfs_path | 可用创建方式 |
|---|---|---|---|
| ZFS | ZFS 数据集名（含快照） | — | ZFS Clone |
| SharedFS | template 骨架目录（配置 + 符号链接） | 共享二进制目录 | SharedFS |

#### 数据结构

```rust
struct BaseSystem {
    name: String,
    type: String,                    // "zfs" 或 "sharedfs"
    source_path: String,             // ZFS: 数据集名 | SharedFS: template 路径
    snapshots: Vec<String>,          // ZFS: 导入时选择的快照全名 | SharedFS: 空
    sharedfs_path: Option<String>,   // SharedFS: 共享二进制目录路径
    created_at: i64,
}
```

#### 创建方式（三种）

`base_import()` handler 通过 `method` 字段区分三种创建方式：

**1. `import`（导入已有）**

注册一个已存在的目录或 ZFS 数据集。与重构前的行为完全一致：
- ZFS：验证 dataset 有效 + 至少一个快照 + mountpoint 有 FreeBSD 结构
- SharedFS：验证 template 有 `etc/` + `sharedfs/`，sharedfs 目录有 `bin/` 等

**2. `from-txz`（从 base.txz 文件创建）**

从系统上已有的 base.txz 文件创建新的基础系统：
- ZFS 类型：
  1. `zfs create <新数据集>`
  2. `tar -xf <base.txz> -C <mountpoint>` 解压到数据集
  3. `zfs snapshot <数据集>@<快照名>` 创建快照
  4. 注册基础系统（source_path = 新数据集，snapshots = [数据集@快照名]）
  5. 任一步骤失败时自动 `zfs destroy -r` 回滚
- SharedFS 类型：
  1. 创建 sharedfs 目录，解压 base.txz 到其中
  2. 调用 `build_sharedfs_template()` 构建 template 结构
  3. 注册基础系统
  4. 失败时自动删除已创建的目录

**3. `download`（自动下载创建）**

从 FreeBSD 官方镜像下载 base.txz，后续流程与 `from-txz` 相同：
1. 使用 `/usr/bin/fetch` 下载 `{mirror}/{releases|snapshots}/{arch}/{version}/base.txz` 到临时文件
2. 验证下载文件非空
3. 走 `from-txz` 流程创建基础系统
4. 无论成功/失败都删除临时文件

镜像选择：前端提供 6 个预设镜像（官方/中国/日本/台湾/德国/美国 NY），API `GET /api/jails/bases/mirrors` 返回列表。

#### SharedFS 结构构建 `build_sharedfs_template()`

参照 qjail 的目录布局，将完整的 FreeBSD 系统（base.txz 解压后）转换为 SharedFS + Template 结构：

**sharedfs 保留**（共享只读二进制）：

| 层级 | 目录 |
|---|---|
| 顶层 | bin, lib, libexec, sbin |
| 符号链接 | sys → usr/src/sys |
| usr/ 子目录 | bin, include, lib, lib32, libdata, libexec, ports, sbin, share, src |

**template 构建**（每 Jail 独立）：

| 分类 | 处理方式 | 目录 |
|---|---|---|
| 共享二进制（符号链接到 /sharedfs/） | `symlink` | bin, lib, libexec, sbin, sys |
| 共享 usr 子目录（符号链接到 /sharedfs/usr/） | `symlink` | bin, include, lib, lib32, libdata, libexec, ports, sbin, share, src |
| 独立配置（从 sharedfs 移入） | `fs::rename` | etc, var, root, tmp |
| 独立 usr 子目录（从 sharedfs/usr 移入） | `fs::rename` | local, obj, tests |
| 空目录（标准 FreeBSD 布局） | 新建 | dev, media, mnt, net, proc, sharedfs |
| 内部符号链接 | `symlink` | home → usr/home |
| 顶层文件 | `fs::rename` | COPYRIGHT, .profile, .cshrc 等 |

**从 sharedfs 删除**（base.txz 中存在但 qjail 不需要）：

boot, rescue, 以及所有顶层文件和非共享目录。

template 中的符号链接使用 jail 内绝对路径（如 `/sharedfs/bin`），因为 jail 启动时 nullfs 将 sharedfs 目录挂载到 `<jail_path>/sharedfs`。

#### 导入校验

导入时验证源是否为有效的 FreeBSD 结构：
- ZFS：dataset 有效 + 至少选一个快照 + mountpoint 有 `bin/ sbin/ usr/bin/ usr/lib/ etc/`
- SharedFS：template 有 `etc/` + `sharedfs/`，sharedfs 目录有 `bin/ lib/ sbin/ usr/bin/`

#### ZFS Clone — 独立 COW 副本

```
zroot/jails/bases/freebsd-15.1@clean     ← 快照
创建 jail:
zfs clone snapshot  target_dataset
zfs set mountpoint=target  target_dataset
→ 完全独立的可写副本
```

#### SharedFS — 模板拷贝 + nullfs 只读挂载

```
基础系统（template 骨架）:          共享二进制（sharedfs）:
/usr/jails/template/               /usr/jails/sharedfs/
├── bin -> /sharedfs/bin           ├── bin/ lib/ sbin/
├── etc/                           └── usr/{bin,lib,...}
├── sharedfs/                      ← nullfs ro 挂载点
└── ...

创建 Jail:
1. cp -R template/.  target/
2. fstab: sharedfs_path  target/sharedfs  nullfs  ro  0  0
```

| 共享（符号链接到 /sharedfs，只读） | 独立（每 jail 拷贝，可写） |
|---|---|
| bin, lib, libexec, sbin | etc, var, root |
| usr/bin, usr/lib, usr/sbin | home, tmp, usr/local |

#### 两种方式对比

| | ZFS Clone | SharedFS |
|---|---|---|
| **jail 独立性** | 完全独立 | 配置独立，系统二进制共享 |
| **创建速度** | 秒级（ZFS clone） | 秒级（cp -R 模板） |
| **磁盘占用** | COW 增量 | 系统二进制一份 + 各 jail 配置 |
| **修改/删除系统文件** | ✅ | ❌ 只读 |
| **系统更新** | 每 jail 独立 | 更新 base 即更新所有 jail |
| **快照/回滚** | ZFS 原生 | 仅 jail 目录 |
| **FS 要求** | ZFS | 任意 |

#### UnionFS / OverlayFS — 未来选项（暂不支持）

UnionFS（联合挂载）在设计阶段曾作为第三种镜像创建方式进行了调研和实测。其原理是将基础系统作为只读底层、jail 目录作为可写上层叠加挂载，实现类似 Docker overlay 的语义。

**暂不支持的原因**：

1. **ZFS 不支持 whiteout** — FreeBSD 13+ 默认使用 ZFS，而 ZFS 文件系统不实现 whiteout inode。实测在 ZFS 上 unionfs 可以创建和修改文件（copy-up），但**无法删除**底层文件（返回 `Operation not supported`）。
2. **稳定性** — unionfs 在 FreeBSD 上有长期稳定性争议。
3. **现有方案已覆盖需求** — ZFS Clone + SharedFS 覆盖了实际使用场景。

**未来可能重新引入的条件**：FreeBSD 为 ZFS 添加 whiteout 支持，或引入新的联合文件系统。

### 前端

#### 容器列表页（`/jails/running`，`JailsListPage.vue`）

- Tab 切换（运行中 / 全部），使用 `filter-group` + `filter-btn` 样式
- 表格列：JID / 名称 / 描述 / IP / 状态 / 操作
- 操作列：启动 / 停止 / 终端（仅运行中），无删除按钮（删除在详情页）
- 行可点击跳转详情页

#### Jail 详情页（`JailDetailPage.vue`）

- 头部按钮：启动/停止、终端、编辑、删除
- 状态栏：JID / 状态 / persist / 父级
- 分区表格（标签使用翻译名，非原始参数键）：
  - 基本信息：描述、自动启动、path、主机名等
  - 网络：接口、IPv4（模式+地址）、IPv6、vnet（✓/✗ 图标）
  - 执行：exec.start/stop/prestart/poststart 等
  - 安全、权限（allow.*）、全部参数
- 运行中时额外显示运行时信息标签页

#### Jail 编辑页（`JailEditPage.vue`）

分区标签页编辑，与详情页结构对应：
- **基本信息**：path、hostname、meta.description、自动启动等
- **网络**：meta.interface（接口）、meta.ip4（统一 IP 字段 = 下拉选模式 + 条件地址输入框）、meta.ip6、vnet、vnet.interface（只读 auto）
- **执行**：exec.start/stop/prestart/poststop 为多行文本框（每行一条命令，保存为 `+=`）、exec.clean 等
- **挂载**：mount.fstab（含 fstab 管理弹窗）、devfs 等
- **安全**、**其它**

IP 字段交互（`type: 'ip'`）：
- 模式选择和地址文本独立存储（`ipModeOverrides` / `ipAddrs`），切换模式不丢失地址
- VNET 模式选项：静态/DHCP/禁用；普通模式：—/静态/inherit/禁用
- 仅提交时合并写入 `form.value`

### 网络配置与 VNET 自动配置

#### meta.* 参数体系

网络配置使用 `meta.*` 命名空间作为唯一数据源（source of truth），存储在 jail.conf 中。所有 `meta.*` 键使用下划线分隔（不用点号），jail(8) 忽略这些参数但 fwp 读取它们来派生实际的 jail.conf 参数。

| meta 键 | 含义 | 示例值 |
|---|---|---|
| `meta.description` | 人类可读描述 | `"Web server"` |
| `meta.interface` | 出口网络接口 | `"bge1"` |
| `meta.ip4` | IPv4 配置（单值） | `"dhcp"` / `"192.168.1.10"` / `"inherit"` / `"disable"` / 空 |
| `meta.ip6` | IPv6 配置（同上） | 同上 |

`meta.ip4` 的值决定 IP 模式：
- **空/不存在** = None（无 IP 配置）
- **`dhcp`** = DHCP 模式（仅 VNET）
- **`inherit`** = 继承主机网络栈（仅非 VNET）
- **`disable`** = 禁用
- **其他值** = 静态 IP 地址（值本身就是地址）

#### 普通模式（非 VNET）

`meta.interface` 直接派生为 `interface` 参数，`meta.ip4` 派生为 `ip4.addr` 或 `ip4 = "inherit"`：

```
myjail {
    meta.description = "Web server";
    meta.interface = "bge1";
    meta.ip4 = "192.168.1.10";
    interface = "bge1";
    ip4.addr = 192.168.1.10;
}
```

#### VNET 自动模式

勾选 `vnet` 后，`vnet.interface` 设为 `auto`。保存时：
1. `meta.interface` 用于查找或创建网桥（`ensure_bridge_for_interface`）
2. 自动生成 epair 生命周期命令（`fwp-vnet` 脚本）
3. 根据 `meta.ip4` 值生成 IP 配置命令

```
myjail {
    meta.description = "DHCP jail";
    meta.interface = "bge1";
    meta.ip4 = "dhcp";
    vnet;
    vnet.interface = "vnet0";
    devfs_ruleset = "11";
    exec.prestart += "/usr/local/libexec/fwp-vnet up ${name} bridge0";
    exec.poststart += "/usr/local/libexec/fwp-vnet init ${name} dhcp";
    exec.poststop += "/usr/local/libexec/fwp-vnet down ${name}";
}
```

静态 IP 的 VNET jail：
```
    exec.poststart += "/usr/local/libexec/fwp-vnet init ${name} static 192.168.1.10/24 192.168.1.1";
```

#### 网桥自动管理

`ensure_bridge_for_interface(iface)`:
1. 如果接口已是某网桥成员 → 返回该网桥
2. 如果接口本身是网桥 → 返回它
3. 否则 → 创建新网桥，加入接口，全部 up

#### fwp-vnet 辅助脚本

`/usr/local/libexec/fwp-vnet`，带版本标记（`# fwp-vnet-version: N`），版本不匹配时自动覆盖：

| 子命令 | 执行位置 | 作用 |
|---|---|---|
| `up <name> <bridge>` | 主机（exec.prestart） | 创建 epair，host 端加入网桥，jail 端重命名 vnet0 |
| `down <name>` | 主机（exec.poststop） | 销毁 vnet0（epair 对自动销毁） |
| `init <name> dhcp` | 主机（exec.poststart） | jexec 进 jail 启动 dhclient |
| `init <name> static <ip> <gw>` | 主机（exec.poststart） | jexec 进 jail 配置 ifconfig + route |

#### 编辑时的 exec 行管理

所有含 `fwp-vnet` 的 exec.* 行由网络配置完全控制：
- **保存时**：通用循环跳过所有含 `fwp-vnet` 的命令，由 VNET 块根据当前 `meta.*` 重新生成
- **读取时**：exec.* 的 `+=` 和 `=` 行（包括 fwp-vnet 行）都被收集到多行文本框中显示
- 不做隐藏——fwp-vnet 行在执行标签页的文本框中可见，但保存时不会重复写入

#### Jail 编辑 API

`PUT /api/jails/{name}` — 完整参数替换：
- 请求体：`{params: HashMap<String, String>, auto_start?: bool}`
- 后端用 `generate_jail_block_from_params()` 重新生成整个 jail 块
- `meta.*` 直接写入，派生参数（interface/ip4 等）从 meta 生成，fwp-vnet 行自动管理

### fstab 管理

- `GET /api/jails/{name}/fstab` — 读取 jail 的 mount.fstab 文件
- `PUT /api/jails/{name}/fstab` — 替换全部 fstab 条目

```
虚拟化 (topbar)
  └── Jail 容器 (collapsible)
      ├── 容器列表  /jails/running
      └── 基础系统  /jails/bases
  └── Bhyve 虚拟机
```

## API

### 运行时查询

| 方法 | 路径 | 请求 | 响应 |
|---|---|---|---|
| GET | `/api/jails` | — | `[{name, jid, description, hostname, path, ip4_addr, ip6_addr, auto_start}]`（全部 jail，jid>0=运行中） |
| GET | `/api/jails?running=true` | — | 同上，仅运行中 jail |
| GET | `/api/jails/{name}` | — | `{name, jid, description, ..., params?, runtime?}` |
| PUT | `/api/jails/{name}` | `{params, auto_start?}` | `200 {name}` |
| POST | `/api/jails/create` | `{name, hostname?, location_type, ...}` | `201 {name, path, fstab?}` |
| POST | `/api/jails/{name}/start` | — | `200 {name, action}` |
| POST | `/api/jails/{name}/stop` | — | `200 {name, action}` |
| DELETE | `/api/jails/{name}?remove_files=true` | — | `204` |
| GET | `/api/jails/{name}/fstab` | — | `[FstabEntry]` |
| PUT | `/api/jails/{name}/fstab` | `{entries}` | `[FstabEntry]` |

### 初始化与默认配置

| 方法 | 路径 | 请求 | 响应 |
|---|---|---|---|
| GET | `/api/jails` | — | `428 {"error":"needs_init"}` — 当 `/etc/jail.conf` 不存在时返回此状态码，前端显示初始化界面 |
| GET | `/api/jails/init` | — | `200 {needs_init, jail_conf_exists, devfs_rules_exists}` |
| POST | `/api/jails/init` | — | `201` — 创建 `/etc/jail.conf`（默认全局参数）和 `/etc/devfs.rules`（bpf 规则），已存在则跳过 |
| GET | `/api/jails/config/global` | — | `200 {content}` — jail.conf 中 jail 块之外的全局参数文本 |
| PUT | `/api/jails/config/global` | `{content}` | `200` — 替换 jail.conf 全局段（保留 jail 块位置，自动备份） |
| GET | `/api/jails/config/devfs` | — | `200 {content}` — `/etc/devfs.rules` 文件内容 |
| PUT | `/api/jails/config/devfs` | `{content}` | `200` — 写入 `/etc/devfs.rules` |
| GET | `/api/jails/config/resolv` | — | `200 {content}` — 默认 jail resolv.conf（存储于 `/var/db/fwp/jail-resolv.conf`） |
| PUT | `/api/jails/config/resolv` | `{content}` | `200` — 写入默认 jail resolv.conf |

### 基础系统管理

| 方法 | 路径 | 请求 | 响应 |
|---|---|---|---|
| GET | `/api/jails/bases` | — | `[{name, type, source_path, snapshots[], sharedfs_path?, created_at}]` |
| POST | `/api/jails/bases` | `{name, method, type, ...method-specific fields}` | `201 {name, type, ...}` |
| GET | `/api/jails/bases/mirrors` | — | `[{name, url}]` |
| PUT | `/api/jails/bases/{name}` | `{snapshots[]}` | `200 {name, type, ...}` |
| DELETE | `/api/jails/bases/{name}` | — | `204` |
| GET | `/api/jails/bases/snapshots?name=dataset` | — | `["pool@snap1", "pool@snap2"]` |
| POST | `/api/jails/bases/{name}/image` | `{method, snapshot?, dataset?, target}` | `201 {method, target, sharedfs_path?, fstab?}` |

POST `/api/jails/bases` 请求体根据 `method` 不同：

- `method: "import"`: `{name, method, type, source_path, snapshots?, sharedfs_path?}`（与重构前兼容）
- `method: "from-txz"` + ZFS: `{name, method, type, txz_path, dataset, snapshot_name}`
- `method: "from-txz"` + SharedFS: `{name, method, type, txz_path, sharedfs_path, template_path}`
- `method: "download"` + ZFS: `{name, method, type, mirror, version, arch?, dataset, snapshot_name}`
- `method: "download"` + SharedFS: `{name, method, type, mirror, version, arch?, sharedfs_path, template_path}`

## 外部依赖

- **libjail**（`-ljail`）— 通过 `build.rs` 链接，提供 `jailparam_*` C API
- **`/usr/sbin/jail`** — jail 启动（`jail -c`）、停止（`jail -r`）
- **`/sbin/zfs`** — ZFS 数据集检测、快照列举、clone、destroy、create、snapshot
- **`/bin/cp`** — SharedFS 模板拷贝
- **`/usr/bin/tar`** — base.txz 解压（from-txz / download 方式）
- **`/usr/bin/fetch`** — base.txz 下载（download 方式，FreeBSD 原生）
- **`/etc/jail.conf`** — Jail 配置文件（读取 + 备份 + 原子写入 + 初始化 + 全局段编辑）
- **`/etc/devfs.rules`** — Devfs 规则文件（初始化 + 读写）
- **`/var/db/fwp/jail-resolv.conf`** — 默认 jail DNS 配置（读写）
- **`/usr/local/libexec/fwp-vnet`** — VNET epair 生命周期管理脚本（首次启用 VNET 时自动创建）
- **`/sbin/ifconfig`** — 网桥检测、epair 创建/销毁（VNET 自动配置）
- **`/sbin/route`** — 默认网关检测（VNET 自动配置）
- **crate: libc** — FFI 类型（`c_char`, `c_int`, `c_void`, `size_t`）
- **crate: serde_json** — 基础系统注册表 JSON 读写

## 配置项

基础系统注册表路径由 `fwp.toml` 的 `[paths] db` 字段派生：

```toml
[paths]
db = "/var/db/fwp/fwp.db"
```

派生路径：
- 注册表：`/var/db/fwp/jail-bases.json`
- SharedFS fstab：`/var/db/fwp/jail-fstabs/<sanitized_target>.fstab`
- jail.conf 备份：`/var/db/fwp/backup/jail.conf.<timestamp>`
- 默认 jail resolv.conf：`/var/db/fwp/jail-resolv.conf`

## 已知限制 / TODO

- **restart** — 未实现（可通过先 stop 再 start 实现）。
- **快照/回滚 UI** — 未实现。
- **jail.conf 写回格式保留** — 当前创建/删除 jail 时是文本追加/移除块，不是 AST 级编辑，不保留块内注释和原始缩进格式。
- **UnionFS/OverlayFS** — 设计阶段调研并实测后暂不实现。原因：ZFS 不支持 whiteout。
- **资源限制** — 未实现（rctl CPU/内存/进程数限制）。
