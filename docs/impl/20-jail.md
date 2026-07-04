# 20 — Jail 容器管理

## 概述

Jail 模块提供运行中 jail 的列表/详情查询，以及基础系统管理（导入源目录/ZFS 数据集、创建镜像）。运行时查询通过 libjail C API（`jailparam_*`）直接调用，不 spawn `jls` 子进程。基础系统支持两种镜像创建方式：ZFS Clone 和 SharedFS。

当前阶段已实现：
- libjail FFI 绑定 + RAII 安全封装
- 运行中 jail 列表 + 详情页（全参数展示）
- 基础系统导入/列表/删除（JSON 注册表）
- 两种方式创建镜像（ZFS Clone / sharedfs）

未实现：jail.conf 解析/写回、jail 创建/编辑/删除（jail.conf CRUD）、start/stop/restart 控制、控制台 WebSocket、快照/回滚 UI。

## 实现细节

### libjail FFI 模块 `src/jail.rs`

所有 `unsafe` 集中在 `sys` 子模块，配安全封装。

#### FFI 声明（`sys` mod）

```rust
mod sys {
    pub const JAIL_DYING: c_int = 0x08;

    #[repr(C)]
    #[derive(Clone)]
    pub struct Jailparam {
        pub jp_name: *mut c_char,
        pub jp_value: *mut c_void,
        pub jp_valuelen: size_t,
        pub jp_elemlen: size_t,
        pub jp_ctltype: c_int,
        pub jp_structtype: c_int,
        pub jp_flags: c_uint,
    }

    extern "C" {
        pub fn jailparam_all(jpp: *mut *mut Jailparam) -> c_int;
        pub fn jailparam_init(jp: *mut Jailparam, name: *const c_char) -> c_int;
        pub fn jailparam_import(jp: *mut Jailparam, value: *const c_char) -> c_int;
        pub fn jailparam_get(jp: *mut Jailparam, njp: c_uint, flags: c_int) -> c_int;
        pub fn jailparam_export(jp: *const Jailparam) -> *mut c_char;
        pub fn jailparam_free(jp: *mut Jailparam, njp: c_uint);
        pub static jail_errmsg: [c_char; 1024];
    }
}
```

#### RAII 封装 `JailParams`

- `JailParams::all()` — 调用 `jailparam_all()` 获取系统全部已知参数名，返回 RAII 封装
- `JailParams::from_names(&[&str])` — 从指定参数名列表创建
- `set(name, value)` — 调用 `jailparam_import()` 导入字符串值
- `query(flags)` — 调用 `jailparam_get()`，返回 JID 或 -1
- `export_all()` — 对每个参数调用 `jailparam_export()`，返回 `HashMap<String, String>`
- `Drop` 实现调用 `jailparam_free()` 释放内存

#### 高层 API

```rust
/// 列出所有运行中 jail（含 dying），返回参数 map 列表
pub fn list_jails() -> Result<Vec<HashMap<String, String>>, String>

/// 按名称查询单个 jail 的全部参数，不存在返回 Ok(None)
pub fn get_jail(name: &str) -> Result<Option<HashMap<String, String>>, String>
```

**列出 jail 的迭代机制**：使用 `lastjid` 参数迭代。初始 `lastjid=0`，每次 `jailparam_get()` 返回下一个 JID，设为新的 `lastjid` 继续，直到返回 -1。使用 `JAIL_DYING` 标志包含正在关闭的 jail。

**libjail 导出值格式**：
- 布尔参数（`JP_BOOL`）→ `"true"` / `"false"`
- jailsys 参数（`JP_JAILSYS`）→ `"disable"` / `"new"` / `"inherit"`
- 字符串参数 → 原始字符串
- 整数参数 → 数字字符串
- IP 地址 → 逗号分隔

### 链接 `build.rs`

```rust
fn main() {
    println!("cargo:rustc-link-lib=jail");
}
```

### Handler 模块 `src/handlers/jails.rs`

#### 运行时查询

- `list()` → `GET /api/jails` — 调用 `jail::list_jails()`，提取 `JailInfo` 结构（jid, name, hostname, path, ip4_addr, ip6_addr, state, persist）
- `detail(name)` → `GET /api/jails/{name}` — 调用 `jail::get_jail()`，返回 `JailDetail`（含 `params: HashMap` 全部参数）
- 输入校验：jailname 匹配 `^[a-zA-Z0-9_.-]+$`

#### 基础系统管理

基础系统注册表存储在 JSON 文件中，路径由 `config.paths.db` 的父目录 + `jail-bases.json` 拼接。

```rust
struct BaseSystem {
    name: String,
    source_path: String,              // ZFS 数据集 / 完整目录 / template 骨架目录
    is_zfs: bool,                     // 是否为 ZFS 数据集
    sharedfs_path: Option<String>,    // Some = SharedFS 模板，共享二进制目录路径
    created_at: i64,
}
```

- `base_list()` — 读取 JSON，对 ZFS 类型的基础系统额外查询快照列表（`zfs list -t snapshot -H -o name -d 1`）
- `base_import(body)` — 校验名称和路径，检测是否为 ZFS 数据集（`zfs list -H -o name`），如有 `sharedfs_path` 则验证其存在，写入 JSON
- `base_destroy(name)` — 从 JSON 移除（不删除源文件）

**基础系统类型判定**：`sharedfs_path` 为 `Some` → SharedFS 模板；为 `None` 且 `is_zfs` → ZFS 完整系统；为 `None` 且非 ZFS → 目录完整系统。

**路径验证**：`validate_source_path()` 同时接受绝对路径（`/`开头）和 ZFS 数据集名（`pool/dataset` 格式）。`validate_target_path()` 仅接受绝对路径。

**ZFS 数据集名解析**：`resolve_fs_path()` 将 ZFS 数据集名转换为文件系统挂载点路径（`zfs list -H -o mountpoint`），用于 sharedfs 创建时拷贝配置目录。

### 基础系统存储架构

#### 核心模型：三种基础系统类型

基础系统有**三种类型**，每种对应一种镜像创建方式。类型在导入时确定，不可在创建镜像时混用：

| 基础系统类型 | source_path 含义 | sharedfs_path | 可用创建方式 |
|---|---|---|---|
| ZFS 快照 | ZFS 数据集名（含快照） | — | ZFS Clone |
| 完整目录 | 完整 FreeBSD 系统目录 | — | （未来可能支持 UnionFS/OverlayFS） |
| SharedFS 模板 | template 骨架目录（配置 + 符号链接） | 共享二进制目录 | SharedFS |

**SharedFS 模板与共享二进制是两个独立的东西**（借鉴 qjail 的设计）：
- `sharedfs`（共享二进制目录）— 含 `bin/ lib/ sbin/ usr/` 等系统二进制，所有 jail 通过 nullfs ro 共享同一份
- `template`（骨架目录）— 含 `etc/ var/ root/` 等配置目录 + 指向 `/sharedfs/*` 的符号链接，每个 jail 从此目录拷贝

导入时提供 `sharedfs_path` 的基础系统即为 SharedFS 模板类型。不提供的则为完整系统（ZFS 或目录）。

#### ZFS Clone — 独立 COW 副本

```
zroot/jails/bases/freebsd-15.1           ← 基础系统（ZFS 数据集）
zroot/jails/bases/freebsd-15.1@clean     ← 快照

创建 jail:
zfs clone zroot/jails/bases/freebsd-15.1@clean  zroot/jails/web01
zfs set mountpoint=/jails/web01  zroot/jails/web01
→ /jails/web01 是完全独立的可写副本
→ jail.conf: path = "/jails/web01"（无需额外 fstab）
```

每个 jail 完全独立，可独立 `freebsd-update`、`pkg upgrade`。ZFS 快照/回滚原生支持。

#### UnionFS / OverlayFS — 未来选项（暂不支持）

UnionFS（联合挂载）在设计阶段曾作为第三种镜像创建方式进行了调研和实测。其原理是将基础系统作为只读底层、jail 目录作为可写上层叠加挂载，实现类似 Docker overlay 的语义。

**暂不支持的原因**：

1. **ZFS 不支持 whiteout** — FreeBSD 13+ 默认使用 ZFS，而 ZFS 文件系统不实现 whiteout inode。实测在 ZFS 上 unionfs 可以创建和修改文件（copy-up），但**无法删除**底层文件（返回 `Operation not supported`）。这导致 `pkg delete`、`freebsd-update` 等需要删除文件的操作失败。UFS 上完整支持 whiteout，但 UFS 不是现代 FreeBSD 的默认选择。

2. **稳定性** — unionfs 在 FreeBSD 上有长期稳定性争议，历史上多次重写。虽然 FreeBSD 13+ 大幅改善，但社区中仍不推荐用于可写 jail 场景。

3. **现有方案已覆盖需求** — ZFS Clone 提供完整独立性（需 ZFS），SharedFS 提供共享二进制 + 独立配置（任意文件系统），两者的组合覆盖了实际使用场景。

**未来可能重新引入的条件**：
- FreeBSD 内核为 ZFS 添加 whiteout 支持，或
- FreeBSD 引入类似 Linux overlayfs 的新联合文件系统，或
- 面向 UFS 用户的明确需求

#### sharedfs — 模板拷贝 + nullfs 只读挂载

借鉴 qjail 的 sharedfs + template 设计。基础系统是**预构建的 template 骨架目录**（含配置目录 + 指向 `/sharedfs/*` 的符号链接），不是完整系统目录。共享二进制（sharedfs）是独立管理的资源。

```
基础系统（template 骨架）:          共享二进制（sharedfs）:
/usr/jails/template/               /usr/jails/sharedfs/
├── bin -> /sharedfs/bin           ├── bin/ lib/ sbin/
├── lib -> /sharedfs/lib           └── usr/{bin,lib,...}
├── sbin -> /sharedfs/sbin
├── usr/
│   ├── bin -> /sharedfs/usr/bin
│   ├── lib -> /sharedfs/usr/lib
│   └── local/                ← 可写（pkg 安装目录）
├── etc/                      ← 配置（每 jail 拷贝一份）
├── var/
├── root/
├── home/
├── tmp/
└── sharedfs/                 ← nullfs ro 挂载点

创建 Jail:
1. cp -R template/.  /jails/web01/    ← 拷贝骨架（保留符号链接）
2. jail 启动时 fstab 挂载:
   /usr/jails/sharedfs  /jails/web01/sharedfs  nullfs  ro  0  0
```

**符号链接解析机制**：链接 `bin -> /sharedfs/bin` 是绝对路径。jail 内 chroot 后，`/sharedfs` 解析到 `jailroot/sharedfs`，即 nullfs 挂载点。每个 jail 的符号链接相同，但各自指向自己的 nullfs 挂载。

**与旧实现的差异**（重构）：旧实现在创建 jail 时动态生成符号链接 + 从完整系统目录拆分 system/config 目录。新实现直接拷贝预构建的 template（`cp -R`，保留符号链接），不再动态生成——template 在导入时已是现成的骨架。

**共享 vs 独立目录划分**（template 内）：

| 共享（符号链接到 /sharedfs，只读） | 独立（每 jail 拷贝，可写） |
|---|---|
| bin, lib, libexec, sbin | etc, var, root |
| usr/bin, usr/include, usr/lib | home, tmp |
| usr/libdata, usr/libexec, usr/sbin | usr/local（pkg 安装目录） |
| usr/share | |

`/usr/local` 独立是关键——每个 jail 可独立 `pkg bootstrap` + 安装包。系统二进制只读共享，jail 无法修改也无法覆盖。

#### 两种方式对比

| | ZFS Clone | sharedfs |
|---|---|---|
| **基础系统** | ZFS 快照（完整系统） | template 骨架 + sharedfs 共享二进制 |
| **jail 独立性** | 完全独立 | 配置独立，系统二进制共享 |
| **创建速度** | 秒级（ZFS clone） | 秒级（cp -R 模板） |
| **磁盘占用** | COW 增量 | 系统二进制一份 + 各 jail 配置 |
| **修改系统文件** | ✅ 自由修改 | ❌ 只读 |
| **删除系统文件** | ✅ | ❌ |
| **系统更新** | 每 jail 独立 | 更新 base 即更新所有 jail |
| **快照/回滚** | ZFS 原生 | 仅 jail 目录 |
| **FS 要求** | ZFS | 任意 |
| **jail.conf 额外项** | 无 | mount.fstab |

#### 创建镜像 `base_create_image()`

创建前根据基础系统类型校验方法合法性：
- SharedFS 模板（`sharedfs_path` 非空）→ 仅允许 `sharedfs`
- ZFS 完整系统 → 仅允许 `zfs-clone`

两种方式：

**ZFS Clone**：
1. 校验 source 是 ZFS 数据集且非模板
2. `zfs clone <snapshot> <target_dataset>`
3. `zfs set mountpoint=<target> <target_dataset>`

**sharedfs**：
1. 校验基础系统是 SharedFS 模板（`sharedfs_path` 非空）
2. `cp -R <template>/. <target>` — 拷贝骨架目录（`-R` 保留符号链接）
3. 生成 fstab 文件到 `/var/db/fwp/jail-fstabs/<sanitized_target>.fstab`，内容：
   ```
   <sharedfs_path>  <target>/sharedfs  nullfs  ro  0  0
   ```
4. API 响应返回 fstab 路径，供后续 jail.conf 写入 `mount.fstab` 引用

### 前端 `web/js/pages/jails.js`

#### 运行中 Jail 列表页（`/jails/running`）

表格展示 JID / 名称 / 主机名 / 路径 / IP / 状态。行可点击跳转到详情页。

#### Jail 详情页（`/jails/detail/<name>`）

- 概览卡片（JID、状态、persist、父级 JID）
- 分区表格：网络 / 主机信息 / 安全 / 系统 / 权限（allow.* 徽章网格）/ 全部参数

#### 基础系统列表页（`/jails/bases`）

- 表格展示：名称 / 源路径 / 类型（ZFS/Directory 徽章）/ 快照数 / 操作按钮
- "导入"按钮 → `formModal`（名称 + 源路径）
- "创建镜像"按钮 → 自定义弹窗（动态字段）
- "删除"按钮 → `confirmDialog`

#### 创建镜像弹窗

自定义 modal（非 `formModal`），字段根据创建方式动态显示/隐藏：
- 创建方式：下拉选择 ZFS Clone / SharedFS（ZFS Clone 仅在源为 ZFS 时出现，SharedFS 仅在模板时出现）
- ZFS Clone 时显示：克隆快照（下拉）、目标数据集（输入）
- 目标位置：始终显示

### 导航结构 `web/js/ui/layout.js`

```
虚拟化 (topbar)
  └── Jail 容器 (collapsible)
      ├── 运行中    /jails/running
      └── 基础系统  /jails/bases
  └── Bhyve 虚拟机
```

## API

### 运行时查询

| 方法 | 路径 | 请求 | 响应 |
|---|---|---|---|
| GET | `/api/jails` | — | `[{jid, name, hostname, path, ip4_addr[], ip6_addr[], state, persist}]` |
| GET | `/api/jails/{name}` | — | `{jid, name, hostname, path, ip4_addr[], ip6_addr[], state, persist, params: {key: value}}` |

### 基础系统管理

| 方法 | 路径 | 请求 | 响应 |
|---|---|---|---|
| GET | `/api/jails/bases` | — | `[{name, source_path, is_zfs, sharedfs_path?, created_at, snapshots[]}]` |
| POST | `/api/jails/bases` | `{name, source_path, sharedfs_path?}` | `201 {name, source_path, is_zfs, sharedfs_path?, created_at}` |
| DELETE | `/api/jails/bases/{name}` | — | `204` |
| POST | `/api/jails/bases/{name}/image` | `{method, snapshot?, dataset?, target}` | `201 {method, target, sharedfs_path?, fstab?}` |

## 外部依赖

- **libjail**（`-ljail`）— 通过 `build.rs` 链接，提供 `jailparam_*` C API
- **`/sbin/zfs`** — ZFS 数据集检测、快照列举、clone、set mountpoint
- **`/bin/cp`** — sharedfs 模式拷贝配置目录
- **crate: libc** — FFI 类型（`c_char`, `c_int`, `c_void`, `size_t`）
- **crate: serde_json** — 基础系统注册表 JSON 读写

## 配置项

基础系统注册表路径由 `fwp.toml` 的 `[paths] db` 字段派生：

```toml
[paths]
db = "/var/db/fwp/fwp.db"
```

注册表文件路径 = `db` 的父目录 + `jail-bases.json`（即 `/var/db/fwp/jail-bases.json`）。

## 已知限制 / TODO

- **jail.conf 解析/写回** — 未实现。设计文档 `docs/plan/10-jail.md` §2 规划了 AST 解析器，保留注释和格式，原子写回 + 备份。
- **jail CRUD** — 未实现创建/编辑/删除 jail.conf 条目。
- **start/stop/restart** — 未实现。libjail 的 `jailparam_set` + `JAIL_CREATE` 和 `jail_remove` 已在 FFI 模块中声明但未封装为安全 API。
- **控制台 WebSocket** — 未实现。设计为 `jexec` + PTY。
- **UnionFS/OverlayFS** — 设计阶段调研并实测后暂不实现。原因：ZFS 不支持 whiteout（无法删除底层文件），unionfs 稳定性争议。详见上文"UnionFS / OverlayFS — 未来选项"章节。
- **基础系统源路径校验** — 当前仅检查路径存在或 ZFS 数据集有效，不验证目录内容是否为有效的 FreeBSD 基础系统。
- **镜像创建后不写 jail.conf** — 当前只准备文件系统，不自动生成 jail.conf 条目。jail.conf 解析器实现后才能完成完整的 jail 创建流程。
