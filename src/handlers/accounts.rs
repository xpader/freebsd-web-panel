//! System account management — list, create, modify, and delete FreeBSD
//! users and groups.
//!
//! Read listings parse `/etc/passwd` and `/etc/group` directly. All write
//! operations (create/modify/delete) go through the `pw(8)` command with
//! validated arguments — never shell interpolation. The panel runs as root,
//! so it has the privileges `pw` requires.

use std::collections::HashMap;
use std::fs;
use std::sync::LazyLock;

use axum::extract::{Path as AxumPath, Query, State};
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::cmd;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

const PASSWD_PATH: &str = "/etc/passwd";
const GROUP_PATH: &str = "/etc/group";
const SHELLS_PATH: &str = "/etc/shells";
const MASTER_PASSWD_PATH: &str = "/etc/master.passwd";
const PW: &str = "/usr/sbin/pw";

/// Accounts with uid/gid below this are system accounts (root, daemon, bin,
/// wheel, operator, …) that ship with the OS. They are protected from
/// deletion and from changes to their identity fields (login name, uid, gid).
const SYSTEM_ID_BOUNDARY: u32 = 1000;

/// Login and group names: must start with a letter, digit, or underscore;
/// may then contain letters, digits, `_`, `.`, `-`; max 32 chars. The leading
/// character restriction also prevents names beginning with `-`, which `pw`
/// would otherwise misread as a flag.
static RE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_][a-zA-Z0-9_.-]{0,31}$").unwrap());

#[derive(Debug, Serialize)]
pub struct SystemUser {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub gecos: String,
    pub home: String,
    pub shell: String,
    pub group_name: Option<String>,
    /// Supplementary group memberships (groups where this user is listed as
    /// a member in `/etc/group`, excluding the primary group).
    pub groups: Vec<String>,
    /// Whether the account is locked (password prefixed `*LOCKED*` in
    /// /etc/master.passwd).
    pub locked: bool,
}

#[derive(Debug, Serialize)]
pub struct SystemGroup {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

/// GET /api/accounts/users — list system users from /etc/passwd, sorted by uid.
pub async fn list_users() -> ApiResult<Json<Vec<SystemUser>>> {
    let passwd = fs::read_to_string(PASSWD_PATH)
        .map_err(|e| ApiError::Internal(format!("read /etc/passwd: {e}")))?;
    let group_map = read_group_map()?;
    let supp_map = read_supp_group_map()?;
    let locked_map = read_locked_map();

    let mut users: Vec<SystemUser> = passwd
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| parse_passwd_line(line, &group_map, &supp_map, &locked_map))
        .collect();

    users.sort_by_key(|u| u.uid);
    Ok(Json(users))
}

fn parse_passwd_line(
    line: &str,
    group_map: &HashMap<u32, String>,
    supp_map: &HashMap<String, Vec<String>>,
    locked_map: &HashMap<String, bool>,
) -> Option<SystemUser> {
    let fields: Vec<&str> = line.splitn(7, ':').collect();
    if fields.len() < 7 {
        return None;
    }
    let uid = fields[2].parse::<u32>().ok()?;
    let gid = fields[3].parse::<u32>().ok()?;
    let name = fields[0].to_string();
    let groups = supp_map.get(&name).cloned().unwrap_or_default();
    let locked = *locked_map.get(&name).unwrap_or(&false);
    Some(SystemUser {
        name,
        uid,
        gid,
        gecos: fields[4].to_string(),
        home: fields[5].to_string(),
        shell: fields[6].trim_end().to_string(),
        group_name: group_map.get(&gid).cloned(),
        groups,
        locked,
    })
}

/// Build a username → locked lookup from /etc/master.passwd. An account is
/// locked when its password field begins with `*LOCKED*`. Returns an empty
/// map if master.passwd is unreadable (never errors — locking is informational).
fn read_locked_map() -> HashMap<String, bool> {
    let Ok(content) = fs::read_to_string(MASTER_PASSWD_PATH) else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    for line in content.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(2, ':');
        let Some(name) = fields.next() else { continue };
        let Some(pw_field) = fields.next() else { continue };
        if pw_field.starts_with("*LOCKED*") {
            map.insert(name.to_string(), true);
        }
    }
    map
}

/// Build a gid → group-name lookup from /etc/group.
fn read_group_map() -> ApiResult<HashMap<u32, String>> {
    let content = fs::read_to_string(GROUP_PATH)
        .map_err(|e| ApiError::Internal(format!("read /etc/group: {e}")))?;
    let mut map = HashMap::new();
    for line in content.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.splitn(4, ':').collect();
        if fields.len() < 3 {
            continue;
        }
        if let Ok(gid) = fields[2].parse::<u32>() {
            map.insert(gid, fields[0].to_string());
        }
    }
    Ok(map)
}

/// Build a username → [supplementary group names] lookup from /etc/group.
fn read_supp_group_map() -> ApiResult<HashMap<String, Vec<String>>> {
    let content = fs::read_to_string(GROUP_PATH)
        .map_err(|e| ApiError::Internal(format!("read /etc/group: {e}")))?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for line in content.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.splitn(4, ':').collect();
        if fields.len() < 4 || fields[3].is_empty() {
            continue;
        }
        let gname = fields[0];
        for member in fields[3].split(',') {
            let m = member.trim();
            if !m.is_empty() {
                map.entry(m.to_string()).or_default().push(gname.to_string());
            }
        }
    }
    Ok(map)
}

/// GET /api/accounts/groups — list system groups from /etc/group, sorted by gid.
pub async fn list_groups() -> ApiResult<Json<Vec<SystemGroup>>> {
    let content = fs::read_to_string(GROUP_PATH)
        .map_err(|e| ApiError::Internal(format!("read /etc/group: {e}")))?;

    let mut groups: Vec<SystemGroup> = content
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let fields: Vec<&str> = line.splitn(4, ':').collect();
            if fields.len() < 3 {
                return None;
            }
            let gid = fields[2].parse::<u32>().ok()?;
            let members = if fields.len() >= 4 && !fields[3].is_empty() {
                fields[3].split(',').map(|s| s.to_string()).collect()
            } else {
                Vec::new()
            };
            Some(SystemGroup {
                name: fields[0].to_string(),
                gid,
                members,
            })
        })
        .collect();

    groups.sort_by_key(|g| g.gid);
    Ok(Json(groups))
}

/// GET /api/accounts/shells — valid login shells from /etc/shells.
pub async fn list_shells() -> ApiResult<Json<Vec<String>>> {
    Ok(Json(read_shells_list()))
}

fn read_shells_list() -> Vec<String> {
    let mut shells: Vec<String> = fs::read_to_string(SHELLS_PATH)
        .map(|c| {
            c.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();

    // Restricted shells like nologin are deliberately absent from /etc/shells
    // but are valid choices for service accounts. Append the ones that exist.
    for nologin in ["/usr/sbin/nologin", "/sbin/nologin"] {
        if fs::metadata(nologin).is_ok() && !shells.iter().any(|s| s == nologin) {
            shells.push(nologin.to_string());
        }
    }

    shells
}

// ── validation helpers ──────────────────────────────────────────────

fn validate_name(name: &str) -> ApiResult<()> {
    if !RE_NAME.is_match(name) {
        return Err(ApiError::BadRequest(
            "Invalid name: allowed letters, digits, _, ., -; must start with a letter, \
             digit, or _; max 32 chars"
                .into(),
        ));
    }
    Ok(())
}

fn validate_gecos(s: &str) -> ApiResult<()> {
    if s.contains(':') || s.contains('\n') || s.contains('\0') {
        return Err(ApiError::BadRequest("Invalid comment field".into()));
    }
    if s.chars().count() > 256 {
        return Err(ApiError::BadRequest("Comment too long (max 256 chars)".into()));
    }
    Ok(())
}

fn validate_path_str(p: &str) -> ApiResult<()> {
    if !p.starts_with('/') {
        return Err(ApiError::BadRequest("Path must be absolute".into()));
    }
    if p.contains('\0') || p.contains('\n') {
        return Err(ApiError::BadRequest("Invalid path".into()));
    }
    Ok(())
}

fn validate_shell(shell: &str) -> ApiResult<()> {
    if shell.is_empty() {
        return Ok(());
    }
    let shells = read_shells_list();
    if !shells.iter().any(|s| s == shell) {
        return Err(ApiError::BadRequest(format!(
            "Invalid shell '{shell}': not listed in /etc/shells"
        )));
    }
    Ok(())
}

/// Accept a group reference as either a numeric gid or a group name.
fn validate_group_ref(s: &str) -> ApiResult<()> {
    if s.is_empty() {
        return Ok(());
    }
    if s.parse::<u32>().is_ok() {
        return Ok(());
    }
    validate_name(s)
}

fn validate_uid(uid: u32) -> ApiResult<()> {
    if uid == 0 {
        return Err(ApiError::BadRequest("UID 0 (root) is reserved".into()));
    }
    Ok(())
}

/// Look up a user's uid by name from /etc/passwd.
fn lookup_uid(name: &str) -> ApiResult<Option<u32>> {
    let passwd = fs::read_to_string(PASSWD_PATH)
        .map_err(|e| ApiError::Internal(format!("read /etc/passwd: {e}")))?;
    for line in passwd.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.splitn(7, ':').collect();
        if f.len() >= 3 && f[0] == name {
            return Ok(f[2].parse::<u32>().ok());
        }
    }
    Ok(None)
}

/// Look up a group's gid by name from /etc/group.
fn lookup_gid(name: &str) -> ApiResult<Option<u32>> {
    let content = fs::read_to_string(GROUP_PATH)
        .map_err(|e| ApiError::Internal(format!("read /etc/group: {e}")))?;
    for line in content.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.splitn(4, ':').collect();
        if f.len() >= 3 && f[0] == name {
            return Ok(f[2].parse::<u32>().ok());
        }
    }
    Ok(None)
}

/// Build a `Vec<&str>` view of a `Vec<String>` for `cmd::run_sync`.
fn argv(a: &[String]) -> Vec<&str> {
    a.iter().map(String::as_str).collect()
}

// ── user write endpoints ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateUserReq {
    pub name: String,
    pub uid: Option<u32>,
    /// Primary group: group name or numeric gid.
    pub gid: Option<String>,
    pub gecos: Option<String>,
    pub home: Option<String>,
    pub shell: Option<String>,
    /// Supplementary group memberships.
    pub groups: Option<Vec<String>>,
    pub password: Option<String>,
    /// Create the home directory and populate it from the skel dir.
    #[serde(default)]
    pub create_home: bool,
}

/// POST /api/accounts/users — create a system user via `pw useradd`.
pub async fn create_user(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateUserReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = req.name.trim().to_string();
    validate_name(&name)?;
    if name == "root" {
        return Err(ApiError::BadRequest("Cannot create user 'root'".into()));
    }
    let gecos = req.gecos.as_deref().unwrap_or("").trim().to_string();
    validate_gecos(&gecos)?;
    if let Some(uid) = req.uid {
        validate_uid(uid)?;
    }
    if let Some(gid) = &req.gid {
        validate_group_ref(gid)?;
    }
    if let Some(home) = &req.home {
        validate_path_str(home)?;
    }
    if let Some(shell) = &req.shell {
        validate_shell(shell)?;
    }
    let groups = req.groups.clone().unwrap_or_default();
    for g in &groups {
        validate_group_ref(g)?;
    }

    if lookup_uid(&name)?.is_some() {
        return Err(ApiError::Conflict(format!("User '{name}' already exists")));
    }

    let auth_user = user.username.clone();
    let password = req.password.clone().filter(|p| !p.is_empty());

    // Argument construction is cheap/synchronous; only the fork+exec needs
    // to run off the async worker.
    let mut args: Vec<String> = vec!["useradd".into(), "-n".into(), name.clone()];
    if let Some(uid) = req.uid {
        args.push("-u".into());
        args.push(uid.to_string());
    }
    if let Some(gid) = &req.gid {
        args.push("-g".into());
        args.push(gid.clone());
    }
    if !groups.is_empty() {
        args.push("-G".into());
        args.push(groups.join(","));
    }
    if !gecos.is_empty() {
        args.push("-c".into());
        args.push(gecos);
    }
    if let Some(home) = &req.home {
        args.push("-d".into());
        args.push(home.clone());
    }
    if let Some(shell) = &req.shell {
        args.push("-s".into());
        args.push(shell.clone());
    }
    if req.create_home {
        args.push("-m".into());
    }

    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        if let Some(pw) = &password {
            let mut a = args.clone();
            a.push("-h".into());
            a.push("0".into());
            cmd::run_sync_stdin(PW, &argv(&a), format!("{pw}\n").as_bytes())?;
        } else {
            cmd::run_sync(PW, &argv(&args))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth_user),
        "POST",
        "/api/accounts/users",
        200,
        Some(format!("Created system user '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserReq {
    /// Rename the user (login name).
    pub new_name: Option<String>,
    pub uid: Option<u32>,
    pub gid: Option<String>,
    pub gecos: Option<String>,
    pub home: Option<String>,
    pub shell: Option<String>,
    /// When `Some`, replace the full supplementary-group set.
    pub groups: Option<Vec<String>>,
    pub password: Option<String>,
    /// `true` → lock the account, `false` → unlock.
    pub locked: Option<bool>,
}

/// PUT /api/accounts/users/{name} — modify a system user via `pw usermod`
/// (and `pw lock`/`pw unlock`).
pub async fn update_user(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
    Json(req): Json<UpdateUserReq>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_name(&name)?;
    let uid = lookup_uid(&name)?
        .ok_or_else(|| ApiError::NotFound(format!("User '{name}' not found")))?;

    let new_name = req.new_name.as_deref().map(|s| s.trim().to_string());
    let will_rename = new_name.as_ref().map_or(false, |nn| nn != &name);
    if will_rename {
        validate_name(new_name.as_deref().unwrap())?;
        if new_name.as_deref() == Some("root") {
            return Err(ApiError::BadRequest("Cannot rename to 'root'".into()));
        }
        if lookup_uid(new_name.as_deref().unwrap())?.is_some() {
            return Err(ApiError::Conflict("Target name already in use".into()));
        }
    }
    if let Some(u) = req.uid {
        validate_uid(u)?;
    }
    // System accounts (uid < 1000) are protected from identity changes:
    // login name, uid, and primary group must not be altered.
    if uid < SYSTEM_ID_BOUNDARY {
        if will_rename {
            return Err(ApiError::BadRequest(
                "Cannot rename a system account".into(),
            ));
        }
        if req.uid.is_some() {
            return Err(ApiError::BadRequest(
                "Cannot change the uid of a system account".into(),
            ));
        }
        if req.gid.is_some() {
            return Err(ApiError::BadRequest(
                "Cannot change the primary group of a system account".into(),
            ));
        }
    }
    if let Some(g) = &req.gid {
        validate_group_ref(g)?;
    }
    let gecos = req.gecos.as_deref().map(|s| s.trim().to_string());
    if let Some(g) = &gecos {
        validate_gecos(g)?;
    }
    if let Some(h) = &req.home {
        validate_path_str(h)?;
    }
    if let Some(s) = &req.shell {
        validate_shell(s)?;
    }
    let groups = req.groups.clone().unwrap_or_default();
    for g in &groups {
        validate_group_ref(g)?;
    }
    let set_groups = req.groups.is_some();
    let password = req.password.clone().filter(|p| !p.is_empty());

    let auth_user = user.username.clone();
    let locked = req.locked;
    let is_root = uid == 0;
    let detail = if will_rename {
        format!(
            "Renamed system user '{}' → '{}'",
            name,
            new_name.as_deref().unwrap_or(&name)
        )
    } else {
        format!("Modified system user '{name}'")
    };

    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        let mut args: Vec<String> = vec!["usermod".into(), "-n".into(), name.clone()];
        let mut any = false;
        if will_rename {
            args.push("-l".into());
            args.push(new_name.clone().unwrap());
            any = true;
        }
        if let Some(u) = req.uid {
            args.push("-u".into());
            args.push(u.to_string());
            any = true;
        }
        if let Some(g) = &req.gid {
            args.push("-g".into());
            args.push(g.clone());
            any = true;
        }
        if let Some(g) = &gecos {
            args.push("-c".into());
            args.push(g.clone());
            any = true;
        }
        if let Some(h) = &req.home {
            args.push("-d".into());
            args.push(h.clone());
            any = true;
        }
        if let Some(s) = &req.shell {
            args.push("-s".into());
            args.push(s.clone());
            any = true;
        }
        if set_groups {
            args.push("-G".into());
            args.push(if groups.is_empty() {
                "".to_string()
            } else {
                groups.join(",")
            });
            any = true;
        }

        let effective = if will_rename {
            new_name.clone().unwrap_or_else(|| name.clone())
        } else {
            name.clone()
        };

        if any || password.is_some() {
            if let Some(pw) = &password {
                let mut a = args.clone();
                a.push("-h".into());
                a.push("0".into());
                cmd::run_sync_stdin(PW, &argv(&a), format!("{pw}\n").as_bytes())?;
            } else {
                cmd::run_sync(PW, &argv(&args))?;
            }
        }

        if let Some(lock) = locked {
            // Guard root against being locked out.
            if is_root && lock {
                return Err(ApiError::BadRequest(
                    "Refusing to lock the root account".into(),
                ));
            }
            // pw lock/unlock error if the account is already in the target
            // state ("pw: user X is not locked"). Only toggle when the state
            // actually changes.
            let currently_locked = read_locked_map().get(&effective).copied().unwrap_or(false);
            if lock && !currently_locked {
                cmd::run_sync(PW, &["lock", &effective])?;
            } else if !lock && currently_locked {
                cmd::run_sync(PW, &["unlock", &effective])?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth_user),
        "PUT",
        "/api/accounts/users",
        200,
        Some(detail),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Debug, Deserialize)]
pub struct RemoveHomeQuery {
    #[serde(default)]
    pub remove_home: bool,
}

/// DELETE /api/accounts/users/{name} — delete a system user via `pw userdel`.
/// Pass `?remove_home=true` to also remove the home directory (`pw userdel -r`).
pub async fn delete_user(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
    Query(q): Query<RemoveHomeQuery>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_name(&name)?;
    let uid = lookup_uid(&name)?
        .ok_or_else(|| ApiError::NotFound(format!("User '{name}' not found")))?;
    if uid < SYSTEM_ID_BOUNDARY {
        return Err(ApiError::BadRequest(
            "Refusing to delete a system account (uid < 1000)".into(),
        ));
    }

    let auth_user = user.username.clone();
    let remove_home = q.remove_home;
    let target = name.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        let mut args = vec!["userdel".to_string(), "-n".to_string(), target.clone()];
        if remove_home {
            args.push("-r".into());
        }
        cmd::run_sync(PW, &argv(&args))?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth_user),
        "DELETE",
        "/api/accounts/users",
        200,
        Some(format!("Deleted system user '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

// ── group write endpoints ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateGroupReq {
    pub name: String,
    pub gid: Option<u32>,
    /// Initial member list (user names).
    pub members: Option<Vec<String>>,
}

/// POST /api/accounts/groups — create a system group via `pw groupadd`.
pub async fn create_group(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateGroupReq>,
) -> ApiResult<Json<serde_json::Value>> {
    let name = req.name.trim().to_string();
    validate_name(&name)?;
    if let Some(gid) = req.gid {
        if gid == 0 {
            return Err(ApiError::BadRequest("GID 0 (wheel) is reserved".into()));
        }
    }
    let members = req.members.clone().unwrap_or_default();
    for m in &members {
        validate_group_ref(m)?;
    }
    if lookup_gid(&name)?.is_some() {
        return Err(ApiError::Conflict(format!("Group '{name}' already exists")));
    }

    let auth_user = user.username.clone();
    let mut args: Vec<String> = vec!["groupadd".into(), "-n".into(), name.clone()];
    if let Some(gid) = req.gid {
        args.push("-g".into());
        args.push(gid.to_string());
    }
    if !members.is_empty() {
        args.push("-M".into());
        args.push(members.join(","));
    }

    let arg_owned = args.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        cmd::run_sync(PW, &argv(&arg_owned))?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth_user),
        "POST",
        "/api/accounts/groups",
        200,
        Some(format!("Created system group '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

#[derive(Debug, Deserialize)]
pub struct UpdateGroupReq {
    /// Rename the group.
    pub new_name: Option<String>,
    pub gid: Option<u32>,
    /// When `Some`, replace the full member list.
    pub members: Option<Vec<String>>,
}

/// PUT /api/accounts/groups/{name} — modify a system group via `pw groupmod`.
pub async fn update_group(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
    Json(req): Json<UpdateGroupReq>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_name(&name)?;
    let gid = lookup_gid(&name)?
        .ok_or_else(|| ApiError::NotFound(format!("Group '{name}' not found")))?;

    let new_name = req.new_name.as_deref().map(|s| s.trim().to_string());
    let will_rename = new_name.as_ref().map_or(false, |nn| nn != &name);
    if will_rename {
        validate_name(new_name.as_deref().unwrap())?;
        if lookup_gid(new_name.as_deref().unwrap())?.is_some() {
            return Err(ApiError::Conflict("Target group name already in use".into()));
        }
    }
    if let Some(g) = req.gid {
        if g == 0 {
            return Err(ApiError::BadRequest("GID 0 (wheel) is reserved".into()));
        }
    }
    // System groups (gid < 1000) are protected from identity changes:
    // group name and gid must not be altered.
    if gid < SYSTEM_ID_BOUNDARY {
        if will_rename {
            return Err(ApiError::BadRequest(
                "Cannot rename a system group".into(),
            ));
        }
        if req.gid.is_some() {
            return Err(ApiError::BadRequest(
                "Cannot change the gid of a system group".into(),
            ));
        }
    }
    let members = req.members.clone().unwrap_or_default();
    for m in &members {
        validate_group_ref(m)?;
    }
    let set_members = req.members.is_some();

    let auth_user = user.username.clone();
    let is_wheel = gid == 0;
    let detail = if will_rename {
        format!(
            "Renamed system group '{}' → '{}'",
            name,
            new_name.as_deref().unwrap_or(&name)
        )
    } else {
        format!("Modified system group '{name}'")
    };
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        if is_wheel && will_rename {
            return Err(ApiError::BadRequest(
                "Refusing to rename the wheel group".into(),
            ));
        }
        let mut args: Vec<String> = vec!["groupmod".into(), "-n".into(), name.clone()];
        let mut any = false;
        if will_rename {
            args.push("-l".into());
            args.push(new_name.clone().unwrap());
            any = true;
        }
        if let Some(g) = req.gid {
            args.push("-g".into());
            args.push(g.to_string());
            any = true;
        }
        if set_members {
            args.push("-M".into());
            args.push(if members.is_empty() {
                "".to_string()
            } else {
                members.join(",")
            });
            any = true;
        }
        if any {
            cmd::run_sync(PW, &argv(&args))?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth_user),
        "PUT",
        "/api/accounts/groups",
        200,
        Some(detail),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}

/// DELETE /api/accounts/groups/{name} — delete a system group via `pw groupdel`.
pub async fn delete_group(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath(name): AxumPath<String>,
) -> ApiResult<Json<serde_json::Value>> {
    validate_name(&name)?;
    let gid = lookup_gid(&name)?
        .ok_or_else(|| ApiError::NotFound(format!("Group '{name}' not found")))?;
    if gid < SYSTEM_ID_BOUNDARY {
        return Err(ApiError::BadRequest(
            "Refusing to delete a system group (gid < 1000)".into(),
        ));
    }

    let auth_user = user.username.clone();
    let target = name.clone();
    tokio::task::spawn_blocking(move || -> ApiResult<()> {
        cmd::run_sync(PW, &argv(&[
            "groupdel".to_string(),
            "-n".to_string(),
            target.clone(),
        ]))?;
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))??;

    audit::record(
        &state,
        Some(&auth_user),
        "DELETE",
        "/api/accounts/groups",
        200,
        Some(format!("Deleted system group '{name}'")),
    );
    Ok(Json(serde_json::json!({"ok": true})))
}
