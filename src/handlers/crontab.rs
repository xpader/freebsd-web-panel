//! Crontab management — list, add, edit, delete and toggle cron entries
//! across `/etc/crontab` and every per-user tab in `/var/cron/tabs/`.
//!
//! ## Sources
//!
//! - **system** — `/etc/crontab`, the 6-field "system" format (extra `who`
//!   column naming the run-as user). Read/written directly as a file; cron
//!   picks up changes via mtime within ~1 minute.
//! - **per-user** — `/var/cron/tabs/<name>`, the 5-field "user" format
//!   (run-as user = filename). Reads scan the directory directly; writes go
//!   through `crontab -u <name> -` so cron validates the syntax and
//!   regenerates the three auto-generated header lines it owns.
//!
//! ## Preserved headers
//!
//! Each source has a fixed preamble that is never treated as an entry and is
//! rewritten verbatim:
//!
//! - `/etc/crontab`: everything up to and including the column-legend line
//!   `#minute hour mday month wday who command` (the standard FreeBSD header
//!   including SHELL=/PATH=). If the legend is absent the preamble is empty.
//! - per-user tabs: the leading `# DO NOT EDIT …`, `# (… installed on …)` and
//!   `# (Cron version …)` lines that `crontab` generates. These are detected
//!   by pattern and skipped on read; on write they are dropped because
//!   `crontab -` regenerates them.
//!
//! ## Comments
//!
//! Contiguous `#`-comment lines immediately above a task are that task's
//! "comment" (editable, newline-joined). A `#` line whose body itself parses
//! as a schedule is treated as a *disabled* task rather than a comment. Blank
//! comment lines (`#` alone) are inert separators, preserved verbatim.

use std::fs;
use std::io::Write;
use std::mem;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use once_regex::{RE_FIELD, RE_LEGEND, RE_USERNAME};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::AppState;

const CRONTAB: &str = "/usr/bin/crontab";
const ETC_CRONTAB: &str = "/etc/crontab";
const TABS_DIR: &str = "/var/cron/tabs";

/// The `@`-style schedule aliases recognised by cron.
const SPECIALS: &[&str] = &[
    "@reboot",
    "@yearly",
    "@annually",
    "@monthly",
    "@weekly",
    "@daily",
    "@midnight",
    "@hourly",
];

/// Compile a regex once (the kernel-param pattern reuses `regex`).
mod once_regex {
    use super::Regex;
    use std::sync::LazyLock;
    /// Matches the `/etc/crontab` column-legend line (tab- or space-separated).
    pub static RE_LEGEND: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^#\s*minute\s+hour\s+mday\s+month\s+wday\s+who\s+command").unwrap()
    });
    /// A run-as username (`who` column): `[a-zA-Z0-9_.-]{1,32}`.
    pub static RE_USERNAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_.-]{1,32}$").unwrap());
    /// A valid cron time field: digits and the schedule metacharacters
    /// `* / , -` only. Prose words like "Save"/"some" fail this, so a
    /// `#`-comment line whose body has ≥6 "words" is treated as a comment
    /// rather than a (nonsense) disabled task. (Month/day *names* such as
    /// `jan`/`mon` are not supported — rare, and rejected to avoid
    /// mis-parsing prose.)
    pub static RE_FIELD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[0-9*/,\-]+$").unwrap());
}

// ---- parsed entry model ---------------------------------------------------

#[derive(Debug, Clone)]
enum Parsed {
    Schedule {
        minute: String,
        hour: String,
        dom: String,
        month: String,
        dow: String,
        user: Option<String>,
        command: String,
    },
    Special {
        special: String,
        user: Option<String>,
        command: String,
    },
}

/// A logical block within a crontab body (everything after the preamble).
#[derive(Debug, Clone)]
enum Block {
    /// A line preserved verbatim: blank, env assignment, orphan comment, …
    Inert(String),
    /// An editable schedule/special entry, owning its attached comment lines.
    Entry(EntryBlock),
}

#[derive(Debug, Clone)]
struct EntryBlock {
    /// Raw comment lines (each still starting with `#`) attached to this task.
    comment_raw: Vec<String>,
    /// Absolute line index of the task line within the whole file (API handle).
    task_idx: usize,
    /// The original task line, unchanged on re-serialize.
    raw_task: String,
    parsed: Parsed,
    disabled: bool,
}

#[derive(Debug, Serialize)]
pub struct CronEntry {
    /// `"system"` (→ /etc/crontab) or a username (→ /var/cron/tabs/<name>).
    pub source: String,
    /// Task line index within that source (handle for edit/delete).
    pub line: usize,
    /// `"schedule"` (5/6 fields) or `"special"` (@-alias).
    pub kind: String,
    pub minute: Option<String>,
    pub hour: Option<String>,
    pub dom: Option<String>,
    pub month: Option<String>,
    pub dow: Option<String>,
    pub special: Option<String>,
    /// Run-as user (`who` column); present only for system-format entries.
    pub user: Option<String>,
    pub command: String,
    /// Attached comments, newline-joined (editable).
    pub comment: String,
    pub disabled: bool,
    /// True for FreeBSD's built-in `/etc/crontab` tasks (save-entropy,
    /// newsyslog, periodic *, adjkerntz) — display hint, not enforced.
    pub system_task: bool,
}

// ---- parsing --------------------------------------------------------------

/// Try to classify a (non-`#`, non-blank) line as a cron entry.
fn classify(content: &str, is_system: bool) -> Option<Parsed> {
    let fields: Vec<&str> = content.split_whitespace().collect();
    let first = fields.first()?;

    if SPECIALS.contains(first) {
        if is_system {
            // @alias user command...
            if fields.len() < 3 {
                return None;
            }
            Some(Parsed::Special {
                special: first.to_string(),
                user: Some(fields[1].to_string()),
                command: fields[2..].join(" "),
            })
        } else {
            if fields.len() < 2 {
                return None;
            }
            Some(Parsed::Special {
                special: first.to_string(),
                user: None,
                command: fields[1..].join(" "),
            })
        }
    } else {
        if is_env_var(content) {
            return None;
        }
        let need = if is_system { 7 } else { 6 };
        if fields.len() < need {
            return None;
        }
        // The five time fields must look like real cron fields; this rejects
        // prose (e.g. "# Save some entropy …") that merely happens to have
        // enough whitespace-separated tokens.
        if !fields[..5].iter().all(|f| RE_FIELD.is_match(f)) {
            return None;
        }
        let (minute, hour, dom, month, dow) = (
            fields[0].to_string(),
            fields[1].to_string(),
            fields[2].to_string(),
            fields[3].to_string(),
            fields[4].to_string(),
        );
        if is_system {
            Some(Parsed::Schedule {
                minute,
                hour,
                dom,
                month,
                dow,
                user: Some(fields[5].to_string()),
                command: fields[6..].join(" "),
            })
        } else {
            Some(Parsed::Schedule {
                minute,
                hour,
                dom,
                month,
                dow,
                user: None,
                command: fields[5..].join(" "),
            })
        }
    }
}

/// Detect a `NAME=VALUE` crontab environment assignment.
fn is_env_var(content: &str) -> bool {
    let Some(eq) = content.find('=') else {
        return false;
    };
    let name = content[..eq].trim();
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether an entry is one of FreeBSD's stock `/etc/crontab` tasks
/// (save-entropy, newsyslog, periodic, adjkerntz). Identified by the command's
/// leading token — these are base-system binaries a user would not normally
/// schedule themselves. Only meaningful when `source == "system"`.
fn is_freebsd_builtin(source: &str, command: &str) -> bool {
    if source != "system" {
        return false;
    }
    let cmd = command.trim_start();
    if cmd.starts_with("/usr/libexec/save-entropy") {
        return true;
    }
    match cmd.split_whitespace().next() {
        Some("adjkerntz") | Some("newsyslog") | Some("periodic") => true,
        _ => false,
    }
}

/// Strip a single leading `#` and (if present) one space from a comment line.
fn strip_comment(line: &str) -> String {
    let t = line.trim_start();
    let t = t.trim_start_matches('#');
    let t = t.strip_prefix(' ').unwrap_or(t);
    t.to_string()
}

/// Flush buffered comment lines as inert blocks (orphan comments).
fn flush_comments(buf: &mut Vec<String>, blocks: &mut Vec<Block>) {
    for raw in buf.drain(..) {
        blocks.push(Block::Inert(raw));
    }
}

/// Parse the body (lines after the preamble) into ordered blocks. `offset` is
/// the preamble length so absolute line indices stay aligned with the file.
fn parse_body(rest: &[String], offset: usize, is_system: bool) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut buf: Vec<String> = Vec::new();
    for (j, raw) in rest.iter().enumerate() {
        let abs = offset + j;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            flush_comments(&mut buf, &mut blocks);
            blocks.push(Block::Inert(raw.clone()));
            continue;
        }
        if let Some(body) = raw.trim_start().strip_prefix('#') {
            let body = body.strip_prefix(' ').unwrap_or(body);
            if let Some(parsed) = classify(body, is_system) {
                // A commented-out task.
                let comments = mem::take(&mut buf);
                blocks.push(Block::Entry(EntryBlock {
                    comment_raw: comments,
                    task_idx: abs,
                    raw_task: raw.clone(),
                    parsed,
                    disabled: true,
                }));
            } else if body.trim().is_empty() {
                // Bare `#` separator — inert.
                flush_comments(&mut buf, &mut blocks);
                blocks.push(Block::Inert(raw.clone()));
            } else {
                buf.push(raw.clone());
            }
            continue;
        }
        // Not blank, not a comment.
        if let Some(parsed) = classify(raw, is_system) {
            let comments = mem::take(&mut buf);
            blocks.push(Block::Entry(EntryBlock {
                comment_raw: comments,
                task_idx: abs,
                raw_task: raw.clone(),
                parsed,
                disabled: false,
            }));
        } else {
            flush_comments(&mut buf, &mut blocks);
            blocks.push(Block::Inert(raw.clone()));
        }
    }
    flush_comments(&mut buf, &mut blocks);
    blocks
}

/// Number of leading lines that form the protected preamble.
fn preamble_len(lines: &[String], is_system: bool) -> usize {
    if is_system {
        for (i, l) in lines.iter().enumerate() {
            if RE_LEGEND.is_match(l.trim_start()) {
                return i + 1;
            }
        }
        0
    } else {
        let mut n = 0;
        for l in lines {
            let t = l.trim_start();
            if t.starts_with("# DO NOT EDIT THIS FILE") {
                n += 1;
            } else if t.starts_with("# (")
                && (t.contains("installed on") || t.contains("Cron version"))
            {
                n += 1;
            } else {
                break;
            }
        }
        n
    }
}

// ---- file IO --------------------------------------------------------------

fn source_path(source: &str) -> ApiResult<(PathBuf, bool)> {
    if source == "system" {
        Ok((PathBuf::from(ETC_CRONTAB), true))
    } else {
        validate_username(source)?;
        Ok((PathBuf::from(TABS_DIR).join(source), false))
    }
}

fn read_raw(source: &str) -> ApiResult<(String, bool)> {
    let (path, is_system) = source_path(source)?;
    match fs::read_to_string(&path) {
        Ok(c) => Ok((c, is_system)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((String::new(), is_system)),
        Err(e) => Err(ApiError::Internal(format!(
            "read {}: {}",
            path.display(),
            e
        ))),
    }
}

fn parse_source(source: &str) -> ApiResult<Vec<CronEntry>> {
    let (content, is_system) = read_raw(source)?;
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let plen = preamble_len(&lines, is_system);
    let blocks = parse_body(&lines[plen..], plen, is_system);

    let mut out = Vec::new();
    for b in &blocks {
        if let Block::Entry(e) = b {
            out.push(entry_to_cron(source, e));
        }
    }
    Ok(out)
}

fn entry_to_cron(source: &str, e: &EntryBlock) -> CronEntry {
    let comment = e
        .comment_raw
        .iter()
        .map(|l| strip_comment(l))
        .collect::<Vec<_>>()
        .join("\n");
    let (
        kind,
        minute,
        hour,
        dom,
        month,
        dow,
        special,
        user,
        command,
    ) = match &e.parsed {
        Parsed::Schedule {
            minute,
            hour,
            dom,
            month,
            dow,
            user,
            command,
        } => (
            "schedule",
            Some(minute.clone()),
            Some(hour.clone()),
            Some(dom.clone()),
            Some(month.clone()),
            Some(dow.clone()),
            None,
            user.clone(),
            command.clone(),
        ),
        Parsed::Special {
            special,
            user,
            command,
        } => (
            "special",
            None,
            None,
            None,
            None,
            None,
            Some(special.clone()),
            user.clone(),
            command.clone(),
        ),
    };
    let system_task = is_freebsd_builtin(source, &command);
    CronEntry {
        source: source.to_string(),
        line: e.task_idx,
        kind: kind.to_string(),
        minute,
        hour,
        dom,
        month,
        dow,
        special,
        user,
        command,
        comment,
        disabled: e.disabled,
        system_task,
    }
}

/// Enumerate per-user tab names (regular files, sorted). Missing dir → empty.
fn list_tab_users() -> ApiResult<Vec<String>> {
    let mut users = Vec::new();
    match fs::read_dir(TABS_DIR) {
        Ok(rd) => {
            for ent in rd {
                let ent = ent?;
                let name = ent.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') {
                    continue;
                }
                if let Ok(ft) = ent.file_type() {
                    if !ft.is_file() {
                        continue;
                    }
                }
                users.push(name.into_owned());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(ApiError::Internal(format!("read {}: {}", TABS_DIR, e))),
    }
    users.sort();
    Ok(users)
}

// ---- serialization --------------------------------------------------------

fn serialize_body(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Inert(raw) => {
                out.push_str(raw);
                out.push('\n');
            }
            Block::Entry(e) => {
                for c in &e.comment_raw {
                    out.push_str(c);
                    out.push('\n');
                }
                out.push_str(&e.raw_task);
                out.push('\n');
            }
        }
    }
    out
}

fn write_source(
    source: &str,
    is_system: bool,
    preamble: &[String],
    blocks: &[Block],
) -> ApiResult<()> {
    if is_system {
        let mut full = String::new();
        for p in preamble {
            full.push_str(p);
            full.push('\n');
        }
        full.push_str(&serialize_body(blocks));
        atomic_write(ETC_CRONTAB, &full)
    } else {
        let body = serialize_body(blocks);
        if body.trim().is_empty() {
            run_crontab_remove(source)
        } else {
            run_crontab_install(source, &body)
        }
    }
}

/// Atomically replace a system file (tmp + rename), keeping mode 0644.
fn atomic_write(path: &str, content: &str) -> ApiResult<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = format!("{path}.fwp.tmp");
    fs::write(&tmp, content)?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Install `body` as user `<name>`'s crontab via `crontab -u name -`.
/// cron validates syntax before installing and regenerates its header.
fn run_crontab_install(name: &str, body: &str) -> ApiResult<()> {
    let mut child = Command::new(CRONTAB)
        .arg("-u")
        .arg(name)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(body.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(ApiError::Command(if stderr.is_empty() {
            format!("crontab rejected input for {name}")
        } else {
            stderr
        }));
    }
    Ok(())
}

/// Remove user `<name>`'s crontab entirely (used when the last entry is gone).
fn run_crontab_remove(name: &str) -> ApiResult<()> {
    let _ = Command::new(CRONTAB)
        .arg("-u")
        .arg(name)
        .arg("-r")
        .output()?;
    Ok(())
}

// ---- request / response types --------------------------------------------

#[derive(Debug, Deserialize)]
pub struct EntryInput {
    pub minute: Option<String>,
    pub hour: Option<String>,
    pub dom: Option<String>,
    pub month: Option<String>,
    pub dow: Option<String>,
    pub special: Option<String>,
    pub user: Option<String>,
    pub command: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub disabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRequest {
    pub source: String,
    #[serde(flatten)]
    pub entry: EntryInput,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRequest {
    pub source: String,
    pub line: usize,
    #[serde(flatten)]
    pub entry: EntryInput,
}

#[derive(Debug, Deserialize)]
pub struct SourceLineQuery {
    pub source: String,
    pub line: usize,
}

// ---- validation -----------------------------------------------------------

fn validate_field(v: &str, name: &str) -> ApiResult<()> {
    if v.is_empty() {
        return Err(ApiError::BadRequest(format!("{name} must not be empty")));
    }
    if v.len() > 128 || v.contains('\n') || v.contains('\r') || v.contains('\0') {
        return Err(ApiError::BadRequest(format!("invalid {name}")));
    }
    Ok(())
}

fn validate_command(v: &str) -> ApiResult<()> {
    let t = v.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest("command must not be empty".into()));
    }
    if t.len() > 4096 || t.contains('\n') || t.contains('\r') || t.contains('\0') {
        return Err(ApiError::BadRequest("invalid command".into()));
    }
    Ok(())
}

fn validate_special(sp: &str) -> ApiResult<()> {
    if !SPECIALS.contains(&sp) {
        return Err(ApiError::BadRequest(format!(
            "unknown schedule alias: {sp}"
        )));
    }
    Ok(())
}

fn validate_username(name: &str) -> ApiResult<()> {
    if !RE_USERNAME.is_match(name) {
        return Err(ApiError::BadRequest("invalid username".into()));
    }
    Ok(())
}

fn validate_comment(c: &str) -> ApiResult<()> {
    if c.contains('\0') || c.len() > 2000 {
        return Err(ApiError::BadRequest("invalid comment".into()));
    }
    Ok(())
}

/// Build the task schedule line (without the disabled `#` prefix) and its
/// structured form, validating all fields.
fn build_task(input: &EntryInput, is_system: bool) -> ApiResult<(String, Parsed)> {
    let cmd = input.command.trim();
    validate_command(cmd)?;
    let body = if let Some(sp) = input.special.as_deref().filter(|s| !s.is_empty()) {
        validate_special(sp)?;
        if is_system {
            let user = input.user.as_deref().unwrap_or("").trim();
            validate_username(user)?;
            (format!("{sp} {user} {cmd}"), classify(&format!("{sp} {user} {cmd}"), true))
        } else {
            (format!("{sp} {cmd}"), classify(&format!("{sp} {cmd}"), false))
        }
    } else {
        let m = input.minute.as_deref().unwrap_or("*");
        let h = input.hour.as_deref().unwrap_or("*");
        let dom = input.dom.as_deref().unwrap_or("*");
        let mo = input.month.as_deref().unwrap_or("*");
        let dow = input.dow.as_deref().unwrap_or("*");
        validate_field(m, "minute")?;
        validate_field(h, "hour")?;
        validate_field(dom, "day of month")?;
        validate_field(mo, "month")?;
        validate_field(dow, "day of week")?;
        if is_system {
            let user = input.user.as_deref().unwrap_or("").trim();
            validate_username(user)?;
            let line = format!("{m} {h} {dom} {mo} {dow} {user} {cmd}");
            (line.clone(), classify(&line, true))
        } else {
            let line = format!("{m} {h} {dom} {mo} {dow} {cmd}");
            (line.clone(), classify(&line, false))
        }
    };
    let parsed = body.1.expect("just-validated line must classify");
    Ok((body.0, parsed))
}

/// Rebuild comment lines from the editable text (one `# line` per non-empty row).
fn build_comment_lines(comment: &str) -> Vec<String> {
    comment
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("# {l}"))
        .collect()
}

// ---- handlers -------------------------------------------------------------

/// GET /api/crontab — list all entries from /etc/crontab and every user tab.
pub async fn list() -> ApiResult<Json<Vec<CronEntry>>> {
    let mut out = Vec::new();
    if Path::new(ETC_CRONTAB).exists() {
        out.extend(parse_source("system")?);
    }
    for user in list_tab_users()? {
        out.extend(parse_source(&user)?);
    }
    Ok(Json(out))
}

/// Selectable creation target: `/etc/crontab` or a per-user crontab.
#[derive(Debug, Serialize)]
pub struct CronTarget {
    pub source: String,
    pub label: String,
    pub has_tab: bool,
}

/// GET /api/crontab/targets — selectable creation targets. Covers
/// `/etc/crontab` plus every user who could own a crontab: anyone with an
/// existing tab, and real users from /etc/passwd. Excludes pseudo-system
/// accounts: names starting with `_`, `nobody` (UID 65534), and reserved
/// UIDs ≤ 26 (except root). Lets the "Add" dialog target users that don't
/// yet appear in the list (no tab created).
pub async fn targets() -> ApiResult<Json<Vec<CronTarget>>> {
    use std::collections::{BTreeSet, HashSet};
    let tab_users: HashSet<String> = list_tab_users()?.into_iter().collect();

    let mut names: BTreeSet<String> = BTreeSet::new();
    names.extend(tab_users.iter().cloned());
    if let Ok(passwd) = fs::read_to_string("/etc/passwd") {
        for line in passwd.lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let f: Vec<&str> = line.splitn(7, ':').collect();
            if f.len() < 3 {
                continue;
            }
            let name = f[0];
            // Skip underscore-prefixed system accounts (_dhcp, _pflogd, …).
            if name.starts_with('_') {
                continue;
            }
            if let Ok(uid) = f[2].parse::<u32>() {
                // nobody pseudo-account and reserved UIDs ≤ 26 (daemon, bin,
                // tty, smmsp, mailnull, …) — except root (UID 0).
                if uid == 65534 || (uid <= 26 && name != "root") {
                    continue;
                }
                names.insert(name.to_string());
            }
        }
    }

    let mut out = Vec::with_capacity(names.len() + 1);
    out.push(CronTarget {
        source: "system".into(),
        label: "/etc/crontab".into(),
        has_tab: Path::new(ETC_CRONTAB).exists(),
    });
    for n in names {
        if !RE_USERNAME.is_match(&n) {
            continue;
        }
        out.push(CronTarget {
            has_tab: tab_users.contains(&n),
            source: n.clone(),
            label: n,
        });
    }
    Ok(Json(out))
}

/// POST /api/crontab — append a new entry to the given source.
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateRequest>,
) -> ApiResult<StatusCode> {
    validate_comment(&req.entry.comment)?;
    let (content, is_system) = read_raw(&req.source)?;
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let plen = preamble_len(&lines, is_system);
    let preamble = lines[..plen].to_vec();
    let mut blocks = parse_body(&lines[plen..], plen, is_system);

    let (body, parsed) = build_task(&req.entry, is_system)?;
    let task_line = if req.entry.disabled.unwrap_or(false) {
        format!("# {body}")
    } else {
        body
    };
    let comments = build_comment_lines(&req.entry.comment);
    blocks.push(Block::Entry(EntryBlock {
        comment_raw: comments,
        task_idx: lines.len(),
        raw_task: task_line,
        parsed,
        disabled: req.entry.disabled.unwrap_or(false),
    }));

    write_source(&req.source, is_system, &preamble, &blocks)?;

    audit::record(
        &state,
        Some(&auth.username),
        "POST",
        "/api/crontab",
        201,
        Some(format!(
            "added crontab entry for {}",
            if is_system { "system".to_string() } else { req.source.clone() }
        )),
    );
    Ok(StatusCode::CREATED)
}

/// PUT /api/crontab — replace the entry at `source`/`line` (also toggles).
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<UpdateRequest>,
) -> ApiResult<StatusCode> {
    validate_comment(&req.entry.comment)?;
    let (content, is_system) = read_raw(&req.source)?;
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let plen = preamble_len(&lines, is_system);
    let preamble = lines[..plen].to_vec();
    let mut blocks = parse_body(&lines[plen..], plen, is_system);

    let idx = blocks
        .iter()
        .position(|b| matches!(b, Block::Entry(e) if e.task_idx == req.line))
        .ok_or_else(|| ApiError::NotFound("crontab line not found".into()))?;

    let (body, parsed) = build_task(&req.entry, is_system)?;
    let task_line = if req.entry.disabled.unwrap_or(false) {
        format!("# {body}")
    } else {
        body
    };
    let comments = build_comment_lines(&req.entry.comment);

    let old = match mem::replace(&mut blocks[idx], Block::Inert(String::new())) {
        Block::Entry(e) => e,
        _ => unreachable!(),
    };
    blocks[idx] = Block::Entry(EntryBlock {
        comment_raw: comments,
        raw_task: task_line,
        disabled: req.entry.disabled.unwrap_or(false),
        parsed,
        task_idx: old.task_idx,
    });

    write_source(&req.source, is_system, &preamble, &blocks)?;

    audit::record(
        &state,
        Some(&auth.username),
        "PUT",
        "/api/crontab",
        200,
        Some(format!("updated crontab {} line {}", req.source, req.line)),
    );
    Ok(StatusCode::OK)
}

/// DELETE /api/crontab?source=&line= — remove the entry at `source`/`line`.
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<SourceLineQuery>,
) -> ApiResult<StatusCode> {
    let (content, is_system) = read_raw(&q.source)?;
    let lines: Vec<String> = content.lines().map(String::from).collect();
    let plen = preamble_len(&lines, is_system);
    let preamble = lines[..plen].to_vec();
    let mut blocks = parse_body(&lines[plen..], plen, is_system);

    let idx = blocks
        .iter()
        .position(|b| matches!(b, Block::Entry(e) if e.task_idx == q.line))
        .ok_or_else(|| ApiError::NotFound("crontab line not found".into()))?;
    blocks.remove(idx);

    write_source(&q.source, is_system, &preamble, &blocks)?;

    audit::record(
        &state,
        Some(&auth.username),
        "DELETE",
        "/api/crontab",
        200,
        Some(format!("deleted crontab {} line {}", q.source, q.line)),
    );
    Ok(StatusCode::NO_CONTENT)
}
