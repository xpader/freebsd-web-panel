//! Firewall rule types, config-file generators, and DB access.
//!
//! Defines the driver-agnostic structured rule model, generates ipfw shell
//! scripts and pf.conf files from that model, and provides SQLite CRUD for
//! rules + firewall state.

use std::fs;

use rusqlite::{params, Connection, OptionalExtension};

use crate::cmd;
use crate::error::{ApiError, ApiResult};

// ── binary paths ───────────────────────────────────────────────────

#[allow(dead_code)]
const IPFW: &str = "/sbin/ipfw";
const PFCTL: &str = "/sbin/pfctl";
const KLDLOAD: &str = "/sbin/kldload";
const KLDSTAT: &str = "/sbin/kldstat";
const SYSCTL: &str = "/sbin/sysctl";
const SERVICE: &str = "/usr/sbin/service";

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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IpTableEntry {
    pub id: i64,
    pub table_id: i64,
    pub address: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

// ── NAT rule types ─────────────────────────────────────────────────

/// NAT rule kind: source NAT, destination NAT (port forward), or 1:1 bidir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatKind {
    Snat,
    Dnat,
    Binat,
}

impl NatKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Snat => "snat",
            Self::Dnat => "dnat",
            Self::Binat => "binat",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "snat" => Some(Self::Snat),
            "dnat" => Some(Self::Dnat),
            "binat" => Some(Self::Binat),
            _ => None,
        }
    }
}

/// Address family for a NAT rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatFamily {
    Ip,
    Ip6,
}

impl NatFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ip => "ip",
            Self::Ip6 => "ip6",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "ip" => Some(Self::Ip),
            "ip6" => Some(Self::Ip6),
            _ => None,
        }
    }

    pub fn pf_kw(&self) -> &'static str {
        match self {
            Self::Ip => "inet",
            Self::Ip6 => "inet6",
        }
    }
}

/// NAT protocol selector. `Both` means TCP + UDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatProto {
    Tcp,
    Udp,
    Both,
}

impl NatProto {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Both => "both",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "tcp" => Some(Self::Tcp),
            "udp" => Some(Self::Udp),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

/// Structured NAT rule — driver-agnostic abstraction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NatRule {
    pub id: i64,
    pub position: u32,
    pub enabled: bool,
    pub kind: NatKind,
    pub family: NatFamily,
    pub interface: String,
    pub src_addr: String,
    #[serde(default)]
    pub dst_addr: Option<String>,
    #[serde(default)]
    pub src_port: Option<String>,
    #[serde(default)]
    pub dst_port: Option<String>,
    pub protocol: NatProto,
    #[serde(default)]
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Fields accepted when creating or updating a NAT rule.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct NatBody {
    pub kind: NatKind,
    pub family: NatFamily,
    pub interface: String,
    pub src_addr: String,
    #[serde(default)]
    pub dst_addr: Option<String>,
    #[serde(default)]
    pub src_port: Option<String>,
    #[serde(default)]
    pub dst_port: Option<String>,
    pub protocol: NatProto,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub description: Option<String>,
}

// ── core structs ───────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AddressSpec {
    pub kind: AddressKind,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FirewallRule {
    pub id: i64,
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
        "SELECT id, position, enabled, action, direction, protocol, \
         src_kind, src_value, src_port, dst_kind, dst_value, dst_port, \
         interface, log, icmp_type, description, created_at, updated_at \
         FROM firewall_rules ORDER BY position ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FirewallRule {
                id: r.get(0)?,
                position: r.get::<_, i64>(1)? as u32,
                enabled: r.get::<_, i64>(2)? != 0,
                action: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or(RuleAction::Allow),
                direction: serde_json::from_str(&r.get::<_, String>(4)?)
                    .unwrap_or(RuleDirection::In),
                protocol: serde_json::from_str(&r.get::<_, String>(5)?)
                    .unwrap_or(RuleProtocol::Any),
                source: AddressSpec {
                    kind: serde_json::from_str(&r.get::<_, String>(6)?)
                        .unwrap_or(AddressKind::Any),
                    value: r.get(7)?,
                },
                source_port: r.get(8)?,
                destination: AddressSpec {
                    kind: serde_json::from_str(&r.get::<_, String>(9)?)
                        .unwrap_or(AddressKind::Any),
                    value: r.get(10)?,
                },
                destination_port: r.get(11)?,
                interface: r.get(12)?,
                log: r.get::<_, i64>(13)? != 0,
                icmp_type: r.get(14)?,
                description: r.get(15)?,
                created_at: r.get(16)?,
                updated_at: r.get(17)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Replace all rules in DB with the given list (for staging confirm).
pub fn replace_all_rules(conn: &Connection, rules: &[FirewallRule]) -> ApiResult<()> {
    conn.execute("DELETE FROM firewall_rules", [])?;
    for (i, rule) in rules.iter().enumerate() {
        conn.execute(
            "INSERT INTO firewall_rules \
             (id, position, enabled, action, direction, protocol, \
              src_kind, src_value, src_port, dst_kind, dst_value, dst_port, \
              interface, log, icmp_type, description, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17)",
            params![
                rule.id,
                i as i64,
                rule.enabled as i64,
                serde_json::to_string(&rule.action).unwrap_or_default(),
                serde_json::to_string(&rule.direction).unwrap_or_default(),
                serde_json::to_string(&rule.protocol).unwrap_or_default(),
                serde_json::to_string(&rule.source.kind).unwrap_or_default(),
                rule.source.value,
                rule.source_port,
                serde_json::to_string(&rule.destination.kind).unwrap_or_default(),
                rule.destination.value,
                rule.destination_port,
                rule.interface,
                rule.log as i64,
                rule.icmp_type,
                rule.description,
                rule.created_at,
            ],
        )?;
    }
    Ok(())
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
    Ok(max.map(|v| (v + 1) as u32).unwrap_or(0))
}

pub fn create_rule(
    conn: &Connection,
    body: &RuleBody,
    now: i64,
) -> ApiResult<i64> {
    let pos = next_position(conn)?;
    conn.execute(
        "INSERT INTO firewall_rules \
         (position, enabled, action, direction, protocol, \
          src_kind, src_value, src_port, dst_kind, dst_value, dst_port, \
          interface, log, icmp_type, description, created_at, updated_at) \
         VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
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

/// Replace all tables + entries in DB with the given list (for staging confirm).
pub fn replace_all_tables(conn: &Connection, tables: &[IpTable]) -> ApiResult<()> {
    conn.execute("DELETE FROM firewall_table_entries", [])?;
    conn.execute("DELETE FROM firewall_tables", [])?;
    for table in tables {
        conn.execute(
            "INSERT INTO firewall_tables (id, name, description, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![table.id, table.name, table.description, table.created_at, table.updated_at],
        )?;
        for entry in &table.entries {
            conn.execute(
                "INSERT INTO firewall_table_entries (id, table_id, address, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![entry.id, entry.table_id, entry.address, entry.created_at],
            )?;
        }
    }
    Ok(())
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

// ── NAT rule CRUD ──────────────────────────────────────────────────

pub fn list_nat_rules(conn: &Connection) -> ApiResult<Vec<NatRule>> {
    let mut stmt = conn.prepare(
        "SELECT id, position, enabled, kind, family, interface, \
         src_addr, dst_addr, src_port, dst_port, protocol, description, \
         created_at, updated_at \
         FROM firewall_nat_rules ORDER BY position ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(NatRule {
                id: r.get(0)?,
                position: r.get::<_, i64>(1)? as u32,
                enabled: r.get::<_, i64>(2)? != 0,
                kind: NatKind::from_str(&r.get::<_, String>(3)?).unwrap_or(NatKind::Snat),
                family: NatFamily::from_str(&r.get::<_, String>(4)?).unwrap_or(NatFamily::Ip),
                interface: r.get(5)?,
                src_addr: r.get(6)?,
                dst_addr: r.get(7)?,
                src_port: r.get(8)?,
                dst_port: r.get(9)?,
                protocol: NatProto::from_str(&r.get::<_, String>(10)?).unwrap_or(NatProto::Both),
                description: r.get(11)?,
                created_at: r.get::<_, i64>(12)?,
                updated_at: r.get::<_, i64>(13)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn next_nat_position(conn: &Connection) -> ApiResult<u32> {
    let max: Option<i64> = conn
        .query_row(
            "SELECT COALESCE(MAX(position), -1) FROM firewall_nat_rules",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(max.map(|v| (v + 1) as u32).unwrap_or(0))
}

pub fn create_nat_rule(
    conn: &Connection,
    body: &NatBody,
    now: i64,
) -> ApiResult<i64> {
    let pos = next_nat_position(conn)?;
    conn.execute(
        "INSERT INTO firewall_nat_rules \
         (position, enabled, kind, family, interface, src_addr, dst_addr, \
          src_port, dst_port, protocol, description, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
        params![
            pos,
            body.enabled as i64,
            body.kind.as_str(),
            body.family.as_str(),
            body.interface,
            body.src_addr,
            body.dst_addr,
            body.src_port,
            body.dst_port,
            body.protocol.as_str(),
            body.description,
            now,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_nat_rule(
    conn: &Connection,
    id: i64,
    body: &NatBody,
    now: i64,
) -> ApiResult<()> {
    let n = conn.execute(
        "UPDATE firewall_nat_rules SET \
         kind = ?1, family = ?2, interface = ?3, src_addr = ?4, dst_addr = ?5, \
         src_port = ?6, dst_port = ?7, protocol = ?8, enabled = ?9, \
         description = ?10, updated_at = ?11 \
         WHERE id = ?12",
        params![
            body.kind.as_str(),
            body.family.as_str(),
            body.interface,
            body.src_addr,
            body.dst_addr,
            body.src_port,
            body.dst_port,
            body.protocol.as_str(),
            body.enabled as i64,
            body.description,
            now,
            id,
        ],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("NAT rule not found".into()));
    }
    Ok(())
}

pub fn delete_nat_rule(conn: &Connection, id: i64) -> ApiResult<()> {
    let n = conn.execute(
        "DELETE FROM firewall_nat_rules WHERE id = ?1",
        params![id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("NAT rule not found".into()));
    }
    Ok(())
}

pub fn toggle_nat_rule(conn: &Connection, id: i64) -> ApiResult<()> {
    let n = conn.execute(
        "UPDATE firewall_nat_rules SET enabled = 1 - enabled WHERE id = ?1",
        params![id],
    )?;
    if n == 0 {
        return Err(ApiError::NotFound("NAT rule not found".into()));
    }
    Ok(())
}

pub fn reorder_nat_rules(
    conn: &Connection,
    ordered_ids: &[i64],
) -> ApiResult<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt =
            tx.prepare("UPDATE firewall_nat_rules SET position = ?1 WHERE id = ?2")?;
        for (pos, id) in ordered_ids.iter().enumerate() {
            stmt.execute(params![pos as i64, id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Replace all NAT rules in DB with the given list (for staging confirm).
pub fn replace_all_nat_rules(conn: &Connection, rules: &[NatRule]) -> ApiResult<()> {
    conn.execute("DELETE FROM firewall_nat_rules", [])?;
    for (i, rule) in rules.iter().enumerate() {
        conn.execute(
            "INSERT INTO firewall_nat_rules \
             (id, position, enabled, kind, family, interface, src_addr, dst_addr, \
              src_port, dst_port, protocol, description, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                rule.id,
                i as i64,
                rule.enabled as i64,
                rule.kind.as_str(),
                rule.family.as_str(),
                rule.interface,
                rule.src_addr,
                rule.dst_addr,
                rule.src_port,
                rule.dst_port,
                rule.protocol.as_str(),
                rule.description,
                rule.created_at,
                rule.updated_at,
            ],
        )?;
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
        AddressKind::Table => format!("table({})", addr.value),
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

/// Return the set of table names referenced by enabled rules.
fn referenced_table_names(rules: &[FirewallRule]) -> std::collections::HashSet<String> {
    rules
        .iter()
        .filter(|r| r.enabled)
        .flat_map(|r| [&r.source, &r.destination])
        .filter_map(|addr| {
            if addr.kind == AddressKind::Table {
                Some(addr.value.clone())
            } else {
                None
            }
        })
        .collect()
}

/// Filter tables to only those referenced by enabled rules.
fn filter_referenced_tables<'a>(rules: &[FirewallRule], tables: &'a [IpTable]) -> Vec<&'a IpTable> {
    let names = referenced_table_names(rules);
    tables.iter().filter(|t| names.contains(&t.name)).collect()
}

/// Generate the full ipfw rules file from enabled rules.
///
/// Output is in ipfw "pathname" format (native rules, no `ipfw` command prefix).
/// Loaded via `ipfw -q /etc/ipfw.rules`. Each line's content becomes arguments
/// to the ipfw utility. Lines starting with `#` are comments (including inline).
///
/// ## Rule layout (whitelist mode)
///
/// ```text
/// 1         allow loopback
/// 10-19     NAT rules (out + in) — BEFORE check-state so NAT runs first
/// 50        check-state           — dynamic state checkpoint
/// 100+      user filter rules (with keep-state)
/// 40000+    NAT auto-pass (jail subnet allow, with keep-state)
/// 65000     allow outbound from me
/// 65534     default deny
/// ```
///
/// The `check-state` rule at 50 is the critical divider: dynamic states are
/// evaluated at this point, NOT implicitly before all static rules. This lets
/// NAT rules (10-19) run unconditionally before state lookup, so translated
/// addresses get correct state entries. Filter rules after check-state only
/// process packets that didn't match an existing state.
pub fn generate_ipfw(rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable], nat_rules: &[NatRule]) -> String {
    let mut buf = header(FirewallDriver::Ipfw, mode);
    buf.push_str("-f flush\n\n");

    // Loopback — always allow.
    buf.push_str("add 001 allow ip from any to any via lo0\n\n");

    // IP tables — only include those referenced by enabled rules.
    let active_tables = filter_referenced_tables(rules, tables);
    if !active_tables.is_empty() {
        buf.push_str("# --- IP Tables ---\n");
        // Destroy all existing tables first — `-f flush` only clears rules,
        // tables persist across reloads.
        buf.push_str("table all destroy\n");
        for table in &active_tables {
            // Explicitly create with type addr to avoid DEPRECATED auto-create.
            buf.push_str(&format!("table {} create type addr\n", table.name));
            for entry in &table.entries {
                buf.push_str(&format!("table {} add {}\n", table.name, entry.address));
            }
        }
        buf.push('\n');
    }

    // NAT instance configuration (must appear before rules that use `nat N`).
    let nat_config = generate_ipfw_nat_config(nat_rules);
    if !nat_config.is_empty() {
        buf.push_str(&nat_config);
    }

    // NAT rules — BEFORE check-state so translation happens unconditionally.
    // With one_pass=0, translated packets re-enter the firewall and reach
    // check-state, where dynamic state is evaluated on the post-translation
    // addresses.
    let nat_rules_str = generate_ipfw_nat_rules(nat_rules);
    if !nat_rules_str.is_empty() {
        buf.push_str(&nat_rules_str);
    }

    // check-state — the dynamic state checkpoint. MUST appear after NAT rules
    // and before filter rules. Without this, ipfw evaluates dynamic rules
    // implicitly before ALL static rules, which shadows the NAT rules.
    buf.push_str("# --- State checkpoint ---\n");
    buf.push_str("add 050 check-state\n\n");

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
            "# [{number:05}] {desc}\nadd {number:05}{log} {action} {proto} \
             from {src}{src_port} to {dst}{dst_port}{icmp_type_str} {dir}{iface}{state}\n\n",
        ));
    }

    // Auto-injected allow rules for NAT'd traffic (whitelist mode only).
    // Placed AFTER check-state so keep-state is safe — dynamic states created
    // here are evaluated at check-state (rule 50), which runs AFTER NAT.
    let nat_pass_str = generate_ipfw_nat_pass(nat_rules, mode);
    if !nat_pass_str.is_empty() {
        buf.push_str(&nat_pass_str);
    }

    // Default policy rule at 65534 (before kernel default 65535).
    // Whitelist: allow outbound from me + deny all; Blacklist: allow all.
    if mode == FirewallMode::Whitelist {
        // Allow outbound traffic originating from the host itself.
        // NOT `from any` — that would bypass NAT for jail traffic.
        buf.push_str("# [65000] Allow outbound from me (whitelist mode)\n");
        buf.push_str("add 65000 allow ip from me to any out keep-state\n\n");
        buf.push_str("# [65534] Default deny (whitelist mode)\n");
        buf.push_str("add 65534 deny log ip from any to any\n\n");
    } else {
        buf.push_str("# [65534] Default allow (blacklist mode)\n");
        buf.push_str("add 65534 allow ip from any to any\n\n");
    }

    buf
}

/// Generate the full pf.conf from enabled rules.
pub fn generate_pf(rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable], nat_rules: &[NatRule]) -> String {
    let mut buf = header(FirewallDriver::Pf, mode);

    // IP tables — only include those referenced by enabled rules.
    let active_tables = filter_referenced_tables(rules, tables);
    if !active_tables.is_empty() {
        buf.push_str("# --- IP Tables ---\n");
        for table in &active_tables {
            let addrs: Vec<&str> = table.entries.iter().map(|e| e.address.as_str()).collect();
            if addrs.is_empty() {
                buf.push_str(&format!("table <{}> persist\n", table.name));
            } else {
                buf.push_str(&format!("table <{}> {{ {} }}\n", table.name, addrs.join(", ")));
            }
        }
        buf.push('\n');
    }

    // NAT / rdr rules must appear before the default `block all` so the NAT
    // translation happens before filtering. PF evaluates NAT and filter rules
    // in separate passes, but keeping NAT at the top is conventional.
    let nat_str = generate_pf_nat(nat_rules);
    if !nat_str.is_empty() {
        buf.push_str(&nat_str);
    }

    if mode == FirewallMode::Whitelist {
        buf.push_str(
            "# Default policy: block all inbound, allow all outbound (whitelist mode)\n\
             set skip on lo0\n\
             block all\n\
             pass out quick all flags any keep state (sloppy)\n\n",
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

        // PF rule syntax order (pf.conf(5)):
        //   action [direction] [log] [quick] [on interface] [af] [proto] from ... to ...
        // log, interface, af, proto must appear in this exact order.
        let mut parts: Vec<String> = vec![action.into(), dir.into()];
        if rule.log {
            parts.push("log".into());
        }
        parts.push("quick".into());
        if let Some(ifn) = rule.interface.as_deref().filter(|s| !s.is_empty()) {
            parts.push(format!("on {ifn}"));
        }
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
            " flags any keep state (sloppy)"
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
            "# {desc}\n{parts_str} from {src}{src_port} to {dst}{dst_port}{icmp_type_str}{state}\n\n",
        ));
    }

    // Auto-injected filter rules that allow NAT'd traffic to pass the default
    // `block all`. Placed AFTER user rules so that users can still override
    // with explicit `block quick` rules of their own (PF is first-match-quick:
    // a user `block quick` placed ahead of this will terminate first). Only
    // emitted in whitelist mode. See `generate_pf_nat_pass`.
    let nat_pass = generate_pf_nat_pass(nat_rules, mode);
    if !nat_pass.is_empty() {
        buf.push_str(&nat_pass);
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

// ── NAT config generation ─────────────────────────────────────────

/// Generate the PF NAT/rdr rules block. Empty string if no enabled NAT rules.
pub fn generate_pf_nat(rules: &[NatRule]) -> String {
    let active: Vec<&NatRule> = rules.iter().filter(|r| r.enabled).collect();
    if active.is_empty() {
        return String::new();
    }
    let mut buf = String::from("# --- NAT / RDR ---\n");
    for rule in &active {
        let af = rule.family.pf_kw();
        let desc = rule.description.as_deref().unwrap_or("");
        match rule.kind {
            NatKind::Snat => {
                let target = rule
                    .dst_addr
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("({})", rule.interface));
                buf.push_str(&format!(
                    "# [SNAT] {desc}\nnat on {iface} {af} from {src} to any -> {target}\n",
                    iface = rule.interface,
                    af = af,
                    src = rule.src_addr,
                    target = target,
                ));
            }
            NatKind::Dnat => {
                let proto = pf_nat_proto(&rule.protocol);
                let target = rule
                    .dst_addr
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("127.0.0.1");
                let tport = rule
                    .dst_port
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|p| format!(" port {}", p))
                    .unwrap_or_default();
                let port = rule.src_port.as_deref().unwrap_or("0");
                // src_addr: alias IPs for this DNAT. "any"/empty → one rdr matching all IPs.
                let alias_ips: Vec<&str> = if rule.src_addr.is_empty() || rule.src_addr == "any" {
                    Vec::new()
                } else {
                    rule.src_addr.split(',').map(|i| i.trim()).filter(|i| !i.is_empty()).collect()
                };
                if alias_ips.is_empty() {
                    buf.push_str(&format!(
                        "# [DNAT] {desc}\nrdr on {iface} {af}{proto} from any to any port {port} -> {target}{tport}\n",
                        iface = rule.interface,
                        af = af,
                        proto = proto,
                        port = port,
                        target = target,
                        tport = tport,
                    ));
                } else {
                    for ip in &alias_ips {
                        buf.push_str(&format!(
                            "# [DNAT] {desc}\nrdr on {iface} {af}{proto} from any to {ip} port {port} -> {target}{tport}\n",
                            iface = rule.interface,
                            af = af,
                            proto = proto,
                            ip = ip,
                            port = port,
                            target = target,
                            tport = tport,
                        ));
                    }
                }
            }
            NatKind::Binat => {
                let ext = rule
                    .dst_addr
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("255.255.255.255");
                buf.push_str(&format!(
                    "# [BINAT] {desc}\nbinat on {iface} {af} from {src} to any -> {ext}\n",
                    iface = rule.interface,
                    af = af,
                    src = rule.src_addr,
                    ext = ext,
                ));
            }
        }
    }
    buf.push('\n');
    buf
}

/// Generate auto-injected PF filter rules that allow NAT'd traffic to pass in
/// whitelist mode. Without these, the default `block all` would drop inbound
/// packets from NAT'd networks before NAT/state can take effect — forcing users
/// to manually add a separate filter rule for each NAT rule.
///
/// Returns empty string in blacklist mode (the default `pass` already covers it)
/// or when no NAT rules are enabled.
///
/// Generated rules are placed AFTER `block all` and AFTER user filter rules so
/// that users can override with their own `block quick` rules (PF is
/// first-match-quick: a user `block quick` placed ahead of this terminates
/// evaluation first). The default `block all` has no `quick`, so the last
/// matching rule wins — these auto `pass quick` rules ensure NAT traffic flows
/// when no user rule has already made a decision.
fn generate_pf_nat_pass(rules: &[NatRule], mode: FirewallMode) -> String {
    if mode != FirewallMode::Whitelist {
        return String::new();
    }
    let active: Vec<&NatRule> = rules.iter().filter(|r| r.enabled).collect();
    if active.is_empty() {
        return String::new();
    }
    let mut buf = String::from("# --- NAT auto-pass (whitelist mode) ---\n");
    for rule in &active {
        let af = rule.family.pf_kw();
        let desc = rule.description.as_deref().unwrap_or("");
        match rule.kind {
            NatKind::Snat => {
                // Allow NAT'd source network to enter host (inbound direction).
                // Once state is created, return traffic flows out via the
                // default `pass out quick all keep state`.
                buf.push_str(&format!(
                    "# [auto] SNAT pass-in: {desc}\npass in quick {af} from {src} to any keep state\n",
                    af = af,
                    src = rule.src_addr,
                ));
            }
            NatKind::Dnat => {
                // Allow external traffic to reach the redirected port. PF's rdr
                // translates the destination BEFORE filter evaluation, so match
                // the internal target address:port (post-translation).
                let proto = pf_nat_proto(&rule.protocol); // " proto tcp" / " proto { tcp udp }"
                let target = rule
                    .dst_addr
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("any");
                let port_clause = rule
                    .dst_port
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|p| format!(" port {}", port_to_pf(p)))
                    .unwrap_or_default();
                let state = if rule.protocol == NatProto::Tcp {
                    " flags any keep state (sloppy)"
                } else {
                    " keep state"
                };
                buf.push_str(&format!(
                    "# [auto] DNAT pass-in: {desc}\npass in quick on {iface} {af}{proto} from any to {target}{port_clause}{state}\n",
                    iface = rule.interface,
                    af = af,
                    proto = proto,
                    target = target,
                    port_clause = port_clause,
                    state = state,
                ));
            }
            NatKind::Binat => {
                // 1:1 bidirectional mapping. Allow the source address in both
                // directions.
                buf.push_str(&format!(
                    "# [auto] BINAT pass: {desc}\npass quick {af} from {src} to any keep state\n",
                    af = af,
                    src = rule.src_addr,
                ));
            }
        }
    }
    buf.push('\n');
    buf
}

/// Group enabled NAT rules by interface.
/// Returns (interface, rules) pairs in first-seen order.
fn group_nat_by_interface(rules: &[NatRule]) -> Vec<(String, Vec<&NatRule>)> {
    let mut groups: Vec<(String, Vec<&NatRule>)> = Vec::new();
    for rule in rules.iter().filter(|r| r.enabled) {
        if let Some(g) = groups.iter_mut().find(|(iface, _)| iface == &rule.interface) {
            g.1.push(rule);
        } else {
            groups.push((rule.interface.clone(), vec![rule]));
        }
    }
    groups
}

/// Generate the ipfw `nat N config` declarations, one per interface.
///
/// Rules on the same interface are merged into a single NAT instance.
/// SNAT/BINAT contribute `same_ports reset`; each DNAT contributes
/// `redirect_port` clauses. When `src_addr` lists alias IPs, one clause per IP
/// is emitted (using `IP:port` alias syntax). All stay in one instance so
/// SNAT and DNAT share the same libalias state.
pub fn generate_ipfw_nat_config(rules: &[NatRule]) -> String {
    let groups = group_nat_by_interface(rules);
    if groups.is_empty() {
        return String::new();
    }
    let mut buf = String::from("# --- NAT configuration ---\n");
    for (i, (iface, group_rules)) in groups.iter().enumerate() {
        let inst = (i + 1) as u32;
        let mut config = format!("if {iface}");

        // SNAT / BINAT → same_ports reset
        let has_snat = group_rules
            .iter()
            .any(|r| matches!(r.kind, NatKind::Snat | NatKind::Binat));
        if has_snat {
            config.push_str(" same_ports reset");
        }

        // Each DNAT → redirect_port clause(s)
        // ipfw syntax: redirect_port <proto> <localIP:localPort> [aliasIP:]aliasPort
        // src_addr: comma-separated alias IPs to listen on; "any"/empty = all IPs
        for rule in group_rules.iter().filter(|r| r.kind == NatKind::Dnat) {
            let target = rule
                .dst_addr
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("127.0.0.1");
            let port = rule.src_port.as_deref().unwrap_or("0");
            let local_port = rule
                .dst_port
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(port);
            // src_addr: alias IPs for this DNAT. "any"/empty → bare port (all IPs).
            let alias_ips: Vec<&str> = if rule.src_addr.is_empty() || rule.src_addr == "any" {
                Vec::new()
            } else {
                rule.src_addr.split(',').map(|i| i.trim()).filter(|i| !i.is_empty()).collect()
            };
            let protos = match rule.protocol {
                NatProto::Tcp => vec!["tcp"],
                NatProto::Udp => vec!["udp"],
                NatProto::Both => vec!["tcp", "udp"],
            };
            for proto in protos {
                if alias_ips.is_empty() {
                    config.push_str(&format!(
                        " redirect_port {proto} {target}:{local_port} {port}"
                    ));
                } else {
                    for ip in &alias_ips {
                        config.push_str(&format!(
                            " redirect_port {proto} {target}:{local_port} {ip}:{port}"
                        ));
                    }
                }
            }
        }

        let descs: Vec<&str> = group_rules
            .iter()
            .filter_map(|r| r.description.as_deref())
            .filter(|s| !s.is_empty())
            .collect();
        let desc = descs.join(", ");
        buf.push_str(&format!(
            "# [NAT {inst}] {desc}\nnat {inst} config {config}\n",
        ));
    }
    buf.push('\n');
    buf
}

/// Generate ipfw NAT rules using `via` keyword, placed BEFORE check-state.
///
/// Per NAT instance group, two rules are emitted:
///   - Outbound: `nat N ip from <src> to any out via <iface>`
///   - Inbound:  `nat N ip from any to any in via <iface>`
///
/// Both use `via` (matches the interface regardless of direction). The inbound
/// rule is intentionally broad (`from any to any`) — libalias only translates
/// packets with matching state; others pass through unchanged.
///
/// Rule numbers start at 10 (before check-state at 50 and user rules at 100+).
/// This placement is critical: NAT must run BEFORE check-state so that
/// translated addresses get correct dynamic state entries.
pub fn generate_ipfw_nat_rules(rules: &[NatRule]) -> String {
    let groups = group_nat_by_interface(rules);
    if groups.is_empty() {
        return String::new();
    }
    let mut buf = String::from("# --- NAT rules ---\n");
    for (i, (iface, group_rules)) in groups.iter().enumerate() {
        let inst = (i + 1) as u32;
        let base = (10 + (i as u32) * 10) as u32;

        // Outbound: one rule per SNAT/BINAT source network
        let mut n = base;
        for rule in group_rules.iter().filter(|r| matches!(r.kind, NatKind::Snat | NatKind::Binat)) {
            let label = if rule.kind == NatKind::Binat { "BINAT" } else { "SNAT" };
            let desc = rule.description.as_deref().unwrap_or("");
            buf.push_str(&format!(
                "# [{n:05}] [NAT {inst}] {label} outbound: {desc}\nadd {n:05} nat {inst} ip from {src} to any out via {iface}\n",
                n = n, inst = inst, src = rule.src_addr, iface = iface,
            ));
            n += 1;
        }

        // Inbound: broad de-NAT for return traffic on this interface.
        // libalias passes through non-matching packets unchanged.
        buf.push_str(&format!(
            "# [{n:05}] [NAT {inst}] inbound de-NAT via {iface}\nadd {n:05} nat {inst} ip from any to any in via {iface}\n",
            n = n, inst = inst, iface = iface,
        ));

        buf.push('\n');
    }
    buf
}

/// Generate auto-injected ipfw allow rules that permit NAT'd traffic to pass
/// the default `deny ip from any to any` (rule 65534) in whitelist mode.
///
/// Placed AFTER check-state (at 40000+), so `keep-state` is safe: dynamic
/// states created here are evaluated at check-state (rule 50), which runs
/// AFTER the NAT rules (10+). This ensures NAT always processes packets
/// before any dynamic state can shadow it.
///
/// Returns empty string in blacklist mode or when no NAT rules are enabled.
fn generate_ipfw_nat_pass(rules: &[NatRule], mode: FirewallMode) -> String {
    if mode != FirewallMode::Whitelist {
        return String::new();
    }
    let active: Vec<&NatRule> = rules.iter().filter(|r| r.enabled).collect();
    if active.is_empty() {
        return String::new();
    }
    let mut buf = String::from("# --- NAT auto-pass (whitelist mode) ---\n");
    for (i, rule) in active.iter().enumerate() {
        let base = (40000 + (i as u32) * 100) as u32;
        let desc = rule.description.as_deref().unwrap_or("");
        match rule.kind {
            NatKind::Snat | NatKind::Binat => {
                let label = if rule.kind == NatKind::Binat { "BINAT" } else { "SNAT" };
                let src = &rule.src_addr;
                // Allow NAT'd source network to enter host. keep-state is safe
                // because check-state (rule 50) runs AFTER NAT rules.
                buf.push_str(&format!(
                    "# [{n:05}] [auto] {label} pass: {desc}\nadd {n:05} allow ip from {src} to any in keep-state\n",
                    n = base,
                ));
            }
            NatKind::Dnat => {
                let proto = match rule.protocol {
                    NatProto::Tcp => "tcp",
                    NatProto::Udp => "udp",
                    NatProto::Both => "ip",
                };
                let target = rule
                    .dst_addr
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("any");
                let port_clause = rule
                    .dst_port
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|p| format!(" {}", port_to_ipfw(p)))
                    .unwrap_or_default();
                buf.push_str(&format!(
                    "# [{n:05}] [auto] DNAT pass: {desc}\nadd {n:05} allow {proto} from any to {target}{port_clause} in keep-state\n",
                    n = base,
                ));
            }
        }
        buf.push('\n');
    }
    buf
}

/// PF protocol selector for NAT rules.
fn pf_nat_proto(p: &NatProto) -> String {
    match p {
        NatProto::Tcp => " proto tcp".to_string(),
        NatProto::Udp => " proto udp".to_string(),
        NatProto::Both => " proto { tcp udp }".to_string(),
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

/// Validate a NAT rule body. Returns Ok(()) or BadRequest/NotFound.
pub fn validate_nat_body(body: &NatBody) -> ApiResult<()> {
    // Interface (required for NAT)
    if body.interface.is_empty() {
        return Err(ApiError::BadRequest("interface is required for NAT rules".into()));
    }
    let iface_re = regex::Regex::new(IFACE_RE).unwrap();
    if !iface_re.is_match(&body.interface) {
        return Err(ApiError::BadRequest("invalid interface name".into()));
    }

    // Source address (required)
    if body.src_addr.is_empty() || body.src_addr.len() > 200 {
        return Err(ApiError::BadRequest("invalid source address".into()));
    }
    // "any" is allowed as a wildcard; otherwise validate each comma-separated IP/CIDR.
    if body.src_addr != "any" {
        for part in body.src_addr.split(',').map(|s| s.trim()) {
            if !part.is_empty() {
                validate_address(part)?;
            }
        }
    }

    // Destination address (optional for SNAT, required for DNAT/BINAT)
    if let Some(ref dst) = body.dst_addr {
        if !dst.is_empty() {
            validate_address(dst)?;
        }
    }

    // Ports
    let port_re = regex::Regex::new(PORT_RE).unwrap();
    if let Some(ref p) = body.src_port {
        if !p.is_empty() && !port_re.is_match(p) {
            return Err(ApiError::BadRequest("invalid source port".into()));
        }
    }
    if let Some(ref p) = body.dst_port {
        if !p.is_empty() && !port_re.is_match(p) {
            return Err(ApiError::BadRequest("invalid destination port".into()));
        }
    }

    // kind-specific constraints
    match body.kind {
        NatKind::Dnat => {
            if body.src_port.as_deref().map_or(true, |s| s.is_empty()) {
                return Err(ApiError::BadRequest(
                    "DNAT requires a source port (the original port to forward)".into(),
                ));
            }
            if body.dst_addr.as_deref().map_or(true, |s| s.is_empty()) {
                return Err(ApiError::BadRequest(
                    "DNAT requires a destination address (the internal target)".into(),
                ));
            }
        }
        NatKind::Snat => {
            // dst_addr optional (defaults to interface address); src_addr required.
        }
        NatKind::Binat => {
            if body.dst_addr.as_deref().map_or(true, |s| s.is_empty()) {
                return Err(ApiError::BadRequest(
                    "BINAT requires an external address (dst_addr)".into(),
                ));
            }
        }
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

/// Check if the ipfw_nat kernel module is loaded.
pub fn ipfw_nat_loaded() -> bool {
    cmd::status_sync(KLDSTAT, &["-q", "-n", "ipfw_nat"])
}

/// Load the ipfw_nat kernel module if not already loaded.
/// Required for ipfw NAT rules (snat/dnat). Called by apply_ipfw when NAT
/// rules exist. If the module cannot be loaded, apply fails with a clear
/// error — we do not silently apply filter rules without NAT.
pub fn ensure_ipfw_nat() -> ApiResult<()> {
    if ipfw_nat_loaded() {
        return Ok(());
    }
    cmd::run_sync(KLDLOAD, &["ipfw_nat"])?;
    Ok(())
}

/// Generate and write config file WITHOUT loading into kernel.
/// Used when the firewall is disabled — keeps the file ready for next enable.
pub fn write_config_only(driver: FirewallDriver, rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable], nat_rules: &[NatRule]) -> ApiResult<()> {
    match driver {
        FirewallDriver::Ipfw => {
            let content = generate_ipfw(rules, mode, tables, nat_rules);
            atomic_write(IPFW_RULES_PATH, &content)?;
        }
        FirewallDriver::Pf => {
            let content = generate_pf(rules, mode, tables, nat_rules);
            // Just write the file — validation happens at apply/enable time
            // when the PF module is loaded and /dev/pf exists.
            atomic_write(PF_CONF_PATH, &content)?;
        }
    }
    Ok(())
}

/// Apply ipfw rules: generate file, validate, then load.
pub fn apply_ipfw(rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable], nat_rules: &[NatRule]) -> ApiResult<()> {
    // NAT requires the ipfw_nat kernel module. Load it if NAT rules exist.
    if nat_rules.iter().any(|r| r.enabled) {
        ensure_ipfw_nat()?;
        // With one_pass=1 (kernel default), packets matching a nat rule exit
        // the firewall immediately — filter rules after the nat rule (including
        // the default deny at 65534) are never evaluated. Set one_pass=0 so
        // packets continue through the firewall after NAT translation.
        cmd::run_sync(SYSCTL, &["net.inet.ip.fw.one_pass=0"])?;
    }

    let content = generate_ipfw(rules, mode, tables, nat_rules);

    // Write to temp file and validate syntax before applying.
    let tmp = format!("{IPFW_RULES_PATH}.tmp");
    fs::write(&tmp, &content).map_err(|e| ApiError::Internal(format!("write tmp: {e}")))?;

    // Validate syntax (-n = test only, does not modify kernel state).
    if let Err(e) = cmd::run_sync(IPFW, &["-n", "-q", &tmp]) {
        let _ = fs::remove_file(&tmp);
        return Err(ApiError::Command(format!("ipfw.rules validation failed: {e}")));
    }

    // All good — move to real path and load.
    fs::rename(&tmp, IPFW_RULES_PATH)?;

    // Load via pathname mode: `ipfw -q /etc/ipfw.rules`
    // The file's first line is `-f flush` which clears all rules,
    // then re-adds everything from scratch.
    cmd::run_sync(IPFW, &["-q", IPFW_RULES_PATH])?;
    Ok(())
}

/// Apply pf rules: generate file, validate, write, then reload via rc.d.
pub fn apply_pf(rules: &[FirewallRule], mode: FirewallMode, tables: &[IpTable], nat_rules: &[NatRule]) -> ApiResult<()> {
    let content = generate_pf(rules, mode, tables, nat_rules);

    // Write to temp file and validate
    let tmp = format!("{PF_CONF_PATH}.tmp");
    fs::write(&tmp, &content).map_err(|e| ApiError::Internal(format!("write tmp: {e}")))?;

    // Validate syntax
    if let Err(e) = cmd::run_sync(PFCTL, &["-n", "-f", &tmp]) {
        let _ = fs::remove_file(&tmp);
        return Err(ApiError::Command(format!("pf.conf validation failed: {e}")));
    }

    // All good — move to real path and reload.
    // `service pf reload` runs `pfctl -n -f` (redundant check) then `pfctl -f`.
    fs::rename(&tmp, PF_CONF_PATH)?;
    cmd::run_sync(SERVICE, &["pf", "reload"])?;
    // Flush state table so old connections are killed — new rules apply
    // to all connections immediately. The apply HTTP response has already
    // been sent (with Connection: close), so the browser will reconnect.
    cmd::run_forget_sync(PFCTL, &["-F", "states"]);
    Ok(())
}

/// Enable firewall at runtime (does not modify rc.conf).
/// Caller must have already set the appropriate rc.conf flags
/// (e.g. `pf_enable=YES`) so that `service` accepts the start command.
pub fn enable_firewall(driver: FirewallDriver) -> ApiResult<()> {
    match driver {
        FirewallDriver::Ipfw => {
            // `service ipfw start` does (via rc.d/ipfw → rc.firewall):
            //   1. kldload ipfw (required_modules — auto-loaded by rc.subr)
            //   2. ipfw -q /etc/ipfw.rules (pathname mode, loads our rules)
            //   3. sysctl net.inet.ip.fw.enable=1 (enable)
            cmd::run_sync(SERVICE, &["ipfw", "start"])?;
        }
        FirewallDriver::Pf => {
            // `service pf start` does three things (via rc.d/pf):
            //   1. kldload pf (required_modules="pf" — auto-loaded by rc.subr)
            //   2. pfctl -F all + pfctl -f /etc/pf.conf (load rules)
            //   3. pfctl -eq (enable)
            cmd::run_sync(SERVICE, &["pf", "start"])?;
        }
    }
    Ok(())
}

/// Disable firewall at runtime (does not modify rc.conf).
pub fn disable_firewall(driver: FirewallDriver) -> ApiResult<()> {
    match driver {
        FirewallDriver::Ipfw => {
            // `service ipfw stop` runs `sysctl net.inet.ip.fw.enable=0`.
            // Errors if already stopped, so ignore failure.
            cmd::run_forget_sync(SERVICE, &["ipfw", "stop"]);
        }
        FirewallDriver::Pf => {
            // pfctl -d disables PF. PF is running at this point, so /dev/pf
            // exists. Non-zero exit (already disabled) is harmless — ignore.
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
pub fn init_ipfw(mode: FirewallMode, rules: &[FirewallRule], tables: &[IpTable], nat_rules: &[NatRule]) -> ApiResult<()> {
    use crate::sysrc;

    sysrc::set_multi(&[
        ("firewall_enable", "YES"),
        ("firewall_type", IPFW_RULES_PATH),
        ("firewall_quiet", "YES"),
        ("firewall_logging", "YES"),
    ]).map_err(|e| ApiError::Command(e))?;
    // Remove firewall_script so rc.d falls back to /etc/rc.firewall,
    // which loads our rules via `ipfw -q ${firewall_type}` (pathname mode).
    sysrc::delete("firewall_script");

    ensure_module(FirewallDriver::Ipfw)?;

    // kldload ipfw enables ipfw by default (net.inet.ip.fw.enable defaults to 1)
    // with a default-deny rule 65535. Explicitly disable to avoid blocking all traffic.
    disable_firewall(FirewallDriver::Ipfw)?;

    // Persist one_pass=0 so NAT'd packets continue through the firewall after
    // translation (survives reboot). Without this, nat rules at 50000+ would
    // bypass all filter rules including the default deny at 65534.
    crate::sysctl_conf::upsert("net.inet.ip.fw.one_pass", "0")?;

    // Only generate the file — do NOT load it into the kernel yet.
    let content = generate_ipfw(rules, mode, tables, nat_rules);
    atomic_write(IPFW_RULES_PATH, &content)?;
    Ok(())
}

/// Initialize pf: write rc.conf entries, load module, load rules.
pub fn init_pf(mode: FirewallMode, rules: &[FirewallRule], tables: &[IpTable], nat_rules: &[NatRule]) -> ApiResult<()> {
    use crate::sysrc;

    sysrc::set_multi(&[
        ("pf_enable", "YES"),
        ("pf_rules", PF_CONF_PATH),
    ]).map_err(|e| ApiError::Command(e))?;

    ensure_module(FirewallDriver::Pf)?;

    // Only generate the file — do NOT load it into the kernel yet.
    let content = generate_pf(rules, mode, tables, nat_rules);
    atomic_write(PF_CONF_PATH, &content)?;
    Ok(())
}

/// Disable ipfw in rc.conf and at runtime.
pub fn deactivate_ipfw() -> ApiResult<()> {
    use crate::sysrc;
    disable_firewall(FirewallDriver::Ipfw)?;
    sysrc::ensure_no("firewall_enable");
    Ok(())
}

/// Disable pf in rc.conf and at runtime.
pub fn deactivate_pf() -> ApiResult<()> {
    use crate::sysrc;
    disable_firewall(FirewallDriver::Pf)?;
    sysrc::ensure_no("pf_enable");
    Ok(())
}

/// Generate config content without writing to disk (for preview before apply).
pub fn preview_config(
    driver: FirewallDriver,
    rules: &[FirewallRule],
    mode: FirewallMode,
    tables: &[IpTable],
    nat_rules: &[NatRule],
) -> String {
    match driver {
        FirewallDriver::Ipfw => generate_ipfw(rules, mode, tables, nat_rules),
        FirewallDriver::Pf => generate_pf(rules, mode, tables, nat_rules),
    }
}

// ── anti-lockout: backup + rollback ────────────────────────────────

/// Timeout in seconds before auto-rollback.
pub const APPLY_TIMEOUT_SECS: i64 = 60;

/// Path to the pending-apply JSON file.
const PENDING_APPLY_PATH: &str = "/var/db/fwp/firewall_pending.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingApply {
    pub created_at: i64,
    pub expires_at: i64,
    pub operation: String,
    pub driver: FirewallDriver,
    pub was_enabled: bool,
    pub backup_config: String,
    pub status: String,
    /// Snapshot of rule enabled states before the operation (id → enabled).
    /// Used to restore DB state on rollback of an apply.
    #[serde(default)]
    pub rule_snapshot: Vec<(i64, bool)>,
    /// Previous mode before a mode-change operation (for rollback).
    #[serde(default)]
    pub old_mode: Option<String>,
}

/// Write the pending-apply state as a JSON file.
pub fn create_pending_apply(
    operation: &str,
    driver: FirewallDriver,
    was_enabled: bool,
    backup_config: &str,
    now: i64,
) -> ApiResult<()> {
    let pending = PendingApply {
        created_at: now,
        expires_at: now + APPLY_TIMEOUT_SECS,
        operation: operation.to_string(),
        driver,
        was_enabled,
        backup_config: backup_config.to_string(),
        status: "pending".to_string(),
        rule_snapshot: Vec::new(),
        old_mode: None,
    };
    let json = serde_json::to_string_pretty(&pending)
        .map_err(|e| ApiError::Internal(format!("serialize pending: {e}")))?;
    atomic_write(PENDING_APPLY_PATH, &json)?;
    Ok(())
}

/// Read the pending-apply state from the JSON file, if it exists.
pub fn get_pending_apply() -> Option<PendingApply> {
    let data = fs::read_to_string(PENDING_APPLY_PATH).ok()?;
    serde_json::from_str(&data).ok()
}

/// Delete the pending-apply file (used on confirm or after rollback).
pub fn clear_pending_apply() {
    let _ = fs::remove_file(PENDING_APPLY_PATH);
}

/// Read the current config file content for backup.
pub fn read_config_file(driver: FirewallDriver) -> String {
    let path = match driver {
        FirewallDriver::Ipfw => IPFW_RULES_PATH,
        FirewallDriver::Pf => PF_CONF_PATH,
    };
    fs::read_to_string(path).unwrap_or_default()
}

/// Update the old_mode field in an existing pending-apply file.
pub fn set_pending_old_mode(mode: &str) -> ApiResult<()> {
    if let Some(mut p) = get_pending_apply() {
        p.old_mode = Some(mode.to_string());
        let json = serde_json::to_string_pretty(&p)
            .map_err(|e| ApiError::Internal(format!("serialize pending: {e}")))?;
        atomic_write(PENDING_APPLY_PATH, &json)?;
    }
    Ok(())
}

/// Restore a backed-up config and revert runtime state.
/// This runs local system commands — does not depend on network connectivity.
pub fn rollback(driver: FirewallDriver, backup_config: &str, was_enabled: bool) -> ApiResult<()> {
    let path = match driver {
        FirewallDriver::Ipfw => IPFW_RULES_PATH,
        FirewallDriver::Pf => PF_CONF_PATH,
    };

    // Restore the backup config file.
    atomic_write(path, backup_config)?;

    // Reload it into the kernel.
    match driver {
        FirewallDriver::Ipfw => {
            cmd::run_sync(IPFW, &["-q", path])?;
        }
        FirewallDriver::Pf => {
            cmd::run_sync(PFCTL, &["-f", path])?;
        }
    }

    // If the firewall was NOT enabled before the operation, disable it now.
    if !was_enabled {
        disable_firewall(driver)?;
    }

    tracing::warn!("firewall rollback completed (driver={driver:?}, was_enabled={was_enabled})");
    Ok(())
}

// ── staging (uncommitted rule changes when FW is enabled) ──────────

const STAGING_PATH: &str = "/var/db/fwp/firewall_staging.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StagingData {
    rules: Vec<FirewallRule>,
    tables: Vec<IpTable>,
    // Added when NAT support was introduced. Old staging files (pre-NAT)
    // deserialize to an empty Vec — safe default.
    #[serde(default)]
    nat_rules: Vec<NatRule>,
}

/// Write staging file with proposed rules + tables + NAT rules.
pub fn write_staging(rules: &[FirewallRule], tables: &[IpTable], nat_rules: &[NatRule]) -> ApiResult<()> {
    let data = StagingData {
        rules: rules.to_vec(),
        tables: tables.to_vec(),
        nat_rules: nat_rules.to_vec(),
    };
    let json = serde_json::to_string_pretty(&data)
        .map_err(|e| ApiError::Internal(format!("serialize staging: {e}")))?;
    atomic_write(STAGING_PATH, &json)?;
    Ok(())
}

/// Read staging if it exists.
/// Returns (rules, tables, nat_rules).
pub fn read_staging() -> Option<(Vec<FirewallRule>, Vec<IpTable>, Vec<NatRule>)> {
    let data = fs::read_to_string(STAGING_PATH).ok()?;
    let staging: StagingData = serde_json::from_str(&data).ok()?;
    Some((staging.rules, staging.tables, staging.nat_rules))
}

/// Check if staging file exists.
pub fn has_staging() -> bool {
    std::path::Path::new(STAGING_PATH).exists()
}

/// Delete staging file.
pub fn clear_staging() {
    let _ = fs::remove_file(STAGING_PATH);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snat_rule() -> NatRule {
        NatRule {
            id: 1, position: 0, enabled: true,
            kind: NatKind::Snat, family: NatFamily::Ip,
            interface: "vtnet0".into(),
            src_addr: "192.168.1.0/24".into(),
            dst_addr: None, src_port: None, dst_port: None,
            protocol: NatProto::Both,
            description: Some("NAT for jail network".into()),
            created_at: 0, updated_at: 0,
        }
    }

    fn dnat_rule() -> NatRule {
        NatRule {
            id: 2, position: 0, enabled: true,
            kind: NatKind::Dnat, family: NatFamily::Ip,
            interface: "vtnet0".into(),
            src_addr: "any".into(),
            dst_addr: Some("10.0.0.2".into()),
            src_port: Some("80".into()),
            dst_port: Some("8080".into()),
            protocol: NatProto::Tcp,
            description: Some("Forward HTTP".into()),
            created_at: 0, updated_at: 0,
        }
    }

    #[test]
    fn pf_snat_whitelist_includes_auto_pass() {
        let pf = generate_pf(&[], FirewallMode::Whitelist, &[], &[snat_rule()]);
        assert!(pf.contains("nat on vtnet0 inet from 192.168.1.0/24 to any -> (vtnet0)"));
        assert!(pf.contains("# --- NAT auto-pass (whitelist mode) ---"));
        assert!(pf.contains("pass in quick inet from 192.168.1.0/24 to any keep state"));
    }

    #[test]
    fn pf_snat_blacklist_no_auto_pass() {
        let pf = generate_pf(&[], FirewallMode::Blacklist, &[], &[snat_rule()]);
        assert!(pf.contains("nat on vtnet0"));
        assert!(!pf.contains("NAT auto-pass"));
    }

    #[test]
    fn pf_dnat_whitelist_auto_pass_matches_internal_target() {
        let pf = generate_pf(&[], FirewallMode::Whitelist, &[], &[dnat_rule()]);
        assert!(pf.contains("rdr on vtnet0 inet proto tcp from any to any port 80 -> 10.0.0.2 port 8080"));
        // Auto-pass matches the translated destination (internal target)
        assert!(pf.contains("pass in quick on vtnet0 inet proto tcp from any to 10.0.0.2 port 8080"));
    }

    #[test]
    fn ipfw_snat_whitelist_includes_auto_pass() {
        let ipfw = generate_ipfw(&[], FirewallMode::Whitelist, &[], &[snat_rule()]);
        // NAT rules before check-state, low numbers
        assert!(ipfw.contains("add 00010 nat 1 ip from 192.168.1.0/24 to any out via vtnet0"));
        assert!(ipfw.contains("add 00011 nat 1 ip from any to any in via vtnet0"));
        // check-state present
        assert!(ipfw.contains("add 050 check-state"));
        // Auto-pass with keep-state (safe because after check-state)
        assert!(ipfw.contains("add 40000 allow ip from 192.168.1.0/24 to any in keep-state"));
        // Default policy
        assert!(ipfw.contains("add 65000 allow ip from me to any out keep-state"));
    }

    #[test]
    fn ipfw_snat_blacklist_no_auto_pass() {
        let ipfw = generate_ipfw(&[], FirewallMode::Blacklist, &[], &[snat_rule()]);
        assert!(!ipfw.contains("NAT auto-pass"));
    }

    #[test]
    fn ipfw_dnat_whitelist_auto_pass_after_nat() {
        let ipfw = generate_ipfw(&[], FirewallMode::Whitelist, &[], &[dnat_rule()]);
        // Inbound de-NAT rule (before check-state)
        assert!(ipfw.contains("add 00010 nat 1 ip from any to any in via vtnet0"));
        // DNAT redirect_port in config
        assert!(ipfw.contains("redirect_port tcp 10.0.0.2:8080 80"));
        // Auto-pass with keep-state
        assert!(ipfw.contains("add 40000 allow tcp from any to 10.0.0.2 8080 in keep-state"));
    }

    #[test]
    fn pf_no_nat_no_auto_pass() {
        let pf = generate_pf(&[], FirewallMode::Whitelist, &[], &[]);
        assert!(!pf.contains("NAT auto-pass"));
    }

    #[test]
    fn ipfw_disabled_nat_no_auto_pass() {
        let mut r = snat_rule();
        r.enabled = false;
        let ipfw = generate_ipfw(&[], FirewallMode::Whitelist, &[], &[r]);
        assert!(!ipfw.contains("NAT auto-pass"));
        assert!(!ipfw.contains("add 00010"));
    }

    #[test]
    fn pf_user_block_overrides_snat_auto_pass() {
        // A user block rule placed ahead of the auto-pass should win
        // (PF first-match-quick). The auto-pass still allows the rest of
        // the NAT'd subnet.
        let user_block = FirewallRule {
            id: 10, position: 0, enabled: true,
            action: RuleAction::Deny, direction: RuleDirection::In,
            protocol: RuleProtocol::Any,
            source: AddressSpec { kind: AddressKind::Single, value: "192.168.1.66".into() },
            source_port: None,
            destination: AddressSpec { kind: AddressKind::Any, value: String::new() },
            destination_port: None,
            interface: None, log: false, icmp_type: None,
            description: Some("Block bad jail".into()),
            created_at: 0, updated_at: 0,
        };
        let pf = generate_pf(&[user_block], FirewallMode::Whitelist, &[], &[snat_rule()]);
        let user_pos = pf.find("block in quick inet from 192.168.1.66").unwrap();
        let auto_pos = pf.find("# --- NAT auto-pass").unwrap();
        assert!(user_pos < auto_pos, "user block rule must come before auto-pass");
    }

    #[test]
    fn ipfw_dnat_alias_ip_via_src_addr_single_instance() {
        // src_addr with comma-separated alias IPs generates redirect_port
        // clauses with aliasIP:aliasPort, all in the same NAT instance as SNAT.
        let mut dnat = dnat_rule();
        dnat.src_addr = "203.0.113.5,203.0.113.6".into();
        let snat = snat_rule(); // same interface
        let ipfw = generate_ipfw(&[], FirewallMode::Whitelist, &[], &[snat, dnat]);
        // Single instance with `if vtnet0`
        assert!(ipfw.contains("nat 1 config if vtnet0"));
        // redirect_port for each alias IP
        assert!(ipfw.contains("redirect_port tcp 10.0.0.2:8080 203.0.113.5:80"));
        assert!(ipfw.contains("redirect_port tcp 10.0.0.2:8080 203.0.113.6:80"));
    }

    #[test]
    fn pf_dnat_alias_ip_via_src_addr_multiple_rdr() {
        // src_addr with comma-separated alias IPs generates a separate rdr per IP.
        let mut r = dnat_rule();
        r.src_addr = "203.0.113.5,203.0.113.6".into();
        let pf = generate_pf(&[], FirewallMode::Whitelist, &[], &[r]);
        assert!(pf.contains("rdr on vtnet0 inet proto tcp from any to 203.0.113.5 port 80 -> 10.0.0.2 port 8080"));
        assert!(pf.contains("rdr on vtnet0 inet proto tcp from any to 203.0.113.6 port 80 -> 10.0.0.2 port 8080"));
    }
}
