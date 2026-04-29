//! Shared utilities used across multiple crates.
//!
//! These live in `pylon-kernel` because `core` has no I/O dependencies
//! and is already a dependency of every other crate.

// ---------------------------------------------------------------------------
// SQL identifier quoting
// ---------------------------------------------------------------------------

/// Quote a SQL identifier with double quotes to prevent injection.
/// Embedded double quotes are escaped by doubling them (SQL standard,
/// works in SQLite and Postgres).
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

// ---------------------------------------------------------------------------
// ISO-8601 timestamps
// ---------------------------------------------------------------------------

/// Current UTC time as an ISO-8601 string (second precision).
///
/// Uses only `std::time::SystemTime` — no external date library required.
pub fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    epoch_to_iso(secs)
}

/// Convert Unix-epoch seconds to an ISO-8601 string.
pub fn epoch_to_iso(secs: u64) -> String {
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = is_leap(y);
    let month_days: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md {
            m = i;
            break;
        }
        remaining -= md;
    }
    let d = remaining + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        d,
        hours,
        minutes,
        seconds
    )
}

/// Check if a year is a leap year.
pub fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Parse an ISO-8601 / RFC 3339 timestamp into Unix-epoch seconds.
///
/// Accepts the formats pylon emits (`epoch_to_iso` shape) plus the
/// common RFC 3339 variants users send through the API:
/// - `YYYY-MM-DDTHH:MM:SSZ`
/// - `YYYY-MM-DDTHH:MM:SS.fffZ` (fractional seconds dropped)
/// - `YYYY-MM-DDTHH:MM:SS+HH:MM` / `-HH:MM` (offset applied)
///
/// Hand-rolled to keep `pylon-kernel` std-only — no chrono dep. Used
/// by the Postgres adapter to bind TIMESTAMPTZ columns from JSON
/// strings; SQLite stores them as TEXT and never needed parsing.
pub fn iso_to_epoch(s: &str) -> Result<u64, String> {
    // Minimal length check: "YYYY-MM-DDTHH:MM:SS" = 19 chars before the
    // tz suffix.
    if s.len() < 20 {
        return Err(format!("timestamp too short for ISO 8601: {s:?}"));
    }
    let parse_n = |slice: &str| -> Result<i64, String> {
        slice
            .parse::<i64>()
            .map_err(|_| format!("non-numeric segment in {slice:?}"))
    };
    let y = parse_n(&s[0..4])?;
    if &s[4..5] != "-"
        || &s[7..8] != "-"
        || &s[10..11] != "T"
        || &s[13..14] != ":"
        || &s[16..17] != ":"
    {
        return Err(format!("expected YYYY-MM-DDTHH:MM:SS shape, got {s:?}"));
    }
    let mo = parse_n(&s[5..7])?;
    let d = parse_n(&s[8..10])?;
    let h = parse_n(&s[11..13])?;
    let mi = parse_n(&s[14..16])?;
    let se = parse_n(&s[17..19])?;

    // Tz suffix: `Z`, `+HH:MM`, `-HH:MM`, optionally preceded by `.fff`.
    // We tolerate fractional seconds by skipping them — TIMESTAMPTZ
    // round-trips fine at second precision for pylon's surface.
    let mut tz_start = 19;
    if s.as_bytes().get(tz_start) == Some(&b'.') {
        tz_start += 1;
        while let Some(&b) = s.as_bytes().get(tz_start) {
            if b.is_ascii_digit() {
                tz_start += 1;
            } else {
                break;
            }
        }
    }
    let tz = &s[tz_start..];
    let offset_secs: i64 = match tz {
        "Z" | "" => 0,
        _ if tz.len() == 6 && (tz.starts_with('+') || tz.starts_with('-')) => {
            let sign: i64 = if &tz[0..1] == "+" { 1 } else { -1 };
            let oh = parse_n(&tz[1..3])?;
            let om = parse_n(&tz[4..6])?;
            sign * (oh * 3600 + om * 60)
        }
        other => return Err(format!("unrecognized timezone suffix: {other:?}")),
    };

    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return Err(format!("month/day out of range in {s:?}"));
    }

    // Days from epoch (1970-01-01) to the start of the target year.
    let mut days: i64 = 0;
    if y >= 1970 {
        for yr in 1970..y {
            days += if is_leap(yr) { 366 } else { 365 };
        }
    } else {
        for yr in y..1970 {
            days -= if is_leap(yr) { 366 } else { 365 };
        }
    }
    let leap = is_leap(y);
    let month_days: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for i in 0..(mo as usize - 1) {
        days += month_days[i];
    }
    days += d - 1;

    let total = days * 86400 + h * 3600 + mi * 60 + se - offset_secs;
    if total < 0 {
        return Err(format!("pre-1970 timestamp not supported: {s:?}"));
    }
    Ok(total as u64)
}

// ---------------------------------------------------------------------------
// File ID validation (defense-in-depth against path traversal)
// ---------------------------------------------------------------------------

/// Returns true if a user-provided file ID is safe to use as a path component.
/// Rejects empty strings, `..`, slashes, and dotfiles.
pub fn is_safe_file_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains("..")
        && !id.contains('/')
        && !id.contains('\\')
        && !id.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_ident_basic() {
        assert_eq!(quote_ident("users"), "\"users\"");
    }

    #[test]
    fn quote_ident_escapes_embedded_quote() {
        assert_eq!(quote_ident("weird\"name"), "\"weird\"\"name\"");
    }

    #[test]
    fn now_iso_format() {
        let s = now_iso();
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z'));
        assert_eq!(s.chars().nth(4), Some('-'));
        assert_eq!(s.chars().nth(10), Some('T'));
    }

    #[test]
    fn epoch_to_iso_zero() {
        assert_eq!(epoch_to_iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn epoch_to_iso_known() {
        // 2024-01-01T00:00:00Z = 1704067200
        assert_eq!(epoch_to_iso(1704067200), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn leap_year_detection() {
        assert!(is_leap(2000));
        assert!(is_leap(2024));
        assert!(!is_leap(1900));
        assert!(!is_leap(2023));
    }

    #[test]
    fn safe_file_id_accepts_normal() {
        assert!(is_safe_file_id("file_abc123"));
    }

    #[test]
    fn safe_file_id_rejects_traversal() {
        assert!(!is_safe_file_id(""));
        assert!(!is_safe_file_id(".."));
        assert!(!is_safe_file_id("../etc/passwd"));
        assert!(!is_safe_file_id("a/b"));
        assert!(!is_safe_file_id(".hidden"));
    }
}
