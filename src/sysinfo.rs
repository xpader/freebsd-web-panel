//! System metric readers via sysctl(3) and libc — no subprocess spawning.
//!
//! Centralized so that `monitor.rs` (background collector) and
//! `handlers/system.rs` (live endpoints) share the same readers and do not
//! duplicate sysctl parsing logic or spawn `/sbin/sysctl` on every call.

use std::collections::HashMap;
use libc::{c_int, sockaddr, sockaddr_dl, sockaddr_in};
use sysctl::{Ctl, CtlValue, Sysctl};

/// Read a sysctl node as a string (mirrors `sysctl -n <name>`).
pub fn read_string(name: &str) -> Option<String> {
    Ctl::new(name).ok()?.value_string().ok()
}

/// Read the full OS version (including patch level, e.g. `15.1-RELEASE-p1`)
/// via `freebsd-version -k`. Falls back to `kern.osrelease` (without patch
/// level) if the command is unavailable.
pub fn read_os_version() -> String {
    std::process::Command::new("/bin/freebsd-version")
        .arg("-k")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| read_string("kern.osrelease"))
        .unwrap_or_default()
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

/// FFI declarations for `getifaddrs(3)` — the Rust `libc` crate doesn't expose
/// these on FreeBSD, so we declare them ourselves.  This is the same syscall
/// `netstat` and `ifconfig` use internally; calling it directly avoids
/// spawning subprocesses on every poll.
#[repr(C)]
struct Ifaddrs {
    ifa_next: *mut Ifaddrs,
    ifa_name: *mut libc::c_char,
    ifa_flags: libc::c_uint,
    ifa_addr: *mut libc::sockaddr,
    ifa_netmask: *mut libc::sockaddr,
    ifa_dstaddr: *mut libc::sockaddr,
    ifa_data: *mut libc::c_void,
}

extern "C" {
    fn getifaddrs(ifap: *mut *mut Ifaddrs) -> libc::c_int;
    fn freeifaddrs(ifa: *mut Ifaddrs);
}

const AF_LINK: libc::c_int = 18;
const AF_INET: libc::c_int = 2;
const IFF_UP: libc::c_uint = 0x1;

/// Per-interface traffic counters (cumulative since boot).
#[derive(Debug, Clone, Default)]
pub struct NetCounters {
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
}

/// Interface metadata.
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

/// Read per-interface traffic counters via `getifaddrs(3)`.
///
/// Walks the interface address list; for each `AF_LINK` entry the `ifa_data`
/// pointer references a `struct if_data` containing cumulative byte/packet
/// counters.  Virtual/pseudo interfaces are excluded via `is_physical_iface`.
pub fn read_net_counters() -> HashMap<String, NetCounters> {
    let mut map = HashMap::new();
    let mut head: *mut Ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs allocates a linked list; we free it via freeifaddrs.
    if unsafe { getifaddrs(&mut head) } != 0 {
        return map;
    }
    let mut cur = head;
    while !cur.is_null() {
        let entry = unsafe { &*cur };
        if !entry.ifa_addr.is_null() {
            let family = unsafe { (*entry.ifa_addr).sa_family as libc::c_int };
            if family == AF_LINK && !entry.ifa_data.is_null() {
                let name = iface_name(entry.ifa_name);
                if is_physical_iface(&name) {
                    let data = unsafe { &*(entry.ifa_data as *const libc::if_data) };
                    map.insert(name, NetCounters {
                        rx_bytes: data.ifi_ibytes,
                        tx_bytes: data.ifi_obytes,
                        rx_packets: data.ifi_ipackets,
                        tx_packets: data.ifi_opackets,
                    });
                }
            }
        }
        cur = entry.ifa_next;
    }
    unsafe { freeifaddrs(head) };
    map
}

/// Read interface metadata (flags, MTU, MAC, IPv4) via `getifaddrs(3)`.
///
/// A single `getifaddrs` call returns multiple entries per interface (one per
/// address family).  We accumulate data across entries: `AF_LINK` provides
/// flags/MTU/MAC, `AF_INET` provides IPv4 addresses.  Virtual/pseudo
/// interfaces are excluded.
pub fn read_net_info() -> Vec<NetIfaceInfo> {
    let mut ifaces: HashMap<String, NetIfaceInfo> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    let mut head: *mut Ifaddrs = std::ptr::null_mut();
    if unsafe { getifaddrs(&mut head) } != 0 {
        return Vec::new();
    }
    let mut cur = head;
    while !cur.is_null() {
        let entry = unsafe { &*cur };
        if !entry.ifa_addr.is_null() {
            let family = unsafe { (*entry.ifa_addr).sa_family as libc::c_int };
            let name = iface_name(entry.ifa_name);
            if !is_physical_iface(&name) {
                cur = entry.ifa_next;
                continue;
            }
            if !ifaces.contains_key(&name) {
                order.push(name.clone());
                ifaces.insert(name.clone(), NetIfaceInfo {
                    name: name.clone(),
                    mtu: 0,
                    mac: None,
                    up: false,
                    status: String::new(),
                    media: String::new(),
                    ipv4: Vec::new(),
                });
            }
            let iface = ifaces.get_mut(&name).unwrap();
            if family == AF_LINK && !entry.ifa_data.is_null() {
                let data = unsafe { &*(entry.ifa_data as *const libc::if_data) };
                iface.mtu = data.ifi_mtu;
                iface.up = entry.ifa_flags & IFF_UP != 0;
                // Extract MAC from sockaddr_dl.
                let sdl = entry.ifa_addr as *const libc::sockaddr_dl;
                let nlen = unsafe { (*sdl).sdl_nlen } as usize;
                let alen = unsafe { (*sdl).sdl_alen } as usize;
                if alen == 6 && nlen + alen <= unsafe { (*sdl).sdl_data }.len() {
                    let bytes = unsafe { &(*sdl).sdl_data };
                    iface.mac = Some(format!(
                        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                        bytes[nlen] as u8, bytes[nlen+1] as u8, bytes[nlen+2] as u8,
                        bytes[nlen+3] as u8, bytes[nlen+4] as u8, bytes[nlen+5] as u8,
                    ));
                }
            } else if family == AF_INET {
                let sin = entry.ifa_addr as *const libc::sockaddr_in;
                let addr = unsafe { (*sin).sin_addr };
                let ip = u32::from_be(addr.s_addr);
                iface.ipv4.push(format!("{}.{}.{}.{}", ip >> 24, (ip >> 16) & 0xff, (ip >> 8) & 0xff, ip & 0xff));
            }
        }
        cur = entry.ifa_next;
    }
    unsafe { freeifaddrs(head) };

    // Return in the order interfaces were first seen.
    order.into_iter().filter_map(|n| ifaces.remove(&n)).collect()
}

/// Extract the interface name from a C string pointer.
fn iface_name(ptr: *const libc::c_char) -> String {
    unsafe {
        std::ffi::CStr::from_ptr(ptr)
            .to_string_lossy()
            .into_owned()
    }
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

