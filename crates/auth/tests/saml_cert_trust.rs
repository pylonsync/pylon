//! Regression test for the Wave-9 review's open question: does samael's
//! `parse_base64_response` actually bind signature verification to the
//! cert configured in our IdP `EntityDescriptor`, or does it accept a
//! Response signed by ANY cert (e.g. one embedded in the Response's own
//! `<KeyInfo>` block)?
//!
//! The way to prove it: generate two distinct IdP keypairs (cert A and
//! cert B), sign a Response with key A, configure pylon-auth's SAML
//! store with cert B, and assert verify_and_parse_response REJECTS.
//!
//! ## macOS instability — `#[ignore]` by default
//!
//! `samael::idp::IdentityProvider::sign_authn_response` calls
//! libxmlsec1's `sign_document`. On macOS with Homebrew-installed
//! libxmlsec1 + OpenSSL 3.x linked transitively through samael's
//! `openssl-sys` dep, that call SIGSEGVs reproducibly inside
//! `XmlSecKey::from_memory`. The verify path
//! (`Crypto::reduce_xml_to_signed`) is fine — only the signing-side
//! init is broken on this combo.
//!
//! Linux CI links a different libxmlsec1 build and the test runs
//! cleanly there. To run locally on macOS: rebuild Homebrew's
//! `libxmlsec1` against the same OpenSSL 3 you have, OR run the test
//! in a Linux container.
//!
//! Set `PYLON_RUN_XMLSEC_SIGN_TESTS=1` to opt in (CI sets this).

use pylon_auth::saml::{verify_and_parse_response, SamlConfig};
use samael::crypto::CertificateDer;
use samael::idp::response_builder::ResponseAttribute;
use samael::idp::sp_extractor::RequiredAttribute;
use samael::idp::{CertificateParams, IdentityProvider, KeyType, Rsa};
use samael::traits::ToXml;

const ISSUER: &str = "https://test-idp.example/saml";
const SP_ENTITY_ID: &str = "https://my-sp.example";
const ACS_URL: &str = "https://my-sp.example/saml/acs";

fn opt_in() -> bool {
    std::env::var("PYLON_RUN_XMLSEC_SIGN_TESTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn fresh_idp() -> (IdentityProvider, CertificateDer) {
    let idp =
        IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048)).expect("generate IdP keypair");
    let cert = idp
        .create_certificate(&CertificateParams {
            common_name: "Test IdP",
            issuer_name: "Test IdP",
            days_until_expiration: 1,
        })
        .expect("self-sign cert");
    (idp, cert)
}

fn cert_to_pem(der: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

fn config_with_cert(cert_pem: &str) -> SamlConfig {
    SamlConfig {
        org_id: "acme".into(),
        idp_entity_id: ISSUER.into(),
        idp_sso_url: "https://test-idp.example/sso".into(),
        idp_x509_cert_pem: cert_pem.into(),
        default_role: "Member".into(),
        email_domains: vec![],
        email_attribute: "email".into(),
        name_attribute: None,
        created_at: 0,
        updated_at: 0,
    }
}

fn sign_response(idp: &IdentityProvider, cert_der: &CertificateDer, request_id: &str) -> String {
    let attrs = [ResponseAttribute {
        required_attribute: RequiredAttribute {
            name: "email".into(),
            format: None,
        },
        value: "jane@acme.com",
    }];
    let signed = idp
        .sign_authn_response(
            cert_der,
            "jane@acme.com",
            SP_ENTITY_ID,
            ACS_URL,
            ISSUER,
            request_id,
            &attrs,
        )
        .expect("sign Response");
    let xml = ToXml::to_string(&signed).expect("Response → XML");
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(xml.as_bytes())
}

#[test]
#[ignore = "macOS xmlsec1+OpenSSL3 SIGSEGV; set PYLON_RUN_XMLSEC_SIGN_TESTS=1 in CI"]
fn response_signed_by_other_cert_is_rejected() {
    if !opt_in() {
        return;
    }
    // IdP A signs.
    let (idp_a, cert_a) = fresh_idp();
    // IdP B's cert is what the SamlConfig declares as the trust anchor.
    let (_idp_b, cert_b) = fresh_idp();

    let request_id = "_req_other_cert";
    let signed_b64 = sign_response(&idp_a, &cert_a, request_id);
    let cfg = config_with_cert(&cert_to_pem(cert_b.der_data()));

    let result = verify_and_parse_response(&cfg, SP_ENTITY_ID, ACS_URL, &signed_b64, request_id);
    assert!(
        result.is_err(),
        "Response signed by cert A must NOT verify against configured cert B (got {result:?})"
    );
}

#[test]
#[ignore = "macOS xmlsec1+OpenSSL3 SIGSEGV; set PYLON_RUN_XMLSEC_SIGN_TESTS=1 in CI"]
fn response_signed_by_configured_cert_is_accepted() {
    if !opt_in() {
        return;
    }
    // Positive control: same flow, but we configure the cert that
    // actually signed. Confirms the test rig isn't false-rejecting and
    // proves verify_and_parse_response can ACCEPT a properly signed
    // Response — not just refuse everything.
    let (idp, cert) = fresh_idp();
    let request_id = "_req_same_cert";
    let signed_b64 = sign_response(&idp, &cert, request_id);
    let cfg = config_with_cert(&cert_to_pem(cert.der_data()));

    let result = verify_and_parse_response(&cfg, SP_ENTITY_ID, ACS_URL, &signed_b64, request_id);
    let assertion = result.expect("Response signed by configured cert must verify");
    assert_eq!(assertion.email, "jane@acme.com");
    assert_eq!(assertion.in_response_to.as_deref(), Some(request_id));
}
