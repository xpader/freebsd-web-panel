//! System metric readers via sysctl(3) and libc — no subprocess spawning.
//!
//! Centralized so that `monitor.rs` (background collector) and
//! `handlers/system.rs` (live endpoints) share the same readers and do not
//! duplicate sysctl parsing logic or spawn `/sbin/sysctl` on every call.

use std::collections::HashMap;
use sysctl::{Ctl, CtlValue, Sysctl};

/// Read a sysctl node as a string (mirrors `sysctl -n <name>`).
pub fn read_string(name: &str) -> Option<String> {
    Ctl::new(name).ok()?.value_string().ok()
}

/// Read a numeric sysctl node as `u64`. Handles all integer variants.
pub fn read_u64(name: &str) -> Option<u64> {
    let v = Ctl::new(name).ok()?.value().ok()?;
    match v {
        CtlValue::Int(x) => Some(x as u64),
        CtlValue::Uint(x) => Some(x as u64),
        CtlValue::Long(x) => Some(x as u64),
        CtlValue::Ulong(x) => Some(x),
        CtlValue::S64(x) => Some(x as u64),
        CtlValue::U64(x) => Some(x),
        CtlValue::S32(x) => Some(x as u64),
        CtlValue::U32(x) => Some(x as u64),
        CtlValue::S16(x) => Some(x as u64),
        CtlValue::U16(x) => Some(x as u64),
        CtlValue::S8(x) => Some(x as u64),
        CtlValue::U8(x) => Some(x as u64),
        _ => None,
    }
}

/// Convenience wrapper returning a numeric sysctl as `f64`.
pub fn read_f64(name: &str) -> Option<f64> {
    read_u64(name).map(|x| x as f64)
}

/// Read `kern.cp_times` — an array of `long` values (5 per core:
/// user, nice, system, interrupt, idle). On FreeBSD/amd64 `long` is 8 bytes.
/// Returns the values as `u64` (cumulative counters are non-negative).
///
/// Note: the `sysctl` crate misreports this array as a single `Long`, so the
/// raw buffer is read directly via `sysctlbyname(3)`.
pub fn read_cp_times() -> Vec<u64> {
    read_long_array("kern.cp_times")
}

/// Read a variable-length array sysctl (`S,LONG` format) into `u64` values
/// via the raw `sysctlbyname(3)` syscall.
fn read_long_array(name: &str) -> Vec<u64> {
    let cname = match std::ffi::CString::new(name) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut len: usize = 0;
    // First call: discover the buffer size.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; len];
    // Second call: fill the buffer.
    let rc = unsafe {
        libc::sysctlbyname(
            cname.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 {
        return Vec::new();
    }
    // `long` is 8 bytes on amd64.
    buf.chunks_exact(8)
        .map(|c| i64::from_ne_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) as u64)
        .collect()
}

/// Read `kern.boottime` (a `struct timeval`) and return `tv_sec` as Unix
/// timestamp. On FreeBSD/amd64 both `tv_sec` and `tv_usec` are 8 bytes.
pub fn boot_time() -> i64 {
    match Ctl::new("kern.boottime").and_then(|c| c.value()) {
        Ok(CtlValue::Struct(bytes)) if bytes.len() >= 8 => {
            i64::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]])
        }
        _ => 0,
    }
}

/// 1/5/15-minute load averages via `getloadavg(3)`.
pub fn read_loadavg() -> [f64; 3] {
    let mut la = [0.0_f64; 3];
    // SAFETY: getloadavg writes up to `nelem` doubles into the provided buffer.
    unsafe { libc::getloadavg(la.as_mut_ptr(), 3) };
    la
}

/// Read per-core temperatures from `dev.cpu.N.temperature` for cores `0..ncpu`.
/// Returns `(core_index, celsius)` pairs. Cores without a sensor are skipped.
pub fn read_core_temps(ncpu: u32) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    for i in 0..ncpu {
        let Ok(ctl) = Ctl::new(&format!("dev.cpu.{i}.temperature")) else {
            continue;
        };
        if let Ok(CtlValue::Temperature(t)) = ctl.value() {
            out.push((i as usize, t.celsius()));
        }
    }
    out
}

// ---- Network ----

/// Per-interface traffic counters (cumulative since boot).
#[derive(Debug, Clone, Default)]
pub struct NetCounters {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

/// Interface metadata from `ifconfig -a`.
#[derive(Debug, Clone)]
pub struct NetIfaceInfo {
    pub name: String,
    pub mtu: u32,
    pub mac: Option<String>,
    pub up: bool,
    pub status: String,
    pub media: String,
    pub ipv4: Vec<String>,
}

/// Read per-interface traffic counters from `netstat -ibn`.
///
/// Only `<Link#N>` rows are used — they carry raw byte/packet totals.
/// Virtual/pseudo interfaces (loopback, epair, bridge, tap, tunnel, etc.)
/// are excluded so that only physical NICs are returned.
///
/// Column indices are resolved from the header line (not hard-coded) so the
/// parser is robust against FreeBSD versions that add/remove columns such as
/// `Idrop`.
pub fn read_net_counters() -> HashMap<String, NetCounters> {
    let mut map = HashMap::new();
    let out = std::process::Command::new("/usr/bin/netstat")
        .args(["-i", "-b", "-n"])
        .output();
    let Ok(o) = out else {
        return map;
    };
    let stdout = String::from_utf8_lossy(&o.stdout);
    let mut lines = stdout.lines();

    // Parse header to find column positions by name.
    let headers: Vec<&str> = lines
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    let idx_of = |name: &str| headers.iter().position(|h| *h == name);
    let ibytes_i = idx_of("Ibytes");
    let obytes_i = idx_of("Obytes");
    let ipkts_i = idx_of("Ipkts");
    let opkts_i = idx_of("Opkts");

    for line in lines {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // Only <Link#> rows carry the raw byte counters.
        if cols.len() < 2 || !cols[2].starts_with("<Link#") {
            continue;
        }
        // Strip trailing '*' (marks inactive interfaces in netstat output).
        let name = cols[0].trim_end_matches('*').to_string();
        if !is_physical_iface(&name) {
            continue;
        }
        let get = |i: Option<usize>| -> u64 {
            i.and_then(|n| cols.get(n).and_then(|s| s.parse().ok()))
                .unwrap_or(0)
        };
        map.insert(
            name,
            NetCounters {
                rx_bytes: get(ibytes_i),
                tx_bytes: get(obytes_i),
                rx_packets: get(ipkts_i),
                tx_packets: get(opkts_i),
            },
        );
    }
    map
}

/// Read interface metadata (status, addresses, media) from `ifconfig -a`.
///
/// Virtual/pseudo interfaces are excluded (same denylist as
/// `read_net_counters`).  For each physical interface the link status
/// ("active" / "no carrier"), media description, MAC, MTU, and IPv4 addresses
/// are extracted.
pub fn read_net_info() -> Vec<NetIfaceInfo> {
    let mut out = Vec::new();
    let output = std::process::Command::new("/sbin/ifconfig")
        .arg("-a")
        .output();
    let Ok(o) = output else {
        return out;
    };
    let stdout = String::from_utf8_lossy(&o.stdout);

    let mut current: Option<NetIfaceInfo> = None;
    for line in stdout.lines() {
        // Interface definition lines start at column 0 (no leading whitespace).
        if !line.starts_with(|c: char| !c.is_whitespace()) {
            // Detail line — update the current interface.
            if let Some(ref mut iface) = current {
                let trimmed = line.trim();
                if trimmed.starts_with("inet ") {
                    if let Some(addr) = trimmed.split_whitespace().nth(1) {
                        iface.ipv4.push(addr.to_string());
                    }
                } else if trimmed.starts_with("ether ") {
                    iface.mac = trimmed.split_whitespace().nth(1).map(String::from);
                } else if trimmed.starts_with("media:") {
                    iface.media = trimmed
                        .strip_prefix("media:")
                        .unwrap_or(trimmed)
                        .trim()
                        .to_string();
                } else if trimmed.starts_with("status:") {
                    iface.status = trimmed
                        .strip_prefix("status:")
                        .unwrap_or(trimmed)
                        .trim()
                        .to_string();
                }
            }
            continue;
        }

        // Flush previous interface.
        if let Some(iface) = current.take() {
            if is_physical_iface(&iface.name) {
                out.push(iface);
            }
        }

        // Parse: "name: flags=..." → extract name, flags, mtu.
        let name = line.split(':').next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let up = line.contains("<UP");
        let mtu = line
            .split_whitespace()
            .find(|w| w.starts_with("mtu"))
            .and_then(|w| w[3..].parse().ok())
            .unwrap_or(0);
        current = Some(NetIfaceInfo {
            name,
            mtu,
            mac: None,
            up,
            status: String::new(),
            media: String::new(),
            ipv4: Vec::new(),
        });
    }
    // Flush last interface.
    if let Some(iface) = current.take() {
        if is_physical_iface(&iface.name) {
            out.push(iface);
        }
    }
    out
}

/// Whether an interface name refers to a physical NIC.
///
/// Uses a denylist of well-known virtual/pseudo-interface name prefixes used
/// by FreeBSD: loopback, jail epairs, bridges, taps, tunnels, VPN, netgraph,
/// vm-bhyve switches, etc.  Everything else (bge, em, igb, ix, ixl, vmx,
/// vtnet, re, wlan, vlan, ...) is treated as physical.
pub fn is_physical_iface(name: &str) -> bool {
    const VIRTUAL: &[&str] = &[
        "lo",         // loopback
        "epair",      // jail vnet pair
        "bridge",     // software bridge
        "tap",        // bhyve / qemu tap
        "vale",       // netmap vale (bhyve)
        "tun",        // tunnel
        "gif",        // GIF tunnel
        "gre",        // GRE tunnel
        "ipfw",       // ipfw pseudo-dev
        "pflog",      // pf logging
        "pfsync",     // pf state sync
        "enc",        // IPsec enc
        "stf",        // 6to4
        "faith",      // IPv6-to-IPv4 relay
        "ng",         // netgraph node
        "vm-",        // vm-bhyve switch bridge (hyphen avoids matching vmx)
        "tailscale",  // Tailscale VPN
        "wg",         // WireGuard VPN
        "disc",       // discard
        "edsc",       // Ethernet discard
    ];
    !VIRTUAL.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_and_u64_reads_work() {
        assert!(!read_string("kern.hostname").unwrap_or_default().is_empty());
        assert!(read_u64("hw.ncpu").unwrap_or(0) >= 1);
        assert!(read_u64("hw.physmem").unwrap_or(0) > 0);
    }

    #[test]
    fn cp_times_has_multiple_of_5() {
        let times = read_cp_times();
        assert!(times.len() >= 5);
        assert_eq!(times.len() % 5, 0, "cp_times must be 5 values per core");
    }

    #[test]
    fn boot_time_is_in_the_past() {
        let bt = boot_time();
        assert!(bt > 0, "boot time should be a valid epoch");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        assert!(bt < now, "boot time must be before now");
    }

    #[test]
    fn loadavg_is_plausible() {
        let la = read_loadavg();
        assert!(la[0] >= 0.0 && la[2] >= 0.0);
        assert!(la[0] < 1000.0, "1-min loadavg sanity bound");
    }

    #[test]
    fn temps_run_without_panic() {
        let ncpu = read_u64("hw.ncpu").unwrap_or(1) as u32;
        let _ = read_core_temps(ncpu);
    }

    #[test]
    fn net_counters_only_physical() {
        let c = read_net_counters();
        for name in c.keys() {
            assert!(is_physical_iface(name), "virtual interface should be excluded: {name}");
        }
    }

    #[test]
    fn net_info_only_physical() {
        let infos = read_net_info();
        for i in &infos {
            assert!(is_physical_iface(&i.name), "virtual interface should be excluded: {}", i.name);
        }
    }

    #[test]
    fn net_counters_nonzero_on_active_link() {
        // At least one physical interface should have received real traffic.
        let c = read_net_counters();
        let total_rx: u64 = c.values().map(|v| v.rx_bytes).sum();
        assert!(total_rx > 0, "expected non-zero RX on physical NICs, got {c:?}");
    }
}

