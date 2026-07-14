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
    pub media: Option<String>,
    pub mediaopt: Option<String>,
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
) -> ApiResult<(StatusCode, Json<NetworkInterface>)> {
    validate_iface_name(&body.name)?;

    // Reject names that look like physical interfaces.
    if crate::sysinfo::is_hardware_iface(&body.name) {
        return Err(ApiError::BadRequest(
            "interface name conflicts with a physical interface".into(),
        ));
    }

    // Create via ifconfig + read back + persist to rc.conf.
    let iface_name = body.name.clone();
    let iface = tokio::task::spawn_blocking(move || -> ApiResult<NetworkInterface> {
        cmd::run_sync(IFCONFIG, &[&iface_name, "create"])?;

        let interfaces = read_interfaces().map_err(ApiError::Io)?;
        let lookup_name = if iface_name.starts_with("epair")
            && !iface_name.ends_with('a')
            && !iface_name.ends_with('b')
        {
            format!("{}a", iface_name)
        } else {
            iface_name.clone()
        };
        let iface = interfaces
            .into_iter()
            .find(|i| i.name == lookup_name)
            .ok_or_else(|| {
                ApiError::Internal(format!(
                    "interface '{}' created but not found on re-read",
                    iface_name
                ))
            })?;
        add_cloned_interface(&iface_name);
        Ok(iface)
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

    Ok((StatusCode::CREATED, Json(iface)))
}

/// GET `/api/network/interfaces` — list all network interfaces.
pub async fn list_interfaces() -> ApiResult<Json<Vec<NetworkInterface>>> {
    let interfaces = read_interfaces().map_err(ApiError::Io)?;
    Ok(Json(interfaces))
}

/// GET `/api/network/interfaces/{name}` — single interface detail.
pub async fn interface_detail(Path(name): Path<String>) -> ApiResult<Json<NetworkInterface>> {
    validate_iface_name(&name)?;
    let interfaces = read_interfaces().map_err(ApiError::Io)?;
    interfaces
        .into_iter()
        .find(|iface| iface.name == name)
        .map(Json)
        .ok_or_else(|| ApiError::NotFound(format!("interface '{name}' not found")))
}

/// GET `/api/network/interfaces/{name}/rcconf` — parsed rc.conf ifconfig config for an interface.
pub async fn interface_rcconf(Path(name): Path<String>) -> ApiResult<Json<IfaceRcConfConfig>> {
    validate_iface_name(&name)?;
    let result = tokio::task::spawn_blocking(move || {
        let mut cfg = parse_iface_rcconf(&name);
        cfg.interface = name.clone();
        cfg.is_bridge = name.starts_with("bridge");
        cfg.is_lagg = name.starts_with("lagg");
        cfg
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    Ok(Json(result))
}

/// PUT `/api/network/interfaces/{name}/rcconf` — save structured ifconfig config to rc.conf.
pub async fn interface_rcconf_save(
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
    if let Some(ref m) = cfg.media {
        validate_str(m)?;
    }
    if let Some(ref m) = cfg.mediaopt {
        validate_str(m)?;
    }
    if let Some(ref p) = cfg.lagg_proto {
        validate_str(p)?;
    }

    let primary_key = format!("ifconfig_{name}");
    let aliases_key = format!("ifconfig_{name}_aliases");
    let ipv6_key = format!("ifconfig_{name}_ipv6");

    // Apply to live system + persist to rc.conf (all blocking subprocess work).
    let save_name = name.clone();
    let save_cfg = cfg.clone();
    let result = tokio::task::spawn_blocking(move || -> ApiResult<IfaceRcConfConfig> {
        // 1. Apply to live system first — if this fails, don't touch rc.conf.
        apply_ifconfig(&save_name, &save_cfg).map_err(ApiError::Command)?;

        // 2. Persist to rc.conf.
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

        // Re-read to confirm what was stored.
        let mut result = parse_iface_rcconf(&save_name);
        result.interface = save_name.clone();
        result.is_bridge = save_name.starts_with("bridge");
        result.is_lagg = save_name.starts_with("lagg");
        Ok(result)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        &format!("/api/network/interfaces/{name}/rcconf"),
        200,
        Some(format!("updated rc.conf ifconfig_{name}")),
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

    // Media / Mediaopt
    if let Some(ref media) = cfg.media {
        let m = media.trim();
        if !m.is_empty() {
            args.push("media".into());
            args.push(m.into());
        }
    }
    if let Some(ref mediaopt) = cfg.mediaopt {
        let m = mediaopt.trim();
        if !m.is_empty() {
            args.push("mediaopt".into());
            args.push(m.into());
        }
    }

    // UP
    if cfg.is_up {
        args.push("up".into());
    }

    args
}

/// Apply an interface's structured config to the live system via ifconfig.
/// Reads the current live state first and skips properties already in effect
/// (existing bridge members, lagg ports, IP aliases) to avoid duplicate-add errors.
fn apply_ifconfig(name: &str, cfg: &IfaceRcConfConfig) -> Result<String, String> {
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
    let existing_v4: Vec<String> = live
        .as_ref()
        .map(|i| i.ipv4.iter().map(|ip| ip.address.clone()).collect())
        .unwrap_or_default();
    let existing_v6: Vec<String> = live
        .as_ref()
        .map(|i| i.ipv6.iter().map(|ip| ip.address.clone()).collect())
        .unwrap_or_default();

    // 1. Apply primary non-structural config (IP, MTU, description, media, UP).
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

    // 5. Apply each IPv4 alias (skip existing).
    for alias in &cfg.ipv4_aliases {
        let addr = alias.address.trim();
        if addr.is_empty() || existing_v4.iter().any(|a| a == addr) {
            continue;
        }
        let nm = alias.netmask.trim();
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

    // 6. Apply each IPv6 entry (skip existing, skip SLAAC mode).
    if cfg.ipv6_mode != "slaac" {
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

    if errors.is_empty() {
        Ok(output)
    } else {
        Err(errors.join("; "))
    }
}

/// Read `cloned_interfaces` from rc.conf as a Vec of interface names.
fn read_cloned_interfaces() -> Vec<String> {
    crate::sysrc::get_list("cloned_interfaces")
}

/// Add a name to `cloned_interfaces` in rc.conf (idempotent).
/// For epair, the base name (e.g. "epair0" from "epair0a") is used.
fn add_cloned_interface(name: &str) {
    let clone_name = epair_base_name(name).unwrap_or_else(|| name.to_string());
    let mut list = read_cloned_interfaces();
    if list.iter().any(|s| s == &clone_name) {
        return;
    }
    list.push(clone_name);
    let val = list.join(" ");
    crate::sysrc::set_forget("cloned_interfaces", &val);
}

/// Remove a name from `cloned_interfaces` in rc.conf.
/// For epair, the base name is used.
fn remove_cloned_interface(name: &str) {
    let clone_name = epair_base_name(name).unwrap_or_else(|| name.to_string());
    let list = read_cloned_interfaces();
    let new_list: Vec<&String> = list.iter().filter(|s| s.as_str() != clone_name).collect();
    if new_list.is_empty() {
        crate::sysrc::delete("cloned_interfaces");
    } else {
        let val = new_list.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
        crate::sysrc::set_forget("cloned_interfaces", &val);
    }
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
    let output = tokio::task::spawn_blocking(move || {
        let cfg = parse_iface_rcconf(&apply_name);
        apply_ifconfig(&apply_name, &cfg).map_err(ApiError::Command)
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

        // Destroy via ifconfig.
        cmd::run_sync(IFCONFIG, &[&destroy_name, "destroy"])?;

        // Clean up rc.conf entries.
        let primary_key = format!("ifconfig_{destroy_name}");
        let aliases_key = format!("ifconfig_{destroy_name}_aliases");
        let ipv6_key = format!("ifconfig_{destroy_name}_ipv6");
        for key in [&primary_key, &aliases_key, &ipv6_key] {
            crate::sysrc::delete(key);
        }
        remove_cloned_interface(&destroy_name);
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
            crate::sysrc::delete_async("defaultrouter").await?;
            tokio::task::spawn_blocking(|| {
                cmd::run_forget_sync("/sbin/route", &["delete", "default"]);
            })
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
            audit::record(
                &state, Some(&auth.username), "PUT", "/api/network/gateway", 200,
                Some("cleared IPv4 default gateway".into()),
            );
        } else {
            validate_ip(gw)?;
            crate::sysrc::set_async("defaultrouter", gw).await?;
            let gw_owned = gw.to_string();
            tokio::task::spawn_blocking(move || {
                if !cmd::status_sync("/sbin/route", &["change", "default", &gw_owned]) {
                    cmd::run_forget_sync("/sbin/route", &["add", "default", &gw_owned]);
                }
            })
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
            audit::record(
                &state, Some(&auth.username), "PUT", "/api/network/gateway", 200,
                Some(format!("set IPv4 default gateway to {gw}")),
            );
        }
    }

    if let Some(raw) = &body.gateway6 {
        let gw = raw.trim();
        if gw.is_empty() {
            crate::sysrc::delete_async("ipv6_defaultrouter").await?;
            tokio::task::spawn_blocking(|| {
                cmd::run_forget_sync("/sbin/route", &["-6", "delete", "default"]);
            })
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
            audit::record(
                &state, Some(&auth.username), "PUT", "/api/network/gateway", 200,
                Some("cleared IPv6 default gateway".into()),
            );
        } else {
            validate_ip(gw)?;
            crate::sysrc::set_async("ipv6_defaultrouter", gw).await?;
            let gw_owned = gw.to_string();
            tokio::task::spawn_blocking(move || {
                if !cmd::status_sync("/sbin/route", &["-6", "change", "default", &gw_owned]) {
                    cmd::run_forget_sync("/sbin/route", &["-6", "add", "default", &gw_owned]);
                }
            })
            .await
            .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
            audit::record(
                &state, Some(&auth.username), "PUT", "/api/network/gateway", 200,
                Some(format!("set IPv6 default gateway to {gw}")),
            );
        }
    }

    default_gateway().await
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

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Read `defaultrouter` from rc.conf via sysrc.
fn read_defaultrouter() -> Option<String> {
    crate::sysrc::get("defaultrouter")
}

/// Read `ipv6_defaultrouter` from rc.conf via sysrc.
fn read_ipv6_defaultrouter() -> Option<String> {
    crate::sysrc::get("ipv6_defaultrouter")
}

/// Read all `ifconfig_<name>` rc.conf entries and parse into structured config.
fn parse_iface_rcconf(name: &str) -> IfaceRcConfConfig {
    let kv = crate::sysrc::list_all();
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
        cfg.media = parsed.media;
        cfg.mediaopt = parsed.mediaopt;
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
    media: Option<String>,
    mediaopt: Option<String>,
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
            media: None,
            mediaopt: None,
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
            "media" => {
                i += 1;
                if i < tokens.len() && tokens[i] != "mediaopt" && !is_ifconfig_keyword(tokens[i]) {
                    result.media = Some(tokens[i].to_string());
                    i += 1;
                }
            }
            "mediaopt" => {
                i += 1;
                if i < tokens.len() {
                    result.mediaopt = Some(tokens[i].to_string());
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
            "DHCP" | "dhcp" => {
                result.ipv4 = Some("DHCP".into());
                i += 1;
            }
            "SYNCDHCP" | "syncdhcp" => {
                result.ipv4 = Some("SYNCDHCP".into());
                i += 1;
            }
            _ => {
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

    if !cfg.bridge_members.is_empty() {
        let members: Vec<&str> = cfg.bridge_members.iter().map(|s| s.as_str()).collect();
        parts.push(format!("addm {}", members.join(" ")));
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

    if let Some(ref media) = cfg.media {
        let m = media.trim();
        if !m.is_empty() {
            parts.push(format!("media {m}"));
        }
    }

    if let Some(ref mediaopt) = cfg.mediaopt {
        let m = mediaopt.trim();
        if !m.is_empty() {
            parts.push(format!("mediaopt {m}"));
        }
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

/// Write `/etc/resolv.conf` atomically with a timestamped backup.
/// Backups are stored in a `dns-backup/` subdirectory under the configured DB
/// path (e.g. `/var/db/fwp/dns-backup/`), not next to the original file.
fn write_resolv_conf(state: &AppState, content: &str) -> ApiResult<()> {
    let backup_dir = state
        .config
        .paths
        .db
        .parent()
        .unwrap_or_else(|| std::path::Path::new("/var/db/fwp"))
        .join("dns-backup");

    // Backup (non-blocking — a missing backup is better than a blocked edit).
    let ts = state.now_ts();
    let backup = backup_dir.join(format!("resolv.conf.{ts}"));
    if let Err(e) = std::fs::create_dir_all(&backup_dir)
        .and_then(|_| std::fs::copy(RESOLV_CONF, &backup).map(|_| ()))
    {
        tracing::warn!(error = %e, "resolv.conf backup failed (non-blocking)");
    } else {
        prune_backups(&backup_dir, "resolv.conf.", 5);
    }

    // Atomic write: temp file + rename.
    let tmp = format!("{RESOLV_CONF}.fwp.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, RESOLV_CONF)?;
    Ok(())
}

/// Keep at most `max` backup files matching `prefix` in `dir`.
fn prune_backups(dir: &std::path::Path, prefix: &str, max: usize) {
    let mut entries: Vec<(u64, std::path::PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if let Some(suffix) = name.strip_prefix(prefix) {
                if let Ok(ts) = suffix.parse::<u64>() {
                    entries.push((ts, ent.path()));
                }
            }
        }
    }
    if entries.len() <= max {
        return;
    }
    entries.sort_unstable_by_key(|(ts, _)| *ts);
    for (_, path) in entries.iter().take(entries.len() - max) {
        let _ = std::fs::remove_file(path);
    }
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
}
