//! Dynamic per-project trusted-host set for platform ("tenant") domains.
//!
//! A Pylon Cloud app can attach a custom domain for each of its OWN
//! end-customers (`ctx.domains` → the control plane's `provisionTenantDomain`).
//! Those hostnames must be accepted by the app's security gates — CORS, the
//! WebSocket CSWSH check, redirect validation, and the SSR origin resolver —
//! but enumerating every hostname into `PYLON_CORS_ORIGIN` and restarting the
//! machine per domain does not scale. Instead a background thread refreshes a
//! set from the control plane (`listProjectTrustedHosts`, Bearer
//! `PYLON_DOMAINS_TOKEN`), and those gates consult it IN ADDITION to their
//! boot-frozen allowlists — so a newly provisioned tenant host is trusted
//! within one refresh interval, no restart.
//!
//! FAIL CLOSED. An empty set (self-host, or `PYLON_CLOUD_URL` /
//! `PYLON_DOMAINS_TOKEN` unset) trusts nothing extra — behavior identical to
//! before this module existed. A refresh error KEEPS the last-known-good set;
//! it never clears trust and never fails open. Only EXACT host matches count
//! (no wildcards); the set comes from the control plane scoped to this
//! project's token, listing only `ready` hostnames.

use std::collections::HashSet;
use std::env;
use std::sync::{OnceLock, RwLock};
use std::thread;
use std::time::Duration;

/// How often the background thread re-pulls the trusted-host set. Tenant
/// provisioning (DNS propagation + cert issuance) already takes minutes, so a
/// ≤30s trust-propagation delay is negligible — and a fixed interval avoids
/// any unknown-Host-triggered refresh amplification.
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

struct TenantHosts {
    hosts: RwLock<HashSet<String>>,
    agent: ureq::Agent,
    /// `${PYLON_CLOUD_URL}/api/fn/listProjectTrustedHosts`
    url: String,
    /// `PYLON_DOMAINS_TOKEN`
    token: String,
}

// `None` = feature off (self-host / env unset): trusts nothing extra.
static STATE: OnceLock<Option<&'static TenantHosts>> = OnceLock::new();

fn resolve() -> Option<TenantHosts> {
    let cloud = env::var("PYLON_CLOUD_URL").ok().filter(|s| !s.is_empty())?;
    let token = env::var("PYLON_DOMAINS_TOKEN")
        .ok()
        .filter(|s| !s.is_empty())?;
    let agent = ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).build();
    Some(TenantHosts {
        hosts: RwLock::new(HashSet::new()),
        agent,
        url: format!(
            "{}/api/fn/listProjectTrustedHosts",
            cloud.trim_end_matches('/')
        ),
        token,
    })
}

/// Start the refresher once. Safe to call unconditionally at boot — a no-op on
/// self-host / when the cloud env is absent.
pub fn init() {
    STATE.get_or_init(|| match resolve() {
        Some(th) => {
            // Process-lifetime singleton; leak to hand the refresher thread a
            // 'static reference (same lifetime the gates read through).
            let leaked: &'static TenantHosts = Box::leak(Box::new(th));
            let _ = thread::Builder::new()
                .name("pylon-tenant-hosts".into())
                .spawn(move || loop {
                    refresh(leaked);
                    thread::sleep(REFRESH_INTERVAL);
                });
            Some(leaked)
        }
        None => None,
    });
}

fn refresh(th: &TenantHosts) {
    // listProjectTrustedHosts is a Pylon action (args: {}) → POST the empty
    // args object; auth is the Bearer domains token, not a cookie.
    let result = th
        .agent
        .post(&th.url)
        .set("Authorization", &format!("Bearer {}", th.token))
        .set("Content-Type", "application/json")
        .send_string("{}");
    match result {
        Ok(resp) => {
            #[derive(serde::Deserialize)]
            struct Out {
                hosts: Vec<String>,
            }
            match resp.into_json::<Out>() {
                Ok(out) => {
                    let set: HashSet<String> =
                        out.hosts.into_iter().map(|h| h.to_lowercase()).collect();
                    if let Ok(mut w) = th.hosts.write() {
                        *w = set;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[tenant-hosts] malformed listProjectTrustedHosts response (keeping last-good): {e}"
                    );
                }
            }
        }
        Err(e) => {
            // Keep the last-known-good set — never fail open, never clear trust.
            tracing::warn!("[tenant-hosts] refresh failed (keeping last-good): {e}");
        }
    }
}

/// Is `host` (a bare hostname, no scheme/port) a trusted tenant host? Exact
/// match only. False when the feature is off or the set doesn't contain it.
pub fn is_trusted_host(host: &str) -> bool {
    match STATE.get().and_then(|o| *o) {
        Some(th) => th
            .hosts
            .read()
            .map(|s| s.contains(&host.to_lowercase()))
            .unwrap_or(false),
        None => false,
    }
}

/// Is `origin` (`scheme://host[:port]`) a trusted tenant origin? Extracts the
/// host and checks the set. Non-http(s) or hostless origins are never trusted.
pub fn is_trusted_origin(origin: &str) -> bool {
    match host_of_origin(origin) {
        Some(h) => is_trusted_host(h),
        None => false,
    }
}

/// Bare host from an origin string, or None if it isn't an http(s) origin.
fn host_of_origin(origin: &str) -> Option<&str> {
    let rest = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))?;
    // Stop at the first '/' (there shouldn't be a path on an Origin, but be
    // defensive) then strip an optional ':port'.
    let host = rest.split('/').next().unwrap_or(rest);
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_origin_extracts_and_strips() {
        assert_eq!(
            host_of_origin("https://a.example.com"),
            Some("a.example.com")
        );
        assert_eq!(
            host_of_origin("https://a.example.com:8443"),
            Some("a.example.com")
        );
        assert_eq!(
            host_of_origin("http://a.example.com/"),
            Some("a.example.com")
        );
        assert_eq!(host_of_origin("ftp://a.example.com"), None);
        assert_eq!(host_of_origin("a.example.com"), None); // not an origin
        assert_eq!(host_of_origin("https://"), None);
    }

    #[test]
    fn untrusted_when_feature_off() {
        // STATE unset in the unit-test process → is_trusted_* is always false.
        assert!(!is_trusted_host("anything.example.com"));
        assert!(!is_trusted_origin("https://anything.example.com"));
    }
}
