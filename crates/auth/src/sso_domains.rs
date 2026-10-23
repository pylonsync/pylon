//! Which email addresses an org's SSO identity provider may vouch for.
//!
//! An org owner chooses the IdP (OIDC or SAML) and so controls every claim
//! it makes, including `email`. Any user can create an org. If the SSO
//! callback trusted the asserted email as is, an org owner could sign in as
//! any user of the app by having their IdP assert that user's address.
//!
//! So the IdP may only vouch for addresses on a domain the org has proven it
//! owns: the domain is listed in the org's SSO config, and the domain's DNS
//! publishes a TXT record bound to that org
//! ([`domain_verification_record`]). The proof is checked when the config is
//! saved and again at sign-in (cached briefly), so a lapsed or removed record
//! stops sign-ins.
//!
//! An operator can vouch for domains instead with `PYLON_SSO_ALLOWED_DOMAINS`
//! (single-tenant deployments, where the operator owns every org).

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a successful DNS check counts before sign-in checks again.
const VERIFIED_TTL: Duration = Duration::from_secs(10 * 60);

/// The TXT record (name, value) an org publishes to prove it owns `domain`.
/// The value is derived from the org id, so a record published for one org
/// proves nothing for another. It is not secret.
pub fn domain_verification_record(org_id: &str, domain: &str) -> (String, String) {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let digest = Sha256::digest(format!("pylon-sso-domain\n{org_id}\n{domain}").as_bytes());
    let token: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    (
        format!("_pylon-sso.{domain}"),
        format!("pylon-sso-verification={token}"),
    )
}

/// Looks up the TXT strings published at a DNS name.
pub trait TxtLookup: Send + Sync {
    fn txt(&self, name: &str) -> Result<Vec<String>, String>;
}

/// TXT lookups over DNS-over-HTTPS (the JSON API). The endpoint is
/// `PYLON_SSO_DOH_URL` when set, else Cloudflare then Google. Requests go
/// through the private-address guard like every other outbound auth fetch.
pub struct DohTxtLookup;

const DEFAULT_DOH_ENDPOINTS: &[&str] = &[
    "https://cloudflare-dns.com/dns-query",
    "https://dns.google/resolve",
];

impl TxtLookup for DohTxtLookup {
    fn txt(&self, name: &str) -> Result<Vec<String>, String> {
        let configured = std::env::var("PYLON_SSO_DOH_URL")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let endpoints: Vec<String> = match configured {
            Some(url) => vec![url],
            None => DEFAULT_DOH_ENDPOINTS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        let agent = crate::org_sso::guarded_agent();
        let mut last_err = String::new();
        for endpoint in endpoints {
            let response = agent
                .get(&endpoint)
                .query("name", name)
                .query("type", "TXT")
                .set("Accept", "application/dns-json")
                .timeout(Duration::from_secs(5))
                .call()
                .map_err(|e| format!("{endpoint}: {e}"))
                .and_then(|r| r.into_string().map_err(|e| format!("{endpoint}: {e}")));
            match response.and_then(|body| parse_doh_txt(&body)) {
                Ok(values) => return Ok(values),
                Err(e) => last_err = e,
            }
        }
        Err(format!("TXT lookup for {name} failed: {last_err}"))
    }
}

/// The TXT strings in a DNS JSON answer. A record's `data` is one or more
/// quoted strings (`"abc" "def"`), which DNS joins into one value.
fn parse_doh_txt(body: &str) -> Result<Vec<String>, String> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("DNS answer is not JSON: {e}"))?;
    match json.get("Status").and_then(|v| v.as_u64()) {
        Some(0) => {}
        // NXDOMAIN: the name does not exist, so there is no record.
        Some(3) => return Ok(Vec::new()),
        other => return Err(format!("DNS status {other:?}")),
    }
    let answers = json
        .get("Answer")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(answers
        .iter()
        .filter(|a| a.get("type").and_then(|t| t.as_u64()) == Some(16))
        .filter_map(|a| a.get("data").and_then(|d| d.as_str()))
        .map(join_txt_strings)
        .collect())
}

fn join_txt_strings(data: &str) -> String {
    let data = data.trim();
    if !data.starts_with('"') {
        return data.to_string();
    }
    let mut out = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for c in data.chars() {
        if in_quotes {
            if escaped {
                out.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_quotes = false;
            } else {
                out.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsoDomainError {
    /// The asserted email has no domain part.
    BadEmail,
    /// The email's domain is not one the org listed in its SSO config.
    DomainNotClaimed { domain: String },
    /// The org listed the domain but its DNS does not carry the proof.
    DomainNotVerified {
        domain: String,
        record_name: String,
        record_value: String,
    },
    /// The DNS lookup itself failed.
    LookupFailed { domain: String, error: String },
}

impl SsoDomainError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadEmail => "SSO_EMAIL_INVALID",
            Self::DomainNotClaimed { .. } => "SSO_EMAIL_DOMAIN_NOT_CLAIMED",
            Self::DomainNotVerified { .. } => "SSO_DOMAIN_NOT_VERIFIED",
            Self::LookupFailed { .. } => "SSO_DOMAIN_LOOKUP_FAILED",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::BadEmail => "The identity provider sent an email address with no domain.".into(),
            Self::DomainNotClaimed { domain } => format!(
                "This organization's sign-in cannot vouch for {domain} addresses. Add {domain} to the organization's SSO domains and verify it."
            ),
            Self::DomainNotVerified {
                domain,
                record_name,
                record_value,
            } => format!(
                "Verify {domain}: add a TXT record named {record_name} with the value {record_value}."
            ),
            Self::LookupFailed { domain, error } => {
                format!("Could not check the DNS records of {domain}: {error}")
            }
        }
    }
}

fn operator_allowlist() -> Vec<String> {
    std::env::var("PYLON_SSO_ALLOWED_DOMAINS")
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn verified_cache() -> &'static Mutex<HashMap<(String, String), Instant>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<(String, String), Instant>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Whether `org_id` has proven it owns `domain`: the operator allowlist
/// names it, or its DNS carries the org's TXT record.
pub fn verify_domain_ownership(
    lookup: &dyn TxtLookup,
    org_id: &str,
    domain: &str,
) -> Result<(), SsoDomainError> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    if operator_allowlist().contains(&domain) {
        return Ok(());
    }
    let key = (org_id.to_string(), domain.clone());
    if let Some(at) = verified_cache().lock().unwrap().get(&key) {
        if at.elapsed() < VERIFIED_TTL {
            return Ok(());
        }
    }
    let (record_name, record_value) = domain_verification_record(org_id, &domain);
    let values = lookup
        .txt(&record_name)
        .map_err(|error| SsoDomainError::LookupFailed {
            domain: domain.clone(),
            error,
        })?;
    if values.iter().any(|v| v.trim() == record_value) {
        verified_cache().lock().unwrap().insert(key, Instant::now());
        Ok(())
    } else {
        Err(SsoDomainError::DomainNotVerified {
            domain,
            record_name,
            record_value,
        })
    }
}

/// Whether an org's IdP may vouch for `email`: its domain is one of the
/// org's SSO domains and the org has proven it owns that domain. Call this
/// before looking up or creating any user from an SSO assertion.
pub fn check_sso_email(
    lookup: &dyn TxtLookup,
    org_id: &str,
    org_domains: &[String],
    email: &str,
) -> Result<(), SsoDomainError> {
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().trim_end_matches('.').to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .ok_or(SsoDomainError::BadEmail)?;
    if !org_domains.iter().any(|d| d.eq_ignore_ascii_case(&domain)) {
        return Err(SsoDomainError::DomainNotClaimed { domain });
    }
    verify_domain_ownership(lookup, org_id, &domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DNS zone held in memory.
    struct Zone(Vec<(String, String)>);

    impl TxtLookup for Zone {
        fn txt(&self, name: &str) -> Result<Vec<String>, String> {
            Ok(self
                .0
                .iter()
                .filter(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .collect())
        }
    }

    fn published(org_id: &str, domain: &str) -> Zone {
        let (name, value) = domain_verification_record(org_id, domain);
        Zone(vec![(name, value)])
    }

    /// The account takeover: an attacker creates an org, points its SSO at
    /// an IdP they run, and has it assert the victim's address.
    #[test]
    fn an_org_idp_cannot_vouch_for_an_address_outside_its_domains() {
        let zone = published("org_attacker", "attacker.dev");
        let domains = vec!["attacker.dev".to_string()];
        let err = check_sso_email(&zone, "org_attacker", &domains, "victim@victim.com")
            .expect_err("a victim.com address must be refused");
        assert_eq!(err.code(), "SSO_EMAIL_DOMAIN_NOT_CLAIMED");
    }

    #[test]
    fn listing_a_domain_without_the_dns_proof_is_not_enough() {
        let domains = vec!["victim.com".to_string()];
        let err = check_sso_email(&Zone(vec![]), "org_squat", &domains, "victim@victim.com")
            .expect_err("an unverified domain must be refused");
        assert_eq!(err.code(), "SSO_DOMAIN_NOT_VERIFIED");
        assert!(err.message().contains("_pylon-sso.victim.com"));
    }

    #[test]
    fn a_record_published_for_another_org_proves_nothing() {
        let zone = published("org_real", "acme.com");
        let domains = vec!["acme.com".to_string()];
        let err = check_sso_email(&zone, "org_other", &domains, "jane@acme.com").unwrap_err();
        assert_eq!(err.code(), "SSO_DOMAIN_NOT_VERIFIED");
    }

    #[test]
    fn a_verified_domain_accepts_its_addresses() {
        let zone = published("org_acme_verified", "acme.com");
        let domains = vec!["acme.com".to_string()];
        check_sso_email(&zone, "org_acme_verified", &domains, "Jane@ACME.com").unwrap();
    }

    #[test]
    fn a_failed_lookup_refuses_sign_in() {
        struct Down;
        impl TxtLookup for Down {
            fn txt(&self, _: &str) -> Result<Vec<String>, String> {
                Err("timeout".into())
            }
        }
        let domains = vec!["acme.com".to_string()];
        let err = check_sso_email(&Down, "org_down", &domains, "jane@acme.com").unwrap_err();
        assert_eq!(err.code(), "SSO_DOMAIN_LOOKUP_FAILED");
    }

    #[test]
    fn records_differ_per_org_and_ignore_case() {
        let a = domain_verification_record("org_a", "Acme.com.");
        assert_eq!(a, domain_verification_record("org_a", "acme.com"));
        assert_ne!(a.1, domain_verification_record("org_b", "acme.com").1);
        assert_eq!(a.0, "_pylon-sso.acme.com");
    }

    #[test]
    fn doh_answers_join_split_strings() {
        let body = r#"{"Status":0,"Answer":[
            {"name":"_pylon-sso.acme.com.","type":16,"data":"\"pylon-sso-verification=ab\" \"cd\""},
            {"name":"_pylon-sso.acme.com.","type":5,"data":"other.example."}
        ]}"#;
        assert_eq!(
            parse_doh_txt(body).unwrap(),
            vec!["pylon-sso-verification=abcd".to_string()]
        );
        assert_eq!(
            parse_doh_txt(r#"{"Status":3}"#).unwrap(),
            Vec::<String>::new()
        );
        assert!(parse_doh_txt(r#"{"Status":2}"#).is_err());
    }
}
