//! Bhyve virtual machine management — vm-bhyve CLI wrapper.
//!
//! All operations go through the `vm` binary at `/usr/local/sbin/vm`.
//! vm-bhyve itself handles bhyve/nmdm/grub-bhyve etc.

use std::process::{Command, Stdio};

use serde::Serialize;

const VM: &str = "/usr/local/sbin/vm";
const SYSRC: &str = "/usr/sbin/sysrc";
const PKG: &str = "/usr/sbin/pkg";
const ZFS: &str = "/sbin/zfs";

// ── Command helpers ───────────────────────────────────────────────

/// Run a `vm` subcommand that terminates, capturing stdout.
fn vm_run(args: &[&str]) -> Result<String, String> {
    let output = Command::new(VM)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let msg = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("vm {} failed (exit {})", args.join(" "), output.status)
        };
        return Err(msg);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ── Table parsing ─────────────────────────────────────────────────

/// Column descriptor: header label + byte offset in the line.
struct Column {
    label: String,
    offset: usize,
}

/// Parse a table header line to find each column's start offset.
///
/// vm-bhyve outputs fixed-width tables where column values may contain
/// spaces (e.g. `Running (4272)`). We find each header word's byte
/// position, then slice data rows by those offsets.
fn parse_header(header: &str) -> Vec<Column> {
    let bytes = header.as_bytes();
    let mut cols = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let label = String::from_utf8_lossy(&bytes[start..i]).to_string();
        cols.push(Column { label, offset: start });
    }
    cols
}

/// Extract a trimmed value for column `idx` from a data line.
fn col_value(line: &str, cols: &[Column], idx: usize) -> String {
    let start = cols[idx].offset;
    let end = if idx + 1 < cols.len() {
        cols[idx + 1].offset
    } else {
        line.len()
    };
    line.get(start..end).unwrap_or("").trim().to_string()
}

/// Find a column index by header label (case-insensitive).
fn col_index(cols: &[Column], label: &str) -> Option<usize> {
    cols.iter()
        .position(|c| c.label.eq_ignore_ascii_case(label))
}

// ── Data models ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct VmSummary {
    pub name: String,
    pub datastore: String,
    pub loader: String,
    pub cpu: u32,
    pub memory: String,
    pub vnc: Option<String>,
    pub auto_start: bool,
    pub auto_order: Option<u32>,
    pub state: String,
    pub pid: Option<u32>,
    pub locked_by: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VmImage {
    pub uuid: String,
    pub name: String,
    pub created: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VmSwitch {
    pub name: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub iface: String,
    pub address: Option<String>,
    pub private: bool,
    pub mtu: Option<String>,
    pub vlan: Option<String>,
    pub ports: Vec<String>,
}

// ── vm list ───────────────────────────────────────────────────────

/// Parse the `vm list` table into structured data.
pub fn list_vms(running_only: bool) -> Result<Vec<VmSummary>, String> {
    let args = if running_only {
        &["list", "-r"][..]
    } else {
        &["list"][..]
    };
    let output = vm_run(args)?;

    let mut lines = output.lines();
    let header = lines.next().ok_or("vm list: empty output")?;
    let cols = parse_header(header);

    let i_name = col_index(&cols, "NAME").ok_or("vm list: NAME column not found")?;
    let i_ds = col_index(&cols, "DATASTORE").ok_or("vm list: DATASTORE column not found")?;
    let i_loader = col_index(&cols, "LOADER").ok_or("vm list: LOADER column not found")?;
    let i_cpu = col_index(&cols, "CPU").ok_or("vm list: CPU column not found")?;
    let i_mem = col_index(&cols, "MEMORY").ok_or("vm list: MEMORY column not found")?;
    let i_vnc = col_index(&cols, "VNC").ok_or("vm list: VNC column not found")?;
    let i_auto = col_index(&cols, "AUTO").ok_or("vm list: AUTO column not found")?;
    let i_state = col_index(&cols, "STATE").ok_or("vm list: STATE column not found")?;

    let mut vms = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let name = col_value(line, &cols, i_name);
        if name.is_empty() {
            continue;
        }

        let cpu_str = col_value(line, &cols, i_cpu);
        let cpu: u32 = cpu_str.parse().unwrap_or(0);

        let vnc_raw = col_value(line, &cols, i_vnc);
        let vnc = if vnc_raw == "-" {
            None
        } else {
            Some(vnc_raw)
        };

        let auto_raw = col_value(line, &cols, i_auto);
        let (auto_start, auto_order) = parse_auto(&auto_raw);

        let state_raw = col_value(line, &cols, i_state);
        let (state, pid, locked_by) = parse_state(&state_raw);

        vms.push(VmSummary {
            name,
            datastore: col_value(line, &cols, i_ds),
            loader: col_value(line, &cols, i_loader),
            cpu,
            memory: col_value(line, &cols, i_mem),
            vnc,
            auto_start,
            auto_order,
            state,
            pid,
            locked_by,
        });
    }
    Ok(vms)
}

/// Parse the AUTO column: "No", "Yes", "Yes [1]".
fn parse_auto(raw: &str) -> (bool, Option<u32>) {
    let lower = raw.to_ascii_lowercase();
    if !lower.starts_with("yes") {
        return (false, None);
    }
    // Extract [N] if present.
    if let Some(start) = lower.find('[') {
        if let Some(end) = lower.find(']') {
            if start < end {
                let num_str = &lower[start + 1..end];
                if let Ok(n) = num_str.parse::<u32>() {
                    return (true, Some(n));
                }
            }
        }
    }
    (true, None)
}

/// Parse the STATE column: "Stopped", "Running (4272)", "Locked (ppbsd)".
fn parse_state(raw: &str) -> (String, Option<u32>, Option<String>) {
    if let Some(rest) = raw.strip_prefix("Running") {
        let pid = extract_paren_u32(rest);
        return ("running".into(), pid, None);
    }
    if let Some(rest) = raw.strip_prefix("Locked") {
        let host = extract_paren_str(rest);
        return ("locked".into(), None, host);
    }
    if raw.starts_with("Suspended") {
        return ("suspended".into(), None, None);
    }
    ("stopped".into(), None, None)
}

fn extract_paren_u32(s: &str) -> Option<u32> {
    let start = s.find('(')?;
    let end = s.find(')')?;
    if start >= end {
        return None;
    }
    s[start + 1..end].parse().ok()
}

fn extract_paren_str(s: &str) -> Option<String> {
    let start = s.find('(')?;
    let end = s.find(')')?;
    if start >= end {
        return None;
    }
    let val = s[start + 1..end].trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

// ── vm image list ─────────────────────────────────────────────────

/// Parse the `vm image list` table.
pub fn list_images() -> Result<Vec<VmImage>, String> {
    let output = vm_run(&["image", "list"])?;
    let mut lines = output.lines();

    let header = match lines.next() {
        Some(h) => h,
        None => return Ok(Vec::new()),
    };
    let cols = parse_header(header);

    let i_uuid = col_index(&cols, "UUID").ok_or("vm image list: UUID column not found")?;
    let i_name = col_index(&cols, "NAME").ok_or("vm image list: NAME column not found")?;
    let i_created = col_index(&cols, "CREATED").ok_or("vm image list: CREATED column not found")?;
    let i_desc = col_index(&cols, "DESCRIPTION").ok_or("vm image list: DESCRIPTION column not found")?;

    let mut images = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let uuid = col_value(line, &cols, i_uuid);
        if uuid.is_empty() {
            continue;
        }
        images.push(VmImage {
            uuid,
            name: col_value(line, &cols, i_name),
            created: col_value(line, &cols, i_created),
            description: col_value(line, &cols, i_desc),
        });
    }
    Ok(images)
}

// ── vm switch list ────────────────────────────────────────────────

/// Parse the `vm switch list` table.
pub fn list_switches() -> Result<Vec<VmSwitch>, String> {
    let output = vm_run(&["switch", "list"])?;

    let mut lines = output.lines();
    let header = match lines.next() {
        Some(h) => h,
        None => return Ok(Vec::new()),
    };
    let cols = parse_header(header);

    let i_name = col_index(&cols, "NAME").ok_or("vm switch list: NAME column not found")?;
    let i_type = col_index(&cols, "TYPE").ok_or("vm switch list: TYPE column not found")?;
    let i_iface = col_index(&cols, "IFACE").ok_or("vm switch list: IFACE column not found")?;
    let i_addr = col_index(&cols, "ADDRESS").ok_or("vm switch list: ADDRESS column not found")?;
    let i_priv = col_index(&cols, "PRIVATE").ok_or("vm switch list: PRIVATE column not found")?;
    let i_mtu = col_index(&cols, "MTU").ok_or("vm switch list: MTU column not found")?;
    let i_vlan = col_index(&cols, "VLAN").ok_or("vm switch list: VLAN column not found")?;
    let i_ports = col_index(&cols, "PORTS").ok_or("vm switch list: PORTS column not found")?;

    let mut switches = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let name = col_value(line, &cols, i_name);
        if name.is_empty() {
            continue;
        }

        let addr_raw = col_value(line, &cols, i_addr);
        let mtu_raw = col_value(line, &cols, i_mtu);
        let vlan_raw = col_value(line, &cols, i_vlan);
        let ports_raw = col_value(line, &cols, i_ports);

        switches.push(VmSwitch {
            name,
            typ: col_value(line, &cols, i_type),
            iface: col_value(line, &cols, i_iface),
            address: dash_to_none(&addr_raw),
            private: col_value(line, &cols, i_priv).eq_ignore_ascii_case("yes"),
            mtu: dash_to_none(&mtu_raw),
            vlan: dash_to_none(&vlan_raw),
            ports: if ports_raw == "-" {
                Vec::new()
            } else {
                ports_raw.split_whitespace().map(String::from).collect()
            },
        });
    }
    Ok(switches)
}

#[derive(Debug, Clone, Serialize)]
pub struct VmSwitchDetail {
    pub name: String,
    pub fields: std::collections::BTreeMap<String, String>,
}

pub fn get_switch_info(name: &str) -> Result<VmSwitchDetail, String> {
    let output = vm_run(&["switch", "info", name])?;
    Ok(parse_switch_info(&output, name))
}

fn parse_switch_info(raw: &str, name: &str) -> VmSwitchDetail {
    let mut fields = std::collections::BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if !key.is_empty() && !value.is_empty() {
            fields.insert(key.to_string(), value.to_string());
        }
    }
    VmSwitchDetail {
        name: name.to_string(),
        fields,
    }
}

pub fn create_switch(
    name: &str,
    typ: &str,
    iface: Option<&str>,
    vlan: Option<u16>,
    bridge: Option<&str>,
    address: Option<&str>,
    mtu: Option<u16>,
    private: bool,
) -> Result<(), String> {
    let vlan_str;
    let mtu_str;
    let mut args = vec!["switch", "create", "-t", typ];

    if let Some(value) = iface {
        args.push("-i");
        args.push(value);
    }
    if let Some(value) = vlan {
        vlan_str = value.to_string();
        args.push("-n");
        args.push(&vlan_str);
    }
    if let Some(value) = bridge {
        args.push("-b");
        args.push(value);
    }
    if let Some(value) = address {
        args.push("-a");
        args.push(value);
    }
    if let Some(value) = mtu {
        mtu_str = value.to_string();
        args.push("-m");
        args.push(&mtu_str);
    }
    if private {
        args.push("-p");
    }
    args.push(name);

    vm_run(&args)?;
    Ok(())
}

pub fn destroy_switch(name: &str) -> Result<(), String> {
    vm_run(&["switch", "destroy", name])?;
    Ok(())
}

fn dash_to_none(s: &str) -> Option<String> {
    if s == "-" || s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

// ── vm datastore list ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct VmDatastore {
    pub name: String,
    #[serde(rename = "type")]
    pub typ: String,
    pub path: String,
    pub zfs_dataset: Option<String>,
}

/// Parse the `vm datastore list` table.
pub fn list_datastores() -> Result<Vec<VmDatastore>, String> {
    let output = vm_run(&["datastore", "list"])?;

    let mut lines = output.lines();
    let header = match lines.next() {
        Some(h) => h,
        None => return Ok(Vec::new()),
    };
    let mut cols = parse_header(header);
    // "ZFS DATASET" is a two-word header for a single data column.
    // Remove "DATASET" so col_value for ZFS extends to end of line.
    cols.retain(|c| c.label.to_uppercase() != "DATASET");

    let i_name = col_index(&cols, "NAME").ok_or("vm datastore list: NAME column not found")?;
    let i_type = col_index(&cols, "TYPE").ok_or("vm datastore list: TYPE column not found")?;
    let i_path = col_index(&cols, "PATH").ok_or("vm datastore list: PATH column not found")?;
    let i_zfs = col_index(&cols, "ZFS").ok_or("vm datastore list: ZFS column not found")?;

    let mut datastores = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let name = col_value(line, &cols, i_name);
        if name.is_empty() {
            continue;
        }
        let zfs_raw = col_value(line, &cols, i_zfs);
        datastores.push(VmDatastore {
            name,
            typ: col_value(line, &cols, i_type),
            path: col_value(line, &cols, i_path),
            zfs_dataset: dash_to_none(&zfs_raw),
        });
    }
    Ok(datastores)
}

/// Add a new datastore via `vm datastore add <name> <spec>`.
/// The spec is either `/path/to/dir` (directory) or `zfs:pool/dataset` (ZFS).
/// vm-bhyve also supports `iso:/path` and `img:/path`.
pub fn add_datastore(name: &str, spec: &str) -> Result<(), String> {
    vm_run(&["datastore", "add", name, spec])?;
    Ok(())
}

/// Remove a datastore from vm-bhyve configuration via `vm datastore remove`.
/// This only removes the configuration entry; it does not delete any data.
pub fn remove_datastore(name: &str) -> Result<(), String> {
    vm_run(&["datastore", "remove", name])?;
    Ok(())
}

// ── templates ─────────────────────────────────────────────────────

fn default_datastore_path() -> Result<String, String> {
    list_datastores()?
        .into_iter()
        .find(|datastore| datastore.name == "default")
        .map(|datastore| datastore.path)
        .ok_or_else(|| "vm datastore list: default datastore not found".to_string())
}

/// List available VM templates from the default datastore.
pub fn list_templates() -> Result<Vec<String>, String> {
    let path = std::path::Path::new(&default_datastore_path()?).join(".templates");
    let dir = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut templates = Vec::new();
    for entry in dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(base) = name.strip_suffix(".conf") {
            templates.push(base.to_string());
        }
    }
    templates.sort();
    Ok(templates)
}

// ── ISOs ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct IsoImage {
    pub name: String,
    pub size: u64,
}

/// List ISO images from the default datastore.
pub fn list_isos() -> Result<Vec<IsoImage>, String> {
    let path = std::path::Path::new(&default_datastore_path()?).join(".iso");
    let dir = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut isos = Vec::new();
    for entry in dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        isos.push(IsoImage { name, size });
    }
    isos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(isos)
}

// ── vm create ─────────────────────────────────────────────────────

/// Create a new virtual machine via `vm create`.
pub fn create_vm(
    name: &str,
    template: &str,
    datastore: Option<&str>,
    size: Option<&str>,
    cpu: Option<u32>,
    memory: Option<&str>,
) -> Result<(), String> {
    let cpu_str;
    let mut args: Vec<&str> = vec!["create"];

    if let Some(ds) = datastore {
        args.push("-d");
        args.push(ds);
    }
    args.push("-t");
    args.push(template);
    if let Some(s) = size {
        args.push("-s");
        args.push(s);
    }
    if let Some(c) = cpu {
        cpu_str = c.to_string();
        args.push("-c");
        args.push(&cpu_str);
    }
    if let Some(m) = memory {
        args.push("-m");
        args.push(m);
    }
    args.push(name);

    vm_run(&args)?;
    Ok(())
}

// ── vm start / stop ───────────────────────────────────────────────

/// Start a VM. Uses `.status()` not `.output()` — `vm start` forks a
/// long-lived bhyve process; `.output()` would block forever waiting for
/// the pipe to close (same lesson as `jail -c` in jail.rs).
pub fn start_vm(name: &str) -> Result<(), String> {
    let status = Command::new(VM)
        .args(["start", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("vm start {} failed (exit {})", name, status));
    }
    Ok(())
}

/// Stop a VM (graceful shutdown).
pub fn stop_vm(name: &str) -> Result<(), String> {
    let output = Command::new(VM)
        .args(["stop", name])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let msg = if !stderr.is_empty() {
            stderr
        } else {
            format!("vm stop {} failed (exit {})", name, output.status)
        };
        return Err(msg);
    }
    Ok(())
}

/// Create and attach a new disk to a VM via `vm add -d disk -t <type> -s <size> <name>`.
/// Valid types: `zvol`, `sparse-zvol`, `file`.
pub fn add_disk(name: &str, disk_type: &str, size: &str) -> Result<(), String> {
    let valid_types = ["zvol", "sparse-zvol", "file"];
    if !valid_types.contains(&disk_type) {
        return Err(format!("invalid disk type: {disk_type}"));
    }
    if size.is_empty() {
        return Err("disk size must not be empty".to_string());
    }
    let output = Command::new(VM)
        .args(["add", "-d", "disk", "-t", disk_type, "-s", size, name])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let msg = if !stderr.is_empty() {
            stderr
        } else {
            format!("vm add failed (exit {})", output.status)
        };
        return Err(msg);
    }
    Ok(())
}

// ── vm info ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct VmDetail {
    pub name: String,
    pub state: String,
    pub datastore: String,
    pub loader: String,
    pub uuid: String,
    pub cpu: u32,
    pub memory: String,
    pub auto_start: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_resident: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub console_port: Option<String>,
    pub networks: Vec<VmNetwork>,
    pub disks: Vec<VmDisk>,
    pub snapshots: Vec<VmSnapshot>,
    pub config: std::collections::BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vnc_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VmNetwork {
    pub number: u32,
    pub emulation: String,
    pub virtual_switch: Option<String>,
    pub mac_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_in: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_out: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VmDisk {
    pub number: u32,
    pub device_type: String,
    pub emulation: String,
    pub system_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_used: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VmSnapshot {
    pub name: String,
    pub size: String,
    pub date: String,
}

/// Parse `vm info <name>` output into structured data, plus read the
/// `.conf` file for raw config key-values.
pub fn get_vm_info(name: &str) -> Result<VmDetail, String> {
    let output = vm_run(&["info", name])?;
    let mut info = parse_vm_info(&output, name)?;
    let vm_list = read_vm_list();
    info.auto_start = vm_list.iter().any(|n| n == name);
    Ok(info)
}

/// Parse the indented multi-section `vm info` output.
fn parse_vm_info(raw: &str, vm_name: &str) -> Result<VmDetail, String> {
    let mut state = String::new();
    let mut datastore = String::new();
    let mut loader = String::new();
    let mut uuid = String::new();
    let mut cpu: u32 = 0;
    let mut memory = String::new();
    let mut memory_resident = None;
    let mut console_port = None;
    let mut networks = Vec::new();
    let mut disks = Vec::new();
    let mut snapshots = Vec::new();

    // Split into lines and parse sequentially.
    // The format has top-level `key: value` and sub-sections introduced
    // by a bare section header line (indented), followed by more
    // indented key: value lines.
    let lines: Vec<&str> = raw.lines().collect();
    let mut i = 0;

    // Skip the title block: dashes, "Virtual Machine: name", dashes.
    while i < lines.len() {
        let l = lines[i].trim();
        if l.starts_with("Virtual Machine:") {
            i += 1;
            // Skip the second dashed line.
            while i < lines.len() && lines[i].trim().starts_with('-') {
                i += 1;
            }
            break;
        }
        if !l.starts_with('-') && !l.is_empty() {
            break;
        }
        i += 1;
    }

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        // Section headers: bare words with no colon, followed by indented k:v lines.
        if !trimmed.contains(':') {
            let section = trimmed.to_string();
            i += 1;
            // Collect raw sub-lines until blank line or non-indented line.
            let mut raw_lines: Vec<String> = Vec::new();
            while i < lines.len() {
                let sl = lines[i];
                let st = sl.trim();
                if st.is_empty() {
                    break;
                }
                if !sl.starts_with(' ') {
                    break;
                }
                raw_lines.push(st.to_string());
                i += 1;
            }

            // Parse sub-lines as key:value (first colon) for most sections.
            // Snapshots are special: tab-separated, colons appear in dates.
            match section.as_str() {
                "snapshots" => {
                    for rl in &raw_lines {
                        let parts: Vec<&str> = rl.split('\t').collect();
                        if parts.len() >= 3 {
                            snapshots.push(VmSnapshot {
                                name: parts[0].trim().to_string(),
                                size: parts[1].trim().to_string(),
                                date: parts[2..].join(" ").trim().to_string(),
                            });
                        } else if parts.len() == 2 {
                            snapshots.push(VmSnapshot {
                                name: parts[0].trim().to_string(),
                                size: parts[1].trim().to_string(),
                                date: String::new(),
                            });
                        }
                    }
                }
                _ => {
                    let sub: Vec<(String, String)> = raw_lines
                        .iter()
                        .filter_map(|rl| {
                            let colon = rl.find(':')?;
                            Some((
                                rl[..colon].trim().to_string(),
                                rl[colon + 1..].trim().to_string(),
                            ))
                        })
                        .collect();

                    match section.as_str() {
                        "console-ports" => {
                            if let Some(v) = get_kv_opt(&sub, "com1") {
                                console_port = Some(v);
                            }
                        }
                        "network-interface" => {
                            let mut net = VmNetwork {
                                number: get_kv(&sub, "number")
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0),
                                emulation: get_kv(&sub, "emulation")
                                    .unwrap_or_default()
                                    .to_string(),
                                virtual_switch: get_kv_opt(&sub, "virtual-switch"),
                                mac_address: get_kv_opt(&sub, "fixed-mac-address"),
                                active_device: get_kv_opt(&sub, "active-device"),
                                bytes_in: get_kv_opt(&sub, "bytes-in"),
                                bytes_out: get_kv_opt(&sub, "bytes-out"),
                            };
                            if net.mac_address.as_deref() == Some("-") {
                                net.mac_address = None;
                            }
                            if net.active_device.as_deref() == Some("-") {
                                net.active_device = None;
                            }
                            networks.push(net);
                        }
                        "virtual-disk" => {
                            let mut disk = VmDisk {
                                number: get_kv(&sub, "number")
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or(0),
                                device_type: get_kv(&sub, "device-type")
                                    .unwrap_or_default()
                                    .to_string(),
                                emulation: get_kv(&sub, "emulation")
                                    .unwrap_or_default()
                                    .to_string(),
                                system_path: get_kv(&sub, "system-path")
                                    .unwrap_or_default()
                                    .to_string(),
                                bytes_size: get_kv_opt(&sub, "bytes-size"),
                                bytes_used: get_kv_opt(&sub, "bytes-used"),
                            };
                            if disk.bytes_size.as_deref() == Some("-") {
                                disk.bytes_size = None;
                            }
                            if disk.bytes_used.as_deref() == Some("-") {
                                disk.bytes_used = None;
                            }
                            disks.push(disk);
                        }
                        _ => {}
                    }
                }
            }
            continue;
        }

        // Top-level key: value
        if let Some(colon) = trimmed.find(':') {
            let k = trimmed[..colon].trim();
            let v = trimmed[colon + 1..].trim();
            match k {
                "state" => state = v.to_string(),
                "datastore" => datastore = v.to_string(),
                "loader" => loader = v.to_string(),
                "uuid" => uuid = v.to_string(),
                "cpu" => cpu = v.parse().unwrap_or(0),
                "memory" => memory = v.to_string(),
                "memory-resident" => memory_resident = Some(v.to_string()),
                _ => {}
            }
        }
        i += 1;
    }

    // Read .conf file.
    let config = read_vm_config(vm_name);

    // Extract VNC port: prefer config value, fall back to runtime port from console file.
    let vnc_port = if config.get("graphics").map(|v| v.as_str()) == Some("yes") {
        config
            .get("graphics_port")
            .and_then(|p| p.parse::<u16>().ok())
            .or_else(|| read_runtime_vnc_port(vm_name))
    } else {
        None
    };

    Ok(VmDetail {
        name: vm_name.to_string(),
        state,
        datastore,
        loader,
        uuid,
        cpu,
        memory,
        auto_start: false,
        memory_resident,
        console_port,
        networks,
        disks,
        snapshots,
        config,
        vnc_port,
    })
}

fn get_kv<'a>(sub: &'a [(String, String)], key: &str) -> Option<&'a str> {
    sub.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn get_kv_opt(sub: &[(String, String)], key: &str) -> Option<String> {
    get_kv(sub, key).map(|s| s.to_string())
}

fn vm_config_path(name: &str) -> Result<std::path::PathBuf, String> {
    for datastore in list_datastores()? {
        let path = std::path::Path::new(&datastore.path).join(name).join(format!("{name}.conf"));
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!("VM configuration not found: {name}"))
}

fn read_vm_config(name: &str) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    let path = match vm_config_path(name) {
        Ok(path) => path,
        Err(_) => return map,
    };
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return map,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(eq) = line.find('=') {
            let k = line[..eq].trim().to_string();
            let mut v = line[eq + 1..].trim().to_string();
            // Strip surrounding double-quotes.
            if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
                v = v[1..v.len() - 1].to_string();
            }
            map.insert(k, v);
        }
    }
    map
}

/// Serialize a full config map to the vm-bhyve .conf file format string.
fn serialize_config(config: &std::collections::BTreeMap<String, String>) -> String {
    let priority: &[&str] = &[
        "loader", "bhyveload_loader", "bhyveload_args", "loader_timeout",
        "grub_install0", "grub_run0",
        "cpu", "cpu_sockets", "cpu_cores", "cpu_threads",
        "memory", "wired_memory",
    ];
    let mut written = std::collections::HashSet::new();
    let mut content = String::new();

    for &key in priority {
        if let Some(value) = config.get(key) {
            let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
            content.push_str(key);
            content.push_str("=\"");
            content.push_str(&escaped);
            content.push_str("\"\n");
            written.insert(key);
        }
    }

    for (key, value) in config {
        if written.contains(key.as_str()) {
            continue;
        }
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        content.push_str(key);
        content.push_str("=\"");
        content.push_str(&escaped);
        content.push_str("\"\n");
    }
    content
}

/// Write the config map back to the .conf file atomically (with backup).
fn write_vm_config_file(name: &str, config: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
    let path = vm_config_path(name)?;
    let backup = path.with_extension("conf.fwp.bak");
    std::fs::copy(&path, &backup).map_err(|e| format!("backup VM configuration failed: {e}"))?;

    let content = serialize_config(config);
    let tmp = path.with_extension("conf.fwp.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write VM configuration failed: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("replace VM configuration failed: {e}"))?;
    Ok(())
}

pub fn update_vm_config(
    name: &str,
    new_config: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    // Merge: start from the existing config, then overlay the submitted values.
    // Empty string values mean "delete this key".
    let mut config = read_vm_config(name);
    for (key, value) in new_config {
        if value.is_empty() {
            config.remove(key);
        } else {
            config.insert(key.clone(), value.clone());
        }
    }
    write_vm_config_file(name, &config)
}

/// Remove a device's configuration keys (disk{index}_* or network{index}_*) from the .conf file.
/// Does NOT delete any physical resources.
pub fn delete_device(name: &str, prefix: &str, index: u32) -> Result<(), String> {
    let mut config = read_vm_config(name);
    let dev_prefix = format!("{prefix}{index}_");
    let keys_to_remove: Vec<String> = config
        .keys()
        .filter(|k| k.starts_with(&dev_prefix))
        .cloned()
        .collect();
    if keys_to_remove.is_empty() {
        return Ok(());
    }
    for key in &keys_to_remove {
        config.remove(key);
    }
    write_vm_config_file(name, &config)
}

/// Read the runtime VNC port from the VM's `console` file.
/// vm-bhyve writes `vnc=<listen>:<port>` to this file when the VM starts.
fn read_runtime_vnc_port(name: &str) -> Option<u16> {
    let config_path = vm_config_path(name).ok()?;
    let console_path = config_path.parent()?.join("console");
    let content = std::fs::read_to_string(&console_path).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("vnc=") {
            // Format: <listen>:<port>
            if let Some(port_str) = rest.rsplit(':').next() {
                if let Ok(port) = port_str.parse::<u16>() {
                    return Some(port);
                }
            }
        }
    }
    None
}

/// Read the VNC port for a VM.
/// If `graphics_port` is explicitly set in config, use that.
/// Otherwise, if graphics is enabled, try the runtime port from the console file
/// (vm-bhyve auto-allocates a port starting at 5900).
pub fn get_vnc_port(name: &str) -> Option<u16> {
    let config = read_vm_config(name);
    if config.get("graphics").map(|v| v.as_str()) != Some("yes") {
        return None;
    }
    if let Some(port) = config.get("graphics_port").and_then(|p| p.parse().ok()) {
        return Some(port);
    }
    read_runtime_vnc_port(name)
}

// ── Disk resources ────────────────────────────────────────────────

/// Supported file extensions for `file`-type disk images.
const DISK_FILE_EXTENSIONS: &[&str] = &["img", "iso", "raw", "qcow2", "vmdk", "vhd"];

/// Disk resources available for a VM: files in the VM directory and ZVOLs under the VM's dataset.
#[derive(Debug, Clone, Serialize)]
pub struct DiskResources {
    /// Filenames in the VM directory with supported extensions (e.g. disk0.img).
    pub files: Vec<String>,
    /// ZVOL names under the VM's dataset (relative names, e.g. disk1).
    pub zvols: Vec<String>,
}

/// List disk resources for a VM.
/// - Files: scans `<datastore.path>/<vm_name>/` for files with known disk extensions.
/// - ZVOLs: runs `zfs list -H -o name -t volume -r <dataset>/<vm_name>` to find ZVOLs under the VM's dataset.
pub fn list_disk_resources(name: &str) -> Result<DiskResources, String> {
    let config_path = vm_config_path(name)?;
    let vm_dir = config_path
        .parent()
        .ok_or("cannot resolve VM directory")?;

    // ── files ──
    let mut files = Vec::new();
    if let Ok(dir) = std::fs::read_dir(vm_dir) {
        for entry in dir.flatten() {
            let entry_path = entry.path();
            if !entry_path.is_file() {
                continue;
            }
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with('.') {
                continue;
            }
            if let Some(ext) = entry_path.extension().and_then(|e| e.to_str()) {
                if DISK_FILE_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
                    files.push(fname);
                }
            }
        }
    }
    files.sort();

    // ── zvols ──
    let mut zvols = Vec::new();
    // Find the datastore that contains this VM to get the ZFS dataset.
    let vm_dir_str = vm_dir.to_string_lossy().to_string();
    for datastore in list_datastores()? {
        if vm_dir_str.starts_with(&datastore.path) {
            if let Some(dataset) = &datastore.zfs_dataset {
                let child_dataset = format!("{dataset}/{name}");
                let output = Command::new(ZFS)
                    .args(["list", "-H", "-o", "name", "-t", "volume", "-r", &child_dataset])
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .output()
                    .map_err(|e| format!("zfs list failed: {e}"))?;
                if output.status.success() {
                    let prefix = format!("{child_dataset}/");
                    for line in String::from_utf8_lossy(&output.stdout).lines() {
                        let line = line.trim();
                        if let Some(rel) = line.strip_prefix(&prefix) {
                            zvols.push(rel.to_string());
                        }
                    }
                }
            }
            break;
        }
    }
    zvols.sort();

    Ok(DiskResources { files, zvols })
}

// ── vm-bhyve initialization ───────────────────────────────────────

/// Current vm-bhyve setup status, used to determine whether initialization
/// is needed before the VM management UI can be used.
#[derive(Debug, Clone, Serialize)]
pub struct BhyveStatus {
    /// `vm-bhyve` package installed (`/usr/local/sbin/vm` exists).
    pub installed: bool,
    /// `vm_enable="YES"` in rc.conf.
    pub enabled: bool,
    /// `vm_dir` value from rc.conf (e.g. `zfs:zroot/vm` or `/home/vm`).
    pub vm_dir: Option<String>,
    /// `vm init` has been run — the resolved path has `.config/` subdirectory.
    pub initialized: bool,
    /// Human-readable resolved filesystem path (e.g. `/vm`), for display.
    pub resolved_path: Option<String>,
}

/// Check whether vm-bhyve is installed, enabled, and initialized.
pub fn check_status() -> BhyveStatus {
    let installed = std::path::Path::new(VM).exists();
    let vm_enable = sysrc_get("vm_enable");
    let enabled = vm_enable.as_deref() == Some("YES");
    let vm_dir = sysrc_get("vm_dir");
    let resolved_path = vm_dir.as_deref().and_then(resolve_vm_dir);
    let initialized = resolved_path
        .as_ref()
        .map(|p| std::path::Path::new(p).join(".config").exists())
        .unwrap_or(false);

    BhyveStatus {
        installed,
        enabled,
        vm_dir,
        initialized,
        resolved_path,
    }
}

/// Perform full vm-bhyve initialization:
/// 1. Install packages (vm-bhyve, bhyve-firmware, grub2-bhyve)
/// 2. Set `vm_enable="YES"` and `vm_dir` in rc.conf
/// 3. Prepare storage (create ZFS dataset or directory)
/// 4. Run `vm init` (loads kernel modules, creates subdirectories)
/// 5. Copy example templates into `.templates/`
///
/// `spec` is the vm_dir value: `/path/to/dir` or `zfs:pool/dataset`.
/// Returns step descriptions for progress display.
pub fn init_bhyve(spec: &str) -> Result<Vec<String>, String> {
    let mut steps = Vec::new();

    // 1. Install packages
    let pkg_result = Command::new(PKG)
        .args(["install", "-y", "vm-bhyve", "bhyve-firmware", "grub2-bhyve"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to run pkg install: {e}"))?;
    if !pkg_result.status.success() {
        let stderr = String::from_utf8_lossy(&pkg_result.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&pkg_result.stdout).trim().to_string();
        return Err(if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            "package installation failed".to_string()
        });
    }
    steps.push("Installed packages: vm-bhyve, bhyve-firmware, grub2-bhyve".into());

    // 2. Configure rc.conf
    sysrc_set("vm_enable", "YES")?;
    sysrc_set("vm_dir", spec)?;
    steps.push(format!("rc.conf configured: vm_enable=YES, vm_dir={spec}"));

    // 3. Prepare storage
    if let Some(dataset) = spec.strip_prefix("zfs:") {
        let exists = Command::new(ZFS)
            .args(["list", "-H", "-o", "name", dataset])
            .stdin(Stdio::null())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !exists {
            Command::new(ZFS)
                .args(["create", dataset])
                .stdin(Stdio::null())
                .output()
                .map_err(|e| format!("zfs create {dataset} failed: {e}"))
                .and_then(|o| {
                    if o.status.success() {
                        Ok(())
                    } else {
                        Err(format!(
                            "zfs create {dataset} failed: {}",
                            String::from_utf8_lossy(&o.stderr).trim()
                        ))
                    }
                })?;
            steps.push(format!("ZFS dataset created: {dataset}"));
        } else {
            steps.push(format!("ZFS dataset already exists: {dataset}"));
        }
    } else {
        std::fs::create_dir_all(spec)
            .map_err(|e| format!("mkdir -p {spec} failed: {e}"))?;
        steps.push(format!("Directory ready: {spec}"));
    }

    // 4. Run vm init (loads kernel modules, creates .config/.templates/.iso/.img)
    let init_result = Command::new(VM)
        .arg("init")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("vm init failed: {e}"))?;
    if !init_result.status.success() {
        let stderr = String::from_utf8_lossy(&init_result.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&init_result.stdout).trim().to_string();
        return Err(format!(
            "vm init failed: {}",
            if !stderr.is_empty() { stderr } else { stdout }
        ));
    }
    steps.push("vm init completed (kernel modules loaded, directories created)".into());

    // 5. Copy example templates into .templates/
    let resolved = resolve_vm_dir(spec)
        .ok_or_else(|| "cannot resolve vm_dir filesystem path after init".to_string())?;
    let templates_dir = std::path::Path::new(&resolved).join(".templates");
    let examples_dir = "/usr/local/share/examples/vm-bhyve";

    if std::path::Path::new(examples_dir).exists() {
        let mut count = 0;
        for entry in std::fs::read_dir(examples_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.metadata().map(|m| m.is_file()).unwrap_or(false) {
                let dest = templates_dir.join(entry.file_name());
                std::fs::copy(entry.path(), &dest)
                    .map_err(|e| format!("copy template failed: {e}"))?;
                count += 1;
            }
        }
        steps.push(format!("Copied {count} template files to {resolved}/.templates/"));
    } else {
        steps.push("Warning: example templates directory not found".into());
    }

    Ok(steps)
}

// ── sysrc / zfs helpers ────────────────────────────────────────────

/// Read a single sysrc variable value (returns None if unset or error).
fn sysrc_get(key: &str) -> Option<String> {
    let output = Command::new(SYSRC)
        .args(["-n", key])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Set a sysrc variable (`sysrc KEY=VALUE`).
fn sysrc_set(key: &str, value: &str) -> Result<(), String> {
    let assignment = format!("{key}={value}");
    let output = Command::new(SYSRC)
        .arg(&assignment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("sysrc {key} failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("sysrc {key}={value} failed: {stderr}"));
    }
    Ok(())
}

/// Read the `vm_list` variable from rc.conf (space-separated VM names).
pub fn read_vm_list() -> Vec<String> {
    sysrc_get("vm_list")
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// Add or remove a VM from the `vm_list` rc.conf variable (auto-start on boot).
pub fn set_vm_auto_start(name: &str, enabled: bool) -> Result<(), String> {
    let mut list = read_vm_list();
    let was_in = list.iter().any(|n| n == name);
    if enabled && !was_in {
        list.push(name.to_string());
        sysrc_set("vm_list", &list.join(" "))?;
    } else if !enabled && was_in {
        list.retain(|n| n != name);
        let val = list.join(" ");
        if val.is_empty() {
            sysrc_set("vm_list", "")?;
        } else {
            sysrc_set("vm_list", &val)?;
        }
    }
    Ok(())
}

/// Resolve a vm_dir spec to a filesystem path.
/// For `zfs:pool/dataset`, queries the ZFS mountpoint.
/// For plain paths, returns as-is.
fn resolve_vm_dir(vm_dir: &str) -> Option<String> {
    if let Some(dataset) = vm_dir.strip_prefix("zfs:") {
        let output = Command::new(ZFS)
            .args(["get", "-H", "-o", "value", "mountpoint", dataset])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let mp = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if mp.is_empty() || mp == "-" {
            return None;
        }
        Some(mp)
    } else {
        Some(vm_dir.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_state() {
        assert_eq!(
            parse_state("Stopped"),
            ("stopped".into(), None, None)
        );
        assert_eq!(
            parse_state("Running (4272)"),
            ("running".into(), Some(4272), None)
        );
        assert_eq!(
            parse_state("Locked (ppbsd)"),
            ("locked".into(), None, Some("ppbsd".into()))
        );
    }

    #[test]
    fn test_parse_auto() {
        assert_eq!(parse_auto("No"), (false, None));
        assert_eq!(parse_auto("Yes"), (true, None));
        assert_eq!(parse_auto("Yes [1]"), (true, Some(1)));
        assert_eq!(parse_auto("Yes [3]"), (true, Some(3)));
    }

    #[test]
    fn test_parse_header_and_cols() {
        let header = "NAME      DATASTORE  LOADER     CPU  MEMORY  VNC           AUTO     STATE";
        let cols = parse_header(header);
        assert_eq!(col_index(&cols, "NAME"), Some(0));
        assert_eq!(col_index(&cols, "STATE"), Some(7));

        let line = "ubuntu    default    grub       4    4G      -             Yes [1]  Running (4272)";
        assert_eq!(col_value(line, &cols, 0), "ubuntu");
        let state_val = col_value(line, &cols, 7);
        assert_eq!(state_val, "Running (4272)");
    }

    #[test]
    fn test_list_vms_from_sample() {
        let sample = "\
NAME      DATASTORE  LOADER     CPU  MEMORY  VNC           AUTO     STATE
alpine    default    uefi       2    1G      -             No       Locked (ppbsd)
ubuntu    default    grub       4    4G      -             Yes [1]  Running (4272)
alpine2   default    uefi       2    512M    -             No       Stopped
";
        // Simulate what vm_run would return
        let mut lines = sample.lines();
        let header = lines.next().unwrap();
        let cols = parse_header(header);

        let i_state = col_index(&cols, "STATE").unwrap();
        let i_auto = col_index(&cols, "AUTO").unwrap();
        let i_cpu = col_index(&cols, "CPU").unwrap();

        let alpine_line = lines.next().unwrap();
        let (st, _, lb) = parse_state(&col_value(alpine_line, &cols, i_state));
        assert_eq!(st, "locked");
        assert_eq!(lb, Some("ppbsd".into()));

        let ubuntu_line = lines.next().unwrap();
        let (st, pid, _) = parse_state(&col_value(ubuntu_line, &cols, i_state));
        assert_eq!(st, "running");
        assert_eq!(pid, Some(4272));
        assert_eq!(col_value(ubuntu_line, &cols, i_cpu), "4");
        let (auto_start, auto_order) = parse_auto(&col_value(ubuntu_line, &cols, i_auto));
        assert_eq!((auto_start, auto_order), (true, Some(1)));

        let alpine2_line = lines.next().unwrap();
        let (st, _, _) = parse_state(&col_value(alpine2_line, &cols, i_state));
        assert_eq!(st, "stopped");
    }

    #[test]
    fn test_switch_list_parsing() {
        let sample = "\
NAME    TYPE      IFACE      ADDRESS  PRIVATE  MTU  VLAN  PORTS
public  standard  vm-public  -        no       -    -     bge1 epair0a
";
        let mut lines = sample.lines();
        let header = lines.next().unwrap();
        let cols = parse_header(header);

        let i_ports = col_index(&cols, "PORTS").unwrap();
        let line = lines.next().unwrap();
        let ports_raw = col_value(line, &cols, i_ports);
        let ports: Vec<String> = ports_raw.split_whitespace().map(String::from).collect();
        assert_eq!(ports, vec!["bge1", "epair0a"]);
    }
}
