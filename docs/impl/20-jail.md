# 20 — Jail 容器管理

## 概述

Jail 模块提供完整的 jail 生命周期管理：容器列表（运行中/全部）、详情查看、创建（从基础系统或目录）、启动/停止/删除，以及基础系统管理（三种创建方式/编辑/镜像创建）。运行时查询通过 libjail C API（`jailparam_*`）直接调用，配置管理通过解析/写回 `/etc/jail.conf` 实现。

当前阶段已实现：
- libjail FFI 绑定 + RAII 安全封装
- 容器列表页（运行中 Tab / 全部 Tab，全部 Tab 解析 jail.conf）
- Jail 详情页（全参数展示）
- Jail 创建（从基础系统 ZFS Clone / SharedFS，或直接指定目录）
- Jail 启动 / 停止（`jail -c` / `jail -r`）
- Jail 删除（从 jail.conf 移除 + 可选删除文件系统）
- jail.conf 解析器（变量替换、注释处理、全局参数继承）
- jail.conf 块写入（备份 + 原子替换，智能省略与全局默认相同的参数）
- 基础系统创建（三种方式：导入已有目录/数据集、从 base.txz 文件创建、自动下载创建）、列表、编辑快照、删除
- 镜像创建（ZFS Clone / SharedFS，生成 fstab）
- 弹窗提交安全（遮罩 + spin，成功才关闭，失败保留弹窗）

未实现：jail 编辑（修改已有 jail.conf 条目）、restart、控制台 WebSocket、快照/回滚 UI。

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

### 前端 `web/js/pages/jails.js`

#### 容器列表页（`/jails/running`）

- Tab 切换（运行中 / 全部），使用 `filter-group` + `filter-btn` 样式
- 运行中：调用 `GET /api/jails?running=true`（libjail 快速路径）
- 全部：调用 `GET /api/jails`（jail.conf 解析 + libjail 状态合并）
- 表格列：JID / 名称 / 主机名 / 路径 / IP / 状态 / 操作
- 行可点击跳转详情页
- 操作列：启动 / 停止（按状态启用/禁用）/ 删除按钮
- 右上角"创建"按钮

#### 创建 Jail 独立页面（`/jails/create`）

独立页面（非弹窗），动态字段：
- 名称 + hostname + 位置类型选择器
- "目录路径"→ 路径输入框
- "从基础系统创建"→ 基础系统下拉：
  - ZFS 类型 → 快照选择 + 目标数据集（默认值）+ 挂载点（默认值）
  - SharedFS 类型 → 目标目录（默认值）
- 网络接口 + IPv4 + IPv6
- 底部固定操作栏（取消 + 确定），提交时禁用按钮 + loading 状态，成功后跳转列表页

从弹窗改为独立页面的原因：选择基础系统 + ZFS 类型后字段较多，弹窗在小屏幕上过高导致底部按钮不可见。

#### Jail 详情页（`/jails/detail/<name>`）

- 单次 API 调用 `GET /api/jails/{name}`，返回配置参数 + 可选运行时
- 头部行内布局：JID / 状态 / persist / 父级（紧凑展示，非独立卡片）
- 运行中时额外显示运行时信息卡片（osrelease、osreldate、cpuset 等）
- 分区表格：网络 / 主机信息 / 安全 / 系统 / 权限（allow.* 徽章网格）
- conf 参数与 libjail 参数合并后统一展示

#### 基础系统列表页（`/jails/bases`）

- 表格：名称 / 源路径 / 类型徽章（ZFS/SharedFS）/ 快照数 / 操作按钮
- "创建"按钮 → 动态弹窗（三种创建方式）：
  - 创建方式选择器（导入已有 / 从 base.txz / 自动下载）+ 描述框
  - 类型选择器（ZFS / SharedFS）+ 描述框
  - 根据方式×类型组合动态显示/隐藏字段：
    - 导入 + ZFS：现有数据集下拉 + 快照多选
    - 导入 + SharedFS：现有 template + sharedfs 路径
    - 从 base.txz + ZFS：txz 文件路径 + 新数据集 + 快照名
    - 从 base.txz + SharedFS：txz 文件路径 + 新 sharedfs 目录 + 新 template 目录
    - 自动下载：镜像下拉 + 版本输入（datalist 常用版本）+ 架构下拉，然后同 from-txz
- "编辑"按钮（仅 ZFS 类型）→ 快照编辑弹窗
- "创建镜像"按钮 → 按基础系统类型显示对应字段
- "删除"按钮 → 确认弹窗

#### 弹窗提交安全机制 `submitModal()`

所有写操作弹窗（创建 Jail、导入/编辑基础系统、创建镜像）统一使用：
1. 提交时在弹窗内显示遮罩层 + spinner（`.modal-busy`）
2. 调用 API
3. 成功 → 关闭弹窗
4. 失败 → 移除遮罩，弹窗保留，toast 报错

### 导航结构 `web/js/ui/layout.js`

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
| GET | `/api/jails` | — | `[{name, jid, hostname, path, ip4_addr, ip6_addr}]`（全部 jail，jid>0=运行中） |
| GET | `/api/jails?running=true` | — | 同上，仅运行中 jail |
| GET | `/api/jails/{name}` | — | `{name, jid, hostname, path, ip4_addr, ip6_addr, params?, runtime?}` |

### Jail 生命周期

| 方法 | 路径 | 请求 | 响应 |
|---|---|---|---|
| POST | `/api/jails/create` | `{name, hostname?, location_type, path?, base_name?, snapshot?, target_dataset?, interface?, ip4?, ip6?}` | `201 {name, path, fstab?}` |
| POST | `/api/jails/{name}/start` | — | `200 {name, action}` |
| POST | `/api/jails/{name}/stop` | — | `200 {name, action}` |
| DELETE | `/api/jails/{name}?remove_files=true` | — | `204` |

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
- **`/etc/jail.conf`** — Jail 配置文件（读取 + 备份 + 原子写入）
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

## 已知限制 / TODO

- **Jail 编辑** — 未实现修改已有 jail.conf 条目（如修改 IP、接口等参数）。
- **restart** — 未实现（可通过先 stop 再 start 实现）。
- **控制台 WebSocket** — 未实现。设计为 `jexec` + PTY。
- **jail.conf 写回格式保留** — 当前创建/删除 jail 时是文本追加/移除块，不是 AST 级编辑，不保留块内注释和原始缩进格式。设计文档 `docs/plan/10-jail.md` §2 规划了 AST 解析器。
- **UnionFS/OverlayFS** — 设计阶段调研并实测后暂不实现。原因：ZFS 不支持 whiteout。
- **VNET 管理** — 未实现（epair 创建/销毁、bridge 管理）。
- **资源限制** — 未实现（rctl CPU/内存/进程数限制）。
