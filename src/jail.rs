//! libjail FFI bindings and safe wrappers.
//!
//! All `unsafe` code is contained in the `sys` submodule. The public API
//! exposes `JailParams` (RAII) and two convenience functions: `list_jails`
//! and `get_jail`.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_void;

use libc::{c_char, c_int, c_uint, size_t};

// ── FFI declarations ──────────────────────────────────────────────

mod sys {
    use super::*;

    pub const JAIL_DYING: c_int = 0x08;
    pub const JAIL_CREATE: c_int = 0x01;
    pub const JAIL_UPDATE: c_int = 0x02;
    pub const JAIL_ATTACH: c_int = 0x04;

    #[repr(C)]
    #[derive(Clone)]
    pub struct Jailparam {
        pub jp_name: *mut c_char,
        pub jp_value: *mut c_void,
        pub jp_valuelen: size_t,
        pub jp_elemlen: size_t,
        pub jp_ctltype: c_int,
        pub jp_structtype: c_int,
        pub jp_flags: c_uint,
    }

    extern "C" {
        pub fn jailparam_all(jpp: *mut *mut Jailparam) -> c_int;
        pub fn jailparam_init(jp: *mut Jailparam, name: *const c_char) -> c_int;
        pub fn jailparam_import(jp: *mut Jailparam, value: *const c_char) -> c_int;
        pub fn jailparam_set(jp: *mut Jailparam, njp: c_uint, flags: c_int) -> c_int;
        pub fn jailparam_get(jp: *mut Jailparam, njp: c_uint, flags: c_int) -> c_int;
        pub fn jailparam_export(jp: *const Jailparam) -> *mut c_char;
        pub fn jailparam_free(jp: *mut Jailparam, njp: c_uint);
        pub fn jail_remove(jid: c_int) -> c_int;
        pub static jail_errmsg: [c_char; 1024];
    }
}

/// Read the global `jail_errmsg` as a Rust `String`.
fn errmsg() -> String {
    unsafe {
        let ptr = sys::jail_errmsg.as_ptr();
        if ptr.is_null() {
            return String::new();
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

// ── RAII wrapper ──────────────────────────────────────────────────

/// Owned array of `jailparam` structs that calls `jailparam_free` on drop.
pub struct JailParams {
    params: Vec<sys::Jailparam>,
}

impl JailParams {
    /// Create from the system's full set of known jail parameters
    /// (`jailparam_all`).
    pub fn all() -> Result<Self, String> {
        unsafe {
            let mut raw: *mut sys::Jailparam = std::ptr::null_mut();
            let count = sys::jailparam_all(&mut raw);
            if count < 0 {
                return Err(errmsg());
            }
            // Move into a Vec so Drop can free it.
            let slice = std::slice::from_raw_parts(raw, count as usize);
            let params: Vec<_> = slice.to_vec();
            libc::free(raw as *mut c_void);
            Ok(JailParams { params })
        }
    }

    /// Create from a fixed list of parameter names.
    pub fn from_names(names: &[&str]) -> Result<Self, String> {
        let mut params: Vec<sys::Jailparam> = Vec::with_capacity(names.len());
        for name in names {
            let mut jp: sys::Jailparam = unsafe { std::mem::zeroed() };
            let cname = CString::new(*name).unwrap();
            let rc = unsafe { sys::jailparam_init(&mut jp, cname.as_ptr()) };
            if rc != 0 {
                // Free what we have so far.
                if !params.is_empty() {
                    unsafe { sys::jailparam_free(params.as_mut_ptr(), params.len() as u32) };
                }
                return Err(errmsg());
            }
            params.push(jp);
        }
        Ok(JailParams { params })
    }

    /// Import a string value into the named parameter.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
        let idx = self.find(name).ok_or_else(|| format!("parameter \"{name}\" not in set"))?;
        let cval = CString::new(value).unwrap();
        let rc = unsafe { sys::jailparam_import(&mut self.params[idx], cval.as_ptr()) };
        if rc != 0 {
            return Err(errmsg());
        }
        Ok(())
    }

    /// Call `jailparam_get`. Returns the JID, or -1 on failure / no more jails.
    pub fn query(&mut self, flags: c_int) -> c_int {
        unsafe {
            sys::jailparam_get(
                self.params.as_mut_ptr(),
                self.params.len() as u32,
                flags,
            )
        }
    }

    /// Export all parameters as a `HashMap<String, String>`.
    pub fn export_all(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for jp in &self.params {
            if jp.jp_name.is_null() {
                continue;
            }
            let key = unsafe { CStr::from_ptr(jp.jp_name) }
                .to_string_lossy()
                .into_owned();
            let val = unsafe {
                let ptr = sys::jailparam_export(jp);
                if ptr.is_null() {
                    String::new()
                } else {
                    let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
                    libc::free(ptr as *mut c_void);
                    s
                }
            };
            map.insert(key, val);
        }
        map
    }

    /// Index of a named parameter, or `None`.
    fn find(&self, name: &str) -> Option<usize> {
        for (i, jp) in self.params.iter().enumerate() {
            if jp.jp_name.is_null() {
                continue;
            }
            let pname = unsafe { CStr::from_ptr(jp.jp_name) };
            if pname.to_string_lossy() == name {
                return Some(i);
            }
        }
        None
    }
}

impl Drop for JailParams {
    fn drop(&mut self) {
        if !self.params.is_empty() {
            unsafe { sys::jailparam_free(self.params.as_mut_ptr(), self.params.len() as u32) };
        }
    }
}

// ── High-level API ────────────────────────────────────────────────

/// Parameters queried for each jail in `list_jails`.
const LIST_PARAM_NAMES: &[&str] = &[
    "jid",
    "name",
    "host.hostname",
    "path",
    "ip4.addr",
    "ip6.addr",
    "persist",
    "dying",
    "lastjid",
];

/// List all running jails (including dying ones).
///
/// Returns a vector of parameter maps, one per jail.
pub fn list_jails() -> Result<Vec<HashMap<String, String>>, String> {
    let mut params = JailParams::from_names(LIST_PARAM_NAMES)?;
    let mut result = Vec::new();
    let mut lastjid: i32 = 0;

    loop {
        params.set("lastjid", &lastjid.to_string())?;
        let jid = params.query(sys::JAIL_DYING);
        if jid < 0 {
            break; // No more jails.
        }
        let mut map = params.export_all();
        map.remove("lastjid"); // Internal iteration key — don't expose.
        result.push(map);
        lastjid = jid;
    }

    Ok(result)
}

/// Get all parameters for a single jail identified by name (or numeric JID).
///
/// Returns `Ok(None)` if the jail does not exist.
pub fn get_jail(name: &str) -> Result<Option<HashMap<String, String>>, String> {
    let mut params = JailParams::all()?;
    params.set("name", name)?;
    let jid = params.query(sys::JAIL_DYING);
    if jid < 0 {
        // jail_get sets errno=ENOENT when the jail doesn't exist.
        let errno = unsafe { *libc::__error() };
        if errno == libc::ENOENT {
            return Ok(None);
        }
        return Err(errmsg());
    }
    Ok(Some(params.export_all()))
}

/// Start a jail by reading its definition from /etc/jail.conf via the
/// `jail(8)` command. libjail's `jailparam_set` requires assembling all
/// parameters manually, but `jail -c` handles fstab, exec.start, mount.devfs,
/// and global defaults automatically.
pub fn start_jail(name: &str) -> Result<(), String> {
    let output = std::process::Command::new("/usr/sbin/jail")
        .args(["-q", "-c", name])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            errmsg()
        } else {
            stderr
        });
    }
    Ok(())
}

/// Stop a running jail using `jail -r`.
pub fn stop_jail(name: &str) -> Result<(), String> {
    let output = std::process::Command::new("/usr/sbin/jail")
        .args(["-q", "-r", name])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            errmsg()
        } else {
            stderr
        });
    }
    Ok(())
}

/// Check if a jail is currently running.
pub fn is_jail_running(name: &str) -> bool {
    get_jail(name).map(|opt| opt.is_some()).unwrap_or(false)
}
