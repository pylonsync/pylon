//! Cron expression parser and matcher.
//!
//! Supports standard 5-field cron expressions:
//!   minute hour day-of-month month day-of-week
//!
//! Field syntax: `*`, `*/N`, `N`, `N-M`, `N,M,O`, and combinations thereof.

use std::time::{SystemTime, UNIX_EPOCH};

/// A parsed cron expression.
#[derive(Debug, Clone)]
pub struct CronExpr {
    minutes: Vec<u8>,  // 0-59
    hours: Vec<u8>,    // 0-23
    days: Vec<u8>,     // 1-31
    months: Vec<u8>,   // 1-12
    weekdays: Vec<u8>, // 0-6 (Sun=0)
}

impl CronExpr {
    /// Parse a cron expression string.
    /// Supports: `*`, `*/N`, `N`, `N-M`, `N,M,O`
    pub fn parse(expr: &str) -> Result<Self, String> {
        let parts: Vec<&str> = expr.trim().split_whitespace().collect();
        if parts.len() != 5 {
            return Err(format!("Expected 5 fields, got {}", parts.len()));
        }

        Ok(Self {
            minutes: parse_field(parts[0], 0, 59)?,
            hours: parse_field(parts[1], 0, 23)?,
            days: parse_field(parts[2], 1, 31)?,
            months: parse_field(parts[3], 1, 12)?,
            weekdays: parse_field(parts[4], 0, 6)?,
        })
    }

    /// Check if the given unix timestamp matches this cron expression.
    pub fn matches(&self, unix_secs: u64) -> bool {
        let (min, hour, day, month, weekday) = decompose_timestamp(unix_secs);
        self.minutes.contains(&min)
            && self.hours.contains(&hour)
            && self.days.contains(&day)
            && self.months.contains(&month)
            && self.weekdays.contains(&weekday)
    }

    /// Check if the current moment matches.
    pub fn matches_now(&self) -> bool {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.matches(ts)
    }
}

/// Parse a single cron field into an expanded list of valid values.
fn parse_field(field: &str, min: u8, max: u8) -> Result<Vec<u8>, String> {
    if field == "*" {
        return Ok((min..=max).collect());
    }

    // */N -- step over full range
    if let Some(step) = field.strip_prefix("*/") {
        let step: u8 = step
            .parse()
            .map_err(|_| format!("Invalid step: {step}"))?;
        if step == 0 {
            return Err("Step cannot be 0".into());
        }
        return Ok((min..=max).step_by(step as usize).collect());
    }

    let mut values = Vec::new();
    for part in field.split(',') {
        if part.contains('-') {
            // Range: N-M
            let range: Vec<&str> = part.splitn(2, '-').collect();
            let start: u8 = range[0]
                .parse()
                .map_err(|_| format!("Invalid range start: {}", range[0]))?;
            let end: u8 = range[1]
                .parse()
                .map_err(|_| format!("Invalid range end: {}", range[1]))?;
            if start > end || start < min || end > max {
                return Err(format!(
                    "Range {start}-{end} out of bounds ({min}-{max})"
                ));
            }
            values.extend(start..=end);
        } else {
            // Single value
            let val: u8 = part
                .parse()
                .map_err(|_| format!("Invalid value: {part}"))?;
            if val < min || val > max {
                return Err(format!("Value {val} out of bounds ({min}-{max})"));
            }
            values.push(val);
        }
    }

    values.sort();
    values.dedup();
    Ok(values)
}

/// Decompose a unix timestamp into (minute, hour, day, month, weekday).
///
/// Uses Howard Hinnant's civil date algorithm to convert days since epoch
/// into a calendar date.
fn decompose_timestamp(unix_secs: u64) -> (u8, u8, u8, u8, u8) {
    let total_secs = unix_secs as i64;
    let day_secs = total_secs.rem_euclid(86400);
    let hour = (day_secs / 3600) as u8;
    let minute = ((day_secs % 3600) / 60) as u8;

    // Days since epoch
    let days = total_secs.div_euclid(86400);

    // Weekday: Jan 1 1970 was Thursday (4). 0=Sun.
    let weekday = ((days + 4).rem_euclid(7)) as u8;

    // Civil date from days since epoch (Howard Hinnant's algorithm)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month proxy [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;

    (minute, hour, day, month, weekday)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_every_minute() {
        let cron = CronExpr::parse("* * * * *").unwrap();
        assert_eq!(cron.minutes.len(), 60);
        assert_eq!(cron.hours.len(), 24);
        assert_eq!(cron.days.len(), 31);
        assert_eq!(cron.months.len(), 12);
        assert_eq!(cron.weekdays.len(), 7);
    }

    #[test]
    fn parse_every_5_minutes() {
        let cron = CronExpr::parse("*/5 * * * *").unwrap();
        assert_eq!(cron.minutes, vec![0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]);
    }

    #[test]
    fn parse_weekdays_at_9am() {
        let cron = CronExpr::parse("0 9 * * 1-5").unwrap();
        assert_eq!(cron.minutes, vec![0]);
        assert_eq!(cron.hours, vec![9]);
        assert_eq!(cron.weekdays, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn parse_comma_list() {
        let cron = CronExpr::parse("0,15,30,45 * * * *").unwrap();
        assert_eq!(cron.minutes, vec![0, 15, 30, 45]);
    }

    #[test]
    fn parse_range() {
        let cron = CronExpr::parse("* 8-17 * * *").unwrap();
        assert_eq!(cron.hours, vec![8, 9, 10, 11, 12, 13, 14, 15, 16, 17]);
    }

    #[test]
    fn parse_specific_values() {
        let cron = CronExpr::parse("30 12 1 6 0").unwrap();
        assert_eq!(cron.minutes, vec![30]);
        assert_eq!(cron.hours, vec![12]);
        assert_eq!(cron.days, vec![1]);
        assert_eq!(cron.months, vec![6]);
        assert_eq!(cron.weekdays, vec![0]);
    }

    #[test]
    fn parse_rejects_wrong_field_count() {
        assert!(CronExpr::parse("* * *").is_err());
        assert!(CronExpr::parse("* * * * * *").is_err());
        assert!(CronExpr::parse("").is_err());
    }

    #[test]
    fn parse_rejects_out_of_range() {
        assert!(CronExpr::parse("60 * * * *").is_err());
        assert!(CronExpr::parse("* 24 * * *").is_err());
        assert!(CronExpr::parse("* * 0 * *").is_err());
        assert!(CronExpr::parse("* * 32 * *").is_err());
        assert!(CronExpr::parse("* * * 0 *").is_err());
        assert!(CronExpr::parse("* * * 13 *").is_err());
        assert!(CronExpr::parse("* * * * 7").is_err());
    }

    #[test]
    fn parse_rejects_zero_step() {
        assert!(CronExpr::parse("*/0 * * * *").is_err());
    }

    #[test]
    fn parse_rejects_invalid_tokens() {
        assert!(CronExpr::parse("abc * * * *").is_err());
        assert!(CronExpr::parse("* foo * * *").is_err());
    }

    #[test]
    fn decompose_epoch() {
        // 1970-01-01 00:00 is Thursday (weekday=4)
        let (min, hour, day, month, weekday) = decompose_timestamp(0);
        assert_eq!((min, hour, day, month, weekday), (0, 0, 1, 1, 4));
    }

    #[test]
    fn decompose_known_date() {
        // 2024-01-15 10:30:00 UTC = 1705314600
        // Monday, January 15, 2024 10:30 UTC
        let (min, hour, day, month, weekday) = decompose_timestamp(1705314600);
        assert_eq!(min, 30);
        assert_eq!(hour, 10);
        assert_eq!(day, 15);
        assert_eq!(month, 1);
        assert_eq!(weekday, 1); // Monday
    }

    #[test]
    fn decompose_another_known_date() {
        // 2023-12-25 00:00:00 UTC = 1703462400
        // Monday, December 25, 2023
        let (min, hour, day, month, weekday) = decompose_timestamp(1703462400);
        assert_eq!(min, 0);
        assert_eq!(hour, 0);
        assert_eq!(day, 25);
        assert_eq!(month, 12);
        assert_eq!(weekday, 1); // Monday
    }

    #[test]
    fn matches_every_minute() {
        let cron = CronExpr::parse("* * * * *").unwrap();
        // Should match any timestamp
        assert!(cron.matches(0));
        assert!(cron.matches(1705314600));
    }

    #[test]
    fn matches_specific_time() {
        // 2024-01-15 10:30 UTC, Monday
        let cron = CronExpr::parse("30 10 15 1 1").unwrap();
        assert!(cron.matches(1705314600));
        // One minute off should not match
        assert!(!cron.matches(1705314600 + 60));
    }

    #[test]
    fn matches_weekday_schedule() {
        // 0 9 * * 1-5 : weekdays at 9:00
        let cron = CronExpr::parse("0 9 * * 1-5").unwrap();
        // 2024-01-15 09:00 UTC = Monday
        let monday_9am: u64 = 1705309200; // 2024-01-15 09:00:00 UTC
        let (min, hour, _, _, weekday) = decompose_timestamp(monday_9am);
        assert_eq!(min, 0);
        assert_eq!(hour, 9);
        assert_eq!(weekday, 1);
        assert!(cron.matches(monday_9am));
    }

    #[test]
    fn matches_now_does_not_panic() {
        // This is a smoke test -- matches_now should not panic regardless of
        // what the current time is.
        let cron = CronExpr::parse("* * * * *").unwrap();
        // Always true for every-minute schedule.
        assert!(cron.matches_now());
    }
}
