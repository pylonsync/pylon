//! WebAuthn / passkeys — minimal subset focused on getting passkey
//! sign-in working with the platforms users actually use (iOS,
//! macOS, Android, Windows Hello, 1Password, hardware keys).
//!
//! **Status: library only — HTTP endpoints not yet wired.**
//! `verify_assertion` + `PasskeyStore` are production-quality and
//! exposed for apps that want to roll their own register/login
//! handlers. Pylon-shipped `/api/auth/passkey/*` routes are queued
//! for the next wave; until then, treat this module as primitives
//! to compose into your own routes.
//!
//! This implementation supports:
//!   - Registration with `none` attestation (passkeys generally use
//!     `none` to avoid the privacy issues of platform attestation)
//!   - Public key in COSE_Key form: ES256 (alg=-7), Ed25519 (alg=-8).
//!     RS256 omitted to keep the COSE parser small — every passkey
//!     authenticator in the wild does ES256 or Ed25519.
//!   - Assertion verification against a stored public key + counter
//!   - Origin / RP ID validation
//!
//! Out of scope (Wave 5 may extend):
//!   - Attestation statement verification (we accept `none` only)
//!   - Conditional UI / discoverable credentials list (frontend
//!     handles; pylon stores `userHandle` so it works)
//!   - Resident-key enforcement / extensions
//!   - RS256 / RS1 / EdDSA curve negotiation beyond Ed25519
//!
//! Storage shape — pylon stores one row per credential (not per
//! user; users can have multiple passkeys):
//!
//! ```text
//! Passkey {
//!   id (= credentialId base64url),
//!   user_id,
//!   public_key (COSE_Key bytes),
//!   sign_count (u32),
//!   created_at,
//!   last_used_at,
//! }
//! ```
//!
//! The runtime persists this in SQLite + Postgres via the
//! `crate::passkey_backend` module (in `pylon-runtime`).

use crate::apple_jwt::base64_url;
use ring::signature;
use std::collections::HashMap;
use std::sync::Mutex;

/// Per-user, per-credential passkey record. `id` is the credentialId
/// the authenticator returns at registration; the relying party
/// (= pylon) hands it back in the `allowCredentials` list at
/// assertion time so the authenticator knows which key to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passkey {
    /// Base64url-encoded credentialId — what the authenticator
    /// returns as `rawId` and the RP echoes in `allowCredentials`.
    pub id: String,
    pub user_id: String,
    /// COSE_Key bytes. Format depends on the chosen algorithm —
    /// we extract `x`+`y`+`alg` at verify time.
    pub public_key: Vec<u8>,
    /// Authenticator's sign counter — increments on every successful
    /// assertion. RP MUST reject assertions where the new counter
    /// is `<=` the stored one (cloned-credential detection per
    /// WebAuthn §6.1.1). `0` means the authenticator doesn't
    /// implement counters (Touch ID, Face ID — they use secure
    /// enclave isolation instead).
    pub sign_count: u32,
    /// Optional friendly name set by the user ("iPhone", "Yubikey 5").
    pub name: String,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

/// Pending registration challenge — pylon hands a random 32-byte
/// challenge to the frontend, the authenticator signs it, we verify
/// the signature against the stored challenge. Single-use, 5-minute
/// expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyChallenge {
    /// 32 random bytes, base64url-encoded — what we sent.
    pub challenge: String,
    pub user_id: String,
    pub kind: ChallengeKind,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeKind {
    /// `navigator.credentials.create()` flow.
    Registration,
    /// `navigator.credentials.get()` flow.
    Assertion,
}

/// Pluggable storage. Same in-memory default + runtime SQLite/PG
/// pattern as ApiKeyBackend.
pub trait PasskeyBackend: Send + Sync {
    fn put(&self, passkey: &Passkey);
    fn get(&self, id: &str) -> Option<Passkey>;
    fn list_for_user(&self, user_id: &str) -> Vec<Passkey>;
    fn delete(&self, id: &str) -> bool;
    fn update_counter(&self, id: &str, sign_count: u32, last_used: u64);
}

pub struct InMemoryPasskeyBackend {
    keys: Mutex<HashMap<String, Passkey>>,
}

impl Default for InMemoryPasskeyBackend {
    fn default() -> Self {
        Self {
            keys: Mutex::new(HashMap::new()),
        }
    }
}

impl PasskeyBackend for InMemoryPasskeyBackend {
    fn put(&self, p: &Passkey) {
        self.keys.lock().unwrap().insert(p.id.clone(), p.clone());
    }
    fn get(&self, id: &str) -> Option<Passkey> {
        self.keys.lock().unwrap().get(id).cloned()
    }
    fn list_for_user(&self, user_id: &str) -> Vec<Passkey> {
        self.keys
            .lock()
            .unwrap()
            .values()
            .filter(|k| k.user_id == user_id)
            .cloned()
            .collect()
    }
    fn delete(&self, id: &str) -> bool {
        self.keys.lock().unwrap().remove(id).is_some()
    }
    fn update_counter(&self, id: &str, sign_count: u32, last_used: u64) {
        if let Some(k) = self.keys.lock().unwrap().get_mut(id) {
            k.sign_count = sign_count;
            k.last_used_at = Some(last_used);
        }
    }
}

pub struct PasskeyStore {
    backend: Box<dyn PasskeyBackend>,
    challenges: Mutex<HashMap<String, PasskeyChallenge>>,
}

impl Default for PasskeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PasskeyStore {
    pub fn new() -> Self {
        Self::with_backend(Box::new(InMemoryPasskeyBackend::default()))
    }
    pub fn with_backend(backend: Box<dyn PasskeyBackend>) -> Self {
        Self {
            backend,
            challenges: Mutex::new(HashMap::new()),
        }
    }

    /// Mint a fresh challenge — called by `/api/auth/passkey/register/begin`
    /// and `/api/auth/passkey/login/begin`. Returns the base64url
    /// challenge bytes for the frontend to pass to the authenticator.
    pub fn mint_challenge(&self, user_id: String, kind: ChallengeKind) -> String {
        use rand::RngCore;
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let challenge = base64_url(bytes);
        let expires_at = now_secs() + 5 * 60;
        self.challenges.lock().unwrap().insert(
            challenge.clone(),
            PasskeyChallenge {
                challenge: challenge.clone(),
                user_id,
                kind,
                expires_at,
            },
        );
        challenge
    }

    /// Take + validate a stored challenge. Returns the matching record
    /// iff the challenge exists, hasn't expired, and matches the
    /// expected `kind`.
    pub fn take_challenge(
        &self,
        challenge: &str,
        kind: ChallengeKind,
    ) -> Option<PasskeyChallenge> {
        let mut map = self.challenges.lock().unwrap();
        let entry = map.remove(challenge)?;
        if entry.expires_at <= now_secs() || entry.kind != kind {
            return None;
        }
        Some(entry)
    }

    pub fn store_passkey(&self, passkey: Passkey) {
        self.backend.put(&passkey);
    }

    pub fn get_passkey(&self, id: &str) -> Option<Passkey> {
        self.backend.get(id)
    }

    pub fn list_for_user(&self, user_id: &str) -> Vec<Passkey> {
        self.backend.list_for_user(user_id)
    }

    pub fn delete(&self, id: &str) -> bool {
        self.backend.delete(id)
    }

    pub fn record_use(&self, id: &str, new_count: u32) {
        self.backend.update_counter(id, new_count, now_secs());
    }
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// Inputs the frontend posts after `navigator.credentials.get()` completes.
#[derive(Debug, Clone)]
pub struct AssertionInput<'a> {
    pub credential_id: &'a str,
    pub authenticator_data: &'a [u8],
    pub client_data_json: &'a [u8],
    pub signature: &'a [u8],
    /// Optional userHandle — present for discoverable credentials.
    pub user_handle: Option<&'a [u8]>,
}

/// Errors that can occur during assertion verification. We return
/// distinct variants so test/log paths can pinpoint failures, but
/// the HTTP layer collapses them to a single 401 to avoid oracle
/// leaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebauthnError {
    UnknownCredential,
    BadClientData,
    WrongType,
    ChallengeMismatch,
    OriginMismatch,
    RpIdMismatch,
    AuthenticatorDataTooShort,
    UserNotPresent,
    SignatureMismatch,
    UnsupportedAlg,
    CounterRegression,
}

impl std::fmt::Display for WebauthnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnknownCredential => "credential not found",
            Self::BadClientData => "clientDataJSON malformed",
            Self::WrongType => "clientData.type mismatch",
            Self::ChallengeMismatch => "challenge mismatch",
            Self::OriginMismatch => "origin mismatch",
            Self::RpIdMismatch => "rpId hash mismatch",
            Self::AuthenticatorDataTooShort => "authenticatorData too short",
            Self::UserNotPresent => "user-presence flag not set",
            Self::SignatureMismatch => "signature verification failed",
            Self::UnsupportedAlg => "credential alg not supported (need ES256 or Ed25519)",
            Self::CounterRegression => "sign counter regressed — possible cloned credential",
        })
    }
}

/// Verify an assertion against a stored passkey. Updates the sign
/// counter on success. The `expected_origin` and `expected_rp_id`
/// are typically `https://yourapp.com` and `yourapp.com`.
///
/// `expected_user_id` — when set, the credential MUST belong to that
/// user. Pass `None` for discoverable-credential ("usernameless")
/// flows where the user is identified by the credential. Pass
/// `Some(user_id)` for "you claim to be Alice — prove it" flows
/// (this is the defense against a credential from user A being
/// presented as user B during a higher-stakes second-factor step).
pub fn verify_assertion(
    store: &PasskeyStore,
    input: &AssertionInput,
    expected_origin: &str,
    expected_rp_id: &str,
    expected_user_id: Option<&str>,
) -> Result<Passkey, WebauthnError> {
    let stored = store
        .get_passkey(input.credential_id)
        .ok_or(WebauthnError::UnknownCredential)?;
    if let Some(uid) = expected_user_id {
        if stored.user_id != uid {
            return Err(WebauthnError::UnknownCredential);
        }
    }

    // 1. Parse clientDataJSON.
    let client_data: serde_json::Value =
        serde_json::from_slice(input.client_data_json).map_err(|_| WebauthnError::BadClientData)?;
    let kind = client_data
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if kind != "webauthn.get" {
        return Err(WebauthnError::WrongType);
    }
    let challenge_b64 = client_data
        .get("challenge")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let _ = store
        .take_challenge(challenge_b64, ChallengeKind::Assertion)
        .ok_or(WebauthnError::ChallengeMismatch)?;
    let origin = client_data
        .get("origin")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if origin != expected_origin {
        return Err(WebauthnError::OriginMismatch);
    }

    // 2. Validate authenticatorData layout.
    if input.authenticator_data.len() < 37 {
        return Err(WebauthnError::AuthenticatorDataTooShort);
    }
    use sha2::{Digest, Sha256};
    let mut rp_id_hash = Sha256::new();
    rp_id_hash.update(expected_rp_id.as_bytes());
    let expected_rp_hash = rp_id_hash.finalize();
    if input.authenticator_data[..32] != expected_rp_hash[..] {
        return Err(WebauthnError::RpIdMismatch);
    }
    let flags = input.authenticator_data[32];
    if flags & 0x01 == 0 {
        return Err(WebauthnError::UserNotPresent);
    }
    let counter = u32::from_be_bytes([
        input.authenticator_data[33],
        input.authenticator_data[34],
        input.authenticator_data[35],
        input.authenticator_data[36],
    ]);
    // Authenticators that don't implement counters always send 0.
    // Only treat regression as an error when the new value is
    // strictly less than the stored value AND the stored value is > 0.
    if stored.sign_count > 0 && counter <= stored.sign_count {
        return Err(WebauthnError::CounterRegression);
    }

    // 3. Compute signature input = authenticatorData || SHA256(clientDataJSON).
    let mut client_data_hash = Sha256::new();
    client_data_hash.update(input.client_data_json);
    let cd_hash = client_data_hash.finalize();
    let mut signing_input = Vec::with_capacity(input.authenticator_data.len() + 32);
    signing_input.extend_from_slice(input.authenticator_data);
    signing_input.extend_from_slice(&cd_hash);

    // 4. Verify signature against the stored COSE_Key public key.
    let alg = cose_key_alg(&stored.public_key).ok_or(WebauthnError::UnsupportedAlg)?;
    match alg {
        -7 => {
            // ES256 = ECDSA P-256 + SHA-256 with ASN.1 (DER) signature
            let raw = cose_es256_xy(&stored.public_key).ok_or(WebauthnError::UnsupportedAlg)?;
            // Reconstruct uncompressed SEC1 point: 0x04 || X || Y.
            let mut spki = Vec::with_capacity(65);
            spki.push(0x04);
            spki.extend_from_slice(&raw.0);
            spki.extend_from_slice(&raw.1);
            let pubkey =
                signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_ASN1, &spki);
            pubkey
                .verify(&signing_input, input.signature)
                .map_err(|_| WebauthnError::SignatureMismatch)?;
        }
        -8 => {
            // Ed25519 — just X (32 bytes).
            let pubkey_bytes =
                cose_eddsa_x(&stored.public_key).ok_or(WebauthnError::UnsupportedAlg)?;
            let pubkey =
                signature::UnparsedPublicKey::new(&signature::ED25519, &pubkey_bytes);
            pubkey
                .verify(&signing_input, input.signature)
                .map_err(|_| WebauthnError::SignatureMismatch)?;
        }
        _ => return Err(WebauthnError::UnsupportedAlg),
    }

    store.record_use(&stored.id, counter);
    let mut updated = stored;
    updated.sign_count = counter;
    Ok(updated)
}

// ---------------------------------------------------------------------------
// COSE_Key helpers (RFC 8152) — minimal CBOR
// ---------------------------------------------------------------------------

/// Pull `alg` (-7 ES256 or -8 EdDSA) from a COSE_Key map. Returns
/// None for any other algorithm.
fn cose_key_alg(bytes: &[u8]) -> Option<i64> {
    let map = parse_cbor_map(bytes)?;
    map.get(&CborKey::I(3)).and_then(|v| v.as_i64())
}

fn cose_es256_xy(bytes: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let map = parse_cbor_map(bytes)?;
    let x = map.get(&CborKey::I(-2))?.as_bytes().cloned()?;
    let y = map.get(&CborKey::I(-3))?.as_bytes().cloned()?;
    if x.len() != 32 || y.len() != 32 {
        return None;
    }
    Some((x, y))
}

fn cose_eddsa_x(bytes: &[u8]) -> Option<Vec<u8>> {
    let map = parse_cbor_map(bytes)?;
    let x = map.get(&CborKey::I(-2))?.as_bytes().cloned()?;
    if x.len() != 32 {
        return None;
    }
    Some(x)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CborKey {
    I(i64),
    S(String),
}

#[derive(Debug, Clone)]
enum CborVal {
    I(i64),
    Bytes(Vec<u8>),
    Text(String),
    Map(HashMap<CborKey, CborVal>),
    Other,
}

impl CborVal {
    fn as_i64(&self) -> Option<i64> {
        if let CborVal::I(n) = self {
            Some(*n)
        } else {
            None
        }
    }
    fn as_bytes(&self) -> Option<&Vec<u8>> {
        if let CborVal::Bytes(b) = self {
            Some(b)
        } else {
            None
        }
    }
}

/// Parse a CBOR major-type-5 map into a Rust HashMap. Sufficient
/// for COSE_Key (which is a small map of int/text keys to int/bytes
/// values). Doesn't try to be a full CBOR decoder.
fn parse_cbor_map(bytes: &[u8]) -> Option<HashMap<CborKey, CborVal>> {
    let mut p = CborParser { bytes, pos: 0 };
    let val = p.read_value()?;
    if let CborVal::Map(m) = val {
        Some(m)
    } else {
        None
    }
}

struct CborParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> CborParser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.pos + n > self.bytes.len() {
            return None;
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Some(s)
    }
    fn read_u8(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    /// Read a CBOR length/value combo. Returns the integer value
    /// regardless of whether it came from the initial byte (0..=23)
    /// or the following 1/2/4/8 bytes.
    fn read_arg(&mut self, additional: u8) -> Option<u64> {
        match additional {
            0..=23 => Some(additional as u64),
            24 => Some(self.read_u8()? as u64),
            25 => {
                let s = self.take(2)?;
                Some(u16::from_be_bytes([s[0], s[1]]) as u64)
            }
            26 => {
                let s = self.take(4)?;
                Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64)
            }
            27 => {
                let s = self.take(8)?;
                Some(u64::from_be_bytes([
                    s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
                ]))
            }
            _ => None,
        }
    }

    fn read_value(&mut self) -> Option<CborVal> {
        let head = self.read_u8()?;
        let major = head >> 5;
        let additional = head & 0x1F;
        match major {
            0 => Some(CborVal::I(self.read_arg(additional)? as i64)),
            1 => {
                // Negative int: -1 - n.
                let n = self.read_arg(additional)?;
                Some(CborVal::I(-1 - n as i64))
            }
            2 => {
                let len = self.read_arg(additional)? as usize;
                let s = self.take(len)?.to_vec();
                Some(CborVal::Bytes(s))
            }
            3 => {
                let len = self.read_arg(additional)? as usize;
                let s = self.take(len)?;
                Some(CborVal::Text(
                    std::str::from_utf8(s).ok()?.to_string(),
                ))
            }
            4 => {
                let len = self.read_arg(additional)? as usize;
                let mut arr = Vec::with_capacity(len);
                for _ in 0..len {
                    arr.push(self.read_value()?);
                }
                // We don't model arrays — collapse into Other.
                let _ = arr;
                Some(CborVal::Other)
            }
            5 => {
                let len = self.read_arg(additional)? as usize;
                let mut map = HashMap::with_capacity(len);
                for _ in 0..len {
                    let key_val = self.read_value()?;
                    let key = match key_val {
                        CborVal::I(n) => CborKey::I(n),
                        CborVal::Text(s) => CborKey::S(s),
                        _ => return None,
                    };
                    let val = self.read_value()?;
                    map.insert(key, val);
                }
                Some(CborVal::Map(map))
            }
            _ => Some(CborVal::Other),
        }
    }
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
    fn challenge_round_trip() {
        let store = PasskeyStore::new();
        let challenge = store.mint_challenge("u-1".into(), ChallengeKind::Registration);
        let taken = store
            .take_challenge(&challenge, ChallengeKind::Registration)
            .unwrap();
        assert_eq!(taken.user_id, "u-1");
        // Single-use.
        assert!(store
            .take_challenge(&challenge, ChallengeKind::Registration)
            .is_none());
    }

    #[test]
    fn challenge_kind_mismatch_rejected() {
        let store = PasskeyStore::new();
        let challenge = store.mint_challenge("u-1".into(), ChallengeKind::Registration);
        // Posting an Assertion challenge against a Registration
        // record would let an attacker substitute one for the other.
        assert!(store
            .take_challenge(&challenge, ChallengeKind::Assertion)
            .is_none());
    }

    #[test]
    fn passkey_storage_round_trip() {
        let store = PasskeyStore::new();
        let p = Passkey {
            id: "cred1".into(),
            user_id: "u-1".into(),
            public_key: vec![],
            sign_count: 0,
            name: "iPhone".into(),
            created_at: 100,
            last_used_at: None,
        };
        store.store_passkey(p.clone());
        assert_eq!(store.get_passkey("cred1").unwrap(), p);
        assert_eq!(store.list_for_user("u-1").len(), 1);
        store.record_use("cred1", 5);
        let after = store.get_passkey("cred1").unwrap();
        assert_eq!(after.sign_count, 5);
        assert!(after.last_used_at.is_some());
        assert!(store.delete("cred1"));
        assert!(store.get_passkey("cred1").is_none());
    }

    /// Smoke test the CBOR map parser against a hand-rolled COSE_Key
    /// for ES256: kty=2, alg=-7, crv=1, x=<32 bytes>, y=<32 bytes>.
    #[test]
    fn cose_es256_xy_extracts_coords() {
        let mut buf = Vec::new();
        // map of length 5 → major type 5, additional 5 → 0xa5
        buf.push(0xa5);
        // key 1 (kty), value 2
        buf.extend_from_slice(&[0x01, 0x02]);
        // key 3 (alg), value -7 → major 1, arg 6 → 0x26
        buf.extend_from_slice(&[0x03, 0x26]);
        // key -1 (crv), value 1 → key=0x20, value=0x01
        buf.extend_from_slice(&[0x20, 0x01]);
        // key -2 (x), value bytes(32) → key=0x21, header 0x58 0x20, then 32 bytes of 0xAA
        buf.extend_from_slice(&[0x21, 0x58, 0x20]);
        buf.extend_from_slice(&[0xAA; 32]);
        // key -3 (y), value bytes(32)
        buf.extend_from_slice(&[0x22, 0x58, 0x20]);
        buf.extend_from_slice(&[0xBB; 32]);
        let (x, y) = cose_es256_xy(&buf).expect("parse");
        assert_eq!(x, vec![0xAA; 32]);
        assert_eq!(y, vec![0xBB; 32]);
        assert_eq!(cose_key_alg(&buf), Some(-7));
    }

    #[test]
    fn cose_eddsa_extracts_x() {
        let mut buf = Vec::new();
        buf.push(0xa4); // map of 4
        buf.extend_from_slice(&[0x01, 0x01]); // kty=1
        buf.extend_from_slice(&[0x03, 0x27]); // alg=-8
        buf.extend_from_slice(&[0x20, 0x06]); // crv=6 (Ed25519)
        buf.extend_from_slice(&[0x21, 0x58, 0x20]);
        buf.extend_from_slice(&[0xCC; 32]);
        let x = cose_eddsa_x(&buf).expect("parse");
        assert_eq!(x, vec![0xCC; 32]);
        assert_eq!(cose_key_alg(&buf), Some(-8));
    }

    #[test]
    fn assertion_unknown_credential_rejected() {
        let store = PasskeyStore::new();
        let input = AssertionInput {
            credential_id: "missing",
            authenticator_data: &[0u8; 37],
            client_data_json: b"{}",
            signature: &[],
            user_handle: None,
        };
        let err = verify_assertion(&store, &input, "https://app", "app", None).unwrap_err();
        assert_eq!(err, WebauthnError::UnknownCredential);
    }

    /// P3-5 (codex Wave-3 review): credential bound to user A
    /// presented as user B must reject. Defense against an attacker
    /// who registered a passkey on their own account then tries to
    /// use it during a second-factor challenge framed as another user.
    #[test]
    fn assertion_user_mismatch_rejected() {
        let store = PasskeyStore::new();
        store.store_passkey(Passkey {
            id: "cred1".into(),
            user_id: "alice".into(),
            public_key: vec![],
            sign_count: 0,
            name: "key".into(),
            created_at: 1,
            last_used_at: None,
        });
        let input = AssertionInput {
            credential_id: "cred1",
            authenticator_data: &[0u8; 37],
            client_data_json: b"{}",
            signature: &[],
            user_handle: None,
        };
        let err = verify_assertion(&store, &input, "https://app", "app", Some("bob")).unwrap_err();
        assert_eq!(err, WebauthnError::UnknownCredential);
    }
}
