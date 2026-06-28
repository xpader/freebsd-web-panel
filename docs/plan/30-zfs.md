# 模块设计：ZFS 文件系统管理

> 通过子进程执行 `zfs`/`zpool`，使用 `-H -o` 机器可读输出（tab 分隔），稳定解析。环境已确认 pool `zroot` 正常运行。

## 1. 命令封装原则

- **所有列表查询用机器可读格式**：`-H`（无表头、tab 分隔）+ `-o`（指定列）+ `-p`（精确数值，不用人类可读单位）
  - 例：`zfs list -H -p -o name,used,avail,refer,mountpoint,compression,type`
- **属性查询**：`zfs get -H -p -o property,value,source all <dataset>`
- **写操作**：`zfs create|destroy|set|inherit|snapshot|rename|mount|umount`
- **pool 操作**：`zpool status|list|scrub|online|offline|attach|detach|replace`
- 所有命令带超时；失败时捕获 stderr

## 2. 数据模型

```rust
struct Zpool {
    name: String,
    state: PoolState,           // ONLINE|DEGRADED|FAULTED|OFFLINE|REMOVED|UNAVAIL
    scan: Option<ScanInfo>,     // scrub/replace 进度
    error_count: u64,           // READ/WRITE/CKSUM 错误计数
    vdevs: Vec<Vdev>,           // 顶层 vdev 树
    datasets: Vec<Dataset>,     // 直接子 dataset
}

struct Vdev {
    name: String,
    kind: VdevKind,             // mirror|raidz1|raidz2|raidz3|disk|file|spare|log|cache
    state: PoolState,
    children: Vec<Vdev>,        // mirror/raidz 的成员盘
    errors: ErrorCount,         // read/write/cksum
}

struct Dataset {
    name: String,               // zroot/jails/elastic
    typ: DatasetType,           // filesystem | volume | snapshot | bookmark
    used: u64,                  // bytes
    avail: u64,
    refer: u64,
    mountpoint: String,
    compression: String,        // on|off|lz4|zstd-19|...
    properties: IndexMap<String, PropValue>,
    children: Vec<Dataset>,
}

struct PropValue {
    value: String,
    source: String,             // local | inherited | default | ...
}

struct Snapshot {
    name: String,               // zroot/jails/elastic@snap1
    created: DateTime<Utc>,
    used: u64,
    refer: u64,
}
```

## 3. 输出解析

### 3.1 `zpool status`（树形，需缩进解析）

```
  pool: zroot
 state: ONLINE
config:
	NAME        STATE     READ WRITE CKSUM
	zroot       ONLINE       0     0     0
	  mirror-0  ONLINE       0     0     0
	    ada0p3  ONLINE       0     0     0
	    ada1p3  ONLINE       0     0     0
errors: No known data errors
```

解析策略：
- 按 tab/空格缩进深度构建 vdev 树
- 正则提取 `name state read write cksum`
- 识别 `mirror-N`、`raidz1-N`、`raidz2-N`、`raidz3-N`、`spare-N`、`log`、`cache` 关键字

### 3.2 `zfs list -H -p -o ...`（tab 分隔，简单 split）

直接按 `\t` 分割列，映射到 Dataset 结构。`-p` 保证数字为字节数，无单位歧义。

### 3.3 `zfs get -H -p -o property,value,source all <name>`

每个属性一行，构建 properties map。

## 4. API 设计

### Pool 层

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/zfs/pools` | 列出所有 pool + 状态 |
| GET | `/api/zfs/pools/:name` | pool 详情含 vdev 树 |
| POST | `/api/zfs/pools/:name/scrub` | 启动 scrub |
| POST | `/api/zfs/pools/:name/scrub/stop` | 停止 scrub |
| POST | `/api/zfs/pools` | 创建 pool（高级，需详细参数） |
| POST | `/api/zfs/pools/:name/vdevs` | 添加 vdev（扩容） |
| POST | `/api/zfs/pools/:name/attach` | attach 镜像盘 |
| POST | `/api/zfs/pools/:name/detach` | detach 盘 |
| POST | `/api/zfs/pools/:name/replace` | 替换故障盘 |

### Dataset 层

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/zfs/datasets` | 列出所有 dataset（树形） |
| GET | `/api/zfs/datasets/:name` | dataset 详情含属性 |
| POST | `/api/zfs/datasets` | 创建 dataset（body: name + 可选 properties） |
| DELETE | `/api/zfs/datasets/:name` | 销毁 dataset（`-r` 递归，需确认） |
| PUT | `/api/zfs/datasets/:name/properties` | 修改属性 |
| POST | `/api/zfs/datasets/:name/mount` | 挂载 |
| POST | `/api/zfs/datasets/:name/umount` | 卸载 |
| POST | `/api/zfs/datasets/:name/rename` | 重命名 |

### 快照层

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/zfs/datasets/:name/snapshots` | 列出快照 |
| POST | `/api/zfs/datasets/:name/snapshots` | 创建快照 |
| DELETE | `/api/zfs/snapshots/:fullname` | 删除快照 |
| POST | `/api/zfs/snapshots/:fullname/rollback` | 回滚 |
| POST | `/api/zfs/datasets/:name/clone` | 克隆为新 dataset |

### 发送/接收（复制）

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/api/zfs/send` | 发送快照到文件/远程（流式，任务化） |
| POST | `/api/zfs/receive` | 接收快照流 |

## 5. 实现里程碑

1. **M1 — zpool status 解析器**（树形 vdev + scrub 状态）
2. **M2 — zfs list/get 解析器**（tab 分隔属性）
3. **M3 — 只读 API**（pools/datasets/snapshots 查询）
4. **M4 — Dataset CRUD + 属性管理**
5. **M5 — 快照/克隆/回滚**
6. **M6 — Pool 维护**（scrub/attach/replace，复杂操作）
7. **M7 — send/receive**（流式传输，需任务队列支持）

## 6. 危险操作保护

- **destroy**（dataset/snapshot/pool）：默认 dry-run 返回影响范围（`zfs destroy -nv`），需二次确认带 `confirm: true`
- **rollback**：提示会销毁后续快照，需确认
- **replace/detach**：影响冗余，需确认
- 所有危险操作记录到审计日志（操作前后状态快照）

## 7. 风险与缓解

| 风险 | 缓解 |
|---|---|
| `zpool status` 输出跨版本微调 | 解析器对缩进和关键字容错；无法解析的行记录日志但不崩溃 |
| 属性值类型多样（number/string/boolean） | properties 统一存字符串，前端按已知属性元数据渲染控件 |
| 大量 dataset 时全量列表慢 | 支持 `?depth=N` 参数；树形懒加载 |
| send/receive 流式数据 | 转后台任务，WebSocket 推送进度（见 `70-task-queue.md`） |
