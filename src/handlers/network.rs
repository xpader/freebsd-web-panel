//! Network interface management — list interfaces, routes, and default gateway.
//!
//! Interface data is obtained via `getifaddrs(3)` (no subprocess).  The routing
//! table is obtained via `sysctl(NET_RT_DUMP)` (binary buffer, no subprocess).
//! Only `defaultrouter` from rc.conf uses `sysrc`.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::process::Command;

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
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

/// `SIOCGIFGROUP` — get interface group membership. From `<sys/sockio.h>`.
/// `_IOWR('i', 136, struct ifgroupreq)` = 0xc0286988 (sizeof=40 on amd64).
const SIOCGIFGROUP: libc::c_ulong = 0xc0286988;

/// `struct ifgroupreq` — from `<net/if.h>`.
/// Layout: name[16] + len(4) + pad(4) + union{ char[16] | *ifg_req }(16).
/// Total = 40 bytes.
#[repr(C)]
struct IfGroupReq {
    name: [libc::c_char; libc::IFNAMSIZ as usize],
    len: libc::c_uint,
    _pad: libc::c_uint,
    // Union: char[IFNAMSIZ] or struct ifg_req *. In the second call we store
    // a pointer at offset 24 (16+4+4), which is 8 bytes within the 16-byte union.
    groups_ptr: [u8; 16],
}

/// `struct ifg_req` — one group entry. From `<net/if.h>`.
#[repr(C)]
struct IfgReq {
    group: [libc::c_char; libc::IFNAMSIZ as usize],
}

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
}

#[derive(Debug, Serialize)]
pub struct DnsConfig {
    pub nameservers: Vec<String>,
    pub search: Vec<String>,
    pub domain: Option<String>,
    pub options: Vec<String>,
    pub sortlist: Vec<String>,
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
            is_physical: crate::sysinfo::is_physical_iface(&name),
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
/// interface via four ioctls on the same socket fd.
fn fill_iface_ioctl(fd: libc::c_int, iface: &mut NetworkInterface) {
    let cname = match std::ffi::CString::new(iface.name.as_str()) {
        Ok(c) => c,
        Err(_) => return,
    };
    let name_bytes = cname.as_bytes_with_nul();
    if name_bytes.len() > libc::IFNAMSIZ as usize {
        return;
    }

    // ── SIOCGIFGROUP: interface groups ──
    {
        let mut req = IfGroupReq {
            name: [0; libc::IFNAMSIZ as usize],
            len: 0,
            _pad: 0,
            groups_ptr: [0u8; 16],
        };
        for (i, &b) in name_bytes.iter().enumerate() {
            req.name[i] = b as libc::c_char;
        }
        let rc = unsafe { libc::ioctl(fd, SIOCGIFGROUP as libc::c_ulong, &mut req) };
        if rc == 0 && req.len > 0 {
            let len = req.len as usize;
            let entry_size = std::mem::size_of::<IfgReq>();
            if len % entry_size == 0 && len / entry_size <= 64 {
                let mut buf = vec![0u8; len];
                req.len = len as libc::c_uint;
                let buf_ptr = buf.as_mut_ptr() as *mut libc::c_void;
                unsafe {
                    std::ptr::write(
                        (&mut req as *mut IfGroupReq).cast::<u8>().add(24)
                            as *mut *mut libc::c_void,
                        buf_ptr,
                    );
                }
                let rc = unsafe { libc::ioctl(fd, SIOCGIFGROUP as libc::c_ulong, &mut req) };
                if rc == 0 {
                    let count = len / entry_size;
                    for i in 0..count {
                        let entry = unsafe { &*(buf.as_ptr().add(i * entry_size) as *const IfgReq) };
                        let group = unsafe { CStr::from_ptr(entry.group.as_ptr()) }
                            .to_string_lossy()
                            .into_owned();
                        if !group.is_empty() && group != "all" {
                            iface.groups.push(group);
                        }
                    }
                }
            }
        }
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
        .find(|r| r.destination == "default")
        .map(|r| (Some(r.gateway.clone()), Some(r.interface.clone())))
        .unwrap_or((None, None));

    Ok(Json(DefaultGateway {
        gateway,
        interface,
        configured: read_defaultrouter(),
    }))
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

const SYSRC: &str = "/usr/sbin/sysrc";

/// Read `defaultrouter` from rc.conf via `sysrc -n defaultrouter`.
fn read_defaultrouter() -> Option<String> {
    let output = Command::new(SYSRC).args(["-n", "defaultrouter"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let val = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
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
