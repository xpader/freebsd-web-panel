//! Low-level network interface utilities — ioctl and sysctl wrappers.
//!
//! All unsafe code is contained here. Consumers call safe functions that
//! return plain Rust types.

use std::ffi::{CStr, CString};
use std::os::raw::c_void;

// ── Constants from <sys/sockio.h> and <net/if.h> ───────────────────────────

/// `_IOWR('i', 136, struct ifgroupreq)` = 0xc0286988 (sizeof=40 on amd64).
const SIOCGIFGROUP: libc::c_ulong = 0xc0286988;

/// `_IOWR('i', 138, struct ifgroupreq)` = 0xc028698a (sizeof=40 on amd64).
const SIOCGIFGMEMB: libc::c_ulong = 0xc028698a;

/// sysctl MIB components for IFDATA_DRIVERNAME.
const CTL_NET: libc::c_int = 4;
const PF_LINK: libc::c_int = 18;
const NETLINK_GENERIC: libc::c_int = 0;
const IFMIB_IFDATA: libc::c_int = 2;
const IFDATA_DRIVERNAME: libc::c_int = 3;

// ── FFI structs from <net/if.h> ────────────────────────────────────────────

/// `struct ifgroupreq` — 40 bytes on amd64.
/// name[16] + len(4) + pad(4) + union{ char[16] | *ifg_req }(16).
#[repr(C)]
struct IfGroupReq {
    name: [libc::c_char; libc::IFNAMSIZ as usize],
    len: libc::c_uint,
    _pad: libc::c_uint,
    /// Union: the buffer pointer is written at offset 24.
    groups_buf: [u8; 16],
}

/// `struct ifg_req` — one group/member entry (16 bytes).
#[repr(C)]
struct IfgReq {
    name: [libc::c_char; libc::IFNAMSIZ as usize],
}

// ── Private helpers ────────────────────────────────────────────────────────

/// Open a local datagram socket suitable for interface ioctls.
fn open_iface_socket() -> Option<libc::c_int> {
    let fd = unsafe { libc::socket(libc::AF_LOCAL, libc::SOCK_DGRAM, 0) };
    if fd >= 0 { Some(fd) } else { None }
}

/// Copy an interface/group name into a fixed-size `[c_char; IFNAMSIZ]` array.
fn fill_name(buf: &mut [libc::c_char], name: &str) {
    let cname = CString::new(name).unwrap_or_default();
    let bytes = cname.as_bytes_with_nul();
    for (i, &b) in bytes.iter().take(buf.len()).enumerate() {
        buf[i] = b as libc::c_char;
    }
}

/// Parse the variable-length entry buffer returned by SIOCGIFGROUP / SIOCGIFGMEMB.
fn parse_entries(buf: &[u8]) -> Vec<String> {
    let entry_size = std::mem::size_of::<IfgReq>();
    let count = buf.len() / entry_size;
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let start = i * entry_size;
        let entry = unsafe { &*(buf.as_ptr().add(start) as *const IfgReq) };
        let name = unsafe { CStr::from_ptr(entry.name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        result.push(name);
    }
    result
}

/// Common two-call ioctl pattern for SIOCGIFGROUP / SIOCGIFGMEMB.
fn ioctl_group_list(fd: libc::c_int, ioctl_num: libc::c_ulong, name: &str) -> Vec<String> {
    let mut req = IfGroupReq {
        name: [0; libc::IFNAMSIZ as usize],
        len: 0,
        _pad: 0,
        groups_buf: [0u8; 16],
    };
    fill_name(&mut req.name, name);

    // First call: get required buffer length.
    let rc = unsafe { libc::ioctl(fd, ioctl_num, &mut req as *mut _ as *mut c_void) };
    if rc != 0 || req.len == 0 {
        return Vec::new();
    }

    let len = req.len as usize;
    let entry_size = std::mem::size_of::<IfgReq>();
    if len % entry_size != 0 || len / entry_size > 256 {
        return Vec::new();
    }

    let mut buf = vec![0u8; len];
    req.len = len as libc::c_uint;
    // Write the buffer pointer at union offset (24).
    unsafe {
        std::ptr::write(
            (&mut req as *mut IfGroupReq).cast::<u8>().add(24) as *mut *mut c_void,
            buf.as_mut_ptr() as *mut c_void,
        );
    }

    // Second call: fill buffer.
    let rc = unsafe { libc::ioctl(fd, ioctl_num, &mut req as *mut _ as *mut c_void) };
    if rc != 0 {
        return Vec::new();
    }

    parse_entries(&buf)
}

// ── Public API ─────────────────────────────────────────────────────────────

/// List all interface groups that `ifname` belongs to.
/// Uses `SIOCGIFGROUP` ioctl.
pub fn list_interface_groups(ifname: &str) -> Vec<String> {
    let fd = match open_iface_socket() {
        Some(fd) => fd,
        None => return Vec::new(),
    };
    let result = ioctl_group_list(fd, SIOCGIFGROUP, ifname);
    unsafe { libc::close(fd) };
    result
}

/// List all network interfaces that are members of `group`.
/// Uses `SIOCGIFGMEMB` ioctl.
pub fn list_group_members(group: &str) -> Vec<String> {
    let fd = match open_iface_socket() {
        Some(fd) => fd,
        None => return Vec::new(),
    };
    let result = ioctl_group_list(fd, SIOCGIFGMEMB, group);
    unsafe { libc::close(fd) };
    result
}

/// Get the original driver-assigned name of a network interface via sysctl
/// (CTL_NET, PF_LINK, NETLINK_GENERIC, IFMIB_IFDATA, ifindex, IFDATA_DRIVERNAME).
/// Works even if the interface was renamed by the user.
pub fn get_drivername(ifname: &str) -> Option<String> {
    let cname = CString::new(ifname).ok()?;
    let ifindex = unsafe { libc::if_nametoindex(cname.as_ptr()) };
    if ifindex == 0 {
        return None;
    }
    let mib: [libc::c_int; 6] = [
        CTL_NET,
        PF_LINK,
        NETLINK_GENERIC,
        IFMIB_IFDATA,
        ifindex as libc::c_int,
        IFDATA_DRIVERNAME,
    ];
    let mut len = 0usize;
    let rc = unsafe {
        libc::sysctl(
            mib.as_ptr(),
            6,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let rc = unsafe {
        libc::sysctl(
            mib.as_ptr(),
            6,
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            std::ptr::null(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(buf.as_ptr() as *const libc::c_char) };
    cstr.to_str().ok().map(|s| s.to_string())
}
