# 09 — ZFS 管理

## 概述

通过 `zfs`/`zpool` 命令的 `-H -p` 机器可读输出实现 ZFS 存储池、数据集、快照的完整管理。包含三级子菜单（Zpool 管理 / 数据集管理 / 快照管理）。

## 实现细节

### 后端 `src/handlers/zfs.rs`

所有命令通过 `run(cmd, args)` 封装执行，输入经 `validate_name()` / `validate_prop_key()` 校验（正则白名单），参数以 `Command::arg()` 传递（防注入）。

**Zpool**：

| 函数 | 命令 | 说明 |
|---|---|---|
| `pool_list` | `zpool list -H -p` | 所有 pool 摘要（tab 分隔，精确数值） |
| `pool_status` | `zpool status <name>` | 详细状态 + VDEV 树 |
| `pool_scrub` | `zpool scrub <name>` | 启动 scrub |
| `pool_scrub_stop` | `zpool scrub -s <name>` | 停止 scrub |

VDEV 树解析：`zpool status` 输出中 `config:` 后的行以 tab 缩进表示层级。按 `line.starts_with('\t')` 检测，`indent = line.len() - line.trim_start().len()` 计算层级，递归 `build_vdev_tree()` 构建树。

**数据集**：

| 函数 | 命令 | 说明 |
|---|---|---|
| `dataset_list` | `zfs list -H -p -o name,used,avail,refer,mountpoint,type,compression` | 全部数据集，`build_dataset_tree()` 构建父子树 |
| `dataset_create` | `zfs create [-o k=v]... <name>` | 创建（含可选属性） |
| `dataset_destroy` | `zfs destroy -r <name>` | 递归销毁 |
| `dataset_set` | `zfs set k=v <name>` | 设置属性 |
| `dataset_properties` | `zfs get -H -p -o property,value,source all <name>` | 全部属性 |

数据集树构建：按 `/` 分割名称确定父子关系，`HashMap<String, Dataset>` + `parent_map` 递归组装。

**快照**：

| 函数 | 命令 | 说明 |
|---|---|---|
| `snapshot_list` | `zfs list -t snapshot -H -p -o name,used,refer,creation [dataset]` | 快照列表（可按 dataset 过滤） |
| `snapshot_create` | `zfs snapshot <dataset>@<name>` | 创建快照 |
| `snapshot_destroy` | `zfs destroy <dataset>@<name>` | 销毁快照 |
| `snapshot_rollback` | `zfs rollback -r <dataset>@<name>` | 回滚（需 `confirm: true`，`-r` 销毁更新快照） |

快照名 `dataset@snapname` 经 `validate_name()` 校验（允许 `@`）。

### 前端 `web/js/pages/zfs.js`

三个页面：

**Zpool 管理**（`/zfs/pools`）：
- Pool 卡片列表（健康状态徽章 + 容量/已用/碎片/去重 + 进度条）
- Scrub 按钮
- `<details>` 可展开 VDEV 树（懒加载 `pool_status` API）

**数据集管理**（`/zfs/datasets`）：
- 树形表格（缩进表示层级）
- 创建数据集（prompt 输入名称）
- 删除数据集（确认对话框，仅非顶层 pool 可删）
- 查看属性（模态框表格）

**快照管理**（`/zfs/snapshots`）：
- 快照表格（数据集/快照名/已用/引用/时间）
- 过滤框（实时过滤）
- 创建快照（prompt dataset + name）
- 删除/回滚（确认对话框，回滚需二次确认）

### 三级菜单 `web/js/ui/layout.js`

菜单项支持 `children` 数组：
- 有 `children` → 可折叠子组（组头无链接，展开/收起子项）
- 无 `children` → 直接链接

展开逻辑：任一子项为当前路由时自动展开（`expanded` 类）。「文件系统」→「ZFS」下有三个子子项：Zpool 管理 / 数据集管理 / 快照管理。

CSS：`.sub-group` / `.sub-group-header` / `.sub-items` / `.sub-item`（缩进 40px，12px 字号）。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/zfs/pools` | Pool 列表 |
| GET | `/api/zfs/pools/{name}` | Pool 详情（含 VDEV 树） |
| POST | `/api/zfs/pools/{name}/scrub` | 启动 scrub |
| POST | `/api/zfs/pools/{name}/scrub/stop` | 停止 scrub |
| GET | `/api/zfs/datasets` | 数据集树 |
| POST | `/api/zfs/datasets` | 创建数据集 |
| DELETE | `/api/zfs/datasets/{name}` | 销毁数据集 |
| GET | `/api/zfs/datasets/{name}/properties` | 属性列表 |
| PUT | `/api/zfs/datasets/{name}/properties` | 设置属性 |
| GET | `/api/zfs/snapshots?dataset=` | 快照列表 |
| POST | `/api/zfs/snapshots` | 创建快照 |
| DELETE | `/api/zfs/snapshots/{full}` | 销毁快照 |
| POST | `/api/zfs/snapshots/{full}/rollback` | 回滚（需 `confirm: true`） |

## 外部依赖

- 系统命令：`/sbin/zfs`、`/sbin/zpool`
- crate：`regex`（名称校验）、`std::process::Command`

## 已知限制

- 未实现 clone（快照克隆为新数据集）
- 未实现 send/receive（流式传输）
- 未实现 vdev 添加/替换/attach/detach（pool 扩容/维护）
- 未实现 ZFS 加密管理
- 审计日志未关联操作用户（handler 未提取 AuthUser，后续补充）
- 快照名含 `/` 时 URL 路径需注意编码（当前用路径参数 `{full}` 捕获）
