//! Network interface management — list interfaces, routes, and default gateway.
//!
//! Interface data is obtained via `getifaddrs(3)` (no subprocess).  The routing
//! table is obtained via `sysctl(NET_RT_DUMP)` (binary buffer, no subprocess).
//! Only `defaultrouter` from rc.conf uses `sysrc`.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::net::{Ipv4Addr, Ipv6Addr};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::cmd;
use crate::error::{ApiError, ApiResult};
use crate::AppState;

// ─── Constants not provided by the libc crate ──────────────────────────────

/// Route message type for route-table entries.
const RTM_GET: libc::c_int = 0x4;

/// Route address bitmask values — which sockaddrs follow `rt_msghdr`.
const RTA_DST: libc::c_int = 0x1;
const RTA_GATEWAY: libc::c_int = 0x2;
const RTA_NETMASK: libc::c_int = 0x4;
const RTA_GENMASK: libc::c_int = 0x8;
const RTA_IFP: libc::c_int = 0x10;
const RTA_IFA: libc::c_int = 0x20;
const RTA_AUTHOR: libc::c_int = 0x40;
const RTA_BRD: libc::c_int = 0x80;

/// Route flags (from `<net/route.h>`).
const RTF_UP: libc::c_int = 0x1;
const RTF_GATEWAY: libc::c_int = 0x2;
const RTF_HOST: libc::c_int = 0x4;
const RTF_REJECT: libc::c_int = 0x8;
const RTF_DYNAMIC: libc::c_int = 0x10;
const RTF_MODIFIED: libc::c_int = 0x20;
const RTF_CLONING: libc::c_int = 0x100;
const RTF_STATIC: libc::c_int = 0x800;
const RTF_BLACKHOLE: libc::c_int = 0x1000;

/// `struct rt_metrics` — matches FreeBSD `<net/route.h>`.
/// 14 × `u_long` (8 bytes each on amd64) = 112 bytes.
#[repr(C)]
struct RtMetrics {
    _locks: u64,
    _mtu: u64,
    _hopcount: u64,
    rmx_expire: i64,
    _recvpipe: u64,
    _sendpipe: u64,
    _ssthresh: u64,
    _rtt: u64,
    _rttvar: u64,
    _pksent: u64,
    _weight: u64,
    _nhidx: u64,
    _filler: [u64; 2],
}

/// `struct rt_msghdr` — FreeBSD/amd64 layout from `<net/route.h>`.
/// Header size = 40 + 112 = 152 bytes.
#[repr(C)]
struct RtMsghdr {
    rtm_msglen: u16,
    rtm_version: u8,
    rtm_type: u8,
    rtm_index: u16,
    _spare1: u16,
    rtm_flags: i32,
    rtm_addrs: i32,
    _pid: i32,
    _seq: i32,
    _errno: i32,
    _fmask: i32,
    _inits: u64,
    _rmx: RtMetrics,
}

// ─── Constants/structs for SIOCGIFGROUP (interface groups) ────────────────

// ─── Constants/structs for SIOCGIFDESCR (interface description) ───────────

/// `SIOCGIFDESCR` — get interface description. From `<sys/sockio.h>`.
/// `_IOWR('i', 42, struct ifreq)` = 0xc020692a (sizeof=32 on amd64).
const SIOCGIFDESCR: libc::c_ulong = 0xc020692a;

/// `struct ifreq` for SIOCGIFDESCR — name[16] + ifru_buffer{length, buffer}.
/// Matches the `ifru_buffer` member of `struct ifreq`.
#[repr(C)]
struct IfDescrReq {
    name: [libc::c_char; libc::IFNAMSIZ as usize],
    buf_length: usize,
    buf_ptr: *mut libc::c_void,
}

// ─── Constants/structs for SIOCGIFSTATUS (interface status text) ───────────

/// `SIOCGIFSTATUS` — get interface status text. From `<sys/sockio.h>`.
/// `_IOWR('i', 59, struct ifstat)` = 0xc331693b (sizeof=817 on amd64).
const SIOCGIFSTATUS: libc::c_ulong = 0xc331693b;

/// `IFSTATMAX` from `<net/if.h>`.
const IFSTATMAX: usize = 800;

/// `struct ifstat` — from `<net/if.h>`.
/// Layout: ifs_name[16] + ascii[801].
#[repr(C)]
struct IfStat {
    ifs_name: [libc::c_char; libc::IFNAMSIZ as usize],
    ascii: [libc::c_char; IFSTATMAX + 1],
}

// ─── Constants/structs for SIOCGDRVSPEC (bridge members) ──────────────────

/// `SIOCGDRVSPEC` — get driver-specific parameters. From `<sys/sockio.h>`.
/// `_IOWR('i', 123, struct ifdrv)` = 0xc028697b (sizeof=40 on amd64).
const SIOCGDRVSPEC: libc::c_ulong = 0xc028697b;

/// `BRDGGIFS` — get bridge member list. From `<net/if_bridgevar.h>`.
const BRDGGIFS: libc::c_ulong = 6;

/// `struct ifdrv` — from `<net/if.h>`. 40 bytes on amd64.
#[repr(C)]
struct IfDrv {
    ifd_name: [libc::c_char; libc::IFNAMSIZ as usize],
    ifd_cmd: libc::c_ulong,
    ifd_len: usize,
    ifd_data: *mut libc::c_void,
}

/// `struct ifbifconf` — bridge interface list. From `<net/if_bridgevar.h>`.
/// 16 bytes on amd64.
#[repr(C)]
struct IfBifConf {
    ifbic_len: u32,
    ifbic_buf: *mut libc::c_void,
}

/// `struct ifbreq` — bridge member request. From `<net/if_bridgevar.h>`.
/// 80 bytes on amd64.
#[repr(C)]
struct IfBreq {
    ifbr_ifsname: [libc::c_char; libc::IFNAMSIZ as usize],
    ifbr_ifsflags: u32,
    ifbr_stpflags: u32,
    ifbr_path_cost: u32,
    ifbr_portno: u8,
    ifbr_priority: u8,
    ifbr_proto: u8,
    ifbr_role: u8,
    ifbr_state: u8,
    _pad1: [u8; 3],
    ifbr_addrcnt: u32,
    ifbr_addrmax: u32,
    ifbr_addrexceeded: u32,
    ifbr_pvid: u16,
    ifbr_vlanproto: u16,
    _pad2: [u8; 28],
}

// ─── Public data structures ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BridgeMember {
    pub name: String,
    pub info: String,
}

#[derive(Debug, Serialize)]
pub struct IpConfig {
    pub address: String,
    pub netmask: Option<String>,
    pub prefix_len: Option<u8>,
    pub broadcast: Option<String>,
    pub is_alias: bool,
}

#[derive(Debug, Serialize)]
pub struct NetworkInterface {
    pub name: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub flags: Vec<String>,
    pub is_up: bool,
    pub is_loopback: bool,
    pub is_physical: bool,
    pub mtu: u32,
    pub metric: u32,
    pub mac: Option<String>,
    pub link_state: String,
    pub baudrate: u64,
    pub groups: Vec<String>,
    pub members: Vec<BridgeMember>,
    pub ipv4: Vec<IpConfig>,
    pub ipv6: Vec<IpConfig>,
    pub driver_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Route {
    pub family: String,
    pub destination: String,
    pub gateway: String,
    pub flags: String,
    pub interface: String,
    pub expire: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct DefaultGateway {
    pub gateway: Option<String>,
    pub interface: Option<String>,
    pub configured: Option<String>,
    pub gateway6: Option<String>,
    pub interface6: Option<String>,
    pub configured6: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DnsConfig {
    pub nameservers: Vec<String>,
    pub search: Vec<String>,
    pub domain: Option<String>,
    pub options: Vec<String>,
    pub sortlist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RcIpv4Alias {
    pub address: String,
    pub netmask: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RcIpv6Entry {
    pub address: String,
    pub prefixlen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct IfaceRcConfConfig {
    pub interface: String,
    pub is_bridge: bool,
    pub is_lagg: bool,
    pub is_up: bool,
    pub ipv4: Option<String>,
    pub ipv4_netmask: Option<String>,
    pub ipv4_aliases: Vec<RcIpv4Alias>,
    pub ipv6_mode: String,
    pub ipv6: Vec<RcIpv6Entry>,
    pub bridge_members: Vec<String>,
    pub lagg_proto: Option<String>,
    pub lagg_ports: Vec<String>,
    pub mtu: Option<u32>,
    pub description: Option<String>,
    pub options: String,
    /// Desired interface name (rename target). None/empty → keep driver name.
    /// Populated from `ifconfig_<driver>_name` or a stripped `name <X>` directive.
    pub name: Option<String>,
}

// ─── Interface reading via getifaddrs(3) ───────────────────────────────────

/// Read all network interfaces using `getifaddrs(3)`.
///
/// The returned linked list has one entry per address-family per interface;
/// this function aggregates them into a single [`NetworkInterface`] per name.
fn read_interfaces() -> std::io::Result<Vec<NetworkInterface>> {
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    let rc = unsafe { libc::getifaddrs(&mut ifap) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }

    // BTreeMap → deterministic alphabetical ordering.
    let mut map: BTreeMap<String, NetworkInterface> = BTreeMap::new();

    let mut cur = ifap;
    while !cur.is_null() {
        let ifa = unsafe { &*cur };

        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();

        let entry = map.entry(name.clone()).or_insert_with(|| NetworkInterface {
            name: name.clone(),
            description: None,
            status: None,
            flags: flags_to_strings(ifa.ifa_flags as libc::c_int),
            is_up: ifa.ifa_flags & (libc::IFF_UP as u32) != 0,
            is_loopback: ifa.ifa_flags & (libc::IFF_LOOPBACK as u32) != 0,
            is_physical: crate::sysinfo::is_hardware_iface(&name),
            mtu: 0,
            metric: 0,
            mac: None,
            link_state: String::from("unknown"),
            baudrate: 0,
            groups: Vec::new(),
            members: Vec::new(),
            ipv4: Vec::new(),
            ipv6: Vec::new(),
            driver_name: None,
        });

        if ifa.ifa_addr.is_null() {
            cur = ifa.ifa_next;
            continue;
        }

        let family = unsafe { (*ifa.ifa_addr).sa_family } as libc::c_int;

        match family {
            libc::AF_INET => {
                let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                let addr = Ipv4Addr::from(sin.sin_addr.s_addr.to_ne_bytes());

                let netmask = if !ifa.ifa_netmask.is_null() {
                    let nm = unsafe { &*(ifa.ifa_netmask as *const libc::sockaddr_in) };
                    Some(Ipv4Addr::from(nm.sin_addr.s_addr.to_ne_bytes()).to_string())
                } else {
                    None
                };

                let broadcast = if !ifa.ifa_dstaddr.is_null() {
                    let bc = unsafe { &*(ifa.ifa_dstaddr as *const libc::sockaddr_in) };
                    Some(Ipv4Addr::from(bc.sin_addr.s_addr.to_ne_bytes()).to_string())
                } else {
                    None
                };

                let is_alias = !entry.ipv4.is_empty();
                entry.ipv4.push(IpConfig {
                    address: addr.to_string(),
                    prefix_len: netmask.as_ref().map(|nm| ipv4_mask_to_prefix(nm)),
                    netmask,
                    broadcast,
                    is_alias,
                });
            }
            libc::AF_INET6 => {
                let sin6 = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
                let addr = Ipv6Addr::from(sin6.sin6_addr.s6_addr);

                let prefix_len = if !ifa.ifa_netmask.is_null() {
                    let nm6 = unsafe { &*(ifa.ifa_netmask as *const libc::sockaddr_in6) };
                    Some(ipv6_mask_to_prefix(&nm6.sin6_addr.s6_addr))
                } else {
                    None
                };

                let is_alias = !entry.ipv6.is_empty();
                entry.ipv6.push(IpConfig {
                    address: addr.to_string(),
                    netmask: None,
                    prefix_len,
                    broadcast: None,
                    is_alias,
                });
            }
            libc::AF_LINK => {
                let sdl = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_dl) };
                if sdl.sdl_alen > 0 {
                    entry.mac = extract_mac(sdl);
                }
                if !ifa.ifa_data.is_null() {
                    let ifd = unsafe { &*(ifa.ifa_data as *const libc::if_data) };
                    entry.mtu = ifd.ifi_mtu;
                    entry.metric = ifd.ifi_metric;
                    entry.baudrate = ifd.ifi_baudrate;
                    entry.link_state = match ifd.ifi_link_state {
                        0 => String::from("unknown"),
                        1 => String::from("down"),
                        2 => String::from("up"),
                        _ => String::from("unknown"),
                    };
                }
            }
            _ => {}
        }

        cur = ifa.ifa_next;
    }

    unsafe { libc::freeifaddrs(ifap) };

    // Populate groups, descriptions, status, and bridge members via ioctl.
    let fd = unsafe { libc::socket(libc::AF_LOCAL, libc::SOCK_DGRAM, 0) };
    if fd >= 0 {
        for iface in map.values_mut() {
            fill_iface_ioctl(fd, iface);
        }
        unsafe { libc::close(fd); };
    }

    // Filter out loopback interfaces.
    let result: Vec<NetworkInterface> = map
        .into_values()
        .filter(|iface| !iface.is_loopback)
        .collect();
    Ok(result)
}

/// Populate groups, description, status, and bridge members for a single
/// interface via ioctls on the same socket fd.
fn fill_iface_ioctl(fd: libc::c_int, iface: &mut NetworkInterface) {
    // ── Interface groups (via shared ifutil) ──
    for group in crate::ifutil::list_interface_groups(&iface.name) {
        if !group.is_empty() && group != "all" {
            iface.groups.push(group);
        }
    }

    let cname = match std::ffi::CString::new(iface.name.as_str()) {
        Ok(c) => c,
        Err(_) => return,
    };
    let name_bytes = cname.as_bytes_with_nul();
    if name_bytes.len() > libc::IFNAMSIZ as usize {
        return;
    }

    // ── SIOCGIFDESCR: interface description ──
    {
        const DESCR_SIZE: usize = 256;
        let mut buf = [0u8; DESCR_SIZE];
        let mut req = IfDescrReq {
            name: [0; libc::IFNAMSIZ as usize],
            buf_length: DESCR_SIZE,
            buf_ptr: buf.as_mut_ptr() as *mut libc::c_void,
        };
        for (i, &b) in name_bytes.iter().enumerate() {
            req.name[i] = b as libc::c_char;
        }
        let rc = unsafe { libc::ioctl(fd, SIOCGIFDESCR as libc::c_ulong, &mut req) };
        if rc == 0 {
            if let Some(len) = buf.iter().position(|&b| b == 0) {
                if len > 0 {
                    iface.description =
                        Some(String::from_utf8_lossy(&buf[..len]).into_owned());
                }
            }
        }
    }

    // ── SIOCGIFSTATUS: driver status text ──
    {
        let mut req = IfStat {
            ifs_name: [0; libc::IFNAMSIZ as usize],
            ascii: [0; IFSTATMAX + 1],
        };
        for (i, &b) in name_bytes.iter().enumerate() {
            req.ifs_name[i] = b as libc::c_char;
        }
        let rc = unsafe { libc::ioctl(fd, SIOCGIFSTATUS as libc::c_ulong, &mut req) };
        if rc == 0 {
            if let Some(len) = req.ascii.iter().position(|&c| c == 0) {
                if len > 0 {
                    let bytes: Vec<u8> = req.ascii[..len].iter().map(|&c| c as u8).collect();
                    let raw = String::from_utf8_lossy(&bytes);
                    let cleaned: Vec<&str> = raw
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .collect();
                    if !cleaned.is_empty() {
                        iface.status = Some(cleaned.join("\n"));
                    }
                }
            }
        }
    }

    // ── SIOCGDRVSPEC(BRDGGIFS): bridge members ──
    {
        const MAX_ENTRIES: usize = 256;
        let buf_size = MAX_ENTRIES * std::mem::size_of::<IfBreq>();
        let mut buf = vec![0u8; buf_size];
        let mut bifc = IfBifConf {
            ifbic_len: buf_size as u32,
            ifbic_buf: buf.as_mut_ptr() as *mut libc::c_void,
        };
        let mut ifd = IfDrv {
            ifd_name: [0; libc::IFNAMSIZ as usize],
            ifd_cmd: BRDGGIFS,
            ifd_len: std::mem::size_of::<IfBifConf>(),
            ifd_data: &mut bifc as *mut IfBifConf as *mut libc::c_void,
        };
        for (i, &b) in name_bytes.iter().enumerate() {
            ifd.ifd_name[i] = b as libc::c_char;
        }
        let rc = unsafe { libc::ioctl(fd, SIOCGDRVSPEC as libc::c_ulong, &mut ifd) };
        if rc == 0 {
            let entry_size = std::mem::size_of::<IfBreq>();
            let count = (bifc.ifbic_len as usize) / entry_size;
            for i in 0..count {
                let entry = unsafe { &*(buf.as_ptr().add(i * entry_size) as *const IfBreq) };
                let member_name = unsafe { CStr::from_ptr(entry.ifbr_ifsname.as_ptr()) }
                    .to_string_lossy()
                    .into_owned();
                if member_name.is_empty() {
                    continue;
                }
                let mut info = format!(
                    "port {} priority {} path cost {}",
                    entry.ifbr_portno, entry.ifbr_priority, entry.ifbr_path_cost
                );
                if let Some(proto) = decode_vlan_proto(entry.ifbr_vlanproto) {
                    info.push_str(&format!(" vlan protocol {proto}"));
                }
                iface.members.push(BridgeMember { name: member_name, info });
            }
        }
    }
}

/// Extract a MAC address string from a `sockaddr_dl`.
fn extract_mac(sdl: &libc::sockaddr_dl) -> Option<String> {
    let nlen = sdl.sdl_nlen as usize;
    let alen = sdl.sdl_alen as usize;
    if alen < 6 || nlen + 6 > sdl.sdl_data.len() {
        return None;
    }
    let mac: [u8; 6] = sdl.sdl_data[nlen..nlen + 6]
        .iter()
        .map(|&b| b as u8)
        .collect::<Vec<u8>>()
        .try_into()
        .ok()?;
    Some(format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    ))
}

/// Convert dotted-quad netmask to prefix length.
fn ipv4_mask_to_prefix(netmask: &str) -> u8 {
    netmask
        .parse::<Ipv4Addr>()
        .map(|a| u32::from(a).count_ones() as u8)
        .unwrap_or(0)
}

/// Count leading 1-bits in an IPv6 mask array.
fn ipv6_mask_to_prefix(mask: &[u8; 16]) -> u8 {
    let mut len = 0u8;
    for &byte in mask {
        if byte == 0xFF {
            len += 8;
        } else {
            len += byte.count_ones() as u8;
            break;
        }
    }
    len
}

/// Decode `IFF_*` bitmask to human-readable strings.
fn flags_to_strings(flags: libc::c_int) -> Vec<String> {
    let pairs: &[(libc::c_int, &str)] = &[
        (libc::IFF_UP, "UP"),
        (libc::IFF_BROADCAST, "BROADCAST"),
        (libc::IFF_DEBUG, "DEBUG"),
        (libc::IFF_LOOPBACK, "LOOPBACK"),
        (libc::IFF_POINTOPOINT, "POINTOPOINT"),
        (libc::IFF_RUNNING, "RUNNING"),
        (libc::IFF_NOARP, "NOARP"),
        (libc::IFF_PROMISC, "PROMISC"),
        (libc::IFF_SIMPLEX, "SIMPLEX"),
        (libc::IFF_MULTICAST, "MULTICAST"),
    ];
    pairs
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| (*name).to_string())
        .collect()
}

/// Decode VLAN protocol ethertype to a readable name.
fn decode_vlan_proto(proto: u16) -> Option<String> {
    match proto {
        0 => None,
        0x8100 => Some(String::from("802.1Q")),
        0x88a8 => Some(String::from("802.1ad")),
        _ => Some(format!("0x{proto:04x}")),
    }
}

// ─── Route table reading via sysctl(NET_RT_DUMP) ───────────────────────────

/// Read the full routing table via `sysctl(NET_RT_DUMP)`.
fn read_routes() -> std::io::Result<Vec<Route>> {
    let mib: [libc::c_int; 6] = [
        libc::CTL_NET,
        libc::PF_ROUTE,
        0, // protocol (always 0)
        0, // AF_UNSPEC — all address families
        libc::NET_RT_DUMP,
        0,
    ];

    // First call: discover buffer size.
    let mut needed: libc::size_t = 0;
    let rc = unsafe {
        libc::sysctl(
            mib.as_ptr(),
            6,
            std::ptr::null_mut(),
            &mut needed,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 || needed == 0 {
        return Ok(Vec::new());
    }

    let mut buf = vec![0u8; needed];
    let rc = unsafe {
        libc::sysctl(
            mib.as_ptr(),
            6,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut needed,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }

    let hdr_size = std::mem::size_of::<RtMsghdr>();
    let mut routes = Vec::new();
    let mut offset = 0usize;

    while offset + hdr_size <= buf.len() {
        let rtm: &RtMsghdr = unsafe { &*(buf.as_ptr().add(offset) as *const RtMsghdr) };

        let msg_len = rtm.rtm_msglen as usize;
        if msg_len < hdr_size || offset + msg_len > buf.len() {
            break;
        }

        if rtm.rtm_type as libc::c_int == RTM_GET {
            if let Some(route) = parse_route(rtm, &buf[offset..offset + msg_len], hdr_size) {
                routes.push(route);
            }
        }

        offset += msg_len;
    }

    Ok(routes)
}

/// Parse a single `RTM_GET` message into a [`Route`].
fn parse_route(rtm: &RtMsghdr, msg: &[u8], hdr_size: usize) -> Option<Route> {
    let addrs = rtm.rtm_addrs;
    let mut off = hdr_size;

    let mut destination = String::new();
    let mut gateway = String::new();
    let mut interface = String::new();
    let mut prefix_len: Option<u8> = None;
    let mut family = String::new();
    let mut expire: Option<i64> = None;

    let rta_slots: [libc::c_int; 8] = [
        RTA_DST, RTA_GATEWAY, RTA_NETMASK, RTA_GENMASK,
        RTA_IFP, RTA_IFA, RTA_AUTHOR, RTA_BRD,
    ];

    for &bit in &rta_slots {
        if addrs & bit == 0 {
            continue;
        }
        if off >= msg.len() {
            break;
        }

        let sa: *const libc::sockaddr =
            unsafe { msg.as_ptr().add(off) as *const libc::sockaddr };
        let raw = unsafe { &*sa };
        let sa_len = raw.sa_len as usize;
        let fam = raw.sa_family as libc::c_int;

        // Zero-length sockaddrs occupy sizeof(long)=8 bytes in the buffer
        // but carry no useful data.
        if sa_len == 0 {
            match bit {
                RTA_DST => destination = String::from("default"),
                RTA_GATEWAY => {} // directly connected, no gateway
                RTA_NETMASK => {}  // no netmask (host route or omitted)
                _ => {}
            }
            off += std::mem::size_of::<libc::c_long>();
            continue;
        }

        let advance = roundup(sa_len);

        match bit {
            RTA_DST => {
                destination = sockaddr_to_dest(sa, fam, rtm.rtm_index as u32);
                if family.is_empty() {
                    family = family_name(fam);
                }
            }
            RTA_GATEWAY => {
                gateway = sockaddr_to_gw(sa, fam, rtm.rtm_index as u32);
            }
            RTA_NETMASK => {
                prefix_len = sockaddr_to_prefix(sa, fam);
            }
            RTA_IFP if fam == libc::AF_LINK => {
                let sdl = unsafe { &*sa.cast::<libc::sockaddr_dl>() };
                interface = sdl_name(sdl);
            }
            _ => {}
        }

        off += advance;
    }

    // Append prefix length for non-default network routes.
    if let Some(plen) = prefix_len {
        if destination != "default" && plen > 0 && rtm.rtm_flags & RTF_HOST == 0 {
            destination = format!("{destination}/{plen}");
        }
    }

    // Determine family from flags if not set yet.
    if family.is_empty() {
        // Check if it's IPv6 by looking at the gateway or destination
        family = String::from("Internet");
    }

    // Expire: 0 = permanent, future timestamp = expires at that time.
    let raw_expire = rtm._rmx.rmx_expire;
    if raw_expire > 0 {
        expire = Some(raw_expire);
    }

    // Fallback: resolve interface name from index.
    if interface.is_empty() && rtm.rtm_index > 0 {
        interface = if_index_to_name(rtm.rtm_index as u32);
    }

    Some(Route {
        family,
        destination,
        gateway,
        flags: route_flags_to_string(rtm.rtm_flags),
        interface,
        expire,
    })
}

/// Round up a sockaddr length to `sizeof(long)` alignment (8 bytes on amd64).
fn roundup(len: usize) -> usize {
    let align = std::mem::size_of::<libc::c_long>();
    if len > 0 {
        1 + ((len - 1) | (align - 1))
    } else {
        align
    }
}

/// Format a sockaddr as a route destination.
fn sockaddr_to_dest(sa: *const libc::sockaddr, family: libc::c_int, _ifindex: u32) -> String {
    match family {
        libc::AF_INET => {
            let sin = unsafe { &*sa.cast::<libc::sockaddr_in>() };
            let addr = Ipv4Addr::from(sin.sin_addr.s_addr.to_ne_bytes());
            if addr.is_unspecified() {
                String::from("default")
            } else {
                addr.to_string()
            }
        }
        libc::AF_INET6 => {
            let sin6 = unsafe { &*sa.cast::<libc::sockaddr_in6>() };
            let addr = Ipv6Addr::from(sin6.sin6_addr.s6_addr);
            if addr.is_unspecified() {
                String::from("default")
            } else {
                addr.to_string()
            }
        }
        libc::AF_LINK => String::from("link"),
        _ => String::from("?"),
    }
}

/// Format a sockaddr as a route gateway.
/// AF_LINK gateways become `link#N` matching `netstat -rn` output.
fn sockaddr_to_gw(sa: *const libc::sockaddr, family: libc::c_int, ifindex: u32) -> String {
    match family {
        libc::AF_INET => {
            let sin = unsafe { &*sa.cast::<libc::sockaddr_in>() };
            Ipv4Addr::from(sin.sin_addr.s_addr.to_ne_bytes()).to_string()
        }
        libc::AF_INET6 => {
            let sin6 = unsafe { &*sa.cast::<libc::sockaddr_in6>() };
            Ipv6Addr::from(sin6.sin6_addr.s6_addr).to_string()
        }
        libc::AF_LINK => {
            format!("link#{ifindex}")
        }
        _ => String::from("?"),
    }
}

/// Human-readable address family name.
fn family_name(family: libc::c_int) -> String {
    match family {
        libc::AF_INET => String::from("Internet"),
        libc::AF_INET6 => String::from("Internet6"),
        libc::AF_LINK => String::from("Link"),
        _ => String::from("Other"),
    }
}

/// Extract prefix length from a netmask sockaddr.
fn sockaddr_to_prefix(sa: *const libc::sockaddr, family: libc::c_int) -> Option<u8> {
    match family {
        libc::AF_INET => {
            let sin = unsafe { &*sa.cast::<libc::sockaddr_in>() };
            let bits = u32::from_be_bytes(sin.sin_addr.s_addr.to_ne_bytes());
            Some(bits.count_ones() as u8)
        }
        libc::AF_INET6 => {
            let sin6 = unsafe { &*sa.cast::<libc::sockaddr_in6>() };
            Some(ipv6_mask_to_prefix(&sin6.sin6_addr.s6_addr))
        }
        _ => None,
    }
}

/// Extract the interface name from a `sockaddr_dl`.
fn sdl_name(sdl: &libc::sockaddr_dl) -> String {
    let nlen = sdl.sdl_nlen as usize;
    if nlen > 0 && nlen <= sdl.sdl_data.len() {
        let bytes: &[u8] =
            unsafe { std::slice::from_raw_parts(sdl.sdl_data.as_ptr() as *const u8, nlen) };
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        String::new()
    }
}

/// Convert interface index to name via `if_indextoname(3)`.
fn if_index_to_name(index: u32) -> String {
    let mut buf = [0i8; libc::IFNAMSIZ];
    let ptr = unsafe { libc::if_indextoname(index, buf.as_mut_ptr()) };
    if ptr.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

/// Decode `RTF_*` flags into the compact notation used by `netstat -rn`.
fn route_flags_to_string(flags: i32) -> String {
    let mut s = String::new();
    if flags & RTF_UP != 0 {
        s.push('U');
    }
    if flags & RTF_GATEWAY != 0 {
        s.push('G');
    }
    if flags & RTF_HOST != 0 {
        s.push('H');
    }
    if flags & RTF_REJECT != 0 {
        s.push('R');
    }
    if flags & RTF_DYNAMIC != 0 {
        s.push('D');
    }
    if flags & RTF_MODIFIED != 0 {
        s.push('M');
    }
    if flags & RTF_CLONING != 0 {
        s.push('C');
    }
    if flags & RTF_STATIC != 0 {
        s.push('S');
    }
    if flags & RTF_BLACKHOLE != 0 {
        s.push('B');
    }
    if s.is_empty() {
        s.push('?');
    }
    s
}

// ─── Handlers ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateIfaceBody {
    pub name: String,
}

/// POST `/api/network/interfaces` — create a virtual interface via `ifconfig <name> create`.
pub async fn interface_create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateIfaceBody>,
) -> ApiResult<StatusCode> {
    validate_iface_name(&body.name)?;

    // Reject names that look like physical interfaces.
    if crate::sysinfo::is_hardware_iface(&body.name) {
        return Err(ApiError::BadRequest(
            "interface name conflicts with a physical interface".into(),
        ));
    }

    // Create via ifconfig, apply any existing rc.conf config, persist to cloned_interfaces.
    let iface_name = body.name.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        cmd::run_sync(IFCONFIG, &[&iface_name, "create"])?;

        // Immediately apply any existing rc.conf config for this interface.
        // devd also does this asynchronously, but we do it synchronously so
        // the frontend's refresh sees the final state (e.g. a rename via
        // `name vvswitch`) without a timing race.
        let cfg = parse_merged_rcconf(&iface_name);
        // Freshly created interface: live state is empty, old config equals
        // the new one (nothing to reconcile off).
        let _ = apply_ifconfig(&iface_name, &cfg, &cfg);

        add_cloned_interface(&iface_name);
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/network/interfaces",
        201,
        Some(format!("created interface {}", body.name)),
    );

    Ok(StatusCode::CREATED)
}

/// GET `/api/network/interfaces` — list all network interfaces.
pub async fn list_interfaces() -> ApiResult<Json<Vec<NetworkInterface>>> {
    let interfaces = tokio::task::spawn_blocking(|| {
        let mut ifaces = read_interfaces().map_err(ApiError::Io)?;
        for iface in &mut ifaces {
            if let Some(drv) = crate::ifutil::get_drivername(&iface.name) {
                if drv != iface.name {
                    iface.driver_name = Some(drv);
                }
            }
        }
        Ok::<_, ApiError>(ifaces)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;
    Ok(Json(interfaces))
}

/// GET `/api/network/interfaces/{name}` — rc.conf config for a single interface.
pub async fn interface_detail(Path(name): Path<String>) -> ApiResult<Json<IfaceRcConfConfig>> {
    validate_iface_name(&name)?;
    let cfg = tokio::task::spawn_blocking(move || parse_merged_rcconf(&name))
        .await
        .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(cfg))
}

/// PUT `/api/network/interfaces/{name}` — save structured ifconfig config: apply to live system, then persist to rc.conf.
pub async fn interface_update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(name): Path<String>,
    Json(cfg): Json<IfaceRcConfConfig>,
) -> ApiResult<Json<IfaceRcConfConfig>> {
    validate_iface_name(&name)?;

    // Validate string fields — reject null bytes / newlines.
    let validate_str = |s: &str| -> ApiResult<()> {
        if s.contains('\0') || s.contains('\n') || s.contains('\r') {
            return Err(ApiError::BadRequest(
                "value must not contain newlines or null bytes".into(),
            ));
        }
        Ok(())
    };
    if let Some(ref ip) = cfg.ipv4 {
        validate_str(ip)?;
    }
    if let Some(ref nm) = cfg.ipv4_netmask {
        validate_str(nm)?;
    }
    for a in &cfg.ipv4_aliases {
        validate_str(&a.address)?;
        validate_str(&a.netmask)?;
    }
    for e in &cfg.ipv6 {
        validate_str(&e.address)?;
        validate_str(&e.prefixlen)?;
    }
    for m in &cfg.bridge_members {
        validate_str(m)?;
        validate_iface_name(m)?;
    }
    for p in &cfg.lagg_ports {
        validate_str(p)?;
        validate_iface_name(p)?;
    }
    if let Some(ref d) = cfg.description {
        validate_str(d)?;
    }
    if !cfg.options.is_empty() {
        validate_str(&cfg.options)?;
    }
    if let Some(ref p) = cfg.lagg_proto {
        validate_str(p)?;
    }

    // Normalize the rename target (cfg.name) and strip any stray `name <X>`
    // from options, which would re-introduce the rc.d rename-during-config bug.
    let mut save_cfg = cfg.clone();
    let (clean_opts, opt_name) = strip_name_directive(&save_cfg.options);
    save_cfg.options = clean_opts;
    // Name field takes priority; fall back to a `name <X>` directive stripped from options.
    if save_cfg.name.as_deref().map(str::trim).filter(|s| !s.is_empty()).is_none() {
        save_cfg.name = opt_name;
    }
    if let Some(dn) = save_cfg.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        validate_iface_name(dn)?;
    }

    // Resolve the driver name (current kernel driver name) and target live name.
    let driver = resolve_driver_name(&name);
    let target = resolve_target_name(save_cfg.name.as_deref(), &driver, &name, |candidate| {
        // Only consulted on the epair half-revert path; getifaddrs is pure
        // syscalls, cheap enough for at most two probes.
        read_interfaces()
            .map(|ifs| ifs.iter().any(|i| i.name == candidate))
            .unwrap_or(false)
    });

    let save_name = name.clone();
    let save_driver = driver.clone();
    let save_target = target.clone();

    let result = tokio::task::spawn_blocking(move || -> ApiResult<(IfaceRcConfConfig, Option<String>)> {
        // 0. Snapshot the OLD rc.conf config (before any live change): drives
        //    the removal reconciliation in apply_ifconfig. Must precede the
        //    rename — parse_merged_rcconf resolves keys via the live name.
        let old_cfg = parse_merged_rcconf(&save_name);

        // 1. Rename as an isolated first step so all later ifconfig calls operate
        //    on the final name (avoids the rename-during-config timing bug).
        if save_name != save_target {
            let ifaces = read_interfaces().map_err(ApiError::Io)?;
            if ifaces.iter().any(|i| i.name == save_target) {
                return Err(ApiError::Conflict(format!(
                    "interface name '{save_target}' already exists"
                )));
            }
            cmd::run_sync(IFCONFIG, &[&save_name, "name", &save_target])?;
        }

        // 2. Apply configuration under the (possibly new) live name.
        apply_ifconfig(&save_target, &old_cfg, &save_cfg).map_err(ApiError::Command)?;


        // 3. Persist config under the target live-name keys.
        let primary_key = format!("ifconfig_{save_target}");
        let aliases_key = format!("ifconfig_{save_target}_aliases");
        let ipv6_key = format!("ifconfig_{save_target}_ipv6");

        let primary_val = build_primary_value(&save_cfg);
        if primary_val.is_empty() {
            crate::sysrc::delete(&primary_key);
        } else {
            crate::sysrc::set(&primary_key, &primary_val).map_err(ApiError::Command)?;
        }
        let aliases_val = build_aliases_value(&save_cfg.ipv4_aliases);
        if aliases_val.is_empty() {
            crate::sysrc::delete(&aliases_key);
        } else {
            crate::sysrc::set(&aliases_key, &aliases_val).map_err(ApiError::Command)?;
        }
        let ipv6_val = build_ipv6_value(&save_cfg.ipv6_mode, &save_cfg.ipv6);
        if ipv6_val.is_empty() {
            crate::sysrc::delete(&ipv6_key);
        } else {
            crate::sysrc::set(&ipv6_key, &ipv6_val).map_err(ApiError::Command)?;
        }

        // 4. Rename directive: write ifconfig_<driver>_name only when the target
        //    is NOT a default name for the driver (covers renamed interfaces AND
        //    default-named epair like epair0a, whose live != driver base "epair0").
        let name_key = format!("ifconfig_{save_driver}_name");
        if !is_default_iface_name(&save_driver, &save_target) {
            crate::sysrc::set(&name_key, &save_target).map_err(ApiError::Command)?;
        } else {
            crate::sysrc::delete(&name_key);
        }

        // 5. Clean up stale keys (old live-name config + legacy driver-name config).
        //    Keep only the target-name keys just written and the _name directive.
        for n in [&save_name, &save_driver] {
            if n.is_empty() || n == &save_target {
                continue;
            }
            for key in [
                format!("ifconfig_{n}"),
                format!("ifconfig_{n}_aliases"),
                format!("ifconfig_{n}_ipv6"),
            ] {
                crate::sysrc::delete(&key);
            }
        }

        // 6. Restore default routes dropped by address changes (the kernel
        //    removes routes referencing a deleted address, mirroring netif →
        //    routing boot order). Best-effort: failure is reported, not fatal.
        let gw_note = restore_default_routes();

        // 7. Re-read merged config under the final live name.
        Ok((parse_merged_rcconf(&save_target), gw_note))
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;
    let (result, note) = result;
    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        &format!("/api/network/interfaces/{name}"),
        200,
        Some(format!(
            "updated ifconfig_{driver}{}",
            note.as_deref().map(|n| format!("; {n}")).unwrap_or_default()
        )),
    );

    Ok(Json(result))
}

#[derive(Debug, Serialize)]
pub struct ApplyResult {
    pub success: bool,
    pub output: String,
}

const IFCONFIG: &str = "/sbin/ifconfig";

/// Run `ifconfig <name> <args>` and collect stdout+stderr.
fn run_ifconfig(name: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd_args = vec![name];
    cmd_args.extend_from_slice(args);
    cmd::run_sync_str(IFCONFIG, &cmd_args)
}

/// Build ifconfig CLI args (excluding the interface name) from the primary
/// structured config. Only includes non-structural properties (IP, MTU,
/// description, media, UP). Bridge members and LAGG ports are handled
/// separately by `apply_ifconfig` to avoid duplicate-add errors.
fn build_ifconfig_args(cfg: &IfaceRcConfConfig) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // IPv4 (skip DHCP — applied by dhclient, not ifconfig)
    if let Some(ref ipv4) = cfg.ipv4 {
        let ip = ipv4.trim();
        if !ip.is_empty()
            && !ip.eq_ignore_ascii_case("DHCP")
            && !ip.eq_ignore_ascii_case("SYNCDHCP")
        {
            args.push("inet".into());
            args.push(ip.into());
            if let Some(ref nm) = cfg.ipv4_netmask {
                let nm = nm.trim();
                if !nm.is_empty() {
                    args.push("netmask".into());
                    args.push(nm.into());
                }
            }
        }
    }

    // MTU
    if let Some(mtu) = cfg.mtu {
        if mtu > 0 {
            args.push("mtu".into());
            args.push(mtu.to_string());
        }
    }

    // Description — always emit, even if empty (to clear existing description).
    if let Some(ref desc) = cfg.description {
        args.push("description".into());
        args.push(desc.trim().into());
    }

    // Extra options (media, mediaopt, vlan, etc.)
    for tok in cfg.options.split_whitespace() {
        args.push(tok.into());
    }

    // UP
    if cfg.is_up {
        args.push("up".into());
    }

    args
}
// ─── Config reconciliation helpers ─────────────────────────────────────────

/// Whether the interface's IPv4 is DHCP-managed (address owned by dhclient).
fn is_dhcp(cfg: &IfaceRcConfConfig) -> bool {
    cfg.ipv4
        .as_deref()
        .map(str::trim)
        .map(|s| s.eq_ignore_ascii_case("DHCP") || s.eq_ignore_ascii_case("SYNCDHCP"))
        .unwrap_or(false)
}

/// IPv4 addresses this config manages: the static primary plus aliases.
/// DHCP/SYNCDHCP primaries are excluded (not ours to manage).
fn managed_v4(cfg: &IfaceRcConfConfig) -> Vec<String> {
    let mut v = Vec::new();
    if !is_dhcp(cfg) {
        if let Some(ip) = cfg.ipv4.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            v.push(ip.to_string());
        }
    }
    for a in &cfg.ipv4_aliases {
        let addr = a.address.trim();
        if !addr.is_empty() {
            v.push(addr.to_string());
        }
    }
    v
}

/// IPv6 addresses this config manages: static-mode entries only.
/// SLAAC/auto addresses are kernel-managed; link-local (fe80::/10) never is.
fn managed_v6(cfg: &IfaceRcConfConfig) -> Vec<String> {
    if cfg.ipv6_mode != "static" {
        return Vec::new();
    }
    cfg.ipv6
        .iter()
        .map(|e| e.address.trim().to_string())
        .filter(|a| !a.is_empty() && !is_link_local_v6(a))
        .collect()
}

/// Whether an IPv6 address string (with optional `%zone`) is link-local.
fn is_link_local_v6(addr: &str) -> bool {
    addr.split('%')
        .next()
        .unwrap_or(addr)
        .parse::<Ipv6Addr>()
        .map(|ip| (ip.segments()[0] & 0xffc0) == 0xfe80)
        .unwrap_or(false)
}


/// Apply an interface's structured config to the live system via ifconfig.
/// Reads the current live state first and skips properties already in effect
/// (existing bridge members, lagg ports, IP aliases) to avoid duplicate-add errors.
fn apply_ifconfig(
    name: &str,
    old: &IfaceRcConfConfig,
    cfg: &IfaceRcConfConfig,
) -> Result<String, String> {
    let mut output = String::new();
    let mut errors: Vec<String> = Vec::new();

    // Read current live state.
    let live = read_interfaces()
        .map_err(|e| format!("failed to read live interfaces: {e}"))?
        .into_iter()
        .find(|i| i.name == name);

    let existing_members: Vec<String> = live
        .as_ref()
        .map(|i| i.members.iter().map(|m| m.name.clone()).collect())
        .unwrap_or_default();
    let existing_v4: Vec<(String, Option<u8>)> = live
        .as_ref()
        .map(|i| {
            i.ipv4
                .iter()
                .map(|ip| (ip.address.clone(), ip.prefix_len))
                .collect()
        })
        .unwrap_or_default();
    let existing_v6: Vec<String> = live
        .as_ref()
        .map(|i| i.ipv6.iter().map(|ip| ip.address.clone()).collect())
        .unwrap_or_default();

    // 1. Apply primary non-structural config (IP, MTU, description, media, UP).
    //    A plain `inet <addr>` replaces the current primary address, so primary
    //    address/netmask changes converge here.
    let primary_args = build_ifconfig_args(cfg);
    if !primary_args.is_empty() {
        let refs: Vec<&str> = primary_args.iter().map(|s| s.as_str()).collect();
        match run_ifconfig(name, &refs) {
            Ok(o) => output.push_str(&o),
            Err(e) => errors.push(format!("ifconfig {name} {}: {e}", primary_args.join(" "))),
        }
    }

    // 2. Apply LAGG protocol.
    if let Some(ref proto) = cfg.lagg_proto {
        let p = proto.trim();
        if !p.is_empty() {
            match run_ifconfig(name, &["laggproto", p]) {
                Ok(o) => output.push_str(&o),
                Err(e) => errors.push(format!("laggproto {p}: {e}")),
            }
        }
    }

    // 3. Add LAGG ports (skip existing).
    for port in &cfg.lagg_ports {
        let p = port.trim();
        if p.is_empty() || existing_members.iter().any(|m| m == p) {
            continue;
        }
        match run_ifconfig(name, &["laggport", p]) {
            Ok(o) => output.push_str(&o),
            Err(e) => errors.push(format!("laggport {p}: {e}")),
        }
    }

    // 4. Add bridge members (skip existing).
    for m in &cfg.bridge_members {
        let m = m.trim();
        if m.is_empty() || existing_members.iter().any(|em| em == m) {
            continue;
        }
        match run_ifconfig(name, &["addm", m]) {
            Ok(o) => output.push_str(&o),
            Err(e) => errors.push(format!("addm {m}: {e}")),
        }
    }

    // 5. Apply each IPv4 alias. An alias that exists with a different netmask
    //    than desired is deleted and re-added (plain `alias` never updates it).
    for alias in &cfg.ipv4_aliases {
        let addr = alias.address.trim();
        if addr.is_empty() {
            continue;
        }
        let nm = alias.netmask.trim();
        let want_prefix: Option<u8> = if nm.is_empty() {
            None
        } else {
            Some(ipv4_mask_to_prefix(nm))
        };
        if let Some((_, cur_prefix)) = existing_v4.iter().find(|(a, _)| a == addr) {
            match want_prefix {
                Some(wp) if cur_prefix == &Some(wp) => continue, // already exact
                Some(_) => {
                    // Netmask drift: remove, then fall through to re-add.
                    if let Err(e) = run_ifconfig(name, &["inet", addr, "delete"]) {
                        errors.push(format!("alias {addr} netmask change: {e}"));
                        continue;
                    }
                }
                None => continue, // netmask unspecified; leave as-is
            }
        }
        let alias_args: Vec<&str> = if nm.is_empty() {
            vec!["alias", addr]
        } else {
            vec!["alias", addr, "netmask", nm]
        };
        match run_ifconfig(name, &alias_args) {
            Ok(o) => output.push_str(&o),
            Err(e) => errors.push(format!("alias {addr}: {e}")),
        }
    }

    // 6. Apply each IPv6 entry (only in static mode).
    if cfg.ipv6_mode == "static" {
        for entry in &cfg.ipv6 {
            let addr = entry.address.trim();
            if addr.is_empty() || existing_v6.iter().any(|a| a == addr) {
                continue;
            }
            let pl = entry.prefixlen.trim();
            let v6_args: Vec<&str> = if pl.is_empty() {
                vec!["inet6", addr]
            } else {
                vec!["inet6", addr, "prefixlen", pl]
            };
            match run_ifconfig(name, &v6_args) {
                Ok(o) => output.push_str(&o),
                Err(e) => errors.push(format!("inet6 {addr}: {e}")),
            }
        }
    }

    // 7. Reconcile removals: delete addresses/members the OLD config managed
    //    but the new one no longer lists. This is old-config driven on purpose —
    //    deleting anything merely absent from the new config would also remove
    //    DHCP-assigned or out-of-band addresses we do not own. Addresses
    //    already gone from the live interface (e.g. a replaced primary) are
    //    skipped, keeping the pass idempotent.
    let live_now = read_interfaces()
        .map_err(|e| format!("failed to read live interfaces: {e}"))?
        .into_iter()
        .find(|i| i.name == name);
    let live_v4_now: Vec<String> = live_now
        .as_ref()
        .map(|i| i.ipv4.iter().map(|ip| ip.address.clone()).collect())
        .unwrap_or_default();
    let live_v6_now: Vec<String> = live_now
        .as_ref()
        .map(|i| i.ipv6.iter().map(|ip| ip.address.clone()).collect())
        .unwrap_or_default();

    // 7a. IPv4 removals (old primary + aliases minus new).
    let new_v4 = managed_v4(cfg);
    for addr in managed_v4(old) {
        if new_v4.iter().any(|a| a == &addr) || !live_v4_now.contains(&addr) {
            continue;
        }
        match run_ifconfig(name, &["inet", &addr, "delete"]) {
            Ok(o) => output.push_str(&o),
            Err(e) => errors.push(format!("delete inet {addr}: {e}")),
        }
    }

    // 7b. IPv6 removals (only entries the old config statically managed;
    //     static→auto transitions drop the old static addresses).
    let new_v6 = managed_v6(cfg);
    for addr in managed_v6(old) {
        if new_v6.iter().any(|a| a == &addr) || !live_v6_now.contains(&addr) {
            continue;
        }
        match run_ifconfig(name, &["inet6", &addr, "delete"]) {
            Ok(o) => output.push_str(&o),
            Err(e) => errors.push(format!("delete inet6 {addr}: {e}")),
        }
    }

    // 7c. Bridge members removed from the config.
    for m in &old.bridge_members {
        let m = m.trim();
        if m.is_empty()
            || cfg.bridge_members.iter().any(|n| n.trim() == m)
            || !existing_members.iter().any(|em| em == m)
        {
            continue;
        }
        match run_ifconfig(name, &["deletem", m]) {
            Ok(o) => output.push_str(&o),
            Err(e) => errors.push(format!("deletem {m}: {e}")),
        }
    }

    // 7d. LAGG ports removed from the config.
    for p in &old.lagg_ports {
        let p = p.trim();
        if p.is_empty()
            || cfg.lagg_ports.iter().any(|n| n.trim() == p)
            || !existing_members.iter().any(|em| em == p)
        {
            continue;
        }
        match run_ifconfig(name, &["-laggport", p]) {
            Ok(o) => output.push_str(&o),
            Err(e) => errors.push(format!("-laggport {p}: {e}")),
        }
    }

    if errors.is_empty() {
        Ok(output)
    } else {
        Err(errors.join("; "))
    }
}

/// Add a name to `cloned_interfaces` in rc.conf (idempotent).
/// For epair, the base name (e.g. "epair0" from "epair0a") is used.
fn add_cloned_interface(name: &str) {
    let clone_name = epair_base_name(name).unwrap_or_else(|| name.to_string());
    let _ = crate::sysrc::list_add("cloned_interfaces", &clone_name);
}

/// Remove a name from `cloned_interfaces` in rc.conf.
/// For epair, the base name is used.
fn remove_cloned_interface(name: &str) {
    let clone_name = epair_base_name(name).unwrap_or_else(|| name.to_string());
    let _ = crate::sysrc::list_remove("cloned_interfaces", &clone_name);
}

/// For epair interfaces like "epair0a" or "epair0b", return "epair0".
fn epair_base_name(name: &str) -> Option<String> {
    if !name.starts_with("epair") {
        return None;
    }
    let suffix = &name[5..];
    // Strip trailing 'a' or 'b'.
    let base = suffix.trim_end_matches(|c| c == 'a' || c == 'b');
    if base.is_empty() {
        return None;
    }
    Some(format!("epair{base}"))
}

/// POST `/api/network/interfaces/{name}/apply` — apply rc.conf config via ifconfig.
pub async fn interface_apply(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(name): Path<String>,
) -> ApiResult<Json<ApplyResult>> {
    validate_iface_name(&name)?;

    let apply_name = name.clone();
    let output = tokio::task::spawn_blocking(move || -> ApiResult<String> {
        let cfg = parse_merged_rcconf(&apply_name);
        let mut out = apply_ifconfig(&apply_name, &cfg, &cfg).map_err(ApiError::Command)?;
        // Re-apply ("apply" button) reconciles against the same config: only
        // netmask drifts get fixed; removals are empty (old == new).
        if let Some(note) = restore_default_routes() {
            out.push_str(&note);
        }
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        &format!("/api/network/interfaces/{name}/apply"),
        200,
        Some(format!("apply ifconfig_{name}: ok")),
    );

    Ok(Json(ApplyResult {
        success: true,
        output,
    }))
}

/// DELETE `/api/network/interfaces/{name}` — destroy a virtual interface via `ifconfig <name> destroy`.
/// Also removes any rc.conf entries for the interface.
pub async fn interface_destroy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_iface_name(&name)?;

    let destroy_name = name.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        // Don't allow destroying physical interfaces or loopback.
        let interfaces = read_interfaces().map_err(ApiError::Io)?;
        let iface = interfaces
            .iter()
            .find(|i| i.name == destroy_name)
            .ok_or_else(|| {
                ApiError::NotFound(format!("interface '{destroy_name}' not found"))
            })?;
        if iface.is_physical || iface.is_loopback {
            return Err(ApiError::BadRequest(
                "cannot destroy a physical or loopback interface".into(),
            ));
        }

        // Resolve driver name BEFORE destroying (get_drivername needs the interface alive).
        let driver = resolve_driver_name(&destroy_name);

        // Destroy via ifconfig.
        cmd::run_sync(IFCONFIG, &[&destroy_name, "destroy"])?;

        // Clean up rc.conf entries — use driver name, and also live name (split config).
        for key_name in [&driver, &destroy_name] {
            if key_name.is_empty() { continue; }
            for key in [
                &format!("ifconfig_{key_name}"),
                &format!("ifconfig_{key_name}_aliases"),
                &format!("ifconfig_{key_name}_ipv6"),
            ] {
                crate::sysrc::delete(key);
            }
        }
        // Also remove the rename directive (ifconfig_<driver>_name).
        crate::sysrc::delete(&format!("ifconfig_{driver}_name"));
        remove_cloned_interface(&driver);
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "DELETE",
        &format!("/api/network/interfaces/{name}"),
        200,
        Some(format!("destroyed interface {name}")),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// GET `/api/network/routes` — full routing table (IPv4 + IPv6).
pub async fn list_routes() -> ApiResult<Json<Vec<Route>>> {
    let routes = read_routes().map_err(ApiError::Io)?;
    Ok(Json(routes))
}

/// GET `/api/network/gateway` — default gateway (runtime + rc.conf value).
pub async fn default_gateway() -> ApiResult<Json<DefaultGateway>> {
    let routes = read_routes().map_err(ApiError::Io)?;

    let (gateway, interface) = routes
        .iter()
        .find(|r| r.destination == "default" && r.family == "Internet")
        .map(|r| (Some(r.gateway.clone()), Some(r.interface.clone())))
        .unwrap_or((None, None));

    let (gateway6, interface6) = routes
        .iter()
        .find(|r| r.destination == "default" && r.family == "Internet6")
        .map(|r| (Some(r.gateway.clone()), Some(r.interface.clone())))
        .unwrap_or((None, None));

    let (configured, configured6) = tokio::task::spawn_blocking(|| {
        (read_defaultrouter(), read_ipv6_defaultrouter())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;

    Ok(Json(DefaultGateway {
        gateway,
        interface,
        configured,
        gateway6,
        interface6,
        configured6,
    }))
}

#[derive(Debug, Deserialize)]
pub struct SetGatewayBody {
    pub gateway: Option<String>,
    pub gateway6: Option<String>,
}

/// PUT `/api/network/gateway` — set or clear IPv4/IPv6 default gateway.
///
/// Sets `defaultrouter` / `ipv6_defaultrouter` in rc.conf (persistent) and
/// applies the route change to the live system. An empty value clears the
/// configuration for that family. Each family is independent; only the
/// provided field(s) are updated.
pub async fn set_default_gateway(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetGatewayBody>,
) -> ApiResult<Json<DefaultGateway>> {
    if let Some(raw) = &body.gateway {
        let gw = raw.trim();
        if gw.is_empty() {
            // Apply first (live route), then the persistent config.
            tokio::task::spawn_blocking(|| delete_default_gateway(false))
                .await
                .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
                .map_err(ApiError::Command)?;
            crate::sysrc::delete_async("defaultrouter").await?;
            audit::record(
                &state, Some(&auth.username), "PUT", "/api/network/gateway", 200,
                Some("cleared IPv4 default gateway".into()),
            );
        } else {
            validate_ipv4(gw)?;
            let g = gw.to_string();
            tokio::task::spawn_blocking(move || apply_default_gateway(false, &g))
                .await
                .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
                .map_err(ApiError::Command)?;
            crate::sysrc::set_async("defaultrouter", gw).await?;
            audit::record(
                &state, Some(&auth.username), "PUT", "/api/network/gateway", 200,
                Some(format!("set IPv4 default gateway to {gw}")),
            );
        }
    }

    if let Some(raw) = &body.gateway6 {
        let gw = raw.trim();
        if gw.is_empty() {
            tokio::task::spawn_blocking(|| delete_default_gateway(true))
                .await
                .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
                .map_err(ApiError::Command)?;
            crate::sysrc::delete_async("ipv6_defaultrouter").await?;
            audit::record(
                &state, Some(&auth.username), "PUT", "/api/network/gateway", 200,
                Some("cleared IPv6 default gateway".into()),
            );
        } else {
            validate_ipv6(gw)?;
            let g = gw.to_string();
            tokio::task::spawn_blocking(move || apply_default_gateway(true, &g))
                .await
                .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?
                .map_err(ApiError::Command)?;
            crate::sysrc::set_async("ipv6_defaultrouter", gw).await?;
            audit::record(
                &state, Some(&auth.username), "PUT", "/api/network/gateway", 200,
                Some(format!("set IPv6 default gateway to {gw}")),
            );
        }
    }

    default_gateway().await
}

// ─── Default gateway helpers ───────────────────────────────────────────────

const ROUTE: &str = "/sbin/route";

/// Set the live default route to `gw`: `route change default <gw>`, falling
/// back to `route add` when no default route exists yet. The error carries
/// route(8) stderr so callers can surface the real cause (typically
/// "Network is unreachable" when the gateway is not on a connected subnet).
fn apply_default_gateway(is_v6: bool, gw: &str) -> Result<(), String> {
    let change_args: Vec<&str> = if is_v6 {
        vec!["-6", "change", "default", gw]
    } else {
        vec!["change", "default", gw]
    };
    if cmd::run_sync_str(ROUTE, &change_args).is_ok() {
        return Ok(());
    }
    let add_args: Vec<&str> = if is_v6 {
        vec!["-6", "add", "default", gw]
    } else {
        vec!["add", "default", gw]
    };
    cmd::run_sync_str(ROUTE, &add_args).map(|_| ())
}

/// Delete the live default route. "not in table" is success (idempotent clear).
fn delete_default_gateway(is_v6: bool) -> Result<(), String> {
    let args: Vec<&str> = if is_v6 {
        vec!["-6", "delete", "default"]
    } else {
        vec!["delete", "default"]
    };
    match cmd::run_sync_str(ROUTE, &args) {
        Ok(_) => Ok(()),
        Err(e) if e.contains("not in table") || e.contains("has not been found") => Ok(()),
        Err(e) => Err(e),
    }
}

/// Current live default-route gateway for a family, or None if absent.
fn live_default_gateway(is_v6: bool) -> Option<String> {
    let family = if is_v6 { "Internet6" } else { "Internet" };
    read_routes()
        .ok()?
        .into_iter()
        .find(|r| r.destination == "default" && r.family == family)
        .map(|r| r.gateway)
}

/// Compare gateway strings ignoring an IPv6 zone suffix (`fe80::1%em0`).
fn same_gw(a: &str, b: &str) -> bool {
    a.split('%').next().unwrap_or(a) == b.split('%').next().unwrap_or(b)
}

/// Ensure the live default routes match rc.conf (`defaultrouter` /
/// `ipv6_defaultrouter`). Interface address changes drop routes that
/// referenced a deleted address; this re-establishes them, mirroring the
/// netif → routing boot order. Returns an error note on failure.
fn restore_default_routes() -> Option<String> {
    let mut errors: Vec<String> = Vec::new();

    for (is_v6, want) in [(false, read_defaultrouter()), (true, read_ipv6_defaultrouter())] {
        let want = match want {
            Some(w) => w,
            None => continue,
        };
        let have = live_default_gateway(is_v6);
        if have.as_deref().map(|h| same_gw(&want, h)).unwrap_or(false) {
            continue; // already in effect
        }
        if let Err(e) = apply_default_gateway(is_v6, &want) {
            let fam = if is_v6 { "IPv6" } else { "IPv4" };
            errors.push(format!("{fam} default route restore failed: {e}"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors.join("; "))
    }
}

/// GET `/api/network/dns` — DNS configuration from `/etc/resolv.conf`.
pub async fn dns_config() -> ApiResult<Json<DnsConfig>> {
    let content = read_resolv_conf()?;
    Ok(Json(parse_resolv_conf(&content)))
}

#[derive(Debug, Deserialize)]
pub struct SetNameserversBody {
    pub nameservers: Vec<String>,
}

/// PUT `/api/network/dns/nameservers` — set all nameservers (max 3).
/// Empty strings are treated as empty slots. Validates each non-empty entry
/// as a valid IP address.
pub async fn set_nameservers(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SetNameserversBody>,
) -> ApiResult<Json<DnsConfig>> {
    if body.nameservers.len() > 3 {
        return Err(ApiError::BadRequest(
            "resolv.conf supports at most 3 nameservers".into(),
        ));
    }

    let mut servers: Vec<String> = Vec::new();
    for ns in &body.nameservers {
        let addr = ns.trim();
        if addr.is_empty() {
            continue;
        }
        validate_ip(addr)?;
        if servers.iter().any(|s| s == addr) {
            return Err(ApiError::Conflict(format!(
                "duplicate nameserver: {addr}"
            )));
        }
        servers.push(addr.to_string());
    }

    let content = read_resolv_conf()?;
    let mut cfg = parse_resolv_conf(&content);
    cfg.nameservers = servers;
    let new_content = build_resolv_conf(&content, &cfg);
    write_resolv_conf(&state, &new_content)?;

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/network/dns/nameservers",
        200,
        Some(format!("set nameservers: {}", cfg.nameservers.join(", "))),
    );

    Ok(Json(cfg))
}

// ─── Static routes ────────────────────────────────────────────────────────

/// A static route entry stored in rc.conf.
#[derive(Debug, Clone, Serialize)]
pub struct StaticRoute {
    pub name: String,
    pub destination: String,
    pub gateway: String,
    pub family: String,
    pub is_host: bool,
}

#[derive(Debug, Deserialize)]
pub struct StaticRouteInput {
    pub destination: String,
    pub gateway: String,
    pub name: Option<String>,
}

/// GET `/api/network/static-routes` — list all configured static routes from rc.conf.
pub async fn list_static_routes() -> ApiResult<Json<Vec<StaticRoute>>> {
    let routes = read_static_routes();
    Ok(Json(routes))
}

/// POST `/api/network/static-routes` — add a static route.
/// Writes `static_routes` and `route_<name>` in rc.conf, then applies with `route add`.
pub async fn create_static_route(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<StaticRouteInput>,
) -> ApiResult<Json<StaticRoute>> {
    let dest = body.destination.trim();
    let gw = body.gateway.trim();
    validate_route_input(dest, gw)?;

    let is_ipv6 = gw.contains(':');
    let is_host = !dest.contains('/');

    let mut existing = read_static_routes();
    let name = match body.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => {
            validate_route_name(&n)?;
            if existing.iter().any(|r| r.name == n) {
                return Err(ApiError::Conflict(format!(
                    "route name '{n}' already exists"
                )));
            }
            n.to_string()
        }
        None => next_route_name(&existing),
    };
    let args = build_route_args(dest, gw, is_ipv6, is_host);

    // Apply to the live system first — on failure nothing is persisted.
    apply_route_add(dest, gw, is_ipv6, is_host).map_err(ApiError::Command)?;

    crate::sysrc::set_async("static_routes", &add_to_list(&existing, &name)).await?;
    crate::sysrc::set_async(&format!("route_{name}"), &args).await?;


    existing.push(StaticRoute {
        name: name.clone(),
        destination: dest.to_string(),
        gateway: gw.to_string(),
        family: if is_ipv6 { "ipv6" } else { "ipv4" }.into(),
        is_host,
    });

    audit::record(
        &state, Some(&auth.username), "POST", "/api/network/static-routes", 200,
        Some(format!("added static route {name}: {dest} via {gw}")),
    );

    let last = existing.last().unwrap().clone();
    Ok(Json(last))
}

/// PUT `/api/network/static-routes/{name}` — update an existing static route.
pub async fn update_static_route(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(name): Path<String>,
    Json(body): Json<StaticRouteInput>,
) -> ApiResult<Json<StaticRoute>> {
    validate_route_name(&name)?;
    let dest = body.destination.trim();
    let gw = body.gateway.trim();
    validate_route_input(dest, gw)?;

    let existing = read_static_routes();
    let old = existing.iter().find(|r| r.name == name).ok_or_else(|| {
        ApiError::NotFound(format!("static route '{name}' not found"))
    })?;

    // Remove old live route, then apply the new one.
    apply_route_delete(&old.destination, old.family == "ipv6", old.is_host)
        .map_err(ApiError::Command)?;

    let is_ipv6 = gw.contains(':');
    let is_host = !dest.contains('/');
    let args = build_route_args(dest, gw, is_ipv6, is_host);

    crate::sysrc::set_async(&format!("route_{name}"), &args).await?;

    apply_route_add(dest, gw, is_ipv6, is_host).map_err(ApiError::Command)?;


    audit::record(
        &state, Some(&auth.username), "PUT", &format!("/api/network/static-routes/{name}"), 200,
        Some(format!("updated static route {name}: {dest} via {gw}")),
    );

    Ok(Json(StaticRoute {
        name,
        destination: dest.to_string(),
        gateway: gw.to_string(),
        family: if is_ipv6 { "ipv6" } else { "ipv4" }.into(),
        is_host,
    }))
}

/// DELETE `/api/network/static-routes/{name}` — remove a static route.
pub async fn delete_static_route(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(name): Path<String>,
) -> ApiResult<StatusCode> {
    validate_route_name(&name)?;

    let existing = read_static_routes();
    let route = existing.iter().find(|r| r.name == name).ok_or_else(|| {
        ApiError::NotFound(format!("static route '{name}' not found"))
    })?;

    let dest = route.destination.clone();
    let is_ipv6 = route.family == "ipv6";
    let is_host = route.is_host;

    // Remove the live route first — on failure rc.conf is untouched.
    apply_route_delete(&dest, is_ipv6, is_host).map_err(ApiError::Command)?;

    let remaining: Vec<&StaticRoute> = existing.iter().filter(|r| r.name != name).collect();
    if remaining.is_empty() {
        crate::sysrc::delete_async("static_routes").await?;
    } else {
        let list_str = remaining.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(" ");
        crate::sysrc::set_async("static_routes", &list_str).await?;
    }

    crate::sysrc::delete_async(&format!("route_{name}")).await?;

    audit::record(
        &state, Some(&auth.username), "DELETE", &format!("/api/network/static-routes/{name}"), 200,
        Some(format!("deleted static route {name}: {dest}")),
    );

    Ok(StatusCode::NO_CONTENT)
}

// ─── Static route helpers ─────────────────────────────────────────────────

/// Read all static routes from rc.conf (`static_routes` + `route_<name>` entries).
fn read_static_routes() -> Vec<StaticRoute> {
    let kv = crate::sysrc::read_rcconf_files();
    let names: Vec<&str> = kv
        .get("static_routes")
        .map(|s| s.split_whitespace().collect())
        .unwrap_or_default();

    names
        .into_iter()
        .filter_map(|name| {
            let key = format!("route_{name}");
            let value = kv.get(&key)?;
            let (dest, gw, is_ipv6, is_host) = parse_route_value(value)?;
            Some(StaticRoute {
                name: name.to_string(),
                destination: dest,
                gateway: gw,
                family: if is_ipv6 { "ipv6" } else { "ipv4" }.into(),
                is_host,
            })
        })
        .collect()
}

/// Parse a `route_<name>` rc.conf value into (destination, gateway, is_ipv6, is_host).
fn parse_route_value(value: &str) -> Option<(String, String, bool, bool)> {
    let tokens: Vec<&str> = value.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    let mut idx = 0;
    let mut is_ipv6 = false;

    if tokens[idx] == "-6" || tokens[idx] == "-inet6" {
        is_ipv6 = true;
        idx += 1;
    }

    let mut explicit_type = false;
    let mut is_host = false;
    while idx < tokens.len() && (tokens[idx] == "-net" || tokens[idx] == "-host") {
        is_host = tokens[idx] == "-host";
        explicit_type = true;
        idx += 1;
    }

    if idx >= tokens.len() {
        return None;
    }
    let destination = tokens[idx].to_string();
    idx += 1;

    let mut gateway = String::new();
    while idx < tokens.len() {
        let t = tokens[idx];
        if t.starts_with('-') {
            idx += 1;
            continue;
        }
        gateway = t.to_string();
        break;
    }
    if gateway.is_empty() {
        return None;
    }

    if !is_ipv6 {
        is_ipv6 = gateway.contains(':') || destination.contains(':');
    }
    if !explicit_type {
        is_host = !destination.contains('/');
    }

    Some((destination, gateway, is_ipv6, is_host))
}

/// Build the rc.conf `route_<name>` value from components.
fn build_route_args(dest: &str, gw: &str, is_ipv6: bool, is_host: bool) -> String {
    let mut parts = Vec::new();
    if is_ipv6 {
        parts.push("-6");
    }
    parts.push(if is_host { "-host" } else { "-net" });
    parts.push(dest);
    parts.push(gw);
    parts.join(" ")
}

/// Generate the next available route name (`fwp_1`, `fwp_2`, ...).
fn next_route_name(existing: &[StaticRoute]) -> String {
    let mut max_n = 0;
    for r in existing {
        if let Some(n_str) = r.name.strip_prefix("net") {
            if let Ok(n) = n_str.parse::<u32>() {
                if n > max_n {
                    max_n = n;
                }
            }
        }
    }
    format!("net{}", max_n + 1)
}

/// Build the space-separated `static_routes` list value after adding a new name.
fn add_to_list(existing: &[StaticRoute], new_name: &str) -> String {
    let mut names: Vec<&str> = existing.iter().map(|r| r.name.as_str()).collect();
    names.push(new_name);
    names.join(" ")
}

/// Validate destination and gateway.
fn validate_route_input(dest: &str, gw: &str) -> ApiResult<()> {
    if dest.is_empty() {
        return Err(ApiError::BadRequest("destination is required".into()));
    }
    if gw.is_empty() {
        return Err(ApiError::BadRequest("gateway is required".into()));
    }
    // Gateway must be a valid IP address.
    if gw.parse::<std::net::IpAddr>().is_err() {
        return Err(ApiError::BadRequest(format!(
            "'{gw}' is not a valid IP address"
        )));
    }
    // Destination must be IP or IP/prefix.
    if let Some((ip, _plen)) = dest.split_once('/') {
        if ip.parse::<std::net::IpAddr>().is_err() {
            return Err(ApiError::BadRequest(format!(
                "'{dest}' is not a valid destination"
            )));
        }
    } else if dest.parse::<std::net::IpAddr>().is_err() {
        return Err(ApiError::BadRequest(format!(
            "'{dest}' is not a valid destination"
        )));
    }
    Ok(())
}

/// Validate route name to prevent injection.
fn validate_route_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(ApiError::BadRequest("invalid route name".into()));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiError::BadRequest(
            "route name must match [a-zA-Z0-9_]+".into(),
        ));
    }
    Ok(())
}

/// Apply `route add` (or `route -6 add`) to the live system. Errors carry
/// route(8) stderr and propagate to the caller.
fn apply_route_add(dest: &str, gw: &str, is_ipv6: bool, is_host: bool) -> Result<(), String> {
    let route_type = if is_host { "-host" } else { "-net" };
    let mut args = Vec::new();
    if is_ipv6 {
        args.push("-6");
    }
    args.extend_from_slice(&["add", route_type, dest, gw]);
    cmd::run_sync_str(ROUTE, &args).map(|_| ())
}

/// Apply `route delete` (or `route -6 delete`) to the live system.
/// An absent route is success (idempotent delete).
fn apply_route_delete(dest: &str, is_ipv6: bool, is_host: bool) -> Result<(), String> {
    let route_type = if is_host { "-host" } else { "-net" };
    let mut args = Vec::new();
    if is_ipv6 {
        args.push("-6");
    }
    args.extend_from_slice(&["delete", route_type, dest]);
    match cmd::run_sync_str(ROUTE, &args) {
        Ok(_) => Ok(()),
        Err(e) if e.contains("not in table") || e.contains("has not been found") => Ok(()),
        Err(e) => Err(e),
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Read `defaultrouter` from rc.conf (direct file read, no subprocess).
fn read_defaultrouter() -> Option<String> {
    let v = crate::sysrc::read_rcconf_files().get("defaultrouter").cloned();
    v.filter(|s| !s.is_empty() && s != "NO")
}

/// Read `ipv6_defaultrouter` from rc.conf (direct file read, no subprocess).
fn read_ipv6_defaultrouter() -> Option<String> {
    let v = crate::sysrc::read_rcconf_files()
        .get("ipv6_defaultrouter")
        .cloned();
    v.filter(|s| !s.is_empty() && s != "NO")
}

/// Resolve the driver name (rc.conf key name) for an interface.
///
/// Uses the kernel sysctl `IFDATA_DRIVERNAME` to get the original
/// driver-assigned name, which survives user renaming. For example,
/// `vvswitch` resolves to `bridge3`. If the driver name equals the
/// live name (not renamed), returns the live name unchanged.
fn resolve_driver_name(live_name: &str) -> String {
    crate::ifutil::get_drivername(live_name).unwrap_or_else(|| live_name.to_string())
}

/// Whether `live` is the natural default name for an interface whose driver
/// (creation) name is `driver`. For most interfaces the default name equals
/// the driver name (e.g. `bridge0`). For epair the driver name is the *base*
/// (`epair0`) and the actual interfaces are `epair0a`/`epair0b`, so both are
/// considered default. Used to avoid mis-detecting a default-named epair as
/// renamed just because its live name differs from the driver base name.
fn is_default_iface_name(driver: &str, live: &str) -> bool {
    if driver == live {
        return true;
    }
    // epair: driver name is the base (epair0); live is epair0a/epair0b.
    if driver.starts_with("epair") {
        if let Some(suffix) = live.strip_prefix(driver) {
            return suffix == "a" || suffix == "b";
        }
    }
    false
}

/// Resolve the effective live-name target for an interface save.
///
/// - Explicit non-empty `cfg_name` → that name (a rename).
/// - Empty `cfg_name` means "use the default name" (the documented contract of
///   the config dialog's name field): for a RENAMED interface this reverts the
///   rename; for a default-named interface (including epair halves, whose
///   live name is not the driver base) the live name is kept unchanged.
/// - An unknown driver name (empty) cannot reveal the default → keep live.
/// - epair: the driver name is the shared base of the a/b pair and does not
///   say which half this was. The renamed half vacated its original name, so
///   exactly one of `<base>a`/`<base>b` is free (`name_taken`) — revert there.
fn resolve_target_name(
    cfg_name: Option<&str>,
    driver: &str,
    live: &str,
    name_taken: impl Fn(&str) -> bool,
) -> String {
    if let Some(n) = cfg_name.map(str::trim).filter(|s| !s.is_empty()) {
        return n.to_string();
    }
    if driver.is_empty() || is_default_iface_name(driver, live) {
        return live.to_string();
    }
    if driver.starts_with("epair") {
        for suffix in ["a", "b"] {
            let candidate = format!("{driver}{suffix}");
            if !name_taken(&candidate) {
                return candidate;
            }
        }
    }
    driver.to_string()
}

/// Parse rc.conf config for an interface, resolving the correct key layout.
///
/// Renamed interfaces use two key families:
/// - `ifconfig_<driver>_name="<live>"` — the rename itself, applied by rc.d's
///   `ifnet_rename()` before configuration.
/// - `ifconfig_<live>` (and `_aliases` / `_ipv6`) — the actual configuration,
///   applied to the renamed interface.
///
/// The live-name key is authoritative. For backward compatibility with the
/// legacy "driver key with embedded `name <live>`" layout, the driver key is
/// also parsed and merged, filling only fields the live-name key leaves empty.
/// The `name` directive is always stripped from `options` and routed to
/// `cfg.name` (priority: explicit `ifconfig_<driver>_name`, then a stripped
/// `name <X>` token, then inferred from driver != live).
fn parse_merged_rcconf(live_name: &str) -> IfaceRcConfConfig {
    let driver = resolve_driver_name(live_name);
    let kv = crate::sysrc::read_rcconf_files();

    // Primary config comes from the live-name key.
    let mut cfg = parse_iface_rcconf(live_name, &kv);
    let mut legacy_name: Option<String> = None;

    if driver != live_name {
        // Legacy compat: parse the driver key (old layout stored everything
        // under `ifconfig_<driver>` with an embedded `name <live>`).
        let drv_cfg = parse_iface_rcconf(&driver, &kv);
        legacy_name = drv_cfg.name.clone();

        // Driver fills only fields the live-name key leaves empty.
        if cfg.ipv4.is_none() && drv_cfg.ipv4.is_some() {
            cfg.ipv4 = drv_cfg.ipv4;
        }
        if cfg.ipv4_netmask.is_none() && drv_cfg.ipv4_netmask.is_some() {
            cfg.ipv4_netmask = drv_cfg.ipv4_netmask;
        }
        if cfg.ipv4_aliases.is_empty() {
            cfg.ipv4_aliases = drv_cfg.ipv4_aliases;
        }
        if cfg.ipv6_mode.is_empty() {
            cfg.ipv6_mode = drv_cfg.ipv6_mode.clone();
        }
        if cfg.ipv6.is_empty() {
            cfg.ipv6 = drv_cfg.ipv6;
        }
        if cfg.bridge_members.is_empty() {
            cfg.bridge_members = drv_cfg.bridge_members;
        }
        if cfg.lagg_proto.is_none() {
            cfg.lagg_proto = drv_cfg.lagg_proto;
        }
        if cfg.lagg_ports.is_empty() {
            cfg.lagg_ports = drv_cfg.lagg_ports;
        }
        if cfg.mtu.is_none() {
            cfg.mtu = drv_cfg.mtu;
        }
        if cfg.description.is_none() {
            cfg.description = drv_cfg.description;
        }
        if cfg.options.is_empty() {
            cfg.options = drv_cfg.options;
        }
        cfg.is_up |= drv_cfg.is_up;
    }

    // Resolve the rename target. Priority:
    //   1. explicit `ifconfig_<driver>_name` key
    //   2. a `name <X>` directive stripped from the live-name value (cfg.name)
    //   3. a `name <X>` directive stripped from the legacy driver value
    //   4. inferred: live name is not a default name for the driver → renamed.
    //      (epair default names epair0a/b differ from the driver base "epair0",
    //      so they must NOT be treated as renames.)
    let name_key = format!("ifconfig_{driver}_name");
    let explicit_name = kv
        .get(&name_key)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    cfg.name = explicit_name
        .or(cfg.name.take())
        .or(legacy_name)
        .or_else(|| {
            if !is_default_iface_name(&driver, live_name) {
                Some(live_name.to_string())
            } else {
                None
            }
        });
    // A name equal to the driver name means "no rename".
    if cfg.name.as_deref() == Some(driver.as_str()) {
        cfg.name = None;
    }

    cfg.interface = live_name.to_string();
    cfg.is_bridge = driver.starts_with("bridge");
    cfg.is_lagg = driver.starts_with("lagg");
    cfg
}

/// Read all `ifconfig_<name>` rc.conf entries and parse into structured config.
fn parse_iface_rcconf(name: &str, kv: &std::collections::HashMap<String, String>) -> IfaceRcConfConfig {
    let primary_key = format!("ifconfig_{name}");
    let aliases_key = format!("ifconfig_{name}_aliases");
    let ipv6_key = format!("ifconfig_{name}_ipv6");

    let mut cfg = IfaceRcConfConfig::default();

    // Parse primary value.
    if let Some(ref val) = kv.get(&primary_key) {
        let parsed = parse_ifconfig_tokens(val);
        cfg.is_up = parsed.is_up;
        cfg.ipv4 = parsed.ipv4;
        cfg.ipv4_netmask = parsed.ipv4_netmask;
        cfg.bridge_members = parsed.bridge_members;
        cfg.lagg_proto = parsed.lagg_proto;
        cfg.lagg_ports = parsed.lagg_ports;
        cfg.mtu = parsed.mtu;
        cfg.description = parsed.description;
        cfg.options = parsed.options_tokens.join(" ");
        cfg.name = parsed.name;
        // Additional inet entries beyond the first become aliases.
        if parsed.extra_inets.len() > 1 {
            for inet in parsed.extra_inets.iter().skip(1) {
                cfg.ipv4_aliases.push(RcIpv4Alias {
                    address: inet.address.clone(),
                    netmask: inet.netmask.clone().unwrap_or_default(),
                });
            }
        }
    }

    // Parse aliases value.
    if let Some(ref val) = kv.get(&aliases_key) {
        let parsed = parse_ifconfig_tokens(val);
        for inet in &parsed.extra_inets {
            cfg.ipv4_aliases.push(RcIpv4Alias {
                address: inet.address.clone(),
                netmask: inet.netmask.clone().unwrap_or_default(),
            });
        }
    }

    // Parse IPv6 value.
    if let Some(ref val) = kv.get(&ipv6_key) {
        let parsed = parse_ifconfig_tokens(val);
        if parsed.ipv6_accept_rtadv {
            cfg.ipv6_mode = "slaac".into();
        } else {
            cfg.ipv6_mode = "static".into();
            for e in &parsed.inet6s {
                cfg.ipv6.push(RcIpv6Entry {
                    address: e.address.clone(),
                    prefixlen: e.prefixlen.clone().unwrap_or_default(),
                });
            }
        }
    }

    cfg
}

/// Intermediate parse result.
struct ParsedIfConfig {
    is_up: bool,
    ipv4: Option<String>,
    ipv4_netmask: Option<String>,
    extra_inets: Vec<InetEntry>,
    inet6s: Vec<Inet6Entry>,
    ipv6_accept_rtadv: bool,
    bridge_members: Vec<String>,
    lagg_proto: Option<String>,
    lagg_ports: Vec<String>,
    mtu: Option<u32>,
    description: Option<String>,
    options_tokens: Vec<String>,
    name: Option<String>,
}

struct InetEntry {
    address: String,
    netmask: Option<String>,
}

struct Inet6Entry {
    address: String,
    prefixlen: Option<String>,
}

impl Default for ParsedIfConfig {
    fn default() -> Self {
        Self {
            is_up: false,
            ipv4: None,
            ipv4_netmask: None,
            extra_inets: Vec::new(),
            inet6s: Vec::new(),
            ipv6_accept_rtadv: false,
            bridge_members: Vec::new(),
            lagg_proto: None,
            lagg_ports: Vec::new(),
            mtu: None,
            description: None,
            options_tokens: Vec::new(),
            name: None,
        }
    }
}

/// Check if a token is a known ifconfig keyword (not an interface name).
fn is_ifconfig_keyword(token: &str) -> bool {
    matches!(
        token,
        "inet" | "inet6"
            | "up"
            | "down"
            | "addm"
            | "deletem"
            | "netmask"
            | "prefixlen"
            | "mtu"
            | "metric"
            | "name"
            | "DHCP"
            | "dhcp"
            | "SYNCDHCP"
            | "syncdhcp"
            | "WPA"
            | "wpa"
            | "polling"
            | "-polling"
            | "staticarp"
            | "-staticarp"
            | "description"
            | "media"
            | "mediaopt"
            | "laggproto"
            | "laggport"
    )
}

/// Parse an ifconfig value string into structured tokens.
fn parse_ifconfig_tokens(value: &str) -> ParsedIfConfig {
    let tokens: Vec<&str> = value.split_whitespace().collect();
    let mut result = ParsedIfConfig::default();

    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "inet" => {
                i += 1;
                if i >= tokens.len() {
                    break;
                }
                if tokens[i].eq_ignore_ascii_case("dhcp") {
                    result.ipv4 = Some("DHCP".into());
                    i += 1;
                } else if tokens[i].eq_ignore_ascii_case("syncdhcp") {
                    result.ipv4 = Some("SYNCDHCP".into());
                    i += 1;
                } else {
                    let addr = tokens[i];
                    let (ip, mask_from_cidr) = if let Some((ip, cidr)) = addr.split_once('/') {
                        (ip.to_string(), cidr_to_netmask(cidr))
                    } else {
                        (addr.to_string(), None)
                    };
                    i += 1;
                    let mut netmask = mask_from_cidr;
                    if i < tokens.len() && tokens[i] == "netmask" {
                        i += 1;
                        if i < tokens.len() {
                            netmask = Some(tokens[i].to_string());
                            i += 1;
                        }
                    }
                    result.extra_inets.push(InetEntry { address: ip, netmask });
                }
            }
            "inet6" => {
                i += 1;
                if i >= tokens.len() {
                    break;
                }
                // Detect SLAAC mode: "inet6 accept_rtadv"
                if tokens[i] == "accept_rtadv" {
                    result.ipv6_accept_rtadv = true;
                    i += 1;
                } else {
                    let addr = tokens[i].to_string();
                    i += 1;
                    let mut prefixlen = None;
                    if i < tokens.len() && tokens[i] == "prefixlen" {
                        i += 1;
                        if i < tokens.len() {
                            prefixlen = Some(tokens[i].to_string());
                            i += 1;
                        }
                    }
                    result.inet6s.push(Inet6Entry { address: addr, prefixlen });
                }
            }
            "up" => {
                result.is_up = true;
                i += 1;
            }
            "down" => {
                result.is_up = false;
                i += 1;
            }
            "addm" => {
                i += 1;
                while i < tokens.len() && !is_ifconfig_keyword(tokens[i]) {
                    if !result.bridge_members.contains(&tokens[i].to_string()) {
                        result.bridge_members.push(tokens[i].to_string());
                    }
                    i += 1;
                }
            }
            "deletem" => {
                i += 1;
                while i < tokens.len() && !is_ifconfig_keyword(tokens[i]) {
                    result.bridge_members.retain(|m| m != tokens[i]);
                    i += 1;
                }
            }
            "mtu" => {
                i += 1;
                if i < tokens.len() {
                    result.mtu = tokens[i].parse().ok();
                    i += 1;
                }
            }
            "description" => {
                i += 1;
                if i >= tokens.len() {
                    break;
                }
                if tokens[i].starts_with('\'') {
                    // Single-quoted description — collect until closing quote.
                    let mut words: Vec<&str> = Vec::new();
                    while i < tokens.len() {
                        words.push(tokens[i]);
                        if tokens[i].ends_with('\'') && tokens[i].len() > 1 {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    let joined = words.join(" ");
                    let cleaned = joined.trim_matches('\'').to_string();
                    result.description = Some(cleaned);
                } else if tokens[i].starts_with('"') {
                    // Double-quoted (old format) — collect until closing quote.
                    let mut words: Vec<&str> = Vec::new();
                    while i < tokens.len() {
                        words.push(tokens[i]);
                        if tokens[i].ends_with('"') && tokens[i].len() > 1 {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    let joined = words.join(" ");
                    let cleaned = joined.trim_matches('"').to_string();
                    result.description = Some(cleaned);
                } else {
                    // Unquoted single word.
                    result.description = Some(tokens[i].to_string());
                    i += 1;
                }
            }
            "laggproto" => {
                i += 1;
                if i < tokens.len() {
                    result.lagg_proto = Some(tokens[i].to_string());
                    i += 1;
                }
            }
            "laggport" => {
                i += 1;
                if i < tokens.len() {
                    result.lagg_ports.push(tokens[i].to_string());
                    i += 1;
                }
            }
            "name" => {
                // Interface rename directive — consume the target, do NOT push to options.
                i += 1;
                if i < tokens.len() {
                    result.name = Some(tokens[i].to_string());
                    i += 1;
                }
            }
            "DHCP" | "dhcp" => {
                result.ipv4 = Some("DHCP".into());
                i += 1;
            }
            "SYNCDHCP" | "syncdhcp" => {
                result.ipv4 = Some("SYNCDHCP".into());
                i += 1;
            }
            _ => {
                result.options_tokens.push(tokens[i].to_string());
                i += 1;
            }
        }
    }

    // Promote first inet entry to primary IPv4.
    if result.ipv4.is_none() {
        if let Some(first) = result.extra_inets.first() {
            result.ipv4 = Some(first.address.clone());
            result.ipv4_netmask = first.netmask.clone();
        }
    }

    result
}

/// Remove a `name <X>` directive from an options string, returning the cleaned
/// options and the extracted name (if any). Prevents a stray rename directive
/// from re-introducing the rc.d rename-during-config timing bug when the value
/// is applied to a live interface.
fn strip_name_directive(options: &str) -> (String, Option<String>) {
    let mut cleaned: Vec<&str> = Vec::new();
    let mut name: Option<String> = None;
    let tokens: Vec<&str> = options.split_whitespace().collect();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "name" {
            i += 1;
            if i < tokens.len() {
                name = Some(tokens[i].to_string());
                i += 1;
            }
        } else {
            cleaned.push(tokens[i]);
            i += 1;
        }
    }
    (cleaned.join(" "), name)
}

/// Convert CIDR prefix length to dotted-quad netmask.
fn cidr_to_netmask(cidr: &str) -> Option<String> {
    let prefix: u32 = cidr.parse().ok()?;
    if prefix > 32 {
        return None;
    }
    let mask: u32 = if prefix == 0 {
        0
    } else {
        !0u32 << (32 - prefix)
    };
    Some(format!(
        "{}.{}.{}.{}",
        (mask >> 24) & 0xff,
        (mask >> 16) & 0xff,
        (mask >> 8) & 0xff,
        mask & 0xff
    ))
}

/// Build the primary `ifconfig_<name>` value from structured config.
fn build_primary_value(cfg: &IfaceRcConfConfig) -> String {
    let mut parts: Vec<String> = Vec::new();

    // LAGG protocol + ports come first (must precede other config).
    if let Some(ref proto) = cfg.lagg_proto {
        let p = proto.trim();
        if !p.is_empty() {
            parts.push(format!("laggproto {p}"));
        }
    }
    for port in &cfg.lagg_ports {
        let p = port.trim();
        if !p.is_empty() {
            parts.push(format!("laggport {p}"));
        }
    }

    if let Some(ref ipv4) = cfg.ipv4 {
        let ip = ipv4.trim();
        if !ip.is_empty() {
            if ip.eq_ignore_ascii_case("DHCP") || ip.eq_ignore_ascii_case("SYNCDHCP") {
                parts.push(ip.to_string());
            } else {
                let mut s = format!("inet {ip}");
                if let Some(ref nm) = cfg.ipv4_netmask {
                    let nm = nm.trim();
                    if !nm.is_empty() {
                        s.push_str(&format!(" netmask {nm}"));
                    }
                }
                parts.push(s);
            }
        }
    }

    for m in &cfg.bridge_members {
        let m = m.trim();
        if !m.is_empty() {
            parts.push(format!("addm {m}"));
        }
    }

    if let Some(mtu) = cfg.mtu {
        if mtu > 0 {
            parts.push(format!("mtu {mtu}"));
        }
    }

    if let Some(ref desc) = cfg.description {
        let d = desc.trim();
        if !d.is_empty() {
            parts.push(format!("description '{d}'"));
        }
    }

    let opts = cfg.options.trim();
    if !opts.is_empty() {
        parts.push(opts.to_string());
    }

    if cfg.is_up {
        parts.push("up".into());
    }

    parts.join(" ")
}

/// Build the `ifconfig_<name>_aliases` value.
fn build_aliases_value(aliases: &[RcIpv4Alias]) -> String {
    aliases
        .iter()
        .filter(|a| !a.address.trim().is_empty())
        .map(|a| {
            let addr = a.address.trim();
            let nm = a.netmask.trim();
            if nm.is_empty() {
                format!("inet {addr}")
            } else {
                format!("inet {addr} netmask {nm}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the `ifconfig_<name>_ipv6` value.
fn build_ipv6_value(mode: &str, entries: &[RcIpv6Entry]) -> String {
    if mode == "slaac" {
        return "inet6 accept_rtadv".to_string();
    }
    if mode == "none" {
        return String::new();
    }
    entries
        .iter()
        .filter(|e| !e.address.trim().is_empty())
        .map(|e| {
            let addr = e.address.trim();
            let pl = e.prefixlen.trim();
            if pl.is_empty() {
                format!("inet6 {addr}")
            } else {
                format!("inet6 {addr} prefixlen {pl}")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Validate an interface name: `^[a-zA-Z0-9_.]+$`, 1–15 chars.
fn validate_iface_name(name: &str) -> ApiResult<()> {
    if name.is_empty() || name.len() > 15 {
        return Err(ApiError::BadRequest("invalid interface name length".into()));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(ApiError::BadRequest(
            "interface name must match [a-zA-Z0-9_.]+".into(),
        ));
    }
    Ok(())
}

const RESOLV_CONF: &str = "/etc/resolv.conf";

/// Read `/etc/resolv.conf`, returning an error on failure.
fn read_resolv_conf() -> ApiResult<String> {
    std::fs::read_to_string(RESOLV_CONF)
        .map_err(|e| ApiError::Internal(format!("cannot read {RESOLV_CONF}: {e}")))
}

/// Write `/etc/resolv.conf` atomically, snapshotting the current copy into
/// the unified `conf_backup/` directory first (non-blocking).
fn write_resolv_conf(state: &AppState, content: &str) -> ApiResult<()> {
    crate::backup::backup_file(state, RESOLV_CONF);

    // Atomic write: temp file + rename.
    let tmp = format!("{RESOLV_CONF}.fwp.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, RESOLV_CONF)?;
    Ok(())
}

/// Rebuild resolv.conf content from the original file (preserving comments and
/// unrecognised lines) and the updated [`DnsConfig`].
fn build_resolv_conf(original: &str, cfg: &DnsConfig) -> String {
    let mut out = String::new();
    let mut wrote_nameserver = false;
    let mut wrote_search = false;
    let mut wrote_domain = false;
    let mut wrote_options = false;

    let mut wrote_sortlist = false;

    for line in original.lines() {
        let trimmed = line.split('#').next().unwrap_or("").trim_end();
        let keyword = trimmed.split_whitespace().next().unwrap_or("");

        match keyword {
            "nameserver" => {
                if !wrote_nameserver {
                    for ns in &cfg.nameservers {
                        out.push_str(&format!("nameserver {ns}\n"));
                    }
                    wrote_nameserver = true;
                }
                // Skip original nameserver lines.
            }
            "search" => {
                if !wrote_search && !cfg.search.is_empty() {
                    out.push_str(&format!("search {}\n", cfg.search.join(" ")));
                    wrote_search = true;
                }
                // Skip original search lines if we're replacing.
            }
            "domain" => {
                if !wrote_domain {
                    if let Some(d) = &cfg.domain {
                        out.push_str(&format!("domain {d}\n"));
                    }
                    wrote_domain = true;
                }
            }
            "options" => {
                if !wrote_options && !cfg.options.is_empty() {
                    out.push_str(&format!("options {}\n", cfg.options.join(" ")));
                    wrote_options = true;
                }
            }
            "sortlist" => {
                if !wrote_sortlist && !cfg.sortlist.is_empty() {
                    out.push_str(&format!("sortlist {}\n", cfg.sortlist.join(" ")));
                    wrote_sortlist = true;
                }
            }
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }

    // Append any sections that were never emitted (because they didn't exist in the original).
    if !wrote_nameserver {
        for ns in &cfg.nameservers {
            out.push_str(&format!("nameserver {ns}\n"));
        }
    }
    if !wrote_search && !cfg.search.is_empty() {
        out.push_str(&format!("search {}\n", cfg.search.join(" ")));
    }
    if !wrote_domain {
        if let Some(d) = &cfg.domain {
            out.push_str(&format!("domain {d}\n"));
        }
    }
    if !wrote_options && !cfg.options.is_empty() {
        out.push_str(&format!("options {}\n", cfg.options.join(" ")));
    }
    if !wrote_sortlist && !cfg.sortlist.is_empty() {
        out.push_str(&format!("sortlist {}\n", cfg.sortlist.join(" ")));
    }

    out
}

/// Validate that a string is a valid IPv4 or IPv6 address.
fn validate_ip(addr: &str) -> ApiResult<()> {
    if addr.parse::<std::net::IpAddr>().is_ok() {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "'{addr}' is not a valid IP address"
        )))
    }
}
/// Validate a strict IPv4 address (gateway fields must be family-correct).
fn validate_ipv4(addr: &str) -> ApiResult<()> {
    if addr.parse::<Ipv4Addr>().is_ok() {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "'{addr}' is not a valid IPv4 address"
        )))
    }
}

/// Validate an IPv6 address with an optional zone suffix (`fe80::1%em0`).
/// Link-local gateways require the zone and route(8) accepts it; std's
/// `Ipv6Addr` parser rejects zones, so the suffix is checked separately.
fn validate_ipv6(addr: &str) -> ApiResult<()> {
    let (ip, zone) = match addr.split_once('%') {
        Some((ip, zone)) => (ip, Some(zone)),
        None => (addr, None),
    };
    if ip.parse::<Ipv6Addr>().is_err() {
        return Err(ApiError::BadRequest(format!(
            "'{addr}' is not a valid IPv6 address"
        )));
    }
    if let Some(z) = zone {
        if z.is_empty()
            || z.len() > 15
            || !z.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        {
            return Err(ApiError::BadRequest(format!(
                "invalid interface zone in '{addr}'"
            )));
        }
    }
    Ok(())
}
fn parse_resolv_conf(content: &str) -> DnsConfig {
    let mut cfg = DnsConfig {
        nameservers: Vec::new(),
        search: Vec::new(),
        domain: None,
        options: Vec::new(),
        sortlist: Vec::new(),
    };
    for line in content.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("nameserver") => {
                if let Some(ns) = parts.next() {
                    cfg.nameservers.push(ns.to_string());
                }
            }
            Some("search") => {
                cfg.search.extend(parts.map(String::from));
            }
            Some("domain") => {
                cfg.domain = parts.next().map(String::from);
            }
            Some("options") => {
                cfg.options.extend(parts.map(String::from));
            }
            Some("sortlist") => {
                cfg.sortlist.extend(parts.map(String::from));
            }
            _ => {}
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_interfaces_runs() {
        let ifaces = read_interfaces().expect("getifaddrs should succeed");
        assert!(!ifaces.is_empty(), "should have at least one interface");
        assert!(
            !ifaces.iter().any(|i| i.is_loopback),
            "loopback should be filtered out"
        );
        // At least one interface should have groups.
        assert!(
            ifaces.iter().any(|i| !i.groups.is_empty()),
            "at least one interface should have groups, got: {ifaces:?}"
        );
    }

    #[test]
    fn read_routes_runs() {
        let routes = read_routes().expect("NET_RT_DUMP should succeed");
        assert!(!routes.is_empty(), "should have at least one route");
        // Verify we see expected routes.
        assert!(
            routes.iter().any(|r| r.destination == "default"),
            "should have a default route, got: {:?}",
            routes.iter().map(|r| &r.destination).collect::<Vec<_>>()
        );
        // Verify link#N gateway format is present.
        assert!(
            routes.iter().any(|r| r.gateway.starts_with("link#")),
            "should have at least one link gateway, got: {:?}",
            routes.iter().map(|r| &r.gateway).collect::<Vec<_>>()
        );
        // Verify family field.
        assert!(
            routes.iter().any(|r| r.family == "Internet"),
            "should have IPv4 routes"
        );
        assert!(
            routes.iter().any(|r| r.family == "Internet6"),
            "should have IPv6 routes"
        );
    }

    #[test]
    fn flags_decode() {
        let f = flags_to_strings(libc::IFF_UP | libc::IFF_BROADCAST | libc::IFF_RUNNING);
        assert!(f.contains(&"UP".to_string()));
        assert!(f.contains(&"BROADCAST".to_string()));
        assert!(f.contains(&"RUNNING".to_string()));
    }

    #[test]
    fn route_flags_string() {
        let s = route_flags_to_string(RTF_UP | RTF_GATEWAY | RTF_STATIC);
        assert!(s.contains('U'));
        assert!(s.contains('G'));
        assert!(s.contains('S'));
    }

    #[test]
    fn ipv4_prefix_calc() {
        assert_eq!(ipv4_mask_to_prefix("255.255.255.0"), 24);
        assert_eq!(ipv4_mask_to_prefix("255.255.0.0"), 16);
        assert_eq!(ipv4_mask_to_prefix("255.255.255.255"), 32);
        assert_eq!(ipv4_mask_to_prefix("0.0.0.0"), 0);
    }

    #[test]
    fn parse_description_single_quoted() {
        // New format: single-quoted description.
        let val = "description 'Hello World' up";
        let parsed = parse_ifconfig_tokens(val);
        assert_eq!(parsed.description.as_deref(), Some("Hello World"));
        assert!(parsed.is_up);
    }

    #[test]
    fn parse_description_double_quoted() {
        // Old rc.conf may have double quotes.
        let val = r#"description "with hello" up"#;
        let parsed = parse_ifconfig_tokens(val);
        assert_eq!(parsed.description.as_deref(), Some("with hello"));
        assert!(parsed.is_up);
    }

    #[test]
    fn parse_description_unquoted() {
        // Single word, no quotes.
        let val = "description WAN up";
        let parsed = parse_ifconfig_tokens(val);
        assert_eq!(parsed.description.as_deref(), Some("WAN"));
        assert!(parsed.is_up);
    }

    #[test]
    fn description_read_and_clear() {
        // Pick a real interface from the system.
        let ifaces = read_interfaces().expect("getifaddrs should succeed");
        let iface = ifaces
            .iter()
            .find(|i| i.is_physical)
            .or_else(|| ifaces.first())
            .expect("should have at least one interface");
        let name = &iface.name;

        // Set a description via ifconfig.
        let test_desc = "fwp-test-description-12345";
        let r = std::process::Command::new("/sbin/ifconfig")
            .args([name, "description", test_desc])
            .output()
            .expect("ifconfig should run");
        assert!(r.status.success(), "ifconfig description set failed");

        // Verify the ioctl-based read picks it up.
        let ifaces2 = read_interfaces().expect("getifaddrs should succeed");
        let iface2 = ifaces2.iter().find(|i| &i.name == name).unwrap();
        assert_eq!(
            iface2.description.as_deref(),
            Some(test_desc),
            "description should be readable via SIOCGIFDESCR"
        );

        // Clean up: remove the description.
        let _ = std::process::Command::new("/sbin/ifconfig")
            .args([name, "description", ""])
            .output();
    }

    #[test]
    fn bridge_members_vm_public() {
        // vm-public is expected to exist on this system as a bridge.
        let ifaces = read_interfaces().expect("getifaddrs should succeed");
        let bridge = ifaces.iter().find(|i| i.name == "vm-public");
        if let Some(bridge) = bridge {
            assert!(
                !bridge.members.is_empty(),
                "vm-public should have bridge members, got: {:?}",
                bridge.members
            );
            // Each member should have a name and info string.
            for m in &bridge.members {
                assert!(!m.name.is_empty(), "member name should not be empty");
                assert!(!m.info.is_empty(), "member info should not be empty");
            }
        }
        // Non-bridge interfaces should have empty members.
        if let Some(phys) = ifaces.iter().find(|i| i.is_physical) {
            assert!(phys.members.is_empty(), "physical interface should not have bridge members");
        }
    }

    #[test]
    fn parse_strips_name_directive() {
        // The `name <X>` directive must be routed to `parsed.name`, not options.
        let parsed = parse_ifconfig_tokens("inet 10.0.0.1/24 name vvswitch up");
        assert_eq!(parsed.name.as_deref(), Some("vvswitch"));
        assert!(!parsed.options_tokens.iter().any(|t| t == "name"),
            "name must not leak into options_tokens");
        assert!(parsed.is_up);
        assert_eq!(parsed.ipv4.as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn strip_name_directive_extracts_and_cleans() {
        let (clean, name) = strip_name_directive("media 1000baseTX name wan0 mediaopt full-duplex");
        assert_eq!(name.as_deref(), Some("wan0"));
        assert_eq!(clean, "media 1000baseTX mediaopt full-duplex");
    }

    #[test]
    fn strip_name_directive_no_name() {
        let (clean, name) = strip_name_directive("media 1000baseTX mediaopt full-duplex");
        assert!(name.is_none());
        assert_eq!(clean, "media 1000baseTX mediaopt full-duplex");
    }

    #[test]
    fn build_primary_value_omits_name() {
        // With name routed to cfg.name and options clean, the built value must
        // not contain a `name` token (avoids the rename-during-config bug).
        let cfg = IfaceRcConfConfig {
            ipv4: Some("10.0.0.1".into()),
            ipv4_netmask: Some("255.255.255.0".into()),
            is_up: true,
            name: Some("wan0".into()),
            ..Default::default()
        };
        let val = build_primary_value(&cfg);
        assert!(!val.contains("name"), "built value must not contain 'name': {val}");
        assert!(val.contains("inet 10.0.0.1"));
        assert!(val.contains("up"));
    }

    #[test]
    fn is_default_iface_name_bridge() {
        assert!(is_default_iface_name("bridge0", "bridge0"));
        assert!(!is_default_iface_name("bridge0", "vm-public"));
    }

    #[test]
    fn is_default_iface_name_epair() {
        // epair driver name is the base; both halves are default names.
        assert!(is_default_iface_name("epair0", "epair0a"));
        assert!(is_default_iface_name("epair0", "epair0b"));
        assert!(is_default_iface_name("epair100", "epair100a"));
        assert!(is_default_iface_name("epair100", "epair100b"));
        // A different base or a custom name is a rename.
        assert!(!is_default_iface_name("epair0", "epair5a"));
        assert!(!is_default_iface_name("epair0", "myport"));
    }

    #[test]
    fn parse_epair_default_name_is_none() {
        // Live check: a default-named epair (epair100a) must NOT report a
        // rename target, even though its driver base name ("epair100") differs.
        let ifaces = read_interfaces().expect("getifaddrs should succeed");
        if !ifaces.iter().any(|i| i.name == "epair100a") {
            return; // epair100a not present on this host — skip.
        }
        let cfg = parse_merged_rcconf("epair100a");
        assert_eq!(cfg.name, None, "default-named epair must not be treated as renamed: {cfg:?}");
    }

    // ─── Reconciliation helpers ────────────────────────────────────────────

    #[test]
    fn managed_v4_excludes_dhcp_primary() {
        let mut cfg = IfaceRcConfConfig::default();
        cfg.ipv4 = Some("DHCP".into());
        cfg.ipv4_aliases = vec![RcIpv4Alias {
            address: "10.0.0.5".into(),
            netmask: "255.255.255.0".into(),
        }];
        assert_eq!(managed_v4(&cfg), vec!["10.0.0.5"]);

        cfg.ipv4 = Some("10.0.0.1".into());
        assert_eq!(managed_v4(&cfg), vec!["10.0.0.1", "10.0.0.5"]);

        cfg.ipv4 = Some("".into());
        assert_eq!(managed_v4(&cfg), vec!["10.0.0.5"]);
    }

    #[test]
    fn managed_v6_static_only_and_no_link_local() {
        let mut cfg = IfaceRcConfConfig::default();
        cfg.ipv6_mode = "slaac".into();
        cfg.ipv6 = vec![RcIpv6Entry {
            address: "2001:db8::1".into(),
            prefixlen: "64".into(),
        }];
        assert!(managed_v6(&cfg).is_empty(), "slaac addresses are not ours");

        cfg.ipv6_mode = "static".into();
        cfg.ipv6.push(RcIpv6Entry {
            address: "fe80::1".into(),
            prefixlen: "64".into(),
        });
        assert_eq!(managed_v6(&cfg), vec!["2001:db8::1"]);
    }

    #[test]
    fn validate_ipv6_accepts_zone_suffix() {
        assert!(validate_ipv6("fe80::1%em0").is_ok());
        assert!(validate_ipv6("2001:db8::1").is_ok());
        assert!(validate_ipv6("fe80::1%").is_err());
        assert!(validate_ipv6("fe80::1%em;0").is_err());
        assert!(validate_ipv6("192.168.1.1").is_err());
    }

    #[test]
    fn validate_ipv4_rejects_v6() {
        assert!(validate_ipv4("192.168.1.1").is_ok());
        assert!(validate_ipv4("fe80::1").is_err());
        assert!(validate_ipv4(" DHCP").is_err());
    }

    #[test]
    fn same_gw_ignores_zone() {
        assert!(same_gw("fe80::1%em0", "fe80::1%em0"));
        assert!(same_gw("fe80::1%em0", "fe80::1"));
        assert!(!same_gw("fe80::1%em0", "fe80::2%em0"));
        assert!(same_gw("192.168.1.1", "192.168.1.1"));
    }

    /// Destroy the interface on scope exit so a failed assertion does not
    /// leak test interfaces.
    struct IfaceGuard(String);
    impl Drop for IfaceGuard {
        fn drop(&mut self) {
            let _ = run_ifconfig(&self.0, &["destroy"]);
        }
    }

    /// Live test: apply_ifconfig must converge live state, including
    /// REMOVING addresses that drop out of the config. Uses a throwaway
    /// epair (getifaddrs filters loopback, so loN cannot be observed).
    #[test]
    fn apply_ifconfig_converges_live_state() {
        let out = run_ifconfig("epair", &["create"]).expect("epair create");
        let name = out.trim().to_string();
        assert!(!name.is_empty(), "epair create should print the new name");
        let _guard = IfaceGuard(name.clone());

        let live = || {
            read_interfaces()
                .expect("getifaddrs")
                .into_iter()
                .find(|i| i.name == name)
                .expect("epair visible via getifaddrs")
        };

        // Empty baseline.
        let empty = IfaceRcConfConfig {
            interface: name.clone(),
            is_up: true,
            ..Default::default()
        };
        apply_ifconfig(&name, &empty, &empty).expect("apply empty config");

        // Add primary + alias + static v6.
        let mut cfg = empty.clone();
        cfg.ipv4 = Some("10.198.7.1".into());
        cfg.ipv4_netmask = Some("255.255.255.0".into());
        cfg.ipv4_aliases = vec![RcIpv4Alias {
            address: "10.198.7.11".into(),
            netmask: "255.255.255.0".into(),
        }];
        cfg.ipv6_mode = "static".into();
        cfg.ipv6 = vec![RcIpv6Entry {
            address: "2001:db8:198::1".into(),
            prefixlen: "64".into(),
        }];
        apply_ifconfig(&name, &empty, &cfg).expect("apply cfg");

        let l = live();
        assert!(l.ipv4.iter().any(|ip| ip.address == "10.198.7.1"), "{:?}", l.ipv4);
        assert!(l.ipv4.iter().any(|ip| ip.address == "10.198.7.11"), "{:?}", l.ipv4);
        assert!(
            l.ipv6.iter().any(|ip| ip.address == "2001:db8:198::1"),
            "{:?}",
            l.ipv6
        );

        // Remove the alias → must be deleted from the live interface.
        let mut cfg2 = cfg.clone();
        cfg2.ipv4_aliases.clear();
        apply_ifconfig(&name, &cfg, &cfg2).expect("remove alias");
        let l = live();
        assert!(
            !l.ipv4.iter().any(|ip| ip.address == "10.198.7.11"),
            "alias must be gone: {:?}",
            l.ipv4
        );
        assert!(l.ipv4.iter().any(|ip| ip.address == "10.198.7.1"));

        // Clear the primary + static v6 → both must be deleted.
        let mut cfg3 = cfg2.clone();
        cfg3.ipv4 = None;
        cfg3.ipv4_netmask = None;
        cfg3.ipv6.clear();
        apply_ifconfig(&name, &cfg2, &cfg3).expect("clear addresses");
        let l = live();
        assert!(l.ipv4.is_empty(), "primary must be gone: {:?}", l.ipv4);
        assert!(
            !l.ipv6.iter().any(|ip| ip.address == "2001:db8:198::1"),
            "static v6 must be gone: {:?}",
            l.ipv6
        );
    }

    /// With the live default route already matching rc.conf (the common host
    /// state), restore_default_routes must be a no-op (None). Live check in
    /// the style of `read_routes_runs`.
    #[test]
    fn restore_default_routes_noop_when_in_effect() {
        let configured = read_defaultrouter();
        let live = live_default_gateway(false);
        if configured.is_none() || live.is_none() {
            return; // no v4 default route on this host — skip.
        }
        assert_eq!(
            restore_default_routes(),
            None,
            "gateway in effect must not trigger a restore"
        );
    }

    #[test]
    fn resolve_target_name_matrix() {
        // Explicit custom name → that name (rename).
        assert_eq!(resolve_target_name(Some("wan0"), "em0", "em0", |_| false), "wan0");
        // Explicit driver name → same, via the normal rename path.
        assert_eq!(resolve_target_name(Some("em0"), "em0", "mgmt0", |_| false), "em0");
        // Whitespace-only is treated as empty.
        assert_eq!(resolve_target_name(Some("  "), "em0", "mgmt0", |_| false), "em0");
        // Empty + default-named → keep live name (no rename).
        assert_eq!(resolve_target_name(None, "em0", "em0", |_| false), "em0");
        // Empty + unknown driver → keep live (cannot know the default).
        assert_eq!(resolve_target_name(None, "", "em0", |_| false), "em0");
        // Empty + renamed → revert to the driver name.
        assert_eq!(resolve_target_name(None, "em0", "mgmt0", |_| false), "em0");
        // Empty + epair half → default name, keep live (NOT the base "epair0").
        assert_eq!(resolve_target_name(None, "epair0", "epair0a", |_| false), "epair0a");
        assert_eq!(
            resolve_target_name(None, "epair100", "epair100b", |_| false),
            "epair100b"
        );
        // Empty + renamed epair half → the FREE half of the pair. The sibling
        // keeps its name, so "taken" identifies which side to return.
        assert_eq!(
            resolve_target_name(None, "epair0", "left0", |n| n == "epair0a"),
            "epair0b"
        );
        assert_eq!(
            resolve_target_name(None, "epair0", "right0", |n| n == "epair0b"),
            "epair0a"
        );
        // Both halves free → the a side.
        assert_eq!(resolve_target_name(None, "epair0", "left0", |_| false), "epair0a");
    }

    /// Live round-trip: rename an epair half, then resolve a cleared name
    /// field the way interface_update does — the target must be the ORIGINAL
    /// half name (the free side of the pair), and renaming back must succeed.
    #[test]
    fn interface_rename_reverts_on_cleared_name() {
        let out = run_ifconfig("epair", &["create"]).expect("epair create");
        let half = out.trim().to_string(); // e.g. "epair1a" — the created half
        assert!(half.ends_with('a'), "created epair half: {half}");
        let _guard = IfaceGuard(half.clone());

        // Rename the half away.
        cmd::run_sync(IFCONFIG, &[&half, "name", "fwp-rev0"]).expect("rename");

        // Resolve exactly as interface_update does.
        let driver = crate::ifutil::get_drivername("fwp-rev0").expect("drivername");
        let target = resolve_target_name(None, &driver, "fwp-rev0", |candidate| {
            read_interfaces()
                .map(|ifs| ifs.iter().any(|i| i.name == candidate))
                .unwrap_or(false)
        });
        assert_eq!(target, half, "cleared name must revert to the original half");

        // Perform the rename back and verify live state.
        cmd::run_sync(IFCONFIG, &["fwp-rev0", "name", &half]).expect("rename back");
        let names: Vec<String> = read_interfaces()
            .expect("getifaddrs")
            .into_iter()
            .map(|i| i.name)
            .collect();
        assert!(names.contains(&half), "{names:?}");
        assert!(!names.iter().any(|n| n == "fwp-rev0"), "{names:?}");
    }

}
