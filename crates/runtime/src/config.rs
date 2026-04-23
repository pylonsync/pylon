//! Unified server configuration loaded once at startup.
//!
//! Replaces scattered `std::env::var(...)` reads. All env vars start with
//! `PYLON_` and are documented in `SECURITY.md` and `README.md`.

use std::time::Duration;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    // -----------------------------------------------------------------------
    // Network
    // -----------------------------------------------------------------------
    pub port: u16,
    pub cors_origin: String,

    // -----------------------------------------------------------------------
    // Storage
    // -----------------------------------------------------------------------
    pub db_path: String,
    pub manifest_path: String,
    pub files_dir: String,
    pub files_url_prefix: String,
    pub session_db: Option<String>,

    // -----------------------------------------------------------------------
    // Auth
    // -----------------------------------------------------------------------
    pub admin_token: Option<String>,

    // -----------------------------------------------------------------------
    // Rate limiting
    // -----------------------------------------------------------------------
    pub rate_limit_max: u32,
    pub rate_limit_window: Duration,
    pub fn_rate_limit_max: u32,
    pub fn_rate_limit_window: Duration,

    // -----------------------------------------------------------------------
    // Functions runtime (Bun process)
    // -----------------------------------------------------------------------
    pub functions_dir: String,
    pub functions_runtime: Option<String>,

    // -----------------------------------------------------------------------
    // Modes
    // -----------------------------------------------------------------------
    pub is_dev: bool,
    pub drain_timeout: Duration,

    // -----------------------------------------------------------------------
    // AI proxy
    // -----------------------------------------------------------------------
    pub ai_provider: String,
    pub ai_api_key: String,
    pub ai_model: String,
    pub ai_base_url: String,

    // -----------------------------------------------------------------------
    // Workflow runner
    // -----------------------------------------------------------------------
    pub workflow_runner_url: String,
}

impl ServerConfig {
    /// Load config from environment variables. Falls back to dev-friendly
    /// defaults when a variable is unset.
    pub fn from_env(default_port: u16) -> Self {
        Self {
            port: env_u16("PYLON_PORT", default_port),
            cors_origin: env_str("PYLON_CORS_ORIGIN", "*"),
            db_path: env_str("PYLON_DB_PATH", "pylon.db"),
            manifest_path: env_str("PYLON_MANIFEST", "pylon.manifest.json"),
            files_dir: env_str("PYLON_FILES_DIR", "uploads"),
            files_url_prefix: env_str("PYLON_FILES_URL_PREFIX", "/api/files"),
            session_db: std::env::var("PYLON_SESSION_DB").ok(),
            admin_token: std::env::var("PYLON_ADMIN_TOKEN").ok(),
            rate_limit_max: env_u32("PYLON_RATE_LIMIT_MAX", 100),
            rate_limit_window: Duration::from_secs(env_u64("PYLON_RATE_LIMIT_WINDOW", 60)),
            fn_rate_limit_max: env_u32("PYLON_FN_RATE_LIMIT_MAX", 30),
            fn_rate_limit_window: Duration::from_secs(env_u64("PYLON_FN_RATE_LIMIT_WINDOW", 60)),
            functions_dir: env_str("PYLON_FUNCTIONS_DIR", "functions"),
            functions_runtime: std::env::var("PYLON_FUNCTIONS_RUNTIME").ok(),
            is_dev: env_bool("PYLON_DEV_MODE", true),
            drain_timeout: Duration::from_secs(env_u64("PYLON_DRAIN_SECS", 10)),
            ai_provider: env_str("PYLON_AI_PROVIDER", ""),
            ai_api_key: env_str("PYLON_AI_API_KEY", ""),
            ai_model: env_str("PYLON_AI_MODEL", ""),
            ai_base_url: env_str("PYLON_AI_BASE_URL", ""),
            workflow_runner_url: env_str("PYLON_WORKFLOW_RUNNER_URL", "http://127.0.0.1:9876/run"),
        }
    }
}

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_u16(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes"))
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = ServerConfig::from_env(4321);
        assert!(c.port > 0);
        assert!(c.rate_limit_max > 0);
        assert!(!c.cors_origin.is_empty());
    }

    #[test]
    fn env_overrides_default() {
        std::env::set_var("PYLON_PORT", "9999");
        let c = ServerConfig::from_env(4321);
        std::env::remove_var("PYLON_PORT");
        assert_eq!(c.port, 9999);
    }
}
