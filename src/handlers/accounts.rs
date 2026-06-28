//! System account management — list FreeBSD users and groups by parsing
//! /etc/passwd and /etc/group.

use std::collections::HashMap;
use std::fs;

use axum::Json;
use serde::Serialize;

use crate::error::{ApiError, ApiResult};

const PASSWD_PATH: &str = "/etc/passwd";
const GROUP_PATH: &str = "/etc/group";

#[derive(Debug, Serialize)]
pub struct SystemUser {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub gecos: String,
    pub home: String,
    pub shell: String,
    pub group_name: Option<String>,
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

    let mut users: Vec<SystemUser> = passwd
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| parse_passwd_line(line, &group_map))
        .collect();

    users.sort_by_key(|u| u.uid);
    Ok(Json(users))
}

fn parse_passwd_line(line: &str, group_map: &HashMap<u32, String>) -> Option<SystemUser> {
    let fields: Vec<&str> = line.splitn(7, ':').collect();
    if fields.len() < 7 {
        return None;
    }
    let uid = fields[2].parse::<u32>().ok()?;
    let gid = fields[3].parse::<u32>().ok()?;
    Some(SystemUser {
        name: fields[0].to_string(),
        uid,
        gid,
        gecos: fields[4].to_string(),
        home: fields[5].to_string(),
        shell: fields[6].trim_end().to_string(),
        group_name: group_map.get(&gid).cloned(),
    })
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
