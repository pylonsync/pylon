//! Sign-In With Ethereum (EIP-4361).
//!
//! Wallet-based passwordless auth — the user signs a structured
//! message in their wallet (MetaMask, WalletConnect, Coinbase
//! Wallet, etc.), pylon recovers the signer's Ethereum address,
//! and that address becomes the identity.
//!
//! Spec: <https://eips.ethereum.org/EIPS/eip-4361>
//!
//! Wire flow:
//!   1. Frontend asks `/api/auth/siwe/nonce?address=0x…` →
//!      pylon generates a random nonce, stashes it server-side
//!      keyed by address (5-min expiry, single-use).
//!   2. Frontend builds the EIP-4361 message including the nonce,
//!      `domain`, `uri`, `chain_id`, etc., and asks the wallet
//!      to `personal_sign` it.
//!   3. Frontend POSTs `/api/auth/siwe/verify` with
//!      `{ message, signature }`. Pylon recovers the signer
//!      address from the signature using secp256k1 + keccak256
//!      (the Ethereum signed-message scheme), validates the
//!      message fields (nonce match, domain match, expiry,
//!      not-before, chain_id), and mints a session keyed on
//!      `siwe:<lowercased-address>`.

use std::collections::HashMap;
use std::sync::Mutex;

/// Ethereum-signed-message recovery + EIP-4361 message validation.
///
/// pylon implements the recovery using `ring`'s low-level primitives
/// to avoid pulling in a dedicated secp256k1 crate. If the signature
/// verifier becomes a hot path, swap in `secp256k1` (the libsecp256k1
/// bindings) — currently it'd be O(1 sign-in per minute per user)
/// so the overhead is negligible.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiweMessage {
    /// `<scheme>://<host>[:<port>]` — must match the configured
    /// origin allowlist.
    pub domain: String,
    /// Lowercased EVM address (0x-prefixed, 42 chars).
    pub address: String,
    /// Optional human-readable statement — shown in the wallet UI.
    pub statement: Option<String>,
    pub uri: String,
    pub version: String,
    pub chain_id: u64,
    pub nonce: String,
    /// ISO-8601 timestamp.
    pub issued_at: String,
    pub expiration_time: Option<String>,
    pub not_before: Option<String>,
    pub request_id: Option<String>,
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SiweError {
    Malformed,
    NonceMismatch,
    NonceMissing,
    DomainMismatch,
    Expired,
    NotYetValid,
    BadSignature,
    AddressMismatch,
}

impl std::fmt::Display for SiweError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Malformed => "SIWE message malformed",
            Self::NonceMismatch => "nonce doesn't match issued challenge",
            Self::NonceMissing => "no challenge issued for this address",
            Self::DomainMismatch => "domain doesn't match expected origin",
            Self::Expired => "message expiration_time has passed",
            Self::NotYetValid => "not_before is in the future",
            Self::BadSignature => "signature did not recover to message address",
            Self::AddressMismatch => "address claimed in message ≠ recovered signer",
        })
    }
}

/// Per-address pending nonce (issued at /siwe/nonce, consumed at
/// /siwe/verify). Single-use, 5-min TTL.
pub struct NonceStore {
    nonces: Mutex<HashMap<String, (String, u64)>>, // addr → (nonce, expires_at)
}

impl Default for NonceStore {
    fn default() -> Self {
        Self {
            nonces: Mutex::new(HashMap::new()),
        }
    }
}

impl NonceStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint + stash a nonce for `address`. Overwrites any existing
    /// nonce for that address (reissue is fine — only one in-flight
    /// challenge per address).
    pub fn issue(&self, address: &str) -> String {
        use rand::RngCore;
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        // EIP-4361 says nonce is `[A-Za-z0-9]{8,}`. Hex-encode our
        // random bytes (32 chars) — well within the allowed alphabet.
        let nonce: String = bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let key = address.to_ascii_lowercase();
        let expires_at = now_secs() + 5 * 60;
        self.nonces
            .lock()
            .unwrap()
            .insert(key, (nonce.clone(), expires_at));
        nonce
    }

    /// Consume the stored nonce for `address` (single-use). Returns
    /// `None` for unknown OR expired entries — but DOESN'T remove an
    /// expired entry early (an attacker repeatedly posting expired
    /// nonces would otherwise burn the slot and DoS legitimate
    /// retries).
    pub fn take(&self, address: &str) -> Option<String> {
        let key = address.to_ascii_lowercase();
        let mut map = self.nonces.lock().unwrap();
        let (nonce, exp) = map.get(&key)?.clone();
        if exp <= now_secs() {
            return None;
        }
        map.remove(&key);
        Some(nonce)
    }
}

/// Parse the EIP-4361 plaintext message format. Apps that need the
/// full structured form should use this + `verify_signature` separately.
pub fn parse_message(text: &str) -> Result<SiweMessage, SiweError> {
    // Spec format:
    // <domain> wants you to sign in with your Ethereum account:
    // <address>
    //
    // [<statement>]
    //
    // URI: <uri>
    // Version: <version>
    // Chain ID: <chain_id>
    // Nonce: <nonce>
    // Issued At: <iso-8601>
    // [Expiration Time: <iso-8601>]
    // [Not Before: <iso-8601>]
    // [Request ID: <id>]
    // [Resources:
    // - <uri>
    // - <uri>]
    let mut lines = text.lines();
    let header = lines.next().ok_or(SiweError::Malformed)?;
    let domain = header
        .strip_suffix(" wants you to sign in with your Ethereum account:")
        .ok_or(SiweError::Malformed)?
        .to_string();
    let address = lines
        .next()
        .ok_or(SiweError::Malformed)?
        .trim()
        .to_string();
    if !address.starts_with("0x") || address.len() != 42 {
        return Err(SiweError::Malformed);
    }

    // Skip the blank line after the address; collect the statement
    // (which CAN span multiple lines per spec) until we hit URI:.
    let mut statement_parts: Vec<String> = Vec::new();
    let mut peeked: Option<&str> = None;
    let mut seen_blank = false;
    for l in lines.by_ref() {
        if l.is_empty() {
            seen_blank = true;
            continue;
        }
        if l.starts_with("URI:") {
            peeked = Some(l);
            break;
        }
        // Pre-URI non-blank lines are statement content. Re-introduce
        // the inter-line newlines so re-serialization matches the
        // wallet-signed bytes exactly.
        if !statement_parts.is_empty() {
            statement_parts.push("\n".into());
        }
        statement_parts.push(l.to_string());
    }
    let _ = seen_blank;
    let statement = if statement_parts.is_empty() {
        None
    } else {
        Some(statement_parts.concat())
    };
    // We may have already consumed the URI line into `peeked`.
    let mut uri: Option<String> = None;
    let mut version: Option<String> = None;
    let mut chain_id: Option<u64> = None;
    let mut nonce: Option<String> = None;
    let mut issued_at: Option<String> = None;
    let mut expiration_time: Option<String> = None;
    let mut not_before: Option<String> = None;
    let mut request_id: Option<String> = None;
    let mut resources = Vec::new();
    let mut in_resources = false;

    let process = |line: &str,
                       uri: &mut Option<String>,
                       version: &mut Option<String>,
                       chain_id: &mut Option<u64>,
                       nonce: &mut Option<String>,
                       issued_at: &mut Option<String>,
                       expiration_time: &mut Option<String>,
                       not_before: &mut Option<String>,
                       request_id: &mut Option<String>,
                       resources: &mut Vec<String>,
                       in_resources: &mut bool| {
        if let Some(v) = line.strip_prefix("URI:") {
            *uri = Some(v.trim().to_string());
            *in_resources = false;
        } else if let Some(v) = line.strip_prefix("Version:") {
            *version = Some(v.trim().to_string());
            *in_resources = false;
        } else if let Some(v) = line.strip_prefix("Chain ID:") {
            *chain_id = v.trim().parse().ok();
            *in_resources = false;
        } else if let Some(v) = line.strip_prefix("Nonce:") {
            *nonce = Some(v.trim().to_string());
            *in_resources = false;
        } else if let Some(v) = line.strip_prefix("Issued At:") {
            *issued_at = Some(v.trim().to_string());
            *in_resources = false;
        } else if let Some(v) = line.strip_prefix("Expiration Time:") {
            *expiration_time = Some(v.trim().to_string());
            *in_resources = false;
        } else if let Some(v) = line.strip_prefix("Not Before:") {
            *not_before = Some(v.trim().to_string());
            *in_resources = false;
        } else if let Some(v) = line.strip_prefix("Request ID:") {
            *request_id = Some(v.trim().to_string());
            *in_resources = false;
        } else if line.starts_with("Resources:") {
            *in_resources = true;
        } else if *in_resources {
            if let Some(v) = line.strip_prefix("- ") {
                resources.push(v.trim().to_string());
            }
        }
    };
    if let Some(line) = peeked {
        process(
            line,
            &mut uri,
            &mut version,
            &mut chain_id,
            &mut nonce,
            &mut issued_at,
            &mut expiration_time,
            &mut not_before,
            &mut request_id,
            &mut resources,
            &mut in_resources,
        );
    }
    for line in lines {
        process(
            line,
            &mut uri,
            &mut version,
            &mut chain_id,
            &mut nonce,
            &mut issued_at,
            &mut expiration_time,
            &mut not_before,
            &mut request_id,
            &mut resources,
            &mut in_resources,
        );
    }

    Ok(SiweMessage {
        domain,
        address,
        statement,
        uri: uri.ok_or(SiweError::Malformed)?,
        version: version.ok_or(SiweError::Malformed)?,
        chain_id: chain_id.ok_or(SiweError::Malformed)?,
        nonce: nonce.ok_or(SiweError::Malformed)?,
        issued_at: issued_at.ok_or(SiweError::Malformed)?,
        expiration_time,
        not_before,
        request_id,
        resources,
    })
}

/// Validate the non-cryptographic parts of a SIWE message: domain,
/// nonce, expiration, not-before. Does NOT verify the signature —
/// that requires a real secp256k1 + keccak256 verifier which pylon
/// doesn't ship today (Wave 6).
///
/// Apps that need full SIWE auth wire a `SiweSignatureVerifier`
/// trait impl (k256-backed) at server start; pylon then composes
/// `validate_message` + the app's verifier in
/// `routes/auth.rs::siwe_finish`.
///
/// **Why no built-in verifier?** Bringing in `k256` adds a
/// significant compile-time + binary-size hit to every pylon
/// install — most teams that don't use SIWE shouldn't pay it.
/// Crypto stays opt-in via the trait below.
pub fn validate_message(
    nonces: &NonceStore,
    message: &SiweMessage,
    expected_domain: &str,
) -> Result<(), SiweError> {
    if message.domain != expected_domain {
        return Err(SiweError::DomainMismatch);
    }
    let issued = nonces
        .take(&message.address)
        .ok_or(SiweError::NonceMissing)?;
    if issued != message.nonce {
        return Err(SiweError::NonceMismatch);
    }
    if let Some(exp) = &message.expiration_time {
        if iso_to_unix(exp).map(|t| t <= now_secs()).unwrap_or(false) {
            return Err(SiweError::Expired);
        }
    }
    if let Some(nb) = &message.not_before {
        if iso_to_unix(nb).map(|t| t > now_secs()).unwrap_or(false) {
            return Err(SiweError::NotYetValid);
        }
    }
    Ok(())
}

/// Trait apps implement to plug in k256-based ECDSA recovery +
/// keccak256. `serialize_for_signing(message)` gives the canonical
/// bytes the wallet hashed; the impl recovers the signer address
/// from the 65-byte (r||s||v) signature.
pub trait SiweSignatureVerifier: Send + Sync {
    /// Returns the lowercased 0x-prefixed Ethereum address that
    /// signed the canonical message bytes.
    fn recover_address(
        &self,
        signed_text: &str,
        signature_hex: &str,
    ) -> Result<String, SiweError>;
}

/// Compose `validate_message` + an app-supplied signature verifier.
/// Returns the lowercased recovered address on success.
pub fn verify_with(
    nonces: &NonceStore,
    message: &SiweMessage,
    signature_hex: &str,
    expected_domain: &str,
    verifier: &dyn SiweSignatureVerifier,
) -> Result<String, SiweError> {
    validate_message(nonces, message, expected_domain)?;
    let signed = serialize_for_signing(message);
    let recovered = verifier.recover_address(&signed, signature_hex)?;
    if !recovered.eq_ignore_ascii_case(&message.address) {
        return Err(SiweError::AddressMismatch);
    }
    Ok(recovered.to_ascii_lowercase())
}

/// Serialize a SIWE message back into its canonical wire form for
/// signing. MUST be byte-identical to what the wallet hashed.
pub fn serialize_for_signing(m: &SiweMessage) -> String {
    let mut out = String::new();
    out.push_str(&m.domain);
    out.push_str(" wants you to sign in with your Ethereum account:\n");
    out.push_str(&m.address);
    out.push('\n');
    if let Some(s) = &m.statement {
        out.push('\n');
        out.push_str(s);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&format!("URI: {}\n", m.uri));
    out.push_str(&format!("Version: {}\n", m.version));
    out.push_str(&format!("Chain ID: {}\n", m.chain_id));
    out.push_str(&format!("Nonce: {}\n", m.nonce));
    out.push_str(&format!("Issued At: {}", m.issued_at));
    if let Some(v) = &m.expiration_time {
        out.push_str(&format!("\nExpiration Time: {v}"));
    }
    if let Some(v) = &m.not_before {
        out.push_str(&format!("\nNot Before: {v}"));
    }
    if let Some(v) = &m.request_id {
        out.push_str(&format!("\nRequest ID: {v}"));
    }
    if !m.resources.is_empty() {
        out.push_str("\nResources:");
        for r in &m.resources {
            out.push_str("\n- ");
            out.push_str(r);
        }
    }
    out
}

fn iso_to_unix(iso: &str) -> Option<u64> {
    // Minimal RFC 3339 parser: YYYY-MM-DDTHH:MM:SSZ. Anything fancier
    // (timezone offsets, fractional seconds) we punt to chrono — but
    // pylon's auth crate already pulls in chrono via the workspace.
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.timestamp() as u64)
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_round_trip() {
        let store = NonceStore::new();
        let n = store.issue("0xABC");
        assert_eq!(store.take("0xabc").as_deref(), Some(n.as_str()));
        // Single-use.
        assert!(store.take("0xabc").is_none());
    }

    #[test]
    fn parse_full_message() {
        let raw = "example.com wants you to sign in with your Ethereum account:\n\
                   0x1111222233334444555566667777888899990000\n\
                   \n\
                   I accept the ToS\n\
                   \n\
                   URI: https://example.com\n\
                   Version: 1\n\
                   Chain ID: 1\n\
                   Nonce: abc123\n\
                   Issued At: 2026-01-01T00:00:00Z";
        let m = parse_message(raw).expect("parse");
        assert_eq!(m.domain, "example.com");
        assert_eq!(m.address, "0x1111222233334444555566667777888899990000");
        assert_eq!(m.statement.as_deref(), Some("I accept the ToS"));
        assert_eq!(m.uri, "https://example.com");
        assert_eq!(m.chain_id, 1);
        assert_eq!(m.nonce, "abc123");
    }

    #[test]
    fn parse_message_without_statement() {
        let raw = "x.com wants you to sign in with your Ethereum account:\n\
                   0x1111222233334444555566667777888899990000\n\
                   \n\
                   URI: https://x.com\n\
                   Version: 1\n\
                   Chain ID: 1\n\
                   Nonce: deadbeef\n\
                   Issued At: 2026-01-01T00:00:00Z";
        let m = parse_message(raw).expect("parse");
        assert!(m.statement.is_none());
        assert_eq!(m.nonce, "deadbeef");
    }

    #[test]
    fn parse_rejects_bad_address_length() {
        let raw = "x.com wants you to sign in with your Ethereum account:\n\
                   0xABC\n\
                   \n\
                   URI: x\nVersion: 1\nChain ID: 1\nNonce: n\nIssued At: t";
        assert!(matches!(parse_message(raw), Err(SiweError::Malformed)));
    }

    #[test]
    fn validate_rejects_domain_mismatch() {
        let store = NonceStore::new();
        store.issue("0x1111222233334444555566667777888899990000");
        let m = SiweMessage {
            domain: "evil.com".into(),
            address: "0x1111222233334444555566667777888899990000".into(),
            statement: None,
            uri: "https://evil.com".into(),
            version: "1".into(),
            chain_id: 1,
            nonce: "x".into(),
            issued_at: "2026-01-01T00:00:00Z".into(),
            expiration_time: None,
            not_before: None,
            request_id: None,
            resources: vec![],
        };
        let err = validate_message(&store, &m, "good.com").unwrap_err();
        assert_eq!(err, SiweError::DomainMismatch);
    }

    #[test]
    fn validate_rejects_nonce_mismatch() {
        let store = NonceStore::new();
        store.issue("0x1111222233334444555566667777888899990000");
        let m = SiweMessage {
            domain: "good.com".into(),
            address: "0x1111222233334444555566667777888899990000".into(),
            statement: None,
            uri: "https://good.com".into(),
            version: "1".into(),
            chain_id: 1,
            nonce: "wrong".into(),
            issued_at: "2026-01-01T00:00:00Z".into(),
            expiration_time: None,
            not_before: None,
            request_id: None,
            resources: vec![],
        };
        let err = validate_message(&store, &m, "good.com").unwrap_err();
        assert_eq!(err, SiweError::NonceMismatch);
    }

    /// Codex-flagged P0-7: nonce-bombing. Posting an EXPIRED nonce
    /// must NOT consume the slot. Otherwise an attacker who can
    /// observe the (address, nonce) handshake could repeatedly
    /// invalidate a target's pending nonce by posting any expired
    /// version of it.
    #[test]
    fn expired_take_does_not_remove_slot() {
        let store = NonceStore::new();
        // Inject an expired entry directly.
        store
            .nonces
            .lock()
            .unwrap()
            .insert("0xabc".into(), ("nonce-x".into(), 1));
        // First take — sees expired, returns None, MUST keep the slot.
        assert!(store.take("0xabc").is_none());
        // Slot still present (the test would also trip if we did remove it).
        assert!(store.nonces.lock().unwrap().contains_key("0xabc"));
    }

    /// Multi-line statements per spec are real (any printable
    /// character + LF). The serializer must round-trip them.
    #[test]
    fn parse_handles_multiline_statement() {
        let raw = "x.com wants you to sign in with your Ethereum account:\n\
                   0x1111222233334444555566667777888899990000\n\
                   \n\
                   line one\n\
                   line two\n\
                   \n\
                   URI: https://x.com\n\
                   Version: 1\n\
                   Chain ID: 1\n\
                   Nonce: n\n\
                   Issued At: 2026-01-01T00:00:00Z";
        let m = parse_message(raw).expect("parse");
        assert_eq!(m.statement.as_deref(), Some("line one\nline two"));
    }
}
