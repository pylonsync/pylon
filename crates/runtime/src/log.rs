use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::sync::OnceLock;
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// LogLevel
// ---------------------------------------------------------------------------

/// Severity levels ordered from most verbose to most severe.
///
/// The discriminant values are intentionally spaced so that ordering
/// comparisons (`>=`, `<=`) work correctly without a manual `Ord` impl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl LogLevel {
    /// Numeric priority — higher means more severe.
    #[inline]
    fn priority(self) -> u8 {
        self as u8
    }
}

impl PartialOrd for LogLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LogLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority().cmp(&other.priority())
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO ",
            LogLevel::Warn => "WARN ",
            LogLevel::Error => "ERROR",
        };
        f.write_str(tag)
    }
}

// ---------------------------------------------------------------------------
// LogEntry
// ---------------------------------------------------------------------------

/// A single structured log record.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub target: String,
    pub timestamp: String,
    pub fields: HashMap<String, String>,
}

impl LogEntry {
    /// Build a new entry, capturing the current UTC timestamp.
    pub fn new(
        level: LogLevel,
        target: impl Into<String>,
        message: impl Into<String>,
        fields: HashMap<String, String>,
    ) -> Self {
        Self {
            level,
            message: message.into(),
            target: target.into(),
            timestamp: iso8601_now(),
            fields,
        }
    }

    /// Format this entry into the canonical one-line representation.
    pub fn format(&self) -> String {
        let mut buf = format!(
            "[{}] {} [{}] {}",
            self.timestamp, self.level, self.target, self.message,
        );

        // Append key=value pairs in deterministic (sorted) order so that
        // tests and human readers get predictable output.
        if !self.fields.is_empty() {
            let mut keys: Vec<&String> = self.fields.keys().collect();
            keys.sort();
            for key in keys {
                buf.push(' ');
                buf.push_str(key);
                buf.push('=');
                buf.push_str(&self.fields[key]);
            }
        }

        buf
    }
}

impl fmt::Display for LogEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format())
    }
}

// ---------------------------------------------------------------------------
// Logger
// ---------------------------------------------------------------------------

/// A thread-safe structured logger that writes to stderr.
///
/// Thread safety is achieved without a `Mutex` — each call formats the full
/// line into a `String` first, then writes it in a single `write_all` call
/// to stderr. On Unix-like systems writes of reasonable size to stderr are
/// atomic at the fd level.
pub struct Logger {
    min_level: LogLevel,
}

impl Logger {
    pub fn new(min_level: LogLevel) -> Self {
        Self { min_level }
    }

    /// Returns `true` if a message at `level` would be emitted.
    #[inline]
    pub fn enabled(&self, level: LogLevel) -> bool {
        level >= self.min_level
    }

    /// Core logging method. Messages below `min_level` are silently dropped.
    pub fn log(
        &self,
        level: LogLevel,
        target: &str,
        message: &str,
        fields: HashMap<String, String>,
    ) {
        if !self.enabled(level) {
            return;
        }

        let entry = LogEntry::new(level, target, message, fields);
        let line = format!("{}\n", entry.format());

        // Single write — no lock needed.
        let _ = std::io::stderr().write_all(line.as_bytes());
    }

    // -- convenience helpers ------------------------------------------------

    pub fn info(&self, target: &str, message: &str, fields: HashMap<String, String>) {
        self.log(LogLevel::Info, target, message, fields);
    }

    pub fn warn(&self, target: &str, message: &str, fields: HashMap<String, String>) {
        self.log(LogLevel::Warn, target, message, fields);
    }

    pub fn error(&self, target: &str, message: &str, fields: HashMap<String, String>) {
        self.log(LogLevel::Error, target, message, fields);
    }

    pub fn debug(&self, target: &str, message: &str, fields: HashMap<String, String>) {
        self.log(LogLevel::Debug, target, message, fields);
    }

    pub fn trace(&self, target: &str, message: &str, fields: HashMap<String, String>) {
        self.log(LogLevel::Trace, target, message, fields);
    }
}

// ---------------------------------------------------------------------------
// Global logger
// ---------------------------------------------------------------------------

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Initialise the global logger. Subsequent calls are no-ops — the first
/// writer wins.
pub fn init_logger(level: LogLevel) {
    let _ = LOGGER.set(Logger::new(level));
}

/// Obtain a reference to the global logger.
///
/// # Panics
///
/// Panics if `init_logger` has not been called yet. Prefer calling
/// `init_logger` early in `main`.
pub fn logger() -> &'static Logger {
    LOGGER
        .get()
        .expect("pylon: logger not initialised — call init_logger() first")
}

// ---------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------

/// Emit a structured log at INFO level via the global logger.
///
/// ```ignore
/// log_info!("server", "listening", "port" => "8080", "host" => "0.0.0.0");
/// log_info!("server", "started");
/// ```
#[macro_export]
macro_rules! log_info {
    ($target:expr, $msg:expr $(, $key:expr => $val:expr)*) => {{
        let fields = $crate::log::__build_fields(&[$(($key, $val)),*]);
        $crate::log::logger().info($target, $msg, fields);
    }};
}

/// Emit a structured log at WARN level via the global logger.
#[macro_export]
macro_rules! log_warn {
    ($target:expr, $msg:expr $(, $key:expr => $val:expr)*) => {{
        let fields = $crate::log::__build_fields(&[$(($key, $val)),*]);
        $crate::log::logger().warn($target, $msg, fields);
    }};
}

/// Emit a structured log at ERROR level via the global logger.
#[macro_export]
macro_rules! log_error {
    ($target:expr, $msg:expr $(, $key:expr => $val:expr)*) => {{
        let fields = $crate::log::__build_fields(&[$(($key, $val)),*]);
        $crate::log::logger().error($target, $msg, fields);
    }};
}

/// Emit a structured log at DEBUG level via the global logger.
#[macro_export]
macro_rules! log_debug {
    ($target:expr, $msg:expr $(, $key:expr => $val:expr)*) => {{
        let fields = $crate::log::__build_fields(&[$(($key, $val)),*]);
        $crate::log::logger().debug($target, $msg, fields);
    }};
}

// ---------------------------------------------------------------------------
// Helpers (public for macro hygiene, not part of the public API)
// ---------------------------------------------------------------------------

/// Convert a slice of key-value pairs into a `HashMap`. Exposed only so that
/// the macros can call it; not intended for direct use.
#[doc(hidden)]
pub fn __build_fields(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// Produce an ISO-8601 timestamp string in UTC (e.g. `2024-01-15T12:00:00Z`).
///
/// Uses only `std` — no chrono dependency.
fn iso8601_now() -> String {
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let secs = duration.as_secs();

    // Manual decomposition — avoids pulling in `chrono` or `time`.
    const SECS_PER_MINUTE: u64 = 60;
    const SECS_PER_HOUR: u64 = 3_600;
    const SECS_PER_DAY: u64 = 86_400;

    let days = secs / SECS_PER_DAY;
    let day_secs = secs % SECS_PER_DAY;
    let hour = day_secs / SECS_PER_HOUR;
    let minute = (day_secs % SECS_PER_HOUR) / SECS_PER_MINUTE;
    let second = day_secs % SECS_PER_MINUTE;

    // Civil date from day count (algorithm from Howard Hinnant).
    let (year, month, day) = civil_from_days(days as i64);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second,
    )
}

/// Convert a Unix day count to (year, month, day). Algorithm by Howard
/// Hinnant — see <http://howardhinnant.github.io/date_algorithms.html>.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    // -- LogLevel ordering --------------------------------------------------

    #[test]
    fn level_ordering_error_is_most_severe() {
        assert!(LogLevel::Error > LogLevel::Warn);
        assert!(LogLevel::Error > LogLevel::Info);
        assert!(LogLevel::Error > LogLevel::Debug);
        assert!(LogLevel::Error > LogLevel::Trace);
    }

    #[test]
    fn level_ordering_warn_gt_info() {
        assert!(LogLevel::Warn > LogLevel::Info);
        assert!(LogLevel::Warn > LogLevel::Debug);
        assert!(LogLevel::Warn > LogLevel::Trace);
    }

    #[test]
    fn level_ordering_info_gt_debug() {
        assert!(LogLevel::Info > LogLevel::Debug);
        assert!(LogLevel::Info > LogLevel::Trace);
    }

    #[test]
    fn level_ordering_debug_gt_trace() {
        assert!(LogLevel::Debug > LogLevel::Trace);
    }

    #[test]
    fn level_ordering_equal() {
        assert_eq!(LogLevel::Info.cmp(&LogLevel::Info), Ordering::Equal);
    }

    #[test]
    fn level_ordering_full_sequence() {
        let mut levels = vec![
            LogLevel::Warn,
            LogLevel::Trace,
            LogLevel::Error,
            LogLevel::Debug,
            LogLevel::Info,
        ];
        levels.sort();
        assert_eq!(
            levels,
            vec![
                LogLevel::Trace,
                LogLevel::Debug,
                LogLevel::Info,
                LogLevel::Warn,
                LogLevel::Error,
            ]
        );
    }

    // -- LogEntry formatting ------------------------------------------------

    #[test]
    fn entry_format_without_fields() {
        let entry = LogEntry {
            level: LogLevel::Info,
            message: "server started".into(),
            target: "server".into(),
            timestamp: "2024-01-15T12:00:00Z".into(),
            fields: HashMap::new(),
        };

        assert_eq!(
            entry.format(),
            "[2024-01-15T12:00:00Z] INFO  [server] server started"
        );
    }

    #[test]
    fn entry_format_with_fields() {
        let mut fields = HashMap::new();
        fields.insert("port".into(), "8080".into());
        fields.insert("host".into(), "0.0.0.0".into());

        let entry = LogEntry {
            level: LogLevel::Warn,
            message: "binding".into(),
            target: "net".into(),
            timestamp: "2024-01-15T12:00:00Z".into(),
            fields,
        };

        // Fields are sorted alphabetically.
        assert_eq!(
            entry.format(),
            "[2024-01-15T12:00:00Z] WARN  [net] binding host=0.0.0.0 port=8080"
        );
    }

    #[test]
    fn entry_format_error_level() {
        let entry = LogEntry {
            level: LogLevel::Error,
            message: "disk full".into(),
            target: "storage".into(),
            timestamp: "2024-01-15T12:00:00Z".into(),
            fields: HashMap::new(),
        };

        assert_eq!(
            entry.format(),
            "[2024-01-15T12:00:00Z] ERROR [storage] disk full"
        );
    }

    #[test]
    fn entry_display_matches_format() {
        let entry = LogEntry {
            level: LogLevel::Debug,
            message: "cache miss".into(),
            target: "cache".into(),
            timestamp: "2024-01-15T12:00:00Z".into(),
            fields: HashMap::new(),
        };

        assert_eq!(entry.to_string(), entry.format());
    }

    // -- Logger filtering ---------------------------------------------------

    #[test]
    fn logger_enabled_at_min_level() {
        let logger = Logger::new(LogLevel::Info);
        assert!(logger.enabled(LogLevel::Info));
        assert!(logger.enabled(LogLevel::Warn));
        assert!(logger.enabled(LogLevel::Error));
    }

    #[test]
    fn logger_filters_below_min_level() {
        let logger = Logger::new(LogLevel::Warn);
        assert!(!logger.enabled(LogLevel::Trace));
        assert!(!logger.enabled(LogLevel::Debug));
        assert!(!logger.enabled(LogLevel::Info));
        assert!(logger.enabled(LogLevel::Warn));
        assert!(logger.enabled(LogLevel::Error));
    }

    #[test]
    fn logger_trace_enables_everything() {
        let logger = Logger::new(LogLevel::Trace);
        assert!(logger.enabled(LogLevel::Trace));
        assert!(logger.enabled(LogLevel::Debug));
        assert!(logger.enabled(LogLevel::Info));
        assert!(logger.enabled(LogLevel::Warn));
        assert!(logger.enabled(LogLevel::Error));
    }

    #[test]
    fn logger_error_only() {
        let logger = Logger::new(LogLevel::Error);
        assert!(!logger.enabled(LogLevel::Trace));
        assert!(!logger.enabled(LogLevel::Debug));
        assert!(!logger.enabled(LogLevel::Info));
        assert!(!logger.enabled(LogLevel::Warn));
        assert!(logger.enabled(LogLevel::Error));
    }

    // -- Convenience methods compile and respect filtering -------------------

    #[test]
    fn convenience_methods_do_not_panic() {
        // We cannot easily capture stderr in a unit test without extra
        // machinery, so we simply verify these do not panic.
        let logger = Logger::new(LogLevel::Error);
        logger.info("t", "m", HashMap::new());
        logger.warn("t", "m", HashMap::new());
        logger.debug("t", "m", HashMap::new());
        logger.trace("t", "m", HashMap::new());
        logger.error("t", "m", HashMap::new());
    }

    // -- Timestamp helper ---------------------------------------------------

    #[test]
    fn iso8601_now_looks_valid() {
        let ts = iso8601_now();
        // Basic shape: YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(ts.len(), 20, "timestamp length: {ts}");
        assert!(ts.ends_with('Z'));
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
        assert_eq!(&ts[13..14], ":");
        assert_eq!(&ts[16..17], ":");
    }

    #[test]
    fn civil_from_days_epoch() {
        // Day 0 = 1970-01-01
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_known_date() {
        // 2024-01-15 is day 19_737
        assert_eq!(civil_from_days(19_737), (2024, 1, 15));
    }

    // -- __build_fields helper ----------------------------------------------

    #[test]
    fn build_fields_empty() {
        let fields = __build_fields(&[]);
        assert!(fields.is_empty());
    }

    #[test]
    fn build_fields_with_pairs() {
        let fields = __build_fields(&[("a", "1"), ("b", "2")]);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields.get("a").unwrap(), "1");
        assert_eq!(fields.get("b").unwrap(), "2");
    }
}
