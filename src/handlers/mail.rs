//! System mail (mbox) reader — list, read, delete, mark read/unread
//! for `/var/mail/$USER` mailboxes.
//!
//! The classic BSD mbox format stores all mail for a user in a single file.
//! Each message begins with an envelope ("From ") line, followed by RFC 2822
//! headers, a blank line, and the message body. The `mail(1)` command reads
//! and writes this file directly.
//!
//! ## Performance strategy
//!
//! mbox has no index, so every operation must scan the file. To match
//! `mail(1)` speed:
//!
//! - [`scan_page`] does a single streaming pass via `BufReader`. For messages
//!   **outside the requested page**, only the byte offset and Status header
//!   are recorded — From/To/Subject/Date are skipped entirely.
//! - Envelope detection uses a plain `starts_with("From ")` check (no regex).
//! - `list_mailboxes` uses an even lighter [`count_messages`] that only counts
//!   envelope lines and scans for `Status:` headers.

use std::fs;
use std::io::{BufRead, BufReader};
use std::time::UNIX_EPOCH;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::audit;
use crate::auth::AuthUser;
use crate::error::{ApiError, ApiResult};
use crate::AppState;

const MAIL_SPOOL: &str = "/var/mail";
const DEFAULT_PAGE_SIZE: usize = 50;

/// Check if a line is an mbox envelope ("From ") line.
///
/// Mirrors `mail(1)`'s `ishead()` logic from `/usr/src/usr.bin/mail/head.c`:
/// 1. Must start with `"From "`
/// 2. The date portion (after sender) must match a valid date format,
///    validated via `isdate()` + `cmatch()` — ported directly from the C.
#[inline]
fn is_envelope(line: &str) -> bool {
    if !line.starts_with("From ") {
        return false;
    }
    let rest = &line[5..];
    // Port of parse(): extract the date portion following the sender.
    // Format: "From sender Day Mon D HH:MM:SS YYYY"
    // Special case: "From Day Mon..." (no sender — entire rest is date).
    let date = match rest.find(' ') {
        None => return false, // "From x" — no date
        Some(pos) => {
            let after_sender = &rest[pos + 1..];
            if isdate(rest) {
                // No sender; entire rest is the date.
                rest
            } else if after_sender.starts_with("tty") {
                // Skip optional tty field: "From sender ttyXX Day Mon..."
                match after_sender.find(' ') {
                    Some(p) => &after_sender[p + 1..],
                    None => return false,
                }
            } else {
                after_sender
            }
        }
    };
    isdate(date)
}

// ── Date validation (ported from mail(1) head.c) ────────────────────────────

/// Date format templates from `head.c:date_formats`.
/// Pattern chars: 'a'=lower, 'A'=upper, ' '=space, '0'=digit,
/// 'O'=space-or-digit, 'p'=punct, 'P'=space-or-punct, ':'=colon.
const DATE_FORMATS: &[&[u8]] = &[
    b"Aaa Aaa O0 00:00:00 0000",      // Mon Jan  1 23:59:59 2001
    b"Aaa Aaa O0 00:00:00 AAA 0000",  // Mon Jan  1 23:59:59 PST 2001
    b"Aaa Aaa O0 00:00:00 0000 p0000",// Mon Jan  1 23:59:59 2001 -0800
    b"Aaa Aaa O0 00:00 0000",         // Mon Jan  1 23:59 2001
    b"Aaa Aaa O0 00:00 AAA 0000",     // Mon Jan  1 23:59 PST 2001
    b"Aaa Aaa O0 00:00 0000 p0000",   // Mon Jan  1 23:59 2001 -0800
];

/// Port of `head.c:isdate()` — tries each template.
fn isdate(date: &str) -> bool {
    let bytes = date.as_bytes();
    DATE_FORMATS.iter().any(|fmt| cmatch(bytes, fmt))
}

/// Port of `head.c:cmatch()` — match string against template.
fn cmatch(cp: &[u8], tp: &[u8]) -> bool {
    let mut ci = 0usize;
    let mut ti = 0usize;
    while ci < cp.len() && ti < tp.len() {
        let t = tp[ti];
        ti += 1;
        let c = cp[ci];
        let ok = match t {
            b'a' => c.is_ascii_lowercase(),
            b'A' => c.is_ascii_uppercase(),
            b' ' => c == b' ',
            b'0' => c.is_ascii_digit(),
            b'O' => c == b' ' || c.is_ascii_digit(),
            b'p' => c.is_ascii_punctuation(),
            b'P' => c == b' ' || c.is_ascii_punctuation(),
            b':' => c == b':',
            _ => false,
        };
        if !ok {
            return false;
        }
        ci += 1;
    }
    ci == cp.len() && ti == tp.len()
}

// ── Response types ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct MailboxInfo {
    pub user: String,
    pub size: u64,
    pub total: usize,
    pub unread: usize,
    pub modified: Option<u64>,
}

#[derive(Serialize)]
pub struct MailSummary {
    pub index: usize,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub date: String,
    pub read: bool,
    pub size: usize,
}

#[derive(Serialize)]
pub struct MailListResponse {
    pub user: String,
    pub total: usize,
    pub unread: usize,
    pub page: usize,
    pub page_size: usize,
    pub mails: Vec<MailSummary>,
}

#[derive(Serialize)]
pub struct MailDetail {
    pub index: usize,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub date: String,
    pub read: bool,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[derive(Deserialize)]
pub struct BatchDeleteRequest {
    pub indices: Vec<usize>,
}

#[derive(Deserialize)]
pub struct PageQuery {
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
}

fn default_page() -> usize {
    1
}

fn default_page_size() -> usize {
    DEFAULT_PAGE_SIZE
}

// ── Core: streaming scan with pagination ────────────────────────────────────

/// Byte offset + size of a message (for seek-based operations).
struct MsgRegion {
    offset: usize,
    size: usize,
}

/// Lightweight count-only scan for `list_mailboxes`.
///
/// Returns `(total_messages, unread_count)` by streaming through the file
/// and checking only envelope lines + `Status:` headers.
fn count_messages(path: &str) -> (usize, usize) {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (0, 0),
    };
    let mut reader = BufReader::new(file);

    let mut total = 0usize;
    let mut unread = 0usize;
    let mut in_msg = false;
    let mut in_headers = false;
    let mut msg_is_read = false;
    let mut buf = String::new();

    loop {
        buf.clear();
        let n = match reader.read_line(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        let trimmed = buf.trim_end_matches(['\r', '\n']);

        if is_envelope(trimmed) {
            // Finish previous message
            if in_msg && !msg_is_read {
                unread += 1;
            }
            // Start new message
            total += 1;
            in_msg = true;
            in_headers = true;
            msg_is_read = false;
        } else if in_headers {
            if trimmed.is_empty() {
                in_headers = false;
            } else if trimmed.starts_with("Status:") {
                msg_is_read = trimmed.contains('R');
            }
        }
        let _ = n; // consume
    }

    // Finish last message
    if in_msg && !msg_is_read {
        unread += 1;
    }

    (total, unread)
}

/// Streaming scan that returns byte regions for **all** messages plus full
/// header summaries for the requested **page** only.
///
fn scan_regions(
    path: &str,
) -> Result<(Vec<MsgRegion>, usize, usize), ApiError> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((vec![], 0, 0));
        }
        Err(e) => return Err(ApiError::Io(e)),
    };
    let mut reader = BufReader::new(file);

    // All message regions (for delete/mark operations)
    let mut regions: Vec<MsgRegion> = Vec::new();
    let mut unread: usize = 0;

    // Per-message parsing state
    let mut byte_pos: usize = 0;
    let mut msg_start: usize = 0;
    let mut in_msg = false;
    let mut in_headers = false;
    let mut msg_is_read = false;

    let mut buf = String::new();
    loop {
        buf.clear();
        let n = reader.read_line(&mut buf).map_err(ApiError::Io)?;
        if n == 0 {
            break;
        }
        let line_len = n;
        let trimmed = buf.trim_end_matches(['\r', '\n']);

        if is_envelope(trimmed) {
            // Finish previous message
            if in_msg {
                if !msg_is_read {
                    unread += 1;
                }
                regions.push(MsgRegion {
                    offset: msg_start,
                    size: byte_pos - msg_start,
                });
            }
            // Start new message
            msg_start = byte_pos;
            in_msg = true;
            in_headers = true;
            msg_is_read = false;
        } else if in_headers {
            if trimmed.is_empty() {
                in_headers = false;
            } else if trimmed.starts_with("Status:") {
                msg_is_read = trimmed.contains('R');
            }
        }

        byte_pos += line_len;
    }

    // Finish last message
    if in_msg {
        if !msg_is_read {
            unread += 1;
        }
        regions.push(MsgRegion {
            offset: msg_start,
            size: byte_pos - msg_start,
        });
    }

    let total = regions.len();
    Ok((regions, total, unread))
}

/// Parse the 5 summary fields from a message slice.
fn parse_summary(msg: &str) -> (String, String, String, String, bool) {
    let mut from = String::new();
    let mut to = String::new();
    let mut subject = String::new();
    let mut date = String::new();
    let mut read = false;

    let mut key: Option<&str> = None;
    let mut val = String::new();

    for line in msg.lines().skip(1) {
        // skip envelope line
        if line.is_empty() {
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if !val.is_empty() {
                val.push(' ');
            }
            val.push_str(line.trim_start());
        } else {
            if let Some(k) = key {
                store_summary_field(k, &val, &mut from, &mut to, &mut subject, &mut date, &mut read);
            }
            val.clear();
            if let Some(pos) = line.find(':') {
                key = Some(&line[..pos]);
                val = line[pos + 1..].trim().to_string();
            }
        }
    }
    if let Some(k) = key {
        store_summary_field(k, &val, &mut from, &mut to, &mut subject, &mut date, &mut read);
    }

    (from, to, subject, date, read)
}

#[inline]
fn store_summary_field(
    key: &str,
    val: &str,
    from: &mut String,
    to: &mut String,
    subject: &mut String,
    date: &mut String,
    read: &mut bool,
) {
    match key.to_ascii_lowercase().as_str() {
        "from" => *from = val.to_string(),
        "to" => *to = val.to_string(),
        "subject" => *subject = val.to_string(),
        "date" => *date = val.to_string(),
        "status" => *read = val.contains('R'),
        _ => {}
    }
}

// ── Single-message operations ───────────────────────────────────────────────

/// Parse all RFC 2822 headers from a message slice (for the detail view).
fn parse_headers(msg: &str) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    let mut lines = msg.lines();
    lines.next(); // skip envelope line

    let mut key: Option<String> = None;
    let mut val = String::new();

    for line in lines {
        if line.is_empty() {
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            val.push('\n');
            val.push_str(line.trim_start());
        } else {
            if let Some(k) = key.take() {
                headers.push((k, std::mem::take(&mut val)));
            }
            if let Some(pos) = line.find(':') {
                key = Some(line[..pos].to_string());
                val = line[pos + 1..].trim().to_string();
            }
        }
    }
    if let Some(k) = key {
        headers.push((k, val));
    }
    headers
}

fn get_header(headers: &[(String, String)], name: &str) -> String {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// Extract body text (everything after the blank line separating headers).
fn get_body(msg: &str) -> String {
    let mut lines = msg.lines();
    lines.next(); // skip envelope line
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
    }
    lines.collect::<Vec<_>>().join("\n")
}

/// Modify the `Status:` header of a single message slice.
fn set_message_read(msg: &str, read: bool) -> String {
    let lines: Vec<&str> = msg.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    let mut in_headers = true;
    let mut found_status = false;

    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            out.push((*line).to_string());
            continue;
        }
        if in_headers && line.is_empty() {
            if !found_status && read {
                out.push("Status: R".to_string());
                found_status = true;
            }
            in_headers = false;
            out.push((*line).to_string());
            continue;
        }
        if in_headers && line.len() >= 7 && line[..7].eq_ignore_ascii_case("Status:") {
            found_status = true;
            let val = line[7..].trim();
            let new_val = if read {
                if val.contains('R') {
                    val.to_string()
                } else {
                    format!("R{val}")
                }
            } else {
                val.replace('R', "").trim().to_string()
            };
            if !new_val.is_empty() {
                out.push(format!("Status: {new_val}"));
            }
        } else {
            out.push((*line).to_string());
        }
    }
    if in_headers && !found_status && read {
        out.push("Status: R".to_string());
    }
    out.join("\n")
}

// ── File I/O helpers ────────────────────────────────────────────────────────

fn valid_user(user: &str) -> Result<(), ApiError> {
    if user.is_empty()
        || user.len() > 64
        || !user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(ApiError::BadRequest("invalid username".into()));
    }
    Ok(())
}

fn mailbox_path(user: &str) -> String {
    format!("{MAIL_SPOOL}/{user}")
}

/// Read a specific message slice from the file by byte offset.
fn read_message_at(path: &str, offset: usize, size: usize) -> Result<String, ApiError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset as u64))?;
    let mut buf = vec![0u8; size];
    file.read_exact(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Rewrite the mbox by replacing the byte range [offset, offset+size) with
/// `replacement`.
fn replace_region(path: &str, offset: usize, size: usize, replacement: &str) -> Result<(), ApiError> {
    let content = fs::read_to_string(path)?;
    let mut out = String::with_capacity(content.len() - size + replacement.len());
    out.push_str(&content[..offset]);
    out.push_str(replacement);
    out.push_str(&content[offset + size..]);
    fs::write(path, out)?;
    Ok(())
}

/// Rewrite the mbox, deleting the byte ranges at the given indices.
fn delete_regions(
    path: &str,
    regions: &[MsgRegion],
    indices: &std::collections::HashSet<usize>,
) -> Result<(), ApiError> {
    let content = fs::read_to_string(path)?;
    let mut out = String::with_capacity(content.len());
    for (i, region) in regions.iter().enumerate() {
        if !indices.contains(&i) {
            out.push_str(&content[region.offset..region.offset + region.size]);
        }
    }
    fs::write(path, out)?;
    Ok(())
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// List all non-empty mailboxes in `/var/mail`.
pub async fn list_mailboxes(
    State(_state): State<AppState>,
    _auth_user: AuthUser,
) -> ApiResult<Json<Vec<MailboxInfo>>> {
    let mut boxes_list = Vec::new();

    let entries = match fs::read_dir(MAIL_SPOOL) {
        Ok(e) => e,
        Err(_) => return Ok(Json(boxes_list)),
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() == 0 {
            continue;
        }

        let user = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        let path_str = path.to_string_lossy();
        let (total, unread) = count_messages(&path_str);
        if total == 0 {
            continue;
        }

        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        boxes_list.push(MailboxInfo {
            user,
            size: meta.len(),
            total,
            unread,
            modified,
        });
    }

    boxes_list.sort_by(|a, b| b.size.cmp(&a.size));
    Ok(Json(boxes_list))
}

/// List mails in a mailbox with server-side pagination.
pub async fn list_mails(
    State(_state): State<AppState>,
    _auth_user: AuthUser,
    AxumPath(user): AxumPath<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Json<MailListResponse>> {
    valid_user(&user)?;
    let path = mailbox_path(&user);
    let page_size = q.page_size.clamp(1, 500);
    let page = q.page.max(1);

    let (regions, total, unread) = scan_regions(&path)?;

    // Display newest-first: page 1 = last `page_size` messages.
    // The original file index (0-based from oldest) is kept in `index`
    // so read/delete/mark operations remain correct.
    let page_start = (page - 1) * page_size;
    let page_end = (page_start + page_size).min(total);

    let mut mails = Vec::with_capacity(page_end.saturating_sub(page_start));
    for p in page_start..page_end {
        // Map page position to file position: newest = highest index.
        let i = total - 1 - p;
        let region = &regions[i];
        let msg = read_message_at(&path, region.offset, region.size)?;
        let (from, to, subject, date, read) = parse_summary(&msg);
        mails.push(MailSummary {
            index: i,
            from,
            to,
            subject,
            date,
            read,
            size: region.size,
        });
    }

    Ok(Json(MailListResponse {
        user,
        total,
        unread,
        page,
        page_size,
        mails,
    }))
}

/// Read a single mail by index. Marks it as read automatically.
pub async fn read_mail(
    State(state): State<AppState>,
    auth_user: AuthUser,
    AxumPath((user, index)): AxumPath<(String, usize)>,
) -> ApiResult<Json<MailDetail>> {
    valid_user(&user)?;
    let path = mailbox_path(&user);
    let (regions, _, _) = scan_regions(&path)?;

    let region = regions
        .get(index)
        .ok_or_else(|| ApiError::NotFound("mail not found".into()))?;

    let msg = read_message_at(&path, region.offset, region.size)?;
    let headers = parse_headers(&msg);

    // Auto-mark as read when opened.
    let was_read = get_header(&headers, "Status").contains('R');
    if !was_read {
        let new_msg = set_message_read(&msg, true);
        replace_region(&path, region.offset, region.size, &new_msg)?;
        audit::record(
            &state,
            Some(&auth_user.username),
            "READ",
            &format!("/api/mail/{user}/{index}"),
            200,
            None,
        );
    }

    Ok(Json(MailDetail {
        index,
        from: get_header(&headers, "From"),
        to: get_header(&headers, "To"),
        subject: get_header(&headers, "Subject"),
        date: get_header(&headers, "Date"),
        read: true,
        headers,
        body: get_body(&msg),
    }))
}

/// Delete a single mail by index.
pub async fn delete_mail(
    State(state): State<AppState>,
    auth_user: AuthUser,
    AxumPath((user, index)): AxumPath<(String, usize)>,
) -> ApiResult<StatusCode> {
    valid_user(&user)?;
    let path = mailbox_path(&user);
    let (regions, _, _) = scan_regions(&path)?;

    if index >= regions.len() {
        return Err(ApiError::NotFound("mail not found".into()));
    }

    let mut indices = std::collections::HashSet::new();
    indices.insert(index);
    delete_regions(&path, &regions, &indices)?;

    audit::record(
        &state,
        Some(&auth_user.username),
        "DELETE",
        &format!("/api/mail/{user}/{index}"),
        200,
        Some(format!("deleted mail {user}#{index}")),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Batch-delete mails by indices.
pub async fn batch_delete(
    State(state): State<AppState>,
    auth_user: AuthUser,
    AxumPath(user): AxumPath<String>,
    Json(req): Json<BatchDeleteRequest>,
) -> ApiResult<StatusCode> {
    valid_user(&user)?;
    let path = mailbox_path(&user);
    let (regions, _, _) = scan_regions(&path)?;

    let indices: std::collections::HashSet<usize> = req.indices.into_iter().collect();
    delete_regions(&path, &regions, &indices)?;

    audit::record(
        &state,
        Some(&auth_user.username),
        "DELETE",
        &format!("/api/mail/{user}/delete"),
        200,
        Some(format!("batch deleted {} mails", indices.len())),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Clear entire mailbox (truncate to empty).
pub async fn clear_mailbox(
    State(state): State<AppState>,
    auth_user: AuthUser,
    AxumPath(user): AxumPath<String>,
) -> ApiResult<StatusCode> {
    valid_user(&user)?;
    let path = mailbox_path(&user);

    fs::write(&path, "")?;

    audit::record(
        &state,
        Some(&auth_user.username),
        "DELETE",
        &format!("/api/mail/{user}"),
        200,
        Some(format!("cleared mailbox {user}")),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// Mark a mail as read.
pub async fn mark_read(
    State(state): State<AppState>,
    auth_user: AuthUser,
    AxumPath((user, index)): AxumPath<(String, usize)>,
) -> ApiResult<StatusCode> {
    mark_status(&state, &auth_user.username, &user, index, true)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Mark a mail as unread.
pub async fn mark_unread(
    State(state): State<AppState>,
    auth_user: AuthUser,
    AxumPath((user, index)): AxumPath<(String, usize)>,
) -> ApiResult<StatusCode> {
    mark_status(&state, &auth_user.username, &user, index, false)?;
    Ok(StatusCode::NO_CONTENT)
}

fn mark_status(
    state: &AppState,
    auth_user: &str,
    user: &str,
    index: usize,
    read: bool,
) -> Result<(), ApiError> {
    valid_user(user)?;
    let path = mailbox_path(user);
    let (regions, _, _) = scan_regions(&path)?;

    let region = regions
        .get(index)
        .ok_or_else(|| ApiError::NotFound("mail not found".into()))?;

    let msg = read_message_at(&path, region.offset, region.size)?;
    let new_msg = set_message_read(&msg, read);
    replace_region(&path, region.offset, region.size, &new_msg)?;

    audit::record(
        state,
        Some(auth_user),
        "PUT",
        &format!("/api/mail/{user}/{index}/{}", if read { "read" } else { "unread" }),
        200,
        None,
    );

    Ok(())
}
