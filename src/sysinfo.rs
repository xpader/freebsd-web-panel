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
const AF_INET6: libc::c_int = 28;
const IFF_UP: libc::c_uint = 0x1;
const IFF_RUNNING: libc::c_uint = 0x40;

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
    pub running: bool,
    pub status: String,
    pub media: String,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
}

/// Read per-interface traffic counters via `getifaddrs(3)`.
///
/// Walks the interface address list; for each `AF_LINK` entry the `ifa_data`
/// pointer references a `struct if_data` containing cumulative byte/packet
/// counters.  Only pure-noise pseudo interfaces are excluded via
/// `is_noise_iface` — everything else (hardware NICs, epairs in jails,
/// bridges, tunnels, VPNs) is reported, and sorted by activity in the
/// dashboard.
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
                if !is_noise_iface(&name) {
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

/// Read interface metadata (flags, MTU, MAC, IPv4, IPv6) via `getifaddrs(3)`.
///
/// A single `getifaddrs` call returns multiple entries per interface (one per
/// address family).  We accumulate data across entries: `AF_LINK` provides
/// flags/MTU/MAC, `AF_INET` provides IPv4 addresses, `AF_INET6` provides
/// global-scope IPv6 addresses.  Only pure-noise pseudo interfaces are
/// excluded.  Results are sorted by activity rank (UP + IP-bearing first)
/// so callers such as the dashboard can display the most relevant interface
/// at the top regardless of whether it's a hardware NIC or a jail epair.
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
            if is_noise_iface(&name) {
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
                    running: false,
                    status: String::new(),
                    media: String::new(),
                    ipv4: Vec::new(),
                    ipv6: Vec::new(),
                });
            }
            let iface = ifaces.get_mut(&name).unwrap();
            if family == AF_LINK && !entry.ifa_data.is_null() {
                let data = unsafe { &*(entry.ifa_data as *const libc::if_data) };
                iface.mtu = data.ifi_mtu;
                iface.up = entry.ifa_flags & IFF_UP != 0;
                iface.running = entry.ifa_flags & IFF_RUNNING != 0;
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
            } else if family == AF_INET6 {
                let sin6 = entry.ifa_addr as *const libc::sockaddr_in6;
                let bytes = unsafe { (*sin6).sin6_addr.s6_addr };
                // Skip non-global addresses (loopback ::1, link-local fe80::/10,
                // multicast ff00::/8) — they're noise on a dashboard that just
                // wants to show "what IPs does this box have".
                if bytes[0] & 0xe0 == 0x20 {
                    iface.ipv6.push(format_ipv6(&bytes));
                }
            }
        }
        cur = entry.ifa_next;
    }
    unsafe { freeifaddrs(head) };

    // Return in rank-descending order (most "active" first); alphabetical
    // within the same rank for stability.
    let mut list: Vec<NetIfaceInfo> = order
        .into_iter()
        .filter_map(|n| ifaces.remove(&n))
        .collect();
    list.sort_by(|a, b| {
        let ra = iface_rank(a);
        let rb = iface_rank(b);
        rb.cmp(&ra).then_with(|| a.name.cmp(&b.name))
    });
    list
}

/// Extract the interface name from a C string pointer.
fn iface_name(ptr: *const libc::c_char) -> String {
    unsafe {
        std::ffi::CStr::from_ptr(ptr)
            .to_string_lossy()
            .into_owned()
    }
}

/// Whether an interface name refers to a real (hardware or hypervisor-backed)
/// NIC, as opposed to a software pseudo interface.
///
/// Uses an allowlist of common FreeBSD NIC driver prefixes: bge, em, igb, ix,
/// ixl, ice, mlx, re, vtnet, vmx, hn (Hyper-V), axge, cdce, ue (USB Ethernet),
/// wlan, lagg, vlan, carp.  Used by the network handler to set the
/// `is_physical` flag on interface records; NOT used to filter what the
/// dashboard shows — see [`is_noise_iface`] for that.
pub fn is_hardware_iface(name: &str) -> bool {
    const DRIVERS: &[&str] = &[
        "bge", "em", "igb", "ix", "ixl", "ice",
        "mlx", "re", "vtnet", "vmx", "hn",
        "axge", "cdce", "ue",
        "wlan", "lagg", "vlan", "carp",
    ];
    DRIVERS.iter().any(|p| name.starts_with(p))
}

/// Whether an interface name is pure noise that should never appear in
/// dashboards or metric collections.
///
/// Keep this list tiny: only pseudo devices that never carry real user
/// traffic, under any deployment scenario (bare metal, VM, jail).  Anything
/// ambiguous (epair, bridge, tap, tun, wg, tailscale, vm-bhyve switches,
/// gif, gre, ng, stf, faith, vale, ...) is intentionally *not* filtered —
/// those interfaces can be the primary carrier in some environments (e.g.
/// `epair*b` inside a jail), and the dashboard ranks by activity so
/// idle ones naturally sink to the bottom instead of being hidden.
pub fn is_noise_iface(name: &str) -> bool {
    const NOISE: &[&str] = &[
        "lo",       // loopback — always present, never interesting
        "pflog",    // pf packet-logging pseudo dev
        "pfsync",   // pf state-sync pseudo dev
        "ipfw",     // ipfw pseudo dev
        "enc",      // IPsec encapsulation pseudo dev
        "disc",     // discard
        "edsc",     // Ethernet discard
    ];
    NOISE.iter().any(|p| name.starts_with(p))
}

/// Format an IPv6 address from a 16-byte `s6_addr`.
///
/// Produces the canonical 8-group colon-separated form.  We don't collapse
/// runs of zeros here — the frontend and JSON consumers are expected to
/// render as-is, and the full form is unambiguous.
fn format_ipv6(bytes: &[u8; 16]) -> String {
    format!(
        "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
        u16::from_be_bytes([bytes[0], bytes[1]]),
        u16::from_be_bytes([bytes[2], bytes[3]]),
        u16::from_be_bytes([bytes[4], bytes[5]]),
        u16::from_be_bytes([bytes[6], bytes[7]]),
        u16::from_be_bytes([bytes[8], bytes[9]]),
        u16::from_be_bytes([bytes[10], bytes[11]]),
        u16::from_be_bytes([bytes[12], bytes[13]]),
        u16::from_be_bytes([bytes[14], bytes[15]]),
    )
}

/// Ranking weight for dashboard ordering.
///
/// Higher = more likely to be the user's "main" interface.  An interface
/// that's UP and has both IPv4 and global IPv6 beats one that's UP with
/// only IPv4, which beats an UP-but-addressless bridge, which beats a
/// DOWN interface.  Used by `read_net_info` (sort order) and by
/// `handlers::system::collect_network` (dashboard snapshot order).
pub fn iface_rank(info: &NetIfaceInfo) -> u32 {
    let mut r = 0u32;
    if info.running { r += 4; }
    else if info.up { r += 2; }
    if !info.ipv4.is_empty() { r += 2; }
    if !info.ipv6.is_empty() { r += 1; }
    r
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
    fn net_counters_exclude_noise() {
        let c = read_net_counters();
        for name in c.keys() {
            assert!(!is_noise_iface(name), "noise interface should be excluded: {name}");
        }
    }

    #[test]
    fn net_info_excludes_noise() {
        let infos = read_net_info();
        for i in &infos {
            assert!(!is_noise_iface(&i.name), "noise interface should be excluded: {}", i.name);
        }
    }

    #[test]
    fn net_info_sorted_by_rank() {
        // Results must be non-increasing in rank; equal ranks alphabetical.
        let infos = read_net_info();
        for w in infos.windows(2) {
            let ra = iface_rank(&w[0]);
            let rb = iface_rank(&w[1]);
            assert!(
                ra > rb || (ra == rb && w[0].name <= w[1].name),
                "read_net_info should return rank-desc then name-asc: {:?} vs {:?}",
                w[0].name, w[1].name
            );
        }
    }

    #[test]
    fn net_counters_nonzero_on_active_link() {
        // At least one interface should have received real traffic.
        let c = read_net_counters();
        let total_rx: u64 = c.values().map(|v| v.rx_bytes).sum();
        assert!(total_rx > 0, "expected non-zero RX on visible interfaces, got {c:?}");
    }

    #[test]
    fn noise_and_hardware_are_disjoint() {
        // The two helpers must agree on nothing — a name can't be both.
        let samples = ["lo0", "pflog0", "epair0b", "bridge0", "bge0", "em1",
                       "vtnet0", "tap0", "tun1", "wg0", "vm-public"];
        for s in samples {
            assert!(
                !(is_noise_iface(s) && is_hardware_iface(s)),
                "{s} should not be both noise and hardware"
            );
        }
        // Sanity on each side.
        assert!(is_noise_iface("lo0"));
        assert!(is_noise_iface("pflog0"));
        assert!(!is_noise_iface("epair0b"));
        assert!(!is_noise_iface("bridge0"));
        assert!(!is_noise_iface("vm-public"));
        assert!(is_hardware_iface("bge0"));
        assert!(is_hardware_iface("vtnet0"));
        assert!(!is_hardware_iface("epair0b"));
        assert!(!is_hardware_iface("bridge0"));
        assert!(!is_hardware_iface("vm-public"));
    }

    #[test]
    fn ipv6_format_is_canonical() {
        // 2001:db8::1
        let mut bytes = [0u8; 16];
        bytes[0] = 0x20; bytes[1] = 0x01; bytes[2] = 0x0d; bytes[3] = 0xb8;
        bytes[15] = 0x01;
        assert_eq!(format_ipv6(&bytes), "2001:db8:0:0:0:0:0:1");
        // All zeros → :: in collapsed form; we emit the long form.
        let z = [0u8; 16];
        assert_eq!(format_ipv6(&z), "0:0:0:0:0:0:0:0");
    }

    #[test]
    fn iface_rank_orders_expectedly() {
        let mk = |up: bool, running: bool, ipv4: bool, ipv6: bool| NetIfaceInfo {
            name: "x".into(),
            mtu: 1500,
            mac: None,
            up,
            running,
            status: String::new(),
            media: String::new(),
            ipv4: if ipv4 { vec!["1.2.3.4".into()] } else { vec![] },
            ipv6: if ipv6 { vec!["2001:db8::1".into()] } else { vec![] },
        };
        let up46 = mk(true, true, true, true);
        let up4  = mk(true, true, true, false);
        let up0  = mk(true, true, false, false);
        let down = mk(false, false, false, false);
        assert!(iface_rank(&up46) > iface_rank(&up4));
        assert!(iface_rank(&up4) > iface_rank(&up0));
        assert!(iface_rank(&up0) > iface_rank(&down));
    }
}

