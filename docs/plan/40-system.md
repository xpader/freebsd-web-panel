# 模块设计：系统配置（sysctl / rc.conf / 网络 / 服务 / pf）

## 1. sysctl 管理

### 1.1 实现

- 读取：子进程 `sysctl <name>` 或 `sysctl -a`（全量）
- **机器可读读取**：`sysctl -N -a`（仅名字）+ `sysctl -T <name>` 查类型；或 `sysctl -e -q <name>` 取裸值
- 写入：子进程 `sysctl <name>=<value>`（需 root）
- **持久化**：编辑 `/etc/sysctl.conf`（key=value 格式，简单文本解析）

### 1.2 数据模型

```rust
struct Sysctl {
    name: String,               // kern.hostname
    value: SysctlValue,
    typ: SysctlType,            // string | int | uint | long | ulong | struct | opaque
    description: Option<String>,// sysctl -d
    writable: bool,
    // 持久化状态
    in_sysctl_conf: bool,
    configured_value: Option<String>,
}
```

### 1.3 安全策略

- **只读 sysctl 拒绝写入**（前端禁用控件；后端校验）
- 危险 sysctl（如 `kern.securelevel`、`vm.swap_enabled`）标记警告，需二次确认
- sysctl.conf 编辑用原子替换 + 备份

### 1.4 API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/sysctl` | 列出全部（分页/搜索 `?q=`） |
| GET | `/api/sysctl/:name` | 详情 |
| PUT | `/api/sysctl/:name` | 运行时设置 + 可选持久化（body: value, persist: bool） |
| GET | `/api/sysctl.conf` | 读取持久化配置 |
| PUT | `/api/sysctl.conf` | 整体替换 sysctl.conf |

## 2. rc.conf 管理

### 2.1 实现

- **读写用 `sysrc`**（系统工具，处理 `/etc/rc.conf`、`/etc/defaults/rc.conf`、`/etc/rc.conf.d/*` 合并语义）
- 读取单值：`sysrc -n <key>`（裸值，无 key 前缀）
- 读取全部：`sysrc -a`（key: value）
- 写入：`sysrc <key>=<value>`；删除：`sysrc -x <key>`
- 也可直接解析 `/etc/rc.conf`（sh 语法子集：`key="value"` / `key=YES` / 注释）

### 2.2 数据模型

```rust
struct RcConfEntry {
    key: String,
    value: String,
    source: RcSource,           // /etc/rc.conf | /etc/rc.conf.d/* | defaults
    enabled: bool,              // *_enable="YES" 的便捷判断
    description: Option<String>,// 来自 rc.conf(5) 已知 key 描述库
}
```

### 2.3 分类视图

rc.conf key 数百个，前端按功能分组：
- 基础（hostname、defaultrouter、keymap）
- 网络接口（ifconfig_*、cloned_interfaces、vlans_*）
- 服务开关（*_enable / *_flags）
- Jail（jail_enable、jail_list、jail_*）
- ZFS/Bhyve（zfs_enable、vm_enable、vm_dir）

key 描述库：内置常见 key 的说明（静态 map），未知 key 标"自定义"。

### 2.4 API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/rcconf` | 列出全部（支持 `?category=` 过滤） |
| GET | `/api/rcconf/:key` | 单值 |
| PUT | `/api/rcconf/:key` | 设置 |
| DELETE | `/api/rcconf/:key` | 删除 |

## 3. 网络管理

### 3.1 实现

- 接口查询：`ifconfig -a`（文本解析）或更稳健用 `ifconfig <if> inet/in6`
- **更优方案**：`ifconfig -l`（接口名列表）+ 逐个 `ifconfig <if>` 解析
- 路由：`netstat -rn`（路由表）+ `route get default`（默认网关）
- 修改：`ifconfig <if> inet <ip> netmask <mask>`（子进程，需 root）
- 别名：`ifconfig <if> alias <ip> netmask <mask>` / `delete`
- VLAN：`ifconfig <if>.<vlan> create` / `vnet`
- 持久化：写回 rc.conf 的 `ifconfig_<if>` / `ifconfig_<if>_aliases`

### 3.2 数据模型

```rust
struct NetworkInterface {
    name: String,               // bge0
    flags: Vec<String>,         // UP,BROADCAST,RUNNING,SIMPLEX,MULTICAST
    mtu: u32,
    mac: String,
    media: Option<String>,      // 1000baseT <full-duplex>
    ipv4: Vec<IpConfig>,
    ipv6: Vec<IpConfig>,
    description: Option<String>,// ifconfig_<if>_description
}

struct IpConfig {
    address: IpAddr,
    prefix_len: u8,
    is_alias: bool,
}

struct Route {
    destination: String,
    gateway: String,
    flags: String,
    interface: String,
}
```

### 3.3 API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/network/interfaces` | 列出全部接口 |
| GET | `/api/network/interfaces/:name` | 接口详情 |
| PUT | `/api/network/interfaces/:name` | 修改 IP/MTU/flags（运行时 + 持久化） |
| POST | `/api/network/interfaces/:name/aliases` | 添加别名 |
| DELETE | `/api/network/interfaces/:name/aliases/:ip` | 删除别名 |
| GET | `/api/network/routes` | 路由表 |
| GET | `/api/network/default-gateway` | 默认网关 |
| PUT | `/api/network/default-gateway` | 修改默认网关（+ 持久化 defaultrouter） |

## 4. 服务管理（rc.d）

### 4.1 实现

- 列表：`service -l`（可用服务名）+ `service -e`（已启用服务）
- 状态：`service <name> status`
- 控制：`service <name> start|stop|restart|reload`
- `rcorder /etc/rc.d/* /usr/local/etc/rc.d/*` 得启动顺序

### 4.2 API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/services` | 列出服务（含 enabled 状态） |
| GET | `/api/services/:name` | 服务详情（rcvar + status） |
| POST | `/api/services/:name/start` | 启动 |
| POST | `/api/services/:name/stop` | 停止 |
| POST | `/api/services/:name/restart` | 重启 |
| POST | `/api/services/:name/reload` | 重载 |

## 5. 防火墙（pf）

### 5.1 实现

- 状态：`pfctl -s info`（运行状态 + 计数器）
- 规则：`pfctl -sr`（规则集）/ `pfctl -sn`（NAT）
- 表：`pfctl -t <table> -T show`
- 加载/重载：`pfctl -f /etc/pf.conf`
- 启停：`pfctl -e` / `pfctl -d`
- 配置：解析 `/etc/pf.conf`（宏、表、规则；语法较复杂，初版做只读展示 + 基本编辑）

### 5.2 API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/pf/status` | pf 运行状态 |
| GET | `/api/pf/rules` | 规则列表 |
| GET | `/api/pf/nat` | NAT 规则 |
| GET | `/api/pf/tables` | 表列表 |
| GET | `/api/pf/tables/:name` | 表内容 |
| POST | `/api/pf/enable` | 启用 |
| POST | `/api/pf/disable` | 禁用 |
| POST | `/api/pf/reload` | 重载规则 |
| GET | `/api/pf.conf` | 读取配置文件 |
| PUT | `/api/pf.conf` | 写入配置文件 + 校验（`pfctl -n -f`） |

## 6. 实现里程碑

1. **M1 — sysctl 读写 + sysctl.conf 解析**
2. **M2 — rc.conf（sysrc 封装）+ 分类描述库**
3. **M3 — 网络接口查询 + 修改**
4. **M4 — 服务管理**
5. **M5 — pf 状态 + 规则只读**
6. **M6 — pf 配置编辑（含语法校验）**
