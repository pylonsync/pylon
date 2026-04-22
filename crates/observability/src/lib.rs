//! Pluggable observability hooks for statecraft.
//!
//! Two separate concerns live here:
//!
//! - **Error reporting** — capture handler/panic-level errors and forward
//!   them to Sentry, Honeycomb Events, or any HTTP error-reporting service.
//!   The framework calls `report_error(...)` in the unhappy paths it owns;
//!   the operator installs a concrete reporter at startup.
//! - **Trace exporting** — hand `tracing` spans off to an OTLP/Jaeger/file
//!   exporter. The framework emits spans via the `tracing` crate; this
//!   module just provides a seam for the operator to plug a subscriber
//!   layer in.
//!
//! Kept deliberately small: no protocol implementations, no network I/O.
//! Operators bring their own `sentry`, `opentelemetry`, or `reqwest`-based
//! exporter and call `set_error_reporter(Box::new(my_reporter))` at boot.

use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Error reporting
// ---------------------------------------------------------------------------

/// Severity levels for reported events. Maps cleanly onto Sentry's levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorLevel {
    Debug,
    Info,
    Warning,
    Error,
    Fatal,
}

/// A single error event, passed to [`ErrorReporter::report`].
///
/// Every field is borrowed so the caller doesn't pay allocation cost on the
/// hot path when no reporter is installed. `context` is a small key-value
/// map — anything deeper should go through a dedicated structured-logging
/// span instead.
pub struct ErrorEvent<'a> {
    pub level: ErrorLevel,
    pub code: &'a str,
    pub message: &'a str,
    pub context: &'a [(&'a str, &'a str)],
}

/// An error reporter — implement this to forward events to Sentry, a
/// homegrown API, or a file.
///
/// Implementations must be thread-safe and should not block the calling
/// thread for more than the time it takes to serialize + enqueue. Network
/// I/O should happen on a background task the reporter owns; the framework
/// doesn't spawn one for you.
pub trait ErrorReporter: Send + Sync {
    fn report(&self, event: &ErrorEvent<'_>);
}

/// A no-op reporter used when none is installed. `report()` is essentially
/// free.
pub struct NoopErrorReporter;

impl ErrorReporter for NoopErrorReporter {
    fn report(&self, _event: &ErrorEvent<'_>) {}
}

static ERROR_REPORTER: OnceLock<Box<dyn ErrorReporter>> = OnceLock::new();

/// Install the process-wide error reporter. Returns `Err` if one is
/// already installed — installation is one-shot, matching `tracing`'s
/// subscriber pattern. Call once at startup, before any handler runs.
pub fn set_error_reporter(reporter: Box<dyn ErrorReporter>) -> Result<(), &'static str> {
    ERROR_REPORTER
        .set(reporter)
        .map_err(|_| "error reporter already installed")
}

/// Report an error via the installed reporter, or no-op if none is
/// installed. Every framework error-path hook calls this; operators only
/// have to hook `set_error_reporter` once.
pub fn report_error(event: &ErrorEvent<'_>) {
    if let Some(r) = ERROR_REPORTER.get() {
        r.report(event);
    }
}

/// Shorthand for the common case: report a handler failure with code +
/// message and no extra context.
pub fn report_handler_error(code: &str, message: &str) {
    report_error(&ErrorEvent {
        level: ErrorLevel::Error,
        code,
        message,
        context: &[],
    });
}

// ---------------------------------------------------------------------------
// Tracing exporter seam
// ---------------------------------------------------------------------------

/// Callback run once at startup so the operator can attach a `tracing`
/// subscriber layer (OTLP, Jaeger, file, stdout). The framework calls
/// `run_tracing_hook()` right before the HTTP server accepts connections.
///
/// Why a callback instead of a trait? Subscribers are not `Send + Sync`
/// without fighting type inference, and `tracing-subscriber::registry()`
/// is easier to configure at the call site than to pass through a vtable.
pub type TracingInitFn = Box<dyn FnOnce() + Send>;

static TRACING_INIT: OnceLock<std::sync::Mutex<Option<TracingInitFn>>> = OnceLock::new();

fn slot() -> &'static std::sync::Mutex<Option<TracingInitFn>> {
    TRACING_INIT.get_or_init(|| std::sync::Mutex::new(None))
}

/// Register a callback that runs once the server is ready to emit spans.
/// Replaces any previously-registered callback.
pub fn set_tracing_hook(init: TracingInitFn) {
    let mut g = slot().lock().unwrap();
    *g = Some(init);
}

/// Run the registered tracing hook, if any. The framework calls this
/// exactly once at startup. Callers should not invoke it directly.
///
/// The hook runs AFTER the mutex is released so a reentrant
/// `set_tracing_hook` / `run_tracing_hook` call from inside the hook
/// can't deadlock, and a panic inside the hook can't poison the mutex
/// for subsequent calls.
pub fn run_tracing_hook() {
    let maybe_fn = {
        let mut g = slot().lock().unwrap();
        g.take()
    };
    if let Some(f) = maybe_fn {
        f();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct Counting(Arc<AtomicUsize>);
    impl ErrorReporter for Counting {
        fn report(&self, _event: &ErrorEvent<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn report_no_op_when_uninstalled() {
        // The global slot is process-wide, so this test runs in isolation —
        // only one process-level install per test binary. We just confirm
        // report_error doesn't panic when nothing is installed.
        report_handler_error("TEST", "no reporter installed, should not panic");
    }

    #[test]
    fn tracing_hook_runs_once() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c2 = Arc::clone(&counter);
        set_tracing_hook(Box::new(move || {
            c2.fetch_add(1, Ordering::Relaxed);
        }));
        run_tracing_hook();
        run_tracing_hook(); // second call should be a no-op (hook consumed)
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }
}
