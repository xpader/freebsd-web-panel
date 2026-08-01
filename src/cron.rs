//! Minimal cron expression parser for FWP scheduler.
//!
//! Supports 5-6 fields: `min hour dom month dow` or `sec min hour dom month dow`.
//! When 5 fields are given, seconds default to 0.
//! Field syntax: `*`, exact numbers (`5`), ranges (`1-10`), steps (`*/15`, `2-10/2`),
//! and lists (`1,5,10`). No named days/months (MON, JAN) — numbers only.
//! All times are evaluated in the system local timezone.
//!
//! This is intentionally minimal — ~130 lines, zero dependencies beyond chrono.

use chrono::{Datelike, Local, TimeZone, Timelike};

/// A single cron field value (one of the comma-separated entries).
#[derive(Debug, Clone, Copy)]
enum Range {
    /// Every value in [min, max].
    All,
    /// Specific values: start, end, step.
    Set(i32, i32, i32),
}

/// Parsed cron expression.
pub struct Cron {
    /// Field bounds: (field_index, min, max).
    /// 0=sec(0-59), 1=min(0-59), 2=hour(0-23), 3=dom(1-31), 4=month(1-12), 5=dow(0-6).
    fields: [Vec<Range>; 6],
}

impl Cron {
    /// Parse a cron expression. 5 fields = `min hour dom month dow` (sec=0),
    /// 6 fields = `sec min hour dom month dow`.
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        let (sec_part, rest) = match parts.len() {
            5 => ("0", parts.as_slice()),
            6 => (parts[0], &parts[1..]),
            n => return Err(format!("expected 5 or 6 fields, got {n}")),
        };
        let bounds = [(0, 59), (0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];
        let raw = [sec_part, rest[0], rest[1], rest[2], rest[3], rest[4]];
        let mut fields = Vec::new();
        for (i, (field_str, &(lo, hi))) in raw.iter().zip(bounds.iter()).enumerate() {
            let mut ranges = Vec::new();
            for entry in field_str.split(',') {
                ranges.push(parse_entry(entry, lo, hi).map_err(|e| format!("field {i}: {e}"))?);
            }
            // Safety: always at least one entry from split.
            fields.push(ranges);
        }
        let fields = [
            fields.remove(0),
            fields.remove(0),
            fields.remove(0),
            fields.remove(0),
            fields.remove(0),
            fields.remove(0),
        ];
        Ok(Cron { fields })
    }

    /// Find the next matching Unix timestamp strictly after `from_ts`.
    pub fn next_after(&self, from_ts: i64) -> Option<i64> {
        // Start one second after from, search up to 366 days ahead.
        let mut ts = from_ts + 1;
        let limit = from_ts + 366 * 86400;
        while ts <= limit {
            if self.matches(ts) {
                return Some(ts);
            }
            ts += 1;
        }
        None
    }

    fn matches(&self, ts: i64) -> bool {
        let dt = Local.timestamp_opt(ts, 0).single().unwrap();
        let vals = [
            dt.second() as i32,
            dt.minute() as i32,
            dt.hour() as i32,
            dt.day() as i32,
            dt.month() as i32,
            dt.weekday().num_days_from_sunday() as i32,
        ];
        for (i, val) in vals.iter().enumerate() {
            if !self.fields[i].iter().any(|r| match r {
                Range::All => true,
                Range::Set(s, e, step) => {
                    let s = *s;
                    let e = *e;
                    let step = *step as usize;
                    (s..=e).step_by(step).any(|v| v == *val)
                }
            }) {
                return false;
            }
        }
        true
    }
}

/// Parse a single field entry like `*`, `5`, `1-10`, `*/15`, `2-10/2`.
fn parse_entry(s: &str, lo: i32, hi: i32) -> Result<Range, String> {
    if s == "*" {
        return Ok(Range::All);
    }
    // Handle step: `*/step` or `range/step` or `value/step`.
    let (base, step) = if let Some((b, sp)) = s.split_once('/') {
        let step: i32 = sp.parse().map_err(|_| "invalid step")?;
        if step < 1 {
            return Err("step must be >= 1".into());
        }
        (b, step)
    } else {
        (s, 1)
    };
    let (start, end) = if base == "*" {
        (lo, hi)
    } else if let Some((a, b)) = base.split_once('-') {
        (a.parse().map_err(|_| "invalid range start")?, b.parse().map_err(|_| "invalid range end")?)
    } else {
        let v: i32 = base.parse().map_err(|_| "invalid value")?;
        (v, v)
    };
    if start < lo || end > hi || start > end {
        return Err(format!("value {start}-{end} out of bounds [{lo},{hi}]"));
    }
    Ok(Range::Set(start, end, step))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_every_minute() {
        let cron = Cron::parse("* * * * *").unwrap();
        // 2024-01-01T00:00:00Z = 1704067200
        let next = cron.next_after(1704067200).unwrap();
        assert_eq!(next, 1704067200 + 60);
    }

    #[test]
    fn test_specific_hour_minute() {
        let cron = Cron::parse("0 3 * * *").unwrap();
        // Cron matches in local timezone. Find next local 03:00 after from_ts.
        let next = cron.next_after(1704067200).unwrap();
        // The result should be a local 03:00 timestamp.
        let dt = Local.timestamp_opt(next, 0).single().unwrap();
        assert_eq!(dt.hour(), 3);
        assert_eq!(dt.minute(), 0);
    }

    #[test]
    fn test_with_seconds() {
        let cron = Cron::parse("30 * * * * *").unwrap();
        let next = cron.next_after(1704067200).unwrap();
        assert_eq!(next, 1704067200 + 30);
    }

    #[test]
    fn test_step() {
        let cron = Cron::parse("*/15 * * * *").unwrap();
        // Every 15 minutes. From 00:00:00, next is 00:15:00 = +900s.
        let next = cron.next_after(1704067200).unwrap();
        assert_eq!(next, 1704067200 + 900);
    }

    #[test]
    fn test_invalid_field_count() {
        assert!(Cron::parse("* * *").is_err());
    }
}
