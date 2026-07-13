# 09 — ZFS 管理

## 概述

通过 `zfs`/`zpool` 命令的 `-H -p` 机器可读输出实现 ZFS 存储池、数据集、快照的完整管理。包含三级子菜单（Zpool / 数据集 / 快照）。

## 实现细节

### 后端 `src/handlers/zfs.rs`

所有命令通过 `run(cmd, args)` 封装执行，输入经 `validate_name()` / `validate_prop_key()` 校验（正则白名单），参数以 `Command::arg()` 传递（防注入）。

**输入校验正则（缓存）**：6 个正则以 `static LazyLock<Regex>` 方式在模块顶层声明，进程生命周期内只编译一次，避免每次请求重新编译：

| 静态变量 | 模式 | 用途 |
|---|---|---|
| `RE_NAME` | `^[a-zA-Z0-9@/_:\-\.]+$` | 数据集/快照名（含 `/`、`@`） |
| `RE_PROP` | `^[a-zA-Z0-9_:\-\.]+$` | 属性键（不允许 `/`、`@`） |
| `RE_DISK` | `^[a-zA-Z0-9]+$` | 裸磁盘名（如 `da0`） |
| `RE_POOL` | `^[a-zA-Z0-9_.\-]+$` | pool 名（查询/操作） |
| `RE_POOL_CREATE` | `^[a-zA-Z][a-zA-Z0-9_.\-]*$` | pool 名（创建时，须字母开头） |
| `RE_DEVICE` | `^[a-zA-Z0-9\-]+$` | vdev 名（含 `mirror-0`、`raidz1-0`） |

为什么用 `LazyLock` 而不是每次 `Regex::new()`：`Regex::new` 内部会编译 DFA，单次开销 ~数十 µs 加一次堆分配。ZFS 列表页一次请求会触发 N 次校验（N = 数据集数），频繁操作下这些临时 Regex 会累积到 jemalloc arena，成为 RES 增长的来源之一。`LazyLock` + `static` 保证只编译一次，线程安全且零运行期开销。同类优化的通用总结见 [25-memory.md](25-memory.md)。


**Zpool**：

| 函数 | 命令 | 说明 |
|---|---|---|
| `pool_list` | `zpool list -H -p` | 所有 pool 摘要（tab 分隔，精确数值） |
| `pool_status` | `zpool status <name>` + `zpool list -H -p <name>` | 详细状态 + VDEV 树 + 容量/碎片/去重数据（合并两个命令输出） |
| `pool_create` | `zpool create -f [-o ashift=N] [-o prop=val] <name> <vdev_spec>` | 创建存储池，支持 mirror/raidz1-3/单盘多组 vdev |
| `pool_destroy` | `zpool destroy [-f] <name>` | 销毁存储池，query: `?force=<bool>` |
| `available_disks` | `geom disk list` + `zpool status` | 可用磁盘列表（过滤已在 pool 中的磁盘，标记 in_use + 所属 pool） |
| `pool_add_vdev` | `zpool add [-f] <name> <vdev_spec>` | 向已有 pool 添加 vdev 组，body: `{vdevs, force?}` |
| `pool_attach` | `zpool attach <name> <device> <new_device>` | 将新设备附加为镜像（单盘→镜像或镜像扩容），body: `{device, new_device}` |
| `pool_detach` | `zpool detach <name> <device>` | 从镜像中分离设备，query: `?device=` |
| `pool_replace` | `zpool replace <name> <old_device> <new_device>` | 替换磁盘（故障更换/升级），body: `{old_device, new_device}` |
| `pool_scrub` | `zpool scrub <name>` | 启动 scrub |
| `pool_scrub_stop` | `zpool scrub -s <name>` | 停止 scrub |

VDEV 树解析：`zpool status` 输出中 `config:` 后的行以 tab 缩进表示层级。按 `line.starts_with('\t')` 检测，`indent = line.len() - line.trim_start().len()` 计算层级，递归 `build_vdev_tree()` 构建树。

Scan（scrub）多行解析：`scan:` 块可能跨多行（scrub 进行中时续行含已扫描量/速率/百分比/ETA）。解析时记录 `scan:` 行缩进 `scan_indent`，后续缩进更深的非空行视为续行一并累积，缩进回退则结束块，拼接为单个 `\n` 分隔字符串。

`pool_status` 会同时执行 `zpool list -H -p <name>` 补充 size/allocated/free/fragmentation/capacity/dedup 字段（这些字段不在 `zpool status` 输出中）。

**数据集**：

| 函数 | 命令 | 说明 |
|---|---|---|
| `dataset_list` | `zfs list -H -p -o name,used,avail,refer,mountpoint,type,compression` | 全部数据集，`build_dataset_tree()` 构建父子树 |
| `dataset_create` | `zfs create [-o k=v]... <name>` | 创建（含可选属性），body: `{name, properties?}` |
| `dataset_destroy` | `zfs destroy -r <name>` | 递归销毁，query: `?name=<dataset>` |
| `dataset_set` | `zfs set k=v <name>` | 设置属性，query: `?name=`，body: `{properties: {k:v}}` |
| `dataset_properties` | `zfs get -H -p -o property,value,source all <name>` | 全部属性，query: `?name=` |

数据集树构建：按 `/` 分割名称确定父子关系，`HashMap<String, Dataset>` + `parent_map` 递归组装。

**快照**：

| 函数 | 命令 | 说明 |
|---|---|---|
| `snapshot_list` | `zfs list -t snapshot -H -p -o name,used,refer,creation [dataset]` | 快照列表（可按 dataset 过滤） |
| `snapshot_clone` | `zfs clone [-o mountpoint=<path>] <snapshot> <dataset>` | 克隆快照为新数据集，body: `{source, target, mountpoint?}`；mountpoint 非空时用 `-o` 设置，经 `validate_mountpoint()` 校验（须绝对路径） |
| `snapshot_destroy` | `zfs destroy [-R] <dataset>@<name>` | 销毁快照，query: `?name=<full>&recursive=<bool>`；recursive=true 时加 `-R`（连同依赖克隆） |
| `snapshot_rollback` | `zfs rollback -r <dataset>@<name>` | 回滚，query: `?name=`，body: `{confirm: true}` |

快照名 `dataset@snapname` 经 `validate_name()` 校验（允许 `@`）。命令执行失败（非零退出码）通过 `ApiError::Command` 透传 stderr（见 [00-framework.md](00-framework.md) 错误处理）。

### 路由设计（query param 替代 path param）

ZFS 名称含 `/`（如 `zroot/vm/alpine@test`），axum 的 `{name}` 路径参数只能匹配单段。因此含 `/` 的操作改用 query 参数：

| 操作 | 路由 | 名称传递方式 |
|---|---|---|
| Pool 详情/Scrub | `/api/zfs/pools/{name}` | path param（pool 名不含 `/`） |
| 数据集销毁 | `DELETE /api/zfs/dataset/destroy?name=` | query param |
| 数据集属性 | `GET/PUT /api/zfs/dataset/properties?name=` | query param |
| 快照销毁 | `DELETE /api/zfs/snapshot/destroy?name=&recursive=` | query param |
| 快照回滚 | `POST /api/zfs/snapshot/rollback?name=` | query param |

`NameQuery` 结构体提取 `?name=`；快照销毁用 `SnapshotDestroyQuery`（含可选 `recursive`）。前端用 `encodeURIComponent()` 编码。

### 前端 `web/js/pages/zfs.js`

四个页面：

**Zpool 列表**（`/zfs/pools`）：
- Pool 卡片列表（可点击，hover 高亮 + 箭头提示）
- 点击卡片跳转到详情页
- 创建 Pool 按钮（打开 PoolManageModal，含 VDEV 组构建器）
- 销毁 Pool 按钮（确认对话框 + 可选强制销毁）

**Zpool 详情**（`/zfs/pools/{name}`）：
- 6 个概览卡片（状态/总容量/已分配/空闲/碎片率/去重比）
- 容量进度条
- Scrub 状态信息
- 阵列结构 VDEV 树（层级缩进，类型标签：镜像/RAID-Z/磁盘，状态徽章，错误计数）
- 每个磁盘的操作按钮：
  - 单盘（无子节点）：附加（转为镜像）+ 替换
  - 镜像成员：分离 + 替换
  - RAID-Z 成员：仅替换
- 添加 VDEV 按钮（打开 PoolManageModal 的 add 模式）
- Scrub 启动/停止按钮
- 销毁 Pool 按钮
- 返回按钮

**数据集管理**（`/zfs/datasets`）：
- 树形表格（缩进表示层级）
- 创建数据集（prompt 输入名称）
- 创建快照（每行「快照」按钮，prompt 输入快照名）
- 删除数据集（确认对话框，仅非顶层 pool 可删）
- 查看属性（模态框表格）

**快照管理**（`/zfs/snapshots`）：
- 快照表格（数据集/快照名/已用/引用/时间）
- 过滤框（实时过滤）
- 创建快照（模态框输入 dataset + name）
- 克隆快照（模态框输入目标数据集名 + 可选挂载点）
- 删除/回滚（确认对话框，删除可选「递归删除」勾选，回滚需二次确认）

### 三级菜单 `web/js/ui/layout.js`

菜单项支持 `children` 数组：
- 有 `children` → 可折叠子组（组头可点击，导航到第一个子项 + 自动展开）
- 无 `children` → 直接链接

CSS：`.sub-group` / `.sub-group-header`（`cursor: pointer`，hover 高亮，箭头旋转动画） / `.sub-items` / `.sub-item`（缩进 40px，12px 字号）。

### 路由器三级匹配 `web/js/router.js`

以 `/` 结尾的路由定义为**纯前缀路由**（detail page parent）：
- 只匹配子路径（`startsWith`），不匹配精确路径
- 非斜杠路由只做精确匹配

示例：`/zfs/pools`（列表）精确匹配，`/zfs/pools/`（详情前缀）匹配 `/zfs/pools/zroot`。优先级：按原始路径长度排序，更长的优先。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/zfs/pools` | Pool 列表 |
| POST | `/api/zfs/pools` | 创建 Pool（body: `{name, ashift?, vdevs, properties?}`） |
| GET | `/api/zfs/pools/available-disks` | 可用磁盘列表 |
| GET | `/api/zfs/pools/{name}` | Pool 详情（含 VDEV 树 + 容量数据） |
| DELETE | `/api/zfs/pools/{name}?force=` | 销毁 Pool |
| POST | `/api/zfs/pools/{name}/add` | 添加 VDEV（body: `{vdevs, force?}`） |
| POST | `/api/zfs/pools/{name}/attach` | 附加设备（body: `{device, new_device}`） |
| POST | `/api/zfs/pools/{name}/detach?device=` | 分离设备 |
| POST | `/api/zfs/pools/{name}/replace` | 替换设备（body: `{old_device, new_device}`） |
| POST | `/api/zfs/pools/{name}/scrub` | 启动 scrub |
| POST | `/api/zfs/pools/{name}/scrub/stop` | 停止 scrub |
| GET | `/api/zfs/datasets` | 数据集树 |
| POST | `/api/zfs/datasets` | 创建数据集 |
| DELETE | `/api/zfs/dataset/destroy?name=` | 销毁数据集 |
| GET | `/api/zfs/dataset/properties?name=` | 属性列表 |
| PUT | `/api/zfs/dataset/properties?name=` | 设置属性 |
| GET | `/api/zfs/snapshots?dataset=` | 快照列表 |
| POST | `/api/zfs/snapshots` | 创建快照 |
| DELETE | `/api/zfs/snapshot/destroy?name=&recursive=` | 销毁快照（recursive=true 连同依赖克隆） |
| POST | `/api/zfs/snapshot/clone` | 克隆快照（body: `{source, target, mountpoint?}`） |
| POST | `/api/zfs/snapshot/rollback?name=` | 回滚（需 `confirm: true`） |

## 外部依赖

- 系统命令：`/sbin/zfs`、`/sbin/zpool`
- crate：`regex`（名称校验）、`std::process::Command`

## 已知限制

- 未实现 send/receive（流式传输）
- 未实现 ZFS 加密管理
- 审计日志未关联操作用户（handler 未提取 AuthUser，后续补充）
- pool_create 使用 `-f` 强制创建（跳过 EFI/overlap 检查），适用于全新磁盘场景
