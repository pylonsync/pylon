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
// Home directory
// ---------------------------------------------------------------------------

/// The current user's home directory.
///
/// `HOME` is the unix spelling and is normally unset on Windows, where the
/// profile path is `USERPROFILE` (or the `HOMEDRIVE` + `HOMEPATH` pair on a
/// domain-joined machine that redirects it). Reading only `HOME` silently
/// resolves to nothing on Windows, which turns a "look under the home
/// directory" lookup into a lookup that never matches.
pub fn home_dir() -> Option<std::path::PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return Some(std::path::PathBuf::from(home));
    }
    #[cfg(windows)]
    {
        if let Some(profile) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
            return Some(std::path::PathBuf::from(profile));
        }
        if let (Some(drive), Some(path)) = (
            std::env::var_os("HOMEDRIVE").filter(|v| !v.is_empty()),
            std::env::var_os("HOMEPATH").filter(|v| !v.is_empty()),
        ) {
            let mut joined = std::ffi::OsString::from(drive);
            joined.push(path);
            return Some(std::path::PathBuf::from(joined));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// ISO-8601 timestamps
// ---------------------------------------------------------------------------

/// Current UTC time as an ISO-8601 string (second precision).
///
/// No external date library required. The epoch seconds come from the
/// platform clock via [`now_epoch_secs`].
pub fn now_iso() -> String {
    epoch_to_iso(now_epoch_secs())
}

/// Current Unix-epoch seconds from the platform clock.
///
/// `wasm32-unknown-unknown` (Cloudflare Workers, and the sync-relay
/// Durable Object) has no `SystemTime` backend — `SystemTime::now()`
/// panics there ("time not implemented on this platform"). The policy
/// evaluator stamps `now` on every check, so that panic would kill the
/// DO isolate the moment it filtered one event. On wasm the time comes
/// from JS `Date.now()` instead; native targets use `SystemTime`.
#[cfg(not(target_arch = "wasm32"))]
fn now_epoch_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(target_arch = "wasm32")]
fn now_epoch_secs() -> u64 {
    // `Date.now()` is milliseconds since the Unix epoch.
    (js_sys::Date::now() / 1000.0) as u64
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

// ---------------------------------------------------------------------------
// Query row cap (bound filtered-query / list result sets)
// ---------------------------------------------------------------------------

/// Maximum number of rows a single filtered query / list may return. A
/// client-supplied `$limit` is clamped to this, AND a query with no `$limit`
/// defaults to it — so `{}` (or a forgotten paginate) can't stream an entire
/// table into memory and, on Postgres, pin the single connection for the
/// whole scan. Generous by default (10k) so typical UI lists are unaffected;
/// override with `PYLON_QUERY_MAX_LIMIT`. Read once.
pub fn query_max_limit() -> u64 {
    use std::sync::OnceLock;
    static CAP: OnceLock<u64> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("PYLON_QUERY_MAX_LIMIT")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(10_000)
    })
}

/// The LIMIT a filtered query / list should actually use: a client-supplied
/// `$limit` clamped to [`query_max_limit`], or the cap itself when the client
/// gave none. The single chokepoint every query builder (SQLite + Postgres)
/// routes through so the bound is identical across backends.
pub fn effective_query_limit(client_limit: Option<u64>) -> u64 {
    let cap = query_max_limit();
    client_limit.map(|n| n.min(cap)).unwrap_or(cap)
}

// ---------------------------------------------------------------------------
// DSN redaction (keep DB passwords out of logs / error strings)
// ---------------------------------------------------------------------------

/// Redact the password out of a connection-string DSN before logging it.
///
/// `postgres://user:secret@host:5432/db` → `postgres://user:***@host:5432/db`.
///
/// Malformed DSNs (no `://` or no `@`) return unchanged — better to surface
/// the raw value than to silently hide a configuration problem. Callers
/// should still treat the output as "may reveal the URL shape" and never as
/// safe-to-share credentials. Shared in `pylon-kernel` so the runtime
/// (boot logs) and CLI (schema/dev output) redact identically.
pub fn redact_dsn(dsn: &str) -> String {
    let scheme_end = match dsn.find("://") {
        Some(i) => i + 3,
        None => return dsn.to_string(),
    };
    let rest = &dsn[scheme_end..];
    // The userinfo ends at the first `@` BEFORE any `/` (the path can
    // legitimately contain `@`). If `@` only appears in the path, there's
    // no userinfo to redact.
    let host_path = rest.split('/').next().unwrap_or(rest);
    // userinfo is everything before the LAST `@` in the authority — an
    // unencoded `@` inside the password must not be mistaken for the
    // host separator (`admin:p@ss@host` → userinfo `admin:p@ss`).
    let at = match host_path.rfind('@') {
        Some(i) => i,
        None => return dsn.to_string(),
    };
    let userinfo = &rest[..at];
    let host_and_rest = &rest[at..]; // starts with '@'
    let redacted_userinfo = match userinfo.find(':') {
        Some(i) => format!("{}:***", &userinfo[..i]),
        None => userinfo.to_string(),
    };
    format!(
        "{}{}{}",
        &dsn[..scheme_end],
        redacted_userinfo,
        host_and_rest
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_query_limit_clamps_and_defaults() {
        // Default cap is 10_000 (no PYLON_QUERY_MAX_LIMIT in the test env).
        let cap = query_max_limit();
        assert_eq!(cap, 10_000);
        // No client $limit → defaults to the cap (so `{}` can't be unbounded).
        assert_eq!(effective_query_limit(None), cap);
        // A client $limit below the cap is preserved.
        assert_eq!(effective_query_limit(Some(25)), 25);
        // A client $limit above the cap is clamped down.
        assert_eq!(effective_query_limit(Some(1_000_000)), cap);
        // Exactly at the cap stays.
        assert_eq!(effective_query_limit(Some(cap)), cap);
    }

    #[test]
    fn redact_dsn_masks_password() {
        assert_eq!(
            redact_dsn("postgres://user:secret@host:5432/db"),
            "postgres://user:***@host:5432/db"
        );
        assert_eq!(
            redact_dsn("postgresql://admin:p@ss:word@db.internal/app"),
            "postgresql://admin:***@db.internal/app"
        );
    }

    /// The exact DSN shape that leaked into a production log stream: a
    /// PlanetScale Postgres URL with a dotted user, a `pscale_pw_` secret, and
    /// libpq TLS params after the database name. `pylon start`'s banner printed
    /// DATABASE_URL verbatim, so every boot published the credential.
    #[test]
    fn redact_dsn_hides_a_planetscale_password() {
        let redacted = redact_dsn(
            "postgresql://pscale_api_abc123.def456:pscale_pw_SUPERSECRET\
             @us-east-2.pg.psdb.cloud:5432/postgres?sslmode=verify-full&sslrootcert=system",
        );
        assert!(
            !redacted.contains("pscale_pw_SUPERSECRET"),
            "password survived redaction: {redacted}"
        );
        // Still useful for diagnosis — host, port and database remain.
        assert!(redacted.contains("us-east-2.pg.psdb.cloud:5432"));
        assert!(redacted.contains("/postgres"));
    }

    #[test]
    fn redact_dsn_no_password_or_malformed_unchanged() {
        // No password component.
        assert_eq!(
            redact_dsn("postgres://user@host/db"),
            "postgres://user@host/db"
        );
        // No userinfo at all.
        assert_eq!(
            redact_dsn("postgres://host:5432/db"),
            "postgres://host:5432/db"
        );
        // Not a URL — returned verbatim rather than silently blanked.
        assert_eq!(redact_dsn("/var/lib/sessions.db"), "/var/lib/sessions.db");
        // An `@` only in the path must not be treated as userinfo.
        assert_eq!(
            redact_dsn("postgres://host/db@weird"),
            "postgres://host/db@weird"
        );
    }

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

    /// Regression: prevents recurrence of the `format!("{}Z", unix_secs)`
    /// bug that wrote garbage like `1748534400Z` into Postgres datetime
    /// columns. now_iso() must emit a proper RFC 3339 string that
    /// round-trips through the parser back to roughly the current epoch.
    #[test]
    fn now_iso_round_trips_through_iso_to_epoch() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let s = now_iso();
        let parsed = iso_to_epoch(&s).expect("now_iso must be parseable RFC3339");
        // Allow a small skew for the system clock between the two reads.
        assert!(
            parsed >= before.saturating_sub(2) && parsed <= before + 2,
            "round-trip drift: before={before} parsed={parsed} s={s:?}"
        );
        // Guard the original bug class: the bad pattern produced strings
        // like "1748534400Z" — no dashes, no 'T'. Assert the shape.
        assert!(
            s.contains('-'),
            "now_iso must contain date separators: {s:?}"
        );
        assert!(
            s.contains('T'),
            "now_iso must contain date/time separator: {s:?}"
        );
        assert!(
            s.len() >= 20,
            "now_iso must be a full RFC3339 timestamp, got {s:?}"
        );
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
