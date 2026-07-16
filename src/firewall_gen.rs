//! Firewall rule types, config-file generators, and DB access.
//!
//! Defines the driver-agnostic structured rule model, generates ipfw shell
//! scripts and pf.conf files from that model, and provides SQLite CRUD for
//! rules + firewall state.

use std::fs;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::cmd;
use crate::error::{ApiError, ApiResult};

// ── binary paths ───────────────────────────────────────────────────

const IPFW: &str = "/sbin/ipfw";
const PFCTL: &str = "/sbin/pfctl";
const KLDLOAD: &str = "/sbin/kldload";
const KLDSTAT: &str = "/sbin/kldstat";
const SYSCTL: &str = "/sbin/sysctl";
const SH: &str = "/bin/sh";

const IPFW_RULES_PATH: &str = "/etc/ipfw.rules";
const PF_CONF_PATH: &str = "/etc/pf.conf";

// ── enums ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirewallDriver {
    Ipfw,
    Pf,
}

impl FirewallDriver {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ipfw => "ipfw",
            Self::Pf => "pf",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "ipfw" => Some(Self::Ipfw),
            "pf" => Some(Self::Pf),
            _ => None,
        }
    }

    pub fn module_loaded(&self) -> bool {
        cmd::status_sync(KLDSTAT, &["-q", "-n", self.as_str()])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FirewallMode {
    Whitelist,
    Blacklist,
}

impl FirewallMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Whitelist => "whitelist",
            Self::Blacklist => "blacklist",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "whitelist" => Some(Self::Whitelist),
            "blacklist" => Some(Self::Blacklist),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Deny,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleProtocol {
    Tcp,
    Udp,
    Icmp,
    Icmpv6,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressKind {
    Any,
    Single,
    Cidr,
    Me,
    Table,
}

// ── IP table types ──

#[derive(Debug, Clone, serde::Serialize)]
pub struct IpTableEntry {
    pub id: i64,
    pub table_id: i64,
    pub address: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IpTable {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub entries: Vec<IpTableEntry>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TableBody {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct EntryBody {
    pub address: String,
}

// ── core structs ───────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AddressSpec {
    pub kind: AddressKind,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FirewallRule {
    pub id: i64,
    pub driver: FirewallDriver,
    pub position: u32,
    pub enabled: bool,
    pub action: RuleAction,
    pub direction: RuleDirection,
    pub protocol: RuleProtocol,
    pub source: AddressSpec,
    pub source_port: Option<String>,
    pub destination: AddressSpec,
    pub destination_port: Option<String>,
    pub interface: Option<String>,
    pub log: bool,
    pub icmp_type: Option<String>,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Fields accepted when creating or updating a rule.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct RuleBody {
    pub action: RuleAction,
    pub direction: RuleDirection,
    pub protocol: RuleProtocol,
    pub source: AddressSpec,
    #[serde(default)]
    pub source_port: Option<String>,
    pub destination: AddressSpec,
    #[serde(default)]
    pub destination_port: Option<String>,
    #[serde(default)]
    pub interface: Option<String>,
    #[serde(default)]
    pub log: bool,
    #[serde(default)]
    pub icmp_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

// ── DB access ──────────────────────────────────────────────────────

pub fn get_state(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM firewall_state WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

pub fn set_state(conn: &Connection, key: &str, value: &str) -> ApiResult<()> {
    conn.execute(
        "INSERT INTO firewall_state (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

pub fn list_rules(conn: &Connection) -> ApiResult<Vec<FirewallRule>> {
    let mut stmt = conn.prepare(
        "SELECT id, driver, position, enabled, action, direction, protocol, \
         src_kind, src_value, src_port, dst_kind, dst_value, dst_port, \
         interface, log, icmp_type, description, created_at, updated_at \
         FROM firewall_rules ORDER BY position ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FirewallRule {
                id: r.get(0)?,
                driver: FirewallDriver::from_str(&r.get::<_, String>(1)?)
                    .unwrap_or(FirewallDriver::Ipfw),
                position: r.get::<_, i64>(2)? as u32,
                enabled: r.get::<_, i64>(3)? != 0,
                action: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(RuleAction::Allow),
                direction: serde_json::from_str(&r.get::<_, String>(5)?)
                    .unwrap_or(RuleDirection::In),
                protocol: serde_json::from_str(&r.get::<_, String>(6)?)
                    .unwrap_or(RuleProtocol::Any),
                source: AddressSpec {
                    kind: serde_json::from_str(&r.get::<_, String>(7)?)
                        .unwrap_or(AddressKind::Any),
                    value: r.get(8)?,
                },
                source_port: r.get(9)?,
                destination: AddressSpec {
                    kind: serde_json::from_str(&r.get::<_, String>(10)?)
                        .unwrap_or(AddressKind::Any),
                    value: r.get(11)?,
                },
                destination_port: r.get(12)?,
                interface: r.get(13)?,
                log: r.get::<_, i64>(14)? != 0,
                icmp_type: r.get(15)?,
                description: r.get(16)?,
                created_at: r.get(17)?,
                updated_at: r.get(18)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn count_enabled_rules(conn: &Connection) -> ApiResult<i64> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM firewall_rules WHERE enabled = 1",
        [],
        |r| r.get(0),
    )?;
    Ok(n)
}

pub fn next_position(conn: &Connection) -> ApiResult<u32> {
    let max: Option<i64> = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) FROM firewall_rules",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(max.map(|v| v as u32 + 1).unwrap_or(0))
}

pub fn create_rule(
    conn: &Connection,
    body: &RuleBody,
    now: i64,
) -> ApiResult<i64> {
    let pos = next_position(conn)?;
    conn.execute(
        "INSERT INTO firewall_rules \
         (driver, position, enabled, action, direction, protocol, \
          src_kind, src_value, src_port, dst_kind, dst_value, dst_port, \
          interface, log, icmp_type, description, created_at, updated_at) \
         VALUES ('ipfw', ?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
        params![
            pos,
            serde_json::to_string(&body.action).unwrap_or_default(),
            serde_json::to_string(&body.direction).unwrap_or_default(),
            serde_json::to_string(&body.protocol).unwrap_or_default(),
            serde_json::to_string(&body.source.kind).unwrap_or_default(),
            body.source.value,
            body.source_port,
            serde_json::to_string(&body.destination.kind).unwrap_or_default(),
            body.destination.value,
            body.destination_port,
            body.interface,
            body.log as i64,
            body.icmp_type,
            body.description,
            now,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_rule(
    conn: &Connection,
    id: i64,
    body: &RuleBody,
    now: i64,
) -> ApiResult<()> {
    let n = conn.execute(
        "UPDATE firewall_rules SET \
         action = ?1, direction = ?2, protocol = ?3, \
         src_kind = ?4, src_value = ?5, src_port = ?6, \
         dst_kind = ?7, dst_value = ?8, dst_port = ?9, \
         interface = ?10, log = ?11, icmp_type = ?12, description = ?13, updated_at = ?14 \
         WHERE id = ?15",
        params![
            serde_json::to_string(&body.action).unwrap_or_default(),
            serde_json::to_string(&body.direction).unwrap_or_default(),
            serde_json::to_string(&body.protocol).unwrap_or_default(),
            serde_json::to_string(&body.source.kind).unwrap_or_default(),
            body.source.value,
            body.source_port,
            serde_json::to_string(&body.destination.kind).unwrap_or_default(),
            body.destination.value,
            body.destination_port,
            body.interface,
            body.log as i64,
            body.icmp_type,
            body.description,
            now,
            id,
        ],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("firewall rule not found".into()));
    }
    Ok(())
}

pub fn delete_rule(conn: &Connection, id: i64) -> ApiResult<()> {
    let n = conn.execute(
        "DELETE FROM firewall_rules WHERE id = ?1",
        params![id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("firewall rule not found".into()));
    }
    Ok(())
}

pub fn toggle_rule(conn: &Connection, id: i64) -> ApiResult<()> {
    let n = conn.execute(
        "UPDATE firewall_rules SET enabled = 1 - enabled WHERE id = ?1",
        params![id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("firewall rule not found".into()));
    }
    Ok(())
}

pub fn reorder_rules(
    conn: &Connection,
    ordered_ids: &[i64],
) -> ApiResult<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt =
            tx.prepare("UPDATE firewall_rules SET position = ?1 WHERE id = ?2")?;
        for (pos, id) in ordered_ids.iter().enumerate() {
            stmt.execute(params![pos as i64, id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

// ── IP table CRUD ──────────────────────────────────────────────────

pub fn list_tables(conn: &Connection) -> ApiResult<Vec<IpTable>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, description, created_at, updated_at \
         FROM firewall_tables ORDER BY name ASC",
    )?;
    let mut tables: Vec<IpTable> = stmt
        .query_map([], |r| {
            Ok(IpTable {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
                entries: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);

    // Load entries for each table in one pass
    let mut entry_stmt = conn.prepare(
        "SELECT id, table_id, address, created_at \
         FROM firewall_table_entries ORDER BY id ASC",
    )?;
    let entries: Vec<(i64, IpTableEntry)> = entry_stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(1)?,
                IpTableEntry {
                    id: r.get(0)?,
                    table_id: r.get(1)?,
                    address: r.get(2)?,
                    created_at: r.get(3)?,
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for table in &mut tables {
        table.entries = entries
            .iter()
            .filter(|(tid, _)| *tid == table.id)
            .map(|(_, e)| e.clone())
            .collect();
    }

    Ok(tables)
}

pub fn get_table_by_name(conn: &Connection, name: &str) -> ApiResult<Option<IpTable>> {
    let tables = list_tables(conn)?;
    Ok(tables.into_iter().find(|t| t.name == name))
}

pub fn create_table(
    conn: &Connection,
    body: &TableBody,
    now: i64,
) -> ApiResult<i64> {
    conn.execute(
        "INSERT INTO firewall_tables (name, description, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?3)",
        params![body.name, body.description, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_table(
    conn: &Connection,
    id: i64,
    body: &TableBody,
    now: i64,
) -> ApiResult<()> {
    let n = conn.execute(
        "UPDATE firewall_tables SET name = ?1, description = ?2, updated_at = ?3 WHERE id = ?4",
        params![body.name, body.description, now, id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("firewall table not found".into()));
    }
    Ok(())
}

pub fn delete_table(conn: &Connection, id: i64) -> ApiResult<()> {
    let n = conn.execute(
        "DELETE FROM firewall_tables WHERE id = ?1",
        params![id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("firewall table not found".into()));
    }
    Ok(())
}

pub fn add_entry(
    conn: &Connection,
    table_id: i64,
    address: &str,
    now: i64,
) -> ApiResult<i64> {
    conn.execute(
        "INSERT INTO firewall_table_entries (table_id, address, created_at) VALUES (?1, ?2, ?3)",
        params![table_id, address, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn delete_entry(conn: &Connection, table_id: i64, entry_id: i64) -> ApiResult<()> {
    let n = conn.execute(
        "DELETE FROM firewall_table_entries WHERE id = ?1 AND table_id = ?2",
        params![entry_id, table_id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("table entry not found".into()));
    }
    Ok(())
}

/// Validate a table name: alphanumeric + underscore/hyphen, 1-32 chars, start with letter.
pub fn validate_table_name(name: &str) -> ApiResult<()> {
    let re = regex::Regex::new(r"^[a-zA-Z][a-zA-Z0-9_-]{0,31}$").unwrap();
    if !re.is_match(name) {
        return Err(ApiError::BadRequest(
            "table name must start with a letter, contain only alphanumeric, underscore, or hyphen (max 32 chars)".into(),
        ));
    }
    Ok(())
}

/// Validate an IP/CIDR address for table entries.
pub fn validate_address(addr: &str) -> ApiResult<()> {
    if addr.is_empty() || addr.len() > 50 {
        return Err(ApiError::BadRequest("invalid address".into()));
    }
    let re = regex::Regex::new(
        r"^[0-9a-fA-F:.]+(/\d{1,3})?$",
    ).unwrap();
    if !re.is_match(addr) {
        return Err(ApiError::BadRequest("invalid IP/CIDR address".into()));
    }
    Ok(())
}

// ── config generation ──────────────────────────────────────────────

fn header(driver: FirewallDriver, mode: FirewallMode) -> String {
    let mode_desc = if mode == FirewallMode::Whitelist {
        "default deny"
    } else {
        "default allow"
    };
    format!(
        "# ============================================================\n\
         # Managed by FreeBSD Web Panel (fwp) - DO NOT EDIT MANUALLY\n\
         # Driver: {driver} | Mode: {mode} ({mode_desc})\n\
         # ============================================================\n\n",
        driver = driver.as_str(),
        mode = mode.as_str(),
        mode_desc = mode_desc,
    )
}

fn address_ipfw(addr: &AddressSpec) -> String {
    match addr.kind {
        AddressKind::Any => "any".into(),
        AddressKind::Me => "me".into(),
        AddressKind::Single | AddressKind::Cidr => addr.value.clone(),
        AddressKind::Table => format!("table\\({}\\)", addr.value),
    }
}

fn address_pf(addr: &AddressSpec, af: &str) -> String {
    match addr.kind {
        AddressKind::Any => "any".into(),
        AddressKind::Me => "(self)".into(),
        AddressKind::Single | AddressKind::Cidr => {
            // pf needs explicit address family for non-any addresses
            let _ = af;
            addr.value.clone()
        }
        AddressKind::Table => format!("<{}>", addr.value),
    }
}

fn proto_ipfw(p: RuleProtocol) -> &'static str {
    match p {
        RuleProtocol::Any => "ip",
        RuleProtocol::Tcp => "tcp",
        RuleProtocol::Udp => "udp",
        RuleProtocol::Icmp => "icmp",
        RuleProtocol::Icmpv6 => "ipv6-icmp",
    }
}

fn proto_pf(p: RuleProtocol) -> Option<&'static str> {
    match p {
        RuleProtocol::Any => None,
        RuleProtocol::Tcp => Some("tcp"),
        RuleProtocol::Udp => Some("udp"),
        RuleProtocol::Icmp => Some("icmp"),
        RuleProtocol::Icmpv6 => Some("ipv6-icmp"),
    }
}

/// Determine if an address value is IPv6.
fn is_ipv6(addr: &str) -> bool {
    addr.contains(':')
}

/// Generate the full ipfw shell script from enabled rules.
pub fn generate_ipfw(rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable]) -> String {
    let mut buf = header(FirewallDriver::Ipfw, mode);
    buf.push_str("ipfw -q flush\n\n");

    // IP tables
    if !tables.is_empty() {
        buf.push_str("# --- IP Tables ---\n");
        for table in tables {
            buf.push_str(&format!("ipfw -q table {} flush\n", table.name));
            for entry in &table.entries {
                buf.push_str(&format!("ipfw -q table {} add {}\n", table.name, entry.address));
            }
            buf.push('\n');
        }
    }

    for (i, rule) in rules.iter().filter(|r| r.enabled).enumerate() {
        let number = ((i + 1) * 100) as u32;

        let action = match rule.action {
            RuleAction::Allow => "allow",
            RuleAction::Deny => "deny",
            RuleAction::Reject => "reject",
        };

        let proto = proto_ipfw(rule.protocol);
        let src = address_ipfw(&rule.source);
        let dst = address_ipfw(&rule.destination);

        let src_port = rule
            .source_port
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|p| format!(" {}", port_to_ipfw(p)))
            .unwrap_or_default();

        let dst_port = rule
            .destination_port
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|p| format!(" dst-port {}", port_to_ipfw(p)))
            .unwrap_or_default();

        let dir = match rule.direction {
            RuleDirection::In => "in",
            RuleDirection::Out => "out",
        };

        let iface = rule
            .interface
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|ifn| {
                let kw = match rule.direction {
                    RuleDirection::In => "recv",
                    RuleDirection::Out => "xmit",
                };
                format!(" {kw} {ifn}")
            })
            .unwrap_or_default();

        let log = if rule.log { " log" } else { "" };

        // ICMP type: ipfw requires numeric type codes, not names.
        let icmp_type_str = rule
            .icmp_type
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|t| {
                let num = icmp_name_to_number(t);
                format!(" icmptypes {num}")
            })
            .unwrap_or_default();

        let state = if rule.action == RuleAction::Allow {
            " keep-state"
        } else {
            ""
        };

        let desc = rule
            .description
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("");

        buf.push_str(&format!(
            "# [{number:05}] {desc}\nipfw -q add {number:05}{log} {action} {proto} \
             from {src}{src_port} to {dst}{dst_port}{icmp_type_str} {dir}{iface}{state}\n\n",
        ));
    }

    // Default policy rule at 65534 (before kernel default 65535).
    // Whitelist: allow outbound + deny all inbound; Blacklist: allow all.
    if mode == FirewallMode::Whitelist {
        // Allow all outbound traffic (mirrors pf's "pass out quick all keep state").
        // Without this, HTTP responses to in-flight connections are dropped.
        buf.push_str("# [65000] Allow all outbound (whitelist mode)\n");
        buf.push_str("ipfw -q add 65000 allow ip from any to any out keep-state\n\n");
        buf.push_str("# [65534] Default deny (whitelist mode)\n");
        buf.push_str("ipfw -q add 65534 deny ip from any to any\n\n");
    } else {
        buf.push_str("# [65534] Default allow (blacklist mode)\n");
        buf.push_str("ipfw -q add 65534 allow ip from any to any\n\n");
    }

    buf
}

/// Generate the full pf.conf from enabled rules.
pub fn generate_pf(rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable]) -> String {
    let mut buf = header(FirewallDriver::Pf, mode);

    // IP tables (must be declared before rules that reference them)
    if !tables.is_empty() {
        buf.push_str("# --- IP Tables ---\n");
        for table in tables {
            let addrs: Vec<&str> = table.entries.iter().map(|e| e.address.as_str()).collect();
            if addrs.is_empty() {
                buf.push_str(&format!("table <{}> persist\n", table.name));
            } else {
                buf.push_str(&format!("table <{}> {{ {} }}\n", table.name, addrs.join(", ")));
            }
        }
        buf.push('\n');
    }

    if mode == FirewallMode::Whitelist {
        buf.push_str(
            "# Default policy: block all inbound, allow all outbound (whitelist mode)\n\
             set skip on lo0\n\
             block all\n\
             pass out quick all keep state\n\n",
        );
    } else {
        buf.push_str(
            "# Default policy: pass all (blacklist mode)\n\
             # pf defaults to pass when no rule matches\n\
             set skip on lo0\n\n",
        );
    }

    for rule in rules.iter().filter(|r| r.enabled) {
        let action = match rule.action {
            RuleAction::Allow => "pass",
            RuleAction::Deny => "block",
            RuleAction::Reject => "block return",
        };

        let dir = match rule.direction {
            RuleDirection::In => "in",
            RuleDirection::Out => "out",
        };

        let proto_str = proto_pf(rule.protocol);

        // ICMP type matching requires an explicit address family. For rules
        // referencing a table, omit AF so a mixed IPv4/IPv6 table can match.
        let has_table = rule.source.kind == AddressKind::Table
            || rule.destination.kind == AddressKind::Table;
        let af = if rule.protocol == RuleProtocol::Icmp {
            Some("inet")
        } else if rule.protocol == RuleProtocol::Icmpv6 {
            Some("inet6")
        } else if has_table {
            None
        } else if rule.source.kind == AddressKind::Me || rule.destination.kind == AddressKind::Me {
            Some("inet")
        } else {
            let src_v6 = rule.source.kind != AddressKind::Any
                && is_ipv6(&rule.source.value);
            let dst_v6 = rule.destination.kind != AddressKind::Any
                && is_ipv6(&rule.destination.value);
            if src_v6 || dst_v6 {
                Some("inet6")
            } else {
                Some("inet")
            }
        };

        let mut parts: Vec<String> = vec![action.into(), dir.into(), "quick".into()];
        if let Some(af) = af {
            parts.push(af.into());
        }
        if let Some(p) = proto_str {
            parts.push(format!("proto {p}"));
        }

        let src = address_pf(&rule.source, af.unwrap_or("inet"));
        let dst = address_pf(&rule.destination, af.unwrap_or("inet"));

        let src_port = rule
            .source_port
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|p| format!(" port {}", port_to_pf(p)))
            .unwrap_or_default();

        let dst_port = rule
            .destination_port
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|p| format!(" port {}", port_to_pf(p)))
            .unwrap_or_default();

        let iface = rule
            .interface
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|ifn| format!(" on {ifn}"))
            .unwrap_or_default();

        let log = if rule.log { " log" } else { "" };

        // ICMP type (only for ICMP protocols)
        let icmp_type_str = rule
            .icmp_type
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|t| {
                let kw = if rule.protocol == RuleProtocol::Icmpv6 {
                    "icmp6-type"
                } else {
                    "icmp-type"
                };
                let num = icmp_name_to_number(t);
                format!(" {kw} {num}")
            })
            .unwrap_or_default();

        let state = if rule.action == RuleAction::Allow && proto_str == Some("tcp") {
            " flags any keep state"
        } else if rule.action == RuleAction::Allow {
            " keep state"
        } else {
            ""
        };

        let desc = rule
            .description
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("");

        let parts_str = parts.join(" ");
        buf.push_str(&format!(
            "# {desc}\n{parts_str}{log}{iface} from {src}{src_port} to {dst}{dst_port}{icmp_type_str}{state}\n\n",
        ));
    }

    buf
}

/// Convert user port spec (e.g. "80,443,8080-8090") to ipfw syntax.
/// ipfw uses comma-separated ports directly: single → "80", range → "8080-8090",
/// multiple → "80,443,8080-8090". Curly braces in ipfw are OR-lists (rule-level),
/// NOT port lists, so must not be used here.
fn port_to_ipfw(spec: &str) -> String {
    spec.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

/// Convert user port spec (e.g. "80,443,8080-8090") to pf syntax.
/// pf: single → "80", range uses ":" not "-", multiple → "{ 80, 443, 8080:8090 }"
fn port_to_pf(spec: &str) -> String {
    let parts: Vec<&str> = spec.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => spec.to_string(),
        1 => parts[0].replace('-', ":"),
        _ => {
            let converted: Vec<String> = parts.iter().map(|p| p.replace('-', ":")).collect();
            format!("{{ {} }}", converted.join(", "))
        }
    }
}

/// Map ICMP type names to numeric codes for ipfw.
/// pf accepts names natively so no mapping needed there.
fn icmp_name_to_number(name: &str) -> &str {
    match name {
        "echo-reply" => "0",
        "destination-unreachable" => "3",
        "source-quench" => "4",
        "redirect" => "5",
        "echo-request" => "8",
        "router-advertisement" => "9",
        "router-solicitation" => "10",
        "time-exceeded" => "11",
        "parameter-problem" => "12",
        "timestamp" => "13",
        "timestamp-reply" => "14",
        // ICMPv6 (pf handles these with names, ipfw for ipv6-icmp uses same numbers)
        "packet-too-big" => "2",
        other => other,
    }
}

// ── validation ─────────────────────────────────────────────────────

const IFACE_RE: &str = r"^[a-zA-Z0-9_.]{1,15}$";
const PORT_RE: &str = r"^(\d+)(-(\d+))?(,(\d+)(-(\d+))?)*$";

pub fn validate_rule_body(body: &RuleBody) -> ApiResult<()> {
    // Interface
    if let Some(ref iface) = body.interface {
        if !iface.is_empty() {
            let re = regex::Regex::new(IFACE_RE).unwrap();
            if !re.is_match(iface) {
                return Err(ApiError::BadRequest("invalid interface name".into()));
            }
        }
    }

    // Ports (only for TCP/UDP)
    if !matches!(body.protocol, RuleProtocol::Any | RuleProtocol::Icmp | RuleProtocol::Icmpv6) {
        let port_re = regex::Regex::new(PORT_RE).unwrap();
        if let Some(ref p) = body.source_port {
            if !p.is_empty() && !port_re.is_match(p) {
                return Err(ApiError::BadRequest("invalid source port".into()));
            }
        }
        if let Some(ref p) = body.destination_port {
            if !p.is_empty() && !port_re.is_match(p) {
                return Err(ApiError::BadRequest("invalid destination port".into()));
            }
        }
    }

    // Address values
    if matches!(body.source.kind, AddressKind::Single | AddressKind::Cidr) {
        if body.source.value.is_empty() {
            return Err(ApiError::BadRequest("source address value required".into()));
        }
    }
    if matches!(body.source.kind, AddressKind::Table) {
        if body.source.value.is_empty() {
            return Err(ApiError::BadRequest("source table name required".into()));
        }
        validate_table_name(&body.source.value)?;
    }
    if matches!(
        body.destination.kind,
        AddressKind::Single | AddressKind::Cidr
    ) {
        if body.destination.value.is_empty() {
            return Err(ApiError::BadRequest(
                "destination address value required".into(),
            ));
        }
    }
    if matches!(body.destination.kind, AddressKind::Table) {
        if body.destination.value.is_empty() {
            return Err(ApiError::BadRequest("destination table name required".into()));
        }
        validate_table_name(&body.destination.value)?;
    }

    // Description
    if let Some(ref d) = body.description {
        if d.len() > 200 {
            return Err(ApiError::BadRequest("description too long (max 200)".into()));
        }
        if d.contains('\n') {
            return Err(ApiError::BadRequest("description must not contain newlines".into()));
        }
    }

    Ok(())
}

// ── atomic file write ──────────────────────────────────────────────

fn atomic_write(path: &str, content: &str) -> ApiResult<()> {
    let tmp = format!("{path}.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

// ── driver operations ──────────────────────────────────────────────

/// Load kernel module if not already loaded.
pub fn ensure_module(driver: FirewallDriver) -> ApiResult<()> {
    if driver.module_loaded() {
        return Ok(());
    }
    cmd::run_sync(KLDLOAD, &[driver.as_str()])?;
    Ok(())
}

/// Apply ipfw rules: generate file, then load.
pub fn apply_ipfw(rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable]) -> ApiResult<()> {
    let content = generate_ipfw(rules, mode, tables);

    // Write to real path (atomic via temp + rename)
    let tmp = format!("{IPFW_RULES_PATH}.tmp");
    fs::write(&tmp, &content).map_err(|e| ApiError::Internal(format!("write tmp: {e}")))?;
    fs::rename(&tmp, IPFW_RULES_PATH)?;

    // Load the script — this runs `ipfw -q flush` then re-adds all rules.
    // If a rule has invalid syntax, ipfw returns non-zero and we propagate the error.
    cmd::run_sync(SH, &[IPFW_RULES_PATH])?;
    Ok(())
}

/// Apply pf rules: generate file, validate with pfctl -n, then load.
pub fn apply_pf(rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable]) -> ApiResult<()> {
    let content = generate_pf(rules, mode, tables);

    // Write to temp file and validate
    let tmp = format!("{PF_CONF_PATH}.tmp");
    fs::write(&tmp, &content).map_err(|e| ApiError::Internal(format!("write tmp: {e}")))?;

    // Validate syntax
    if let Err(e) = cmd::run_sync(PFCTL, &["-n", "-f", &tmp]) {
        let _ = fs::remove_file(&tmp);
        return Err(ApiError::Command(format!("pf.conf validation failed: {e}")));
    }

    // All good — move to real path and load
    fs::rename(&tmp, PF_CONF_PATH)?;
    cmd::run_sync(PFCTL, &["-f", PF_CONF_PATH])?;
    Ok(())
}

/// Enable firewall at runtime (does not modify rc.conf).
/// Rules must already be loaded via Apply before calling this.
pub fn enable_firewall(driver: FirewallDriver) -> ApiResult<()> {
    match driver {
        FirewallDriver::Ipfw => {
            cmd::run_sync(SYSCTL, &["net.inet.ip.fw.enable=1"])?;
        }
        FirewallDriver::Pf => {
            // pfctl -e enables pf. Rules should already be loaded via Apply.
            // Do NOT pfctl -f here — it flushes the state table and kills
            // the current HTTP connection, making the API appear to hang.
            cmd::run_sync(PFCTL, &["-e"])?;
        }
    }
    Ok(())
}

/// Disable firewall at runtime (does not modify rc.conf).
pub fn disable_firewall(driver: FirewallDriver) -> ApiResult<()> {
    match driver {
        FirewallDriver::Ipfw => {
            cmd::run_sync(SYSCTL, &["net.inet.ip.fw.enable=0"])?;
        }
        FirewallDriver::Pf => {
            // pfctl -d errors if pf is not enabled; ignore failure
            cmd::run_forget_sync(PFCTL, &["-d"]);
        }
    }
    Ok(())
}

/// Check if firewall is currently enabled (running).
pub fn is_firewall_enabled(driver: FirewallDriver) -> bool {
    match driver {
        FirewallDriver::Ipfw => {
            let out = cmd::run_sync(SYSCTL, &["-n", "net.inet.ip.fw.enable"]).unwrap_or_default();
            out.trim() == "1"
        }
        FirewallDriver::Pf => {
            let out = cmd::run_sync(PFCTL, &["-s", "info"]).unwrap_or_default();
            out.contains("Status: Enabled")
        }
    }
}

/// Set the ipfw mode. Does NOT touch loader.conf or sysctl.conf.
/// The runtime default policy is enforced via rule 65534 in the generated script.
pub fn set_ipfw_mode(_mode: FirewallMode) -> ApiResult<()> {
    Ok(())
}

/// Initialize ipfw: write rc.conf entries, load module, generate config file.
/// Does NOT load rules or enable the firewall — both happen via Apply/Enable.
pub fn init_ipfw(mode: FirewallMode, rules: &[FirewallRule], tables: &[IpTable]) -> ApiResult<()> {
    use crate::sysrc;

    sysrc::set("firewall_enable", "YES").map_err(|e| ApiError::Command(e))?;
    sysrc::set("firewall_script", IPFW_RULES_PATH).map_err(|e| ApiError::Command(e))?;
    sysrc::set("firewall_logging", "YES").map_err(|e| ApiError::Command(e))?;
    sysrc::delete("firewall_type");

    ensure_module(FirewallDriver::Ipfw)?;

    // kldload ipfw enables ipfw by default (net.inet.ip.fw.enable defaults to 1)
    // with a default-deny rule 65535. Explicitly disable to avoid blocking all traffic.
    disable_firewall(FirewallDriver::Ipfw)?;

    // Only generate the file — do NOT load it into the kernel yet.
    let content = generate_ipfw(rules, mode, tables);
    atomic_write(IPFW_RULES_PATH, &content)?;
    Ok(())
}

/// Initialize pf: write rc.conf entries, load module, load rules.
pub fn init_pf(mode: FirewallMode, rules: &[FirewallRule], tables: &[IpTable]) -> ApiResult<()> {
    use crate::sysrc;

    sysrc::set("pf_enable", "YES").map_err(|e| ApiError::Command(e))?;
    sysrc::set("pf_rules", PF_CONF_PATH).map_err(|e| ApiError::Command(e))?;

    ensure_module(FirewallDriver::Pf)?;

    // Only generate the file — do NOT load it into the kernel yet.
    let content = generate_pf(rules, mode, tables);
    atomic_write(PF_CONF_PATH, &content)?;
    Ok(())
}

/// Disable ipfw in rc.conf and at runtime.
pub fn deactivate_ipfw() -> ApiResult<()> {
    use crate::sysrc;
    disable_firewall(FirewallDriver::Ipfw)?;
    sysrc::set("firewall_enable", "NO").map_err(|e| ApiError::Command(e))?;
    Ok(())
}

/// Disable pf in rc.conf and at runtime.
pub fn deactivate_pf() -> ApiResult<()> {
    use crate::sysrc;
    disable_firewall(FirewallDriver::Pf)?;
    sysrc::set("pf_enable", "NO").map_err(|e| ApiError::Command(e))?;
    Ok(())
}

/// Read generated config file content for preview.
pub fn read_config(driver: FirewallDriver) -> ApiResult<String> {
    let path = match driver {
        FirewallDriver::Ipfw => IPFW_RULES_PATH,
        FirewallDriver::Pf => PF_CONF_PATH,
    };
    fs::read_to_string(path).map_err(|e| ApiError::NotFound(format!("config file: {e}")))
}

/// Generate config content without writing to disk (for preview before apply).
pub fn preview_config(
    driver: FirewallDriver,
    rules: &[FirewallRule],
    mode: FirewallMode,
    tables: &[IpTable],
) -> String {
    match driver {
        FirewallDriver::Ipfw => generate_ipfw(rules, mode, tables),
        FirewallDriver::Pf => generate_pf(rules, mode, tables),
    }
}
