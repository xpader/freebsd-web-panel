# 08 — 文件系统

## 概述

「存储」主菜单下含两个子页面：

- **概览**（`/filesystem`）：物理磁盘、挂载点、ZFS 存储池概要
- **磁盘**（`/filesystem/disks`）：各磁盘详细参数 + 分区表 + SMART 健康（运转时长/温度/属性）

数据来自 `geom`/`mount`/`df`/`zpool` 命令实时采集。

## 实现细节

### 后端 `src/handlers/filesystem.rs`

`GET /api/filesystem/overview` 返回三部分数据：

**物理磁盘** — `geom disk list` 解析：
- 遍历输出行，每行先 trim 再剥离 `N. ` 数字前缀（`geom` 输出格式为 `1. Name: ada0`）
- 提取 `Name:`/`Mediasize:`/`descr:`/`rotationrate:` 字段
- 跳过 `Mediasize = 0` 的设备（如光驱 cd0）
- 每个 Disk：name、descr（型号）、size_bytes、rotation_rate

**挂载点** — `mount` + `df -k` 联合解析：
- `mount` 输出格式 `device on /mountpoint (fstype, options)` → 拆分提取 device/mountpoint/fstype/options
- `df -k`（1K-blocks）补充 size/used/available/capacity（按 mountpoint 匹配，`× 1024` 转字节）

**ZFS 存储池** — `zpool list -H -p` 解析：
- 机器可读格式（tab 分隔，精确数值）：NAME SIZE ALLOC FREE CKPOINT EXPANDSZ FRAG CAP DEDUP HEALTH ALTROOT
- 按列索引提取

### 磁盘详情 `GET /api/filesystem/disks`

`list_disk_details()` 合并两个 `geom` 命令的输出，构建每个磁盘的 `DiskDetail`（含分区表）：

**基础字段** — `geom disk list` 解析（`HashMap<String, DiskDetail>`，以磁盘名为键）：
- 同样剥离 `N. ` 前缀，逐行提取 `Name:`/`Mediasize:`/`Sectorsize:`/`Mode:`/`descr:`/`lunid:`/`ident:`/`rotationrate:`/`fwsectors:`/`fwheads:`
- 跳过 `Mediasize = 0` 的设备

**分区表** — `geom part list` 解析（`parse_geom_part()`）：
- 每个 `geom` 块以 `Geom name: <disk>` 开头，含顶层元数据（`scheme:`/`state:`/`first:`/`last:`/`entries:`）+ `Providers:`（分区列表）+ `Consumers:` 段
- 分区提供者行格式 `N. Name: ada0p1`，后跟 `Mediasize:`/`Sectorsize:`/`type:`/`label:`/`index:`/`start:`/`end:`/`offset:`/`rawuuid:`
- 状态机跟踪当前 geom 块、是否在 `Providers:` 段内；遇新块/`Consumers:` 时 flush 当前分区挂到对应磁盘
- 顶层元数据写入磁盘的 `scheme`/`state`/`first`/`last`/`entries`

最终 `disks.into_values()` 按名称排序返回（ada0, ada1, da0, …）。

### SMART 健康 `GET /api/filesystem/disks/{name}/smart`

`disk_smart()` 对单个磁盘运行 `smartctl -j -a /dev/<name>`，解析其 JSON 输出。`SmartctlRoot` 用宽松反序列化（全部字段 `Option` + `#[serde(default)]`），兼容 ATA/NVMe/USB 等不同协议的输出差异。

**退出码处理（关键）**：smartctl 的退出码是 bitmask 而非简单的成功/失败——bit 3（`& 8`）表示磁盘 *未通过* SMART（此时输出仍然有效），bit 1（`& 2`）表示设备无法打开。因此 handler **不把非零退出码当作错误**：用 `cmd::run_output` 拿原始 `Output`，解析 stdout JSON；仅当 JSON 解析失败时才降级为带 `note` 的空记录。

**字段提取**（统一 ATA 与 NVMe，多级回退）：
- `healthy` — `smart_status.passed`（PASSED→true / FAILED→false；None 表示不支持或未启用 SMART）
- `power_on_hours` — ATA 顶层 `power_on_time.hours` → 属性 id 9（Power_On_Hours）的 raw → NVMe log
- `power_cycle_count` — ATA 顶层 → 属性 id 12（Power_Cycle_Count）的 raw → NVMe `power_cycles`
- `temperature` — `temperature.current` → NVMe log 的 `temperature`
- `attributes`（ATA 专属）— `ata_smart_attributes.table`，每项含归一化 value/worst/thresh + raw 值；`failing` = `value ≤ thresh`（仅当 thresh > 0），前端红色标记告警属性
- `nvme`（NVMe 专属）— `nvme_smart_health_information_log`：磨损百分比（percentage_used）、可用备用块（available_spare）、媒体错误数（media_errors）、异常断电次数（unsafe_shutdowns）、控制器繁忙时间（controller_busy_time）

**降级路径**：设备不支持 SMART、未启用、或无任何 SMART 数据时（如光驱、不存在的设备名），返回 `healthy=null` + 空 `attributes` + `note` 说明原因，**不报错**。前端据 `note` 显示"不支持 SMART"。

**输入校验**：设备名经 `validate_dev_name()` 校验（`^[a-zA-Z0-9_-]{1,32}$`），防御路径分隔符/Shell 元字符注入。

### 前端 `web/js/pages/filesystem.js`

`renderFsOverview` 渲染三段：
1. **ZFS 存储池卡片**：每池一张卡，显示健康状态（ONLINE=绿/其他=红徽章）、容量、已用、碎片率、去重比、容量进度条（>80% 黄色/其他紫色）
2. **物理磁盘表格**：设备名、型号、容量、转速（`unknown` 显示 `SSD?`）
3. **挂载点表格**：设备、挂载点、类型徽章、总容量/已用/可用、使用率迷你进度条

### 前端 `DisksPage.vue`（`/filesystem/disks`）

每张磁盘卡片头部带磁盘图标，关键参数常驻、补充参数与分区表按需展开：
- **卡片默认**：头部（磁盘图标 + 磁盘名 + 型号 + 分区方案徽章 + 状态徽章 + 总容量）+ 右上角按钮组【详情】【SMART】+ 关键参数网格（`stat-grid`，常驻：设备路径/序列号/扇区大小/转速/访问模式）+ 已分配进度条（分区大小之和 / 总容量）
- **详情**（点【详情】内联展开）：补充参数网格（型号/LUN ID/分区方案/GPT 条目上限/固件扇区/固件磁头）+ 分区表（设备/类型/标签/大小/起止扇区/UUID；UUID 截断显示，点击复制到剪贴板并 toast 提示）
- **SMART**（点【SMART】打开模态对话框）：声明式 modal（复用 `.modal-overlay/.modal-wide` 样式），**首次打开才请求** `/api/filesystem/disks/{name}/smart`（按需加载，不在列表挂载时对每盘 spawn smartctl）。内容含健康徽章（PASSED=绿 / FAILED=红 / 未知=黄）、运转时长、温度（≥50°C 黄 / ≥60°C 红）、通电次数；NVMe 盘额外显示磨损度/可用备用/媒体错误/异常断电；ATA 盘显示完整属性表（failing 属性带红色 badge）。ESC 键 / 点击遮罩 / 关闭按钮均可关闭；对话框内可刷新

### 菜单集成

`menu.js` 的「存储」主菜单含：「概览」（`/filesystem`）+「磁盘」（`/filesystem/disks`）+「文件管理器」（`/filesystem/files`，见 [10-file-manager.md](10-file-manager.md)）+「ZFS」（`/zfs`）四个子项。

## API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/filesystem/overview` | 磁盘 + 挂载点 + ZFS 池概览 |
| GET | `/api/filesystem/disks` | 各磁盘详细参数 + 分区表 |
| GET | `/api/filesystem/disks/{name}/smart` | 单个磁盘的 SMART 健康数据（运转时长/温度/属性/NVMe 指标） |

## 外部依赖

- 系统命令：`/sbin/geom`（disk list、part list）、`/sbin/mount`、`/bin/df`、`/sbin/zpool`
- `/usr/local/sbin/smartctl`（smartmontools port）：SMART 健康/属性/温度采集，`-j -a` 输出 JSON。系统未安装时 handler 仍响应（返回带 `note` 的空记录）

## 已知限制

- SMART 健康数据依赖系统安装 smartmontools（`pkg install smartmontools`）；未安装时点【SMART】打开的对话框显示"不支持"提示
- 非 ZFS 的 UFS/MSDOSFS 挂载点也能显示（通过 mount+df），但无专门管理
- 列表无搜索/过滤（挂载点多时可考虑后续加分页）
