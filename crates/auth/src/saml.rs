//! SAML 2.0 single sign-on, scoped to per-org configuration. Mirrors
//! the shape of [`crate::org_sso`] (OIDC) but speaks SAML's protocol:
//! HTTP-Redirect AuthnRequest binding, HTTP-POST Response binding,
//! IdP metadata XML parsing.
//!
//! Signature verification is performed by `samael` with its `xmlsec`
//! feature, which links libxml2 + libxmlsec1 at runtime. The
//! Dockerfile installs both system packages; standalone binaries link
//! against the shared libraries the host has installed.
//!
//! Validation chain on every incoming Response:
//! - XMLDSig signature against the configured IdP cert
//! - InResponseTo matches the AuthnRequest we minted (anti-replay)
//! - Issuer matches the configured idp_entity_id
//! - NotBefore / NotOnOrAfter expiry windows + clock-skew slack
//! - AudienceRestriction includes the SP entity_id
//! - SubjectConfirmation is a bearer token with valid recipient
//!
//! All of this happens inside `samael::ServiceProvider::parse_base64_response`;
//! a single `Err(_)` return terminates the auth flow.

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

/// Decoded SAML assertion (post-validation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SamlAssertion {
    pub email: String,
    pub name: Option<String>,
    /// The original AuthnRequest's ID, echoed back via `InResponseTo`.
    pub in_response_to: Option<String>,
}

/// Verify and parse a base64-encoded SAML Response (the `SAMLResponse`
/// form param from the HTTP-POST binding). Hard-fails on:
///
/// - bad XMLDSig signature (against the cert configured for the org)
/// - InResponseTo mismatch (anti-replay binding to our AuthnRequest)
/// - Issuer mismatch (idp_entity_id)
/// - expired NotBefore / NotOnOrAfter conditions
/// - audience restriction not including the SP entity_id
/// - bearer SubjectConfirmation missing or recipient mismatch
///
/// Returns the validated email + optional display name, ready to look
/// up or create a User row.
pub fn verify_and_parse_response(
    config: &SamlConfig,
    sp_entity_id: &str,
    sp_acs_url: &str,
    encoded_response: &str,
    expected_in_response_to: &str,
) -> Result<SamlAssertion, String> {
    let sp = build_service_provider(config, sp_entity_id, sp_acs_url)?;
    let possible_ids = [expected_in_response_to];
    let assertion = sp
        .parse_base64_response(encoded_response, Some(&possible_ids))
        .map_err(|e| format!("SAML response rejected: {e}"))?;

    // Resolve email: try named attribute first, then NameID@Format=emailAddress.
    let email = extract_attribute_value(&assertion, &config.email_attribute)
        .or_else(|| extract_email_nameid(&assertion))
        .ok_or_else(|| {
            "SAML assertion missing email attribute and NameID@Format=emailAddress".to_string()
        })?;

    // Display name: optional named attribute, falls back to None.
    let name = config
        .name_attribute
        .as_deref()
        .and_then(|attr| extract_attribute_value(&assertion, attr));

    // Echo the InResponseTo back to the caller. samael already verified
    // it matches `possible_ids`, so this is just a courtesy field for
    // downstream audit logs.
    let in_response_to = assertion
        .subject
        .as_ref()
        .and_then(|s| s.subject_confirmations.as_ref())
        .and_then(|confs| {
            confs
                .iter()
                .find_map(|c| c.subject_confirmation_data.as_ref())
        })
        .and_then(|d| d.in_response_to.clone())
        .or_else(|| Some(expected_in_response_to.to_string()));

    Ok(SamlAssertion {
        email,
        name,
        in_response_to,
    })
}

/// Build a `samael::ServiceProvider` from our flat per-org config.
/// Constructs a minimal IdP `EntityDescriptor` with a single signing
/// `KeyDescriptor` carrying the configured X.509 cert (PEM stripped of
/// BEGIN/END markers — samael wants raw base64 of the DER bytes).
fn build_service_provider(
    config: &SamlConfig,
    sp_entity_id: &str,
    sp_acs_url: &str,
) -> Result<samael::service_provider::ServiceProvider, String> {
    use samael::key_info::{KeyInfo, X509Data};
    use samael::metadata::{
        Endpoint, EntityDescriptor, IdpSsoDescriptor, KeyDescriptor, HTTP_POST_BINDING,
        HTTP_REDIRECT_BINDING,
    };
    use samael::service_provider::ServiceProvider;

    let cert_b64 = strip_pem_envelope(&config.idp_x509_cert_pem)?;

    let idp_descriptor = IdpSsoDescriptor {
        id: None,
        valid_until: None,
        cache_duration: None,
        protocol_support_enumeration: Some("urn:oasis:names:tc:SAML:2.0:protocol".to_string()),
        error_url: None,
        signature: None,
        key_descriptors: vec![KeyDescriptor {
            key_use: Some("signing".to_string()),
            encryption_methods: None,
            key_info: KeyInfo {
                id: None,
                x509_data: Some(X509Data {
                    certificates: vec![cert_b64],
                }),
            },
        }],
        organization: None,
        contact_people: vec![],
        artifact_resolution_service: vec![],
        single_logout_services: vec![],
        manage_name_id_services: vec![],
        name_id_formats: vec![],
        want_authn_requests_signed: None,
        single_sign_on_services: vec![
            Endpoint {
                binding: HTTP_REDIRECT_BINDING.to_string(),
                location: config.idp_sso_url.clone(),
                response_location: None,
            },
            Endpoint {
                binding: HTTP_POST_BINDING.to_string(),
                location: config.idp_sso_url.clone(),
                response_location: None,
            },
        ],
        name_id_mapping_services: vec![],
        assertion_id_request_services: vec![],
        attribute_profiles: vec![],
        attributes: vec![],
    };

    let idp_metadata = EntityDescriptor {
        entity_id: Some(config.idp_entity_id.clone()),
        idp_sso_descriptors: Some(vec![idp_descriptor]),
        ..EntityDescriptor::default()
    };

    Ok(ServiceProvider {
        entity_id: Some(sp_entity_id.to_string()),
        acs_url: Some(sp_acs_url.to_string()),
        idp_metadata,
        ..ServiceProvider::default()
    })
}

/// Strip PEM `-----BEGIN/END-----` envelopes + whitespace, leaving the
/// raw base64 of the certificate's DER bytes (what samael's
/// `X509Data::certificates` field expects).
fn strip_pem_envelope(pem: &str) -> Result<String, String> {
    let mut out = String::with_capacity(pem.len());
    for line in pem.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("-----") || trimmed.is_empty() {
            continue;
        }
        out.push_str(trimmed);
    }
    if out.is_empty() {
        return Err("certificate is empty after stripping PEM envelope".into());
    }
    Ok(out)
}

fn extract_attribute_value(
    assertion: &samael::schema::Assertion,
    attribute_name: &str,
) -> Option<String> {
    let statements = assertion.attribute_statements.as_ref()?;
    for statement in statements {
        for attr in &statement.attributes {
            if attr.name.as_deref() == Some(attribute_name)
                || attr.friendly_name.as_deref() == Some(attribute_name)
            {
                if let Some(v) = attr.values.first() {
                    if let Some(text) = v.value.as_deref() {
                        return Some(text.to_string());
                    }
                }
            }
        }
    }
    None
}

fn extract_email_nameid(assertion: &samael::schema::Assertion) -> Option<String> {
    let nameid = assertion.subject.as_ref()?.name_id.as_ref()?;
    if nameid
        .format
        .as_deref()
        .map(|f| f.contains("emailAddress"))
        .unwrap_or(false)
    {
        return Some(nameid.value.clone());
    }
    None
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
    fn strip_pem_envelope_handles_standard_format() {
        let pem = "-----BEGIN CERTIFICATE-----\nMIIBIjANBgkqhkiG9w0\nAQEFAAOCAQ8AMI\n-----END CERTIFICATE-----\n";
        let stripped = strip_pem_envelope(pem).unwrap();
        assert_eq!(stripped, "MIIBIjANBgkqhkiG9w0AQEFAAOCAQ8AMI");
    }

    #[test]
    fn strip_pem_envelope_handles_no_envelope() {
        // Operators sometimes paste raw base64 without the BEGIN/END
        // wrapper; we shouldn't reject that.
        let raw = "MIIBIjANBgkqhkiG9w0AQEFAAOCAQ8AMI";
        assert_eq!(strip_pem_envelope(raw).unwrap(), raw);
    }

    #[test]
    fn strip_pem_envelope_handles_crlf() {
        let pem = "-----BEGIN CERTIFICATE-----\r\nMIIBIj\r\nANBgkq\r\n-----END CERTIFICATE-----";
        assert_eq!(strip_pem_envelope(pem).unwrap(), "MIIBIjANBgkq");
    }

    #[test]
    fn strip_pem_envelope_rejects_empty() {
        let err = strip_pem_envelope("-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----")
            .unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn verify_rejects_unsigned_response() {
        // A handcrafted unsigned XML response must NOT verify against a
        // configured cert. Confirms the samael wire-up actually requires
        // a signature (vs silently passing through).
        let cfg = cfg("acme", vec![]);
        let xml = r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" InResponseTo="_abc">
          <saml:AttributeStatement>
            <saml:Attribute Name="http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress">
              <saml:AttributeValue>jane@acme.com</saml:AttributeValue>
            </saml:Attribute>
          </saml:AttributeStatement>
        </samlp:Response>"#;
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let result = verify_and_parse_response(
            &cfg,
            "https://my-app/sp",
            "https://my-app/acs",
            &b64,
            "_abc",
        );
        assert!(result.is_err(), "unsigned response must be rejected");
    }

    #[test]
    fn verify_rejects_garbage_base64() {
        let cfg = cfg("acme", vec![]);
        let result = verify_and_parse_response(
            &cfg,
            "https://my-app/sp",
            "https://my-app/acs",
            "not-base64!!!",
            "_abc",
        );
        assert!(result.is_err());
    }

    #[test]
    fn verify_rejects_response_with_invalid_pem_cert() {
        let mut cfg = cfg("acme", vec![]);
        cfg.idp_x509_cert_pem = "-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----".into();
        let result = verify_and_parse_response(
            &cfg,
            "https://my-app/sp",
            "https://my-app/acs",
            "Zm9v",
            "_abc",
        );
        assert!(result.is_err());
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
