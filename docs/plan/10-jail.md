# 模块设计：Jail 管理

> 约束：**不依赖任何第三方 jail 工具**（iocage/ezjail/jailutils 等全部排除）。仅使用 FreeBSD 基础系统的 `jail(8)` 和 `libjail`。

## 1. 数据模型

```rust
/// 一个 jail 的完整定义（持久化到 /etc/jail.conf）
struct Jail {
    name: String,
    enabled: bool,            // 是否在 jail_list 中（开机自启）
    // 核心参数
    path: String,             // path=
    host_hostname: Option<String>,
    ip4: IpMode,              // inherit | new(由 ip4.addr 隐含) | disable
    ip4_addr: Vec<IpAddr>,    // ip4.addr
    ip6: IpMode,
    ip6_addr: Vec<IpAddr>,
    interface: Option<String>,
    vnet: bool,               // vnet = new
    // 执行
    exec_start: Vec<String>,
    exec_stop: Vec<String>,
    exec_clean: bool,
    // devfs/mount
    mount_devfs: bool,
    devfs_ruleset: Option<u32>,
    mount_fstab: Option<String>,
    // allow.* 权限（布尔标志集合）
    allow_raw_sockets: bool,
    allow_mount: bool,
    allow_mount_devfs: bool,
    allow_mount_zfs: bool,
    allow_mount_procfs: bool,
    allow_mount_tmpfs: bool,
    allow_socket_af: bool,
    allow_sysvipc: bool,
    allow_chflags: bool,
    allow_quotas: bool,
    allow_read_msgbuf: bool,
    allow_reserved_ports: bool,
    allow_set_hostname: bool,
    // 其他
    enforce_statfs: u8,       // 0|1|2
    persist: bool,
    securelevel: Option<i32>,
    children_max: Option<u32>,
    // 原始参数（透传未在结构体中显式建模的参数，避免功能受限）
    extra_params: IndexMap<String, ParamValue>,
}

enum IpMode { Inherit, New, Disable }

enum ParamValue {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<String>),
}

/// 运行中 jail 的实时状态（来自 jailparam_get）
struct JailRuntime {
    jid: i32,
    name: String,
    running: bool,
    dying: bool,
    parent_jid: i32,
    cpuset_id: i32,
    children_cur: u32,
    osrelease: String,
    path: String,
    hostname: String,
    ip4_addr: Vec<IpAddr>,
    ip6_addr: Vec<IpAddr>,
}
```

## 2. 配置文件解析器（jail.conf）

### 2.1 语法要点（来自 jail.conf(5)）

```
exec.clean;                          // 全局参数（无值=布尔true）
exec.system_user = "root";           // 键值对
path="/jails/${name}";               // 变量替换 ${var} 或 $var

jailname {                           // jail 定义块
    interface = "bge1";              // = 赋值
    exec.start += "/bin/sh /etc/rc"; // += 追加到列表
    mount.devfs;                     // 布尔简写
    devfs_ruleset = "4";             // 引号字符串（数字也常带引号）
    allow.nomount;                   // no 前缀 = false
    ip4.addr = 10.1.1.1, 10.1.1.2;   // 逗号分隔列表
}
```

### 2.2 解析器要求

- **递归下降解析器**，手写（语法简单，不值得引入 parser combinator 库）
- 支持注释 `#`（行注释）和 `/* */` 块注释
- 支持单引号（无变量替换）、双引号（有变量替换）、裸 token
- 支持反斜杠转义（C 风格 `\n \t \\ \xNN \NNN`）
- 区分三种作用域：
  - **全局参数**（文件顶部，所有 jail 继承）
  - **jail 块内参数**（覆盖/补充全局）
  - **变量定义**（`$var = "value";`）
- **变量替换**在读取时展开（`$name`、`${name}`、`${host.hostname}`）
- 列表参数：`+=` 追加、`=` 覆盖、逗号语法糖

### 2.3 写回要求

- **保留注释和原始格式**（不破坏用户手动编辑的内容）
- **最小化 diff** — 仅修改变化的 jail 块/参数
- 实现方式：解析为 AST（保留 token 的行列位置），编辑 AST 节点，序列化时使用原始位置信息尽量保持格式
- 写文件前做 **原子替换**（`write tmp + rename`），并对原文件备份到 `/var/db/fwp/backup/jail.conf.<timestamp>`

## 3. libjail FFI 绑定

### 3.1 C API（来自 `/usr/include/jail.h`）

```c
struct jailparam {
    char    *jp_name;
    void    *jp_value;
    size_t   jp_valuelen;
    size_t   jp_elemlen;
    int      jp_ctltype;
    int      jp_structtype;
    unsigned jp_flags;
};

int  jail_getid(const char *name);
char *jail_getname(int jid);
int  jail_setv(int flags, ...);           // 变长参数 (name,value,...,NULL)
int  jail_getv(int flags, ...);
int  jailparam_all(struct jailparam **jpp);
int  jailparam_init(struct jailparam *jp, const char *name);
int  jailparam_import(struct jailparam *jp, const char *value);
int  jailparam_import_raw(struct jailparam *jp, void *value, size_t valuelen);
int  jailparam_set(struct jailparam *jp, unsigned njp, int flags);
int  jailparam_get(struct jailparam *jp, unsigned njp, int flags);
char *jailparam_export(struct jailparam *jp);
void jailparam_free(struct jailparam *jp, unsigned njp);
int  jail_remove(int jid);
extern char jail_errmsg[JAIL_ERRMSGLEN];  // 错误消息缓冲区
```

### 3.2 Rust 绑定设计

```rust
mod sys {
    use libc::{c_int, c_char, c_void, c_uint, size_t};

    #[repr(C)]
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
        pub fn jail_getid(name: *const c_char) -> c_int;
        pub fn jail_remove(jid: c_int) -> c_int;
        pub fn jailparam_all(jpp: *mut *mut Jailparam) -> c_int;
        pub fn jailparam_init(jp: *mut Jailparam, name: *const c_char) -> c_int;
        pub fn jailparam_import(jp: *mut Jailparam, value: *const c_char) -> c_int;
        pub fn jailparam_import_raw(jp: *mut Jailparam, value: *mut c_void, valuelen: size_t) -> c_int;
        pub fn jailparam_set(jp: *mut Jailparam, njp: c_uint, flags: c_int) -> c_int;
        pub fn jailparam_get(jp: *mut Jailparam, njp: c_uint, flags: c_int) -> c_int;
        pub fn jailparam_export(jp: *mut Jailparam) -> *mut c_char;
        pub fn jailparam_free(jp: *mut Jailparam, njp: c_uint);
        pub static mut jail_errmsg: [c_char; 1024];
    }

    pub const JAIL_CREATE: c_int  = 0x01;
    pub const JAIL_UPDATE: c_int  = 0x02;
    pub const JAIL_ATTACH: c_int  = 0x04;
    pub const JAIL_DYING:  c_int  = 0x08;
    pub const JAIL_GET_MASK: c_int = 0x10; // mask 用于 get
}
```

链接：`Cargo.toml` 或 `build.rs` 中添加 `println!("cargo:rustc-link-lib=jail");`

### 3.3 安全封装

```rust
/// 高层 API
pub struct JailHandle { params: Vec<Jailparam> }

impl JailHandle {
    /// 列出所有运行中的 jail（替代 jls）
    pub fn list() -> Result<Vec<JailRuntime>>;

    /// 获取指定 jail 的完整参数
    pub fn get(name_or_jid: &str) -> Result<JailRuntime>;

    /// 创建/修改 jail（JAIL_CREATE / JAIL_UPDATE）
    pub fn set(name: &str, params: &[(String, String)], create: bool) -> Result<i32>;

    /// 删除 jail
    pub fn remove(jid: i32) -> Result<()>;
}

/// RAII: 析构时调用 jailparam_free
impl Drop for JailHandle { ... }
```

错误处理：检查 `jail_errmsg` + `errno`，映射到 `JailError` 枚举（NotFound/Exists/InvalidParam/SysErr）。

## 4. 功能清单与 API

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/jails` | 列出 jail.conf 定义 + 运行时状态合并 |
| GET | `/api/jails/:name` | 单个 jail 详情（配置 + 运行状态） |
| POST | `/api/jails` | 创建新 jail（写 jail.conf + 可选立即启动） |
| PUT | `/api/jails/:name` | 修改 jail 配置（运行中则 jail -m 修改，必要时重启） |
| DELETE | `/api/jails/:name` | 删除 jail 定义（先 stop） |
| POST | `/api/jails/:name/start` | 启动（jailparam_set + JAIL_CREATE） |
| POST | `/api/jails/:name/stop` | 停止（jail_remove） |
| POST | `/api/jails/:name/restart` | 重启（remove + create） |
| GET | `/api/jails/:name/console` | WebSocket — 通过 `jexec` 进入 jail 的伪终端 |
| GET | `/api/jails/params` | 枚举可用参数（jailparam_all） |

## 5. 实现里程碑

1. **M1 — jail.conf 解析器 + 序列化器**（纯 Rust，带单元测试，覆盖 `/etc/jail.conf` 真实文件）
2. **M2 — libjail FFI 绑定**（unsafe 封装 + 安全层 + 集成测试）
3. **M3 — 配置 CRUD API**（不涉及运行时控制，纯文件操作）
4. **M4 — 运行时控制 API**（start/stop/restart/list 状态）
5. **M5 — 控制台 WebSocket**（jexec + PTY）

## 6. 风险与缓解

| 风险 | 缓解 |
|---|---|
| jail.conf 格式写回破坏用户注释 | AST 保留位置；最小 diff；写前备份 |
| libjail FFI 内存安全 | RAII (Drop) + 所有 unsafe 集中在 `sys` 模块 + miri 可测部分用纯逻辑分离 |
| `jailparam_import` 不识别复杂类型 | 对已知核心参数硬编码类型；未知参数透传字符串 |
| 并发修改 jail.conf | 文件级写锁（`flock` + atomic rename） |
