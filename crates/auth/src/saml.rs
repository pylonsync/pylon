//! SAML 2.0 single sign-on, scoped to per-org configuration. Mirrors
//! the shape of [`crate::org_sso`] (OIDC) but speaks SAML's protocol:
//! HTTP-Redirect AuthnRequest binding, HTTP-POST Response binding,
//! IdP metadata XML parsing.
//!
//! **Security boundary — signature verification is OFF by default.**
//! This module ships the SAML protocol surface (request generation,
//! response parsing, attribute extraction) but does NOT verify the
//! IdP's XMLDSig signature on incoming Responses. That step requires
//! a vetted XML-canonicalization + signature library
//! (`samael` with its `xmlsec` feature, or hand-rolled C14N + ring).
//! Both add libxml2/xmlsec1 system dependencies that materially
//! complicate Pylon's binary distribution.
//!
//! Operators must explicitly opt-in to the unverified path by setting
//! `PYLON_SAML_INSECURE_NO_VERIFY=1`; without it, the callback handler
//! refuses every Response with a 501 + a clear pointer to the open
//! follow-up. This lets enterprise customers who run their own Pylon
//! in a controlled environment use SAML today (gating on operator
//! risk acceptance) while we wire the proper xmlsec path next sprint.
//!
//! Storage uses the existing AES-GCM seal envelope (`PYLON_SSO_ENCRYPTION_KEY`)
//! for the signing certificate's private key when the SP needs to
//! sign its AuthnRequests (most IdPs accept unsigned AuthnRequests
//! over HTTP-Redirect — we do that by default).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// One org's SAML 2.0 SSO configuration. The IdP metadata is parsed
/// at config-write time and the relevant fields cached so the start +
/// callback paths are zero-RPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamlConfig {
    pub org_id: String,
    /// IdP entity ID (the `entityID` attribute in the IdP metadata).
    pub idp_entity_id: String,
    /// IdP's SingleSignOnService endpoint (HTTP-Redirect binding).
    /// We send AuthnRequests here.
    pub idp_sso_url: String,
    /// PEM-encoded X.509 cert used to verify Response signatures. Stored
    /// raw because it's a public key — no encryption envelope needed.
    pub idp_x509_cert_pem: String,
    /// Org role granted to a freshly-auto-joined user. Same constraint
    /// as OrgSsoConfig: never `owner`.
    pub default_role: String,
    /// Email domains routed to this IdP for the domain-detection
    /// sign-in path. Stored lowercase, no leading `@`.
    pub email_domains: Vec<String>,
    /// SAML attribute name that carries the user's email. RFC 8141
    /// suggests urn:oid:0.9.2342.19200300.100.1.3 (mail) or
    /// http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress.
    /// Falls back to NameID@Format=emailAddress when the named attribute
    /// is absent.
    pub email_attribute: String,
    /// Attribute name carrying the user's display name. Optional —
    /// callback falls back to email when the attribute is missing.
    pub name_attribute: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl SamlConfig {
    /// The cert is public — no need to redact, but keep the same
    /// `redacted()` method shape as OrgSsoConfig for API consistency.
    pub fn redacted(&self) -> Self {
        self.clone()
    }
}

/// Pending AuthnRequest state. SAML doesn't have PKCE, but we mint a
/// per-flow token that the IdP echoes back via the `RelayState`
/// parameter. Single-use, 10-minute TTL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamlStateRecord {
    pub relay_state: String,
    pub org_id: String,
    /// AuthnRequest ID we generated. The IdP's Response carries this
    /// in `InResponseTo`; mismatch → reject.
    pub request_id: String,
    pub callback_url: String,
    pub error_callback_url: String,
    pub created_at: u64,
}

/// 10-minute SAML state TTL — same as [`crate::org_sso::STATE_TTL_SECS`].
pub const SAML_STATE_TTL_SECS: u64 = 10 * 60;

pub trait SamlStore: Send + Sync {
    fn get(&self, org_id: &str) -> Option<SamlConfig>;
    fn upsert(&self, config: SamlConfig);
    fn delete(&self, org_id: &str) -> bool;
    fn find_by_email_domain(&self, domain: &str) -> Option<String>;
    fn save_state(&self, record: SamlStateRecord);
    fn take_state(&self, relay_state: &str, expected_org_id: &str) -> Option<SamlStateRecord>;
}

pub struct InMemorySamlStore {
    configs: RwLock<HashMap<String, SamlConfig>>,
    domains: RwLock<HashMap<String, String>>,
    states: RwLock<HashMap<String, SamlStateRecord>>,
}

impl Default for InMemorySamlStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemorySamlStore {
    pub fn new() -> Self {
        Self {
            configs: RwLock::new(HashMap::new()),
            domains: RwLock::new(HashMap::new()),
            states: RwLock::new(HashMap::new()),
        }
    }
}

impl SamlStore for InMemorySamlStore {
    fn get(&self, org_id: &str) -> Option<SamlConfig> {
        self.configs.read().unwrap().get(org_id).cloned()
    }

    fn upsert(&self, config: SamlConfig) {
        let mut configs = self.configs.write().unwrap();
        if let Some(prev) = configs.get(&config.org_id) {
            let mut domains = self.domains.write().unwrap();
            for d in &prev.email_domains {
                if domains.get(d).map(|v| v == &config.org_id).unwrap_or(false) {
                    domains.remove(d);
                }
            }
        }
        let mut domains = self.domains.write().unwrap();
        for d in &config.email_domains {
            domains.insert(d.to_ascii_lowercase(), config.org_id.clone());
        }
        configs.insert(config.org_id.clone(), config);
    }

    fn delete(&self, org_id: &str) -> bool {
        let removed = self.configs.write().unwrap().remove(org_id);
        if let Some(cfg) = &removed {
            let mut domains = self.domains.write().unwrap();
            for d in &cfg.email_domains {
                if domains.get(d).map(|v| v == &cfg.org_id).unwrap_or(false) {
                    domains.remove(d);
                }
            }
        }
        removed.is_some()
    }

    fn find_by_email_domain(&self, domain: &str) -> Option<String> {
        self.domains
            .read()
            .unwrap()
            .get(&domain.to_ascii_lowercase())
            .cloned()
    }

    fn save_state(&self, record: SamlStateRecord) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut states = self.states.write().unwrap();
        states.retain(|_, v| now.saturating_sub(v.created_at) < SAML_STATE_TTL_SECS);
        states.insert(record.relay_state.clone(), record);
    }

    fn take_state(&self, relay_state: &str, expected_org_id: &str) -> Option<SamlStateRecord> {
        let mut states = self.states.write().unwrap();
        let candidate = states.get(relay_state)?.clone();
        if candidate.org_id != expected_org_id {
            return None;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now.saturating_sub(candidate.created_at) >= SAML_STATE_TTL_SECS {
            states.remove(relay_state);
            return None;
        }
        states.remove(relay_state);
        Some(candidate)
    }
}

/// Build the SAML 2.0 AuthnRequest XML. Returns `(request_id, xml)`.
/// The request is unsigned — most IdPs don't require signed requests
/// when received via HTTP-Redirect (they verify the originating SP via
/// the registered ACS URL instead).
///
/// Format follows OASIS SAML 2.0 Core §3.4. `id` is a random
/// `_<hex>` string per spec §1.3.4 (must start with a non-numeric).
pub fn build_authn_request(
    sp_entity_id: &str,
    idp_sso_url: &str,
    acs_url: &str,
) -> (String, String) {
    let request_id = generate_request_id();
    let now = chrono_now_iso();
    let xml = format!(
        r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{id}" Version="2.0" IssueInstant="{now}" Destination="{dest}" ProtocolBinding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST" AssertionConsumerServiceURL="{acs}"><saml:Issuer>{sp}</saml:Issuer><samlp:NameIDPolicy Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress" AllowCreate="true"/></samlp:AuthnRequest>"#,
        id = xml_escape(&request_id),
        now = xml_escape(&now),
        dest = xml_escape(idp_sso_url),
        acs = xml_escape(acs_url),
        sp = xml_escape(sp_entity_id),
    );
    (request_id, xml)
}

fn generate_request_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    use std::fmt::Write;
    let mut hex = String::from("_");
    for b in bytes {
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

fn chrono_now_iso() -> String {
    // SAML wants ISO 8601 UTC with the trailing Z. Format: YYYY-MM-DDTHH:MM:SSZ
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Year-month-day from epoch (no chrono dep — keep auth crate lean).
    let secs_per_day: u64 = 86_400;
    let days = now / secs_per_day;
    let secs_today = now % secs_per_day;
    let h = secs_today / 3600;
    let m = (secs_today % 3600) / 60;
    let s = secs_today % 60;
    let (y, mo, d) = days_to_ymd(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn days_to_ymd(days: i64) -> (i32, u32, u32) {
    // Civil from days algorithm — Howard Hinnant.
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Encode an AuthnRequest for the HTTP-Redirect binding: deflate +
/// base64 + URL-encode the XML, then build the `?SAMLRequest=…&RelayState=…`
/// query string per OASIS SAML 2.0 Bindings §3.4.4.
///
/// Note: deflate (raw, no zlib header) is mandated by the binding spec.
pub fn encode_redirect_binding(xml: &str, relay_state: &str) -> String {
    use std::io::Write;
    // Raw DEFLATE (no zlib wrapper) — flate2's DeflateEncoder fits.
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(xml.as_bytes());
    let compressed = encoder.finish().unwrap_or_default();
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);
    format!(
        "SAMLRequest={}&RelayState={}",
        url_form(&b64),
        url_form(relay_state),
    )
}

fn url_form(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Decoded SAML Response. Populated by `parse_response` after
/// signature verification (when enabled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamlAssertion {
    pub email: String,
    pub name: Option<String>,
    /// The original AuthnRequest's ID, echoed back via `InResponseTo`.
    /// `None` when the IdP omitted it (some IdPs do for IdP-initiated
    /// flows; we reject those by requiring a value).
    pub in_response_to: Option<String>,
}

/// Parse a base64-encoded SAML Response (the `SAMLResponse` form param
/// from the HTTP-POST binding). Extracts the email + name attributes.
///
/// **DOES NOT verify the XMLDSig signature.** The caller MUST gate this
/// behind a feature toggle that explicitly accepts the security trade-
/// off. See [`require_signature_verification_or_refuse`].
pub fn parse_response_unverified(
    b64: &str,
    expected_in_response_to: &str,
) -> Result<SamlAssertion, String> {
    use base64::Engine;
    let xml_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.replace(['\n', '\r', ' '], ""))
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    let xml = std::str::from_utf8(&xml_bytes).map_err(|e| format!("response is not utf-8: {e}"))?;

    // Lightweight extraction: the response is well-formed XML from a
    // trusted IdP (modulo signature). We pull the fields we need by
    // string scan rather than instantiating a full XML DOM. This is
    // brittle against weird-but-valid XML formatting; production
    // operators should layer the proper XML parser via samael.
    let in_response_to =
        pluck_attr(xml, "InResponseTo").or_else(|| pluck_attr(xml, "InResponseTo="));
    if let Some(irt) = &in_response_to {
        if irt != expected_in_response_to {
            return Err(format!(
                "InResponseTo mismatch (got `{irt}`, expected `{expected_in_response_to}`)"
            ));
        }
    } else {
        return Err("Response missing required InResponseTo attribute".into());
    }
    let email = pluck_email_attribute(xml).ok_or_else(|| {
        "Response missing email attribute or NameID with format=emailAddress".to_string()
    })?;
    let name = pluck_attribute_value(xml, "name");
    Ok(SamlAssertion {
        email,
        name,
        in_response_to,
    })
}

fn pluck_attr(xml: &str, name: &str) -> Option<String> {
    let needle = format!(r#"{name}=""#);
    let start = xml.find(&needle)? + needle.len();
    let rest = &xml[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn pluck_email_attribute(xml: &str) -> Option<String> {
    // Try common attribute names in priority order.
    for attr_name in [
        "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress",
        "urn:oid:0.9.2342.19200300.100.1.3",
        "mail",
        "email",
    ] {
        if let Some(v) = pluck_attribute_value(xml, attr_name) {
            return Some(v);
        }
    }
    // Fall back to NameID with Format=emailAddress.
    if xml.contains("Format=\"urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress\"") {
        let needle = "<saml:NameID";
        if let Some(start) = xml.find(needle) {
            let rest = &xml[start..];
            if let Some(open_end) = rest.find('>') {
                let after_open = &rest[open_end + 1..];
                if let Some(close) = after_open.find("</saml:NameID>") {
                    return Some(after_open[..close].trim().to_string());
                }
            }
        }
    }
    None
}

fn pluck_attribute_value(xml: &str, attr_name: &str) -> Option<String> {
    let needle = format!(r#"Name="{attr_name}""#);
    let start = xml.find(&needle)? + needle.len();
    let rest = &xml[start..];
    // Find <saml:AttributeValue>VALUE</saml:AttributeValue> following.
    let v_open = rest.find("<saml:AttributeValue")?;
    let after = &rest[v_open..];
    let close_open = after.find('>')?;
    let after_open = &after[close_open + 1..];
    let close = after_open.find("</saml:AttributeValue>")?;
    Some(after_open[..close].trim().to_string())
}

/// True when the operator has explicitly opted in to running SAML
/// without signature verification. Default (unset) → false → the
/// callback handler must refuse.
pub fn signature_verification_bypassed() -> bool {
    std::env::var("PYLON_SAML_INSECURE_NO_VERIFY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(org: &str, domains: Vec<&str>) -> SamlConfig {
        SamlConfig {
            org_id: org.into(),
            idp_entity_id: "https://acme.okta.com/saml".into(),
            idp_sso_url: "https://acme.okta.com/app/test/sso/saml".into(),
            idp_x509_cert_pem: "-----BEGIN CERTIFICATE-----\nMI...\n-----END CERTIFICATE-----"
                .into(),
            default_role: "Member".into(),
            email_domains: domains.into_iter().map(String::from).collect(),
            email_attribute: "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress"
                .into(),
            name_attribute: Some("name".into()),
            created_at: 100,
            updated_at: 100,
        }
    }

    #[test]
    fn upsert_and_get_round_trip() {
        let s = InMemorySamlStore::new();
        s.upsert(cfg("acme", vec!["acme.com"]));
        let got = s.get("acme").unwrap();
        assert_eq!(got.idp_entity_id, "https://acme.okta.com/saml");
    }

    #[test]
    fn find_by_email_domain_is_case_insensitive() {
        let s = InMemorySamlStore::new();
        s.upsert(cfg("acme", vec!["acme.com"]));
        assert_eq!(s.find_by_email_domain("ACME.COM").as_deref(), Some("acme"));
        assert_eq!(s.find_by_email_domain("nope.com"), None);
    }

    #[test]
    fn upsert_replaces_domain_index() {
        let s = InMemorySamlStore::new();
        s.upsert(cfg("acme", vec!["old.com"]));
        s.upsert(cfg("acme", vec!["new.com"]));
        assert_eq!(s.find_by_email_domain("old.com"), None);
        assert_eq!(s.find_by_email_domain("new.com").as_deref(), Some("acme"));
    }

    #[test]
    fn delete_clears_domain_index() {
        let s = InMemorySamlStore::new();
        s.upsert(cfg("acme", vec!["acme.com"]));
        assert!(s.delete("acme"));
        assert_eq!(s.find_by_email_domain("acme.com"), None);
    }

    #[test]
    fn build_authn_request_xml_has_required_fields() {
        let (id, xml) =
            build_authn_request("https://my-app/sp", "https://idp/sso", "https://my-app/acs");
        assert!(id.starts_with('_'));
        assert!(xml.contains(&format!(r#"ID="{id}""#)));
        assert!(xml.contains(r#"ProtocolBinding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST""#));
        assert!(xml.contains(r#"AssertionConsumerServiceURL="https://my-app/acs""#));
        assert!(xml.contains("<saml:Issuer>https://my-app/sp</saml:Issuer>"));
        assert!(xml.contains(r#"NameIDPolicy"#));
    }

    #[test]
    fn redirect_binding_encoding_round_trips_with_deflate() {
        let (_id, xml) = build_authn_request("sp", "https://idp/sso", "acs");
        let encoded = encode_redirect_binding(&xml, "rs_123");
        assert!(encoded.starts_with("SAMLRequest="));
        assert!(encoded.contains("&RelayState=rs_123"));
        // Pull SAMLRequest, base64 decode, deflate-decompress, verify
        // the XML round-trips.
        let req = encoded
            .strip_prefix("SAMLRequest=")
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        let req_decoded: String = req
            .replace("%2F", "/")
            .replace("%2B", "+")
            .replace("%3D", "=");
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(req_decoded)
            .unwrap();
        use flate2::read::DeflateDecoder;
        use std::io::Read;
        let mut decoder = DeflateDecoder::new(&bytes[..]);
        let mut decoded = String::new();
        decoder.read_to_string(&mut decoded).unwrap();
        assert_eq!(decoded, xml);
    }

    #[test]
    fn parse_response_extracts_email_attribute() {
        let xml = r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" InResponseTo="_abc">
          <saml:AttributeStatement>
            <saml:Attribute Name="http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress">
              <saml:AttributeValue>jane@acme.com</saml:AttributeValue>
            </saml:Attribute>
            <saml:Attribute Name="name">
              <saml:AttributeValue>Jane Doe</saml:AttributeValue>
            </saml:Attribute>
          </saml:AttributeStatement>
        </samlp:Response>"#;
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let assertion = parse_response_unverified(&b64, "_abc").unwrap();
        assert_eq!(assertion.email, "jane@acme.com");
        assert_eq!(assertion.name.as_deref(), Some("Jane Doe"));
        assert_eq!(assertion.in_response_to.as_deref(), Some("_abc"));
    }

    #[test]
    fn parse_response_falls_back_to_nameid() {
        let xml = r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" InResponseTo="_xyz">
          <saml:Subject>
            <saml:NameID Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress">user@example.com</saml:NameID>
          </saml:Subject>
        </samlp:Response>"#;
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let assertion = parse_response_unverified(&b64, "_xyz").unwrap();
        assert_eq!(assertion.email, "user@example.com");
        assert!(assertion.name.is_none());
    }

    #[test]
    fn parse_response_rejects_in_response_to_mismatch() {
        let xml = r#"<samlp:Response InResponseTo="_attacker">
          <saml:Subject><saml:NameID Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress">x</saml:NameID></saml:Subject>
        </samlp:Response>"#;
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let err = parse_response_unverified(&b64, "_my_request").unwrap_err();
        assert!(err.contains("InResponseTo mismatch"));
    }

    #[test]
    fn parse_response_rejects_missing_in_response_to() {
        let xml = r#"<samlp:Response>no in_response_to here</samlp:Response>"#;
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let err = parse_response_unverified(&b64, "_anything").unwrap_err();
        assert!(err.contains("InResponseTo"));
    }

    #[test]
    fn signature_bypass_off_by_default() {
        // Test environment doesn't set the env var; bypass must be off.
        // (We intentionally don't mutate env here — tests run in
        // parallel and process-wide env mutation would cross-contaminate.)
        assert!(!signature_verification_bypassed());
    }

    #[test]
    fn xml_escape_handles_attr_chars() {
        assert_eq!(xml_escape(r#"a<b&c"d'e"#), "a&lt;b&amp;c&quot;d&apos;e");
    }

    #[test]
    fn state_round_trip_and_replay_blocked() {
        let store = InMemorySamlStore::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let rec = SamlStateRecord {
            relay_state: "rs_1".into(),
            org_id: "acme".into(),
            request_id: "_req".into(),
            callback_url: "u".into(),
            error_callback_url: "u".into(),
            created_at: now,
        };
        store.save_state(rec);
        assert!(store.take_state("rs_1", "acme").is_some());
        assert!(store.take_state("rs_1", "acme").is_none(), "single-use");
    }

    #[test]
    fn state_take_rejects_wrong_org() {
        let store = InMemorySamlStore::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        store.save_state(SamlStateRecord {
            relay_state: "rs_2".into(),
            org_id: "acme".into(),
            request_id: "_r".into(),
            callback_url: "u".into(),
            error_callback_url: "u".into(),
            created_at: now,
        });
        assert!(store.take_state("rs_2", "evil").is_none());
        assert!(store.take_state("rs_2", "acme").is_some());
    }
}
