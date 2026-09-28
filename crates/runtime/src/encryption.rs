//! Field-level at-rest encryption for `field.encrypted()` fields.
//!
//! AEAD primitive: ChaCha20-Poly1305 (via `ring`). Same family
//! pylon-auth uses for session cookies — no new system dependency.
//!
//! Wire format on disk: `enc:v2:<key-id>:<base64(nonce)>:<base64(ciphertext)>`
//! - `key-id` names the key that sealed the value (see [`key_id`]), so
//!   several keys can be active at once and a value is opened with the
//!   right one.
//! - 96-bit nonce, generated fresh per cell from `SystemRandom`.
//! - Ciphertext includes the 16-byte Poly1305 tag (`Aead::seal_in_place_append_tag`).
//! - `enc:v1:<nonce>:<ciphertext>` values (no key id) still decrypt: each
//!   configured key is tried in turn.
//!
//! Threat model:
//! - Protects against DB file copy, SQL dump leak, unauthorized
//!   physical access to the disk.
//! - Does NOT protect against an attacker with code execution on the
//!   Pylon process — the keys live in env / process memory.
//! - Does NOT defend against side-channel attacks against the AEAD
//!   itself (constant-time deps from ring).
//! - Encrypted fields are NOT queryable. Indexes + WHERE filters on
//!   encrypted fields don't work (ciphertext differs across writes).
//!
//! Keys: `PYLON_ENCRYPTION_KEY` is the current key; every write uses it.
//! `PYLON_ENCRYPTION_PREVIOUS_KEYS` (comma-separated) lists old keys that
//! are used only to decrypt, during a rotation. Each key is raw 32 bytes,
//! base64-encoded, or a 64-char hex string. Loading happens once at boot
//! via [`EncryptionKey::from_env`].

use std::sync::Arc;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use ring::rand::{SecureRandom, SystemRandom};

/// Prefix of the current wire format: `enc:v2:<key-id>:<nonce>:<ct>`.
pub const ENC_PREFIX: &str = "enc:v2:";

/// Prefix of the first wire format, which has no key id:
/// `enc:v1:<nonce>:<ct>`. Still decrypted; never written.
pub const ENC_PREFIX_V1: &str = "enc:v1:";

const NONCE_LEN: usize = 12;

/// Length in hex characters of a key id.
const KEY_ID_LEN: usize = 16;

/// A set of field-encryption keys: one current key that seals every new
/// value, plus previous keys that only open values sealed before a
/// rotation. Cheap to clone.
#[derive(Clone)]
pub struct EncryptionKey {
    inner: Arc<EncryptionKeyInner>,
}

struct EncryptionKeyInner {
    current: KeyEntry,
    previous: Vec<KeyEntry>,
    rng: SystemRandom,
}

struct KeyEntry {
    id: String,
    key: LessSafeKey,
}

#[derive(Debug)]
pub enum EncryptionError {
    /// A key env var was set but couldn't be decoded (wrong length,
    /// invalid base64/hex). Boot-time error.
    InvalidKey(String),
    /// Encrypt / decrypt operation failed at runtime.
    CryptoFailed,
    /// Decrypt called on data that doesn't have an `enc:` prefix.
    /// Callers usually ignore this (legacy plaintext rows).
    NotEncrypted,
    /// Wire format violated after the prefix.
    MalformedWireFormat,
    /// The value was sealed with a key that is not configured. Add it to
    /// `PYLON_ENCRYPTION_PREVIOUS_KEYS` to read the value.
    UnknownKey(String),
}

impl std::fmt::Display for EncryptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(reason) => write!(f, "Invalid encryption key: {reason}"),
            Self::CryptoFailed => write!(f, "AEAD operation failed"),
            Self::NotEncrypted => write!(f, "Value is not encrypted"),
            Self::MalformedWireFormat => write!(f, "Malformed encrypted wire format"),
            Self::UnknownKey(id) => write!(
                f,
                "Value was encrypted with key {id}, which is not configured; \
                 add that key to PYLON_ENCRYPTION_PREVIOUS_KEYS"
            ),
        }
    }
}

impl std::error::Error for EncryptionError {}

impl EncryptionKey {
    /// Load keys from `PYLON_ENCRYPTION_KEY` (current) and
    /// `PYLON_ENCRYPTION_PREVIOUS_KEYS` (comma-separated, decrypt only).
    /// Returns `Ok(None)` when no current key is set — the caller is
    /// responsible for rejecting writes to `encrypted: true` fields in
    /// that case. Returns `Err` when a value is malformed, or previous
    /// keys are set without a current key (boot-time failure; don't start
    /// serving).
    pub fn from_env() -> Result<Option<Self>, EncryptionError> {
        let current = std::env::var("PYLON_ENCRYPTION_KEY").unwrap_or_default();
        let previous = std::env::var("PYLON_ENCRYPTION_PREVIOUS_KEYS").unwrap_or_default();
        if current.trim().is_empty() {
            if !previous.trim().is_empty() {
                return Err(EncryptionError::InvalidKey(
                    "PYLON_ENCRYPTION_PREVIOUS_KEYS is set but PYLON_ENCRYPTION_KEY is not".into(),
                ));
            }
            return Ok(None);
        }
        let previous: Vec<&str> = previous
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        Self::from_raw_keys(&current, &previous).map(Some)
    }

    /// Build a key set with one key and no previous keys. Accepts either:
    /// - 64-char hex (e.g. `openssl rand -hex 32`)
    /// - 44-char standard base64 with padding (e.g. `openssl rand -base64 32`)
    /// - 43-char base64 without padding (URL-safe or standard)
    pub fn from_raw(raw: &str) -> Result<Self, EncryptionError> {
        Self::from_raw_keys(raw, &[])
    }

    /// Build a key set from a current key and decrypt-only previous keys.
    pub fn from_raw_keys(current: &str, previous: &[&str]) -> Result<Self, EncryptionError> {
        let current = KeyEntry::from_raw(current)?;
        let mut entries: Vec<KeyEntry> = Vec::with_capacity(previous.len());
        for raw in previous {
            let entry = KeyEntry::from_raw(raw)?;
            if entry.id == current.id || entries.iter().any(|e| e.id == entry.id) {
                return Err(EncryptionError::InvalidKey(format!(
                    "key {} is listed more than once",
                    entry.id
                )));
            }
            entries.push(entry);
        }
        Ok(Self {
            inner: Arc::new(EncryptionKeyInner {
                current,
                previous: entries,
                rng: SystemRandom::new(),
            }),
        })
    }

    /// Id of the key new values are sealed with.
    pub fn current_key_id(&self) -> &str {
        &self.inner.current.id
    }

    /// Ids of the decrypt-only previous keys.
    pub fn previous_key_ids(&self) -> Vec<&str> {
        self.inner.previous.iter().map(|e| e.id.as_str()).collect()
    }

    /// True when `wire` is ciphertext that is not sealed with the current
    /// key (a v1 value, or a v2 value with another key id). Rotation
    /// re-encrypts these.
    pub fn needs_rotation(&self, wire: &str) -> bool {
        if wire.starts_with(ENC_PREFIX_V1) {
            return true;
        }
        match wire
            .strip_prefix(ENC_PREFIX)
            .and_then(|r| r.split_once(':'))
        {
            Some((id, _)) => id != self.inner.current.id,
            None => false,
        }
    }

    /// Encrypt a plaintext string with the current key. Returns the
    /// wire-format value (`enc:v2:<key-id>:<nonce-b64>:<ct-b64>`).
    ///
    /// AAD binds `entity || \0 || field_name || \0 || row_id` to the
    /// AEAD tag. An attacker who copies a ciphertext blob between
    /// rows or columns in the DB file cannot make decrypt succeed —
    /// the tag check fails. Row id binding (codex P1) defends
    /// against cross-row copy attacks where a SQL-write primitive
    /// would otherwise let an attacker swap victim's ciphertext
    /// into their own row.
    ///
    /// `row_id` is `None` only for legacy callers that don't have
    /// it in scope (none in the current tree). Decrypt accepts both
    /// row-bound and non-bound AAD for forward compat.
    pub fn encrypt(
        &self,
        plaintext: &str,
        entity: &str,
        field: &str,
        row_id: Option<&str>,
    ) -> Result<String, EncryptionError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        self.inner
            .rng
            .fill(&mut nonce_bytes)
            .map_err(|_| EncryptionError::CryptoFailed)?;

        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let aad = build_aad(entity, field, row_id);
        let mut in_out = plaintext.as_bytes().to_vec();
        self.inner
            .current
            .key
            .seal_in_place_append_tag(nonce, Aad::from(aad.as_slice()), &mut in_out)
            .map_err(|_| EncryptionError::CryptoFailed)?;

        use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
        let nonce_b64 = STANDARD_NO_PAD.encode(nonce_bytes);
        let ct_b64 = STANDARD_NO_PAD.encode(&in_out);
        Ok(format!(
            "{ENC_PREFIX}{}:{nonce_b64}:{ct_b64}",
            self.inner.current.id
        ))
    }

    /// Decrypt a wire-format value. A v2 value is opened with the key its
    /// id names; a v1 value is tried against the current key, then each
    /// previous key. Errors if the input has no `enc:` prefix, names an
    /// unconfigured key, or fails AEAD verification.
    pub fn decrypt(
        &self,
        wire: &str,
        entity: &str,
        field: &str,
        row_id: Option<&str>,
    ) -> Result<String, EncryptionError> {
        if let Some(rest) = wire.strip_prefix(ENC_PREFIX) {
            let (id, body) = rest
                .split_once(':')
                .ok_or(EncryptionError::MalformedWireFormat)?;
            let entry = std::iter::once(&self.inner.current)
                .chain(self.inner.previous.iter())
                .find(|e| e.id == id)
                .ok_or_else(|| EncryptionError::UnknownKey(id.to_string()))?;
            let (nonce, ct) = parse_body(body)?;
            return entry.open(nonce, ct, entity, field, row_id);
        }
        if let Some(body) = wire.strip_prefix(ENC_PREFIX_V1) {
            let (nonce, ct) = parse_body(body)?;
            for entry in std::iter::once(&self.inner.current).chain(self.inner.previous.iter()) {
                if let Ok(plain) = entry.open(nonce, ct.clone(), entity, field, row_id) {
                    return Ok(plain);
                }
            }
            return Err(EncryptionError::CryptoFailed);
        }
        Err(EncryptionError::NotEncrypted)
    }
}

impl KeyEntry {
    fn from_raw(raw: &str) -> Result<Self, EncryptionError> {
        let bytes = decode_key(raw)?;
        if bytes.len() != 32 {
            return Err(EncryptionError::InvalidKey(format!(
                "expected 32 bytes after decode, got {}",
                bytes.len()
            )));
        }
        let unbound = UnboundKey::new(&CHACHA20_POLY1305, &bytes)
            .map_err(|_| EncryptionError::InvalidKey("ring rejected the key material".into()))?;
        Ok(Self {
            id: key_id(&bytes),
            key: LessSafeKey::new(unbound),
        })
    }

    /// Open `ct` with AAD bound to the row id first, then without it
    /// (rows written before the row-id binding was added).
    fn open(
        &self,
        nonce: [u8; NONCE_LEN],
        ct: Vec<u8>,
        entity: &str,
        field: &str,
        row_id: Option<&str>,
    ) -> Result<String, EncryptionError> {
        let mut with_row = ct.clone();
        let aad = build_aad(entity, field, row_id);
        if let Ok(plain) = self.key.open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad.as_slice()),
            &mut with_row,
        ) {
            return String::from_utf8(plain.to_vec()).map_err(|_| EncryptionError::CryptoFailed);
        }
        let mut legacy = ct;
        let aad = build_aad(entity, field, None);
        let plain = self
            .key
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad.as_slice()),
                &mut legacy,
            )
            .map_err(|_| EncryptionError::CryptoFailed)?;
        String::from_utf8(plain.to_vec()).map_err(|_| EncryptionError::CryptoFailed)
    }
}

/// Stable id for a key: the first 8 bytes of SHA-256 over a fixed label
/// and the key, in hex. It identifies which key sealed a value without
/// revealing anything useful about the key.
pub fn key_id(key: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::new()
        .chain_update(b"pylon-encryption-key-id:v1:")
        .chain_update(key)
        .finalize();
    digest
        .iter()
        .take(KEY_ID_LEN / 2)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Split `<nonce-b64>:<ct-b64>` into its decoded parts.
fn parse_body(body: &str) -> Result<([u8; NONCE_LEN], Vec<u8>), EncryptionError> {
    use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
    let (nonce_b64, ct_b64) = body
        .split_once(':')
        .ok_or(EncryptionError::MalformedWireFormat)?;
    let nonce_bytes = STANDARD_NO_PAD
        .decode(nonce_b64)
        .map_err(|_| EncryptionError::MalformedWireFormat)?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err(EncryptionError::MalformedWireFormat);
    }
    let ct = STANDARD_NO_PAD
        .decode(ct_b64)
        .map_err(|_| EncryptionError::MalformedWireFormat)?;
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&nonce_bytes);
    Ok((nonce, ct))
}

/// Result of one [`crate::Runtime::rotate_encrypted_fields`] pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct RotationReport {
    pub rows_scanned: u64,
    /// Values re-encrypted with the current key.
    pub rotated: u64,
    /// Values that changed between the read and the write; the writer
    /// sealed them with the current key.
    pub skipped_changed: u64,
    /// Values no configured key could decrypt. Left unchanged.
    pub failed: u64,
}

/// Bind the (entity, field [, row_id]) tuple as additional
/// authenticated data. A ciphertext encrypted under
/// (Customer, ssn, row-abc) won't decrypt under any other
/// combination. With `row_id = None` the AAD matches the legacy
/// pre-row-binding format for backwards compatibility on decrypt.
fn build_aad(entity: &str, field: &str, row_id: Option<&str>) -> Vec<u8> {
    let mut aad = Vec::with_capacity(entity.len() + field.len() + 64);
    aad.extend_from_slice(b"pylon-aead-v1:");
    aad.extend_from_slice(entity.as_bytes());
    aad.push(0);
    aad.extend_from_slice(field.as_bytes());
    if let Some(rid) = row_id {
        aad.push(0);
        aad.extend_from_slice(rid.as_bytes());
    }
    aad
}

/// Helper: returns true when `value` looks like ciphertext on disk.
/// Used by the read path to decide whether to attempt decryption —
/// rows written before the field was annotated `encrypted: true`
/// stay as plaintext until next write, and reading them must not
/// fail.
pub fn looks_encrypted(value: &str) -> bool {
    value.starts_with(ENC_PREFIX) || value.starts_with(ENC_PREFIX_V1)
}

/// Decode the raw key string into bytes. Tries hex first (most
/// common pattern from `openssl rand -hex 32`), falls back to base64
/// (both URL-safe and standard, with or without padding).
fn decode_key(raw: &str) -> Result<Vec<u8>, EncryptionError> {
    let trimmed = raw.trim();
    // Hex: 64 chars, all hex.
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return hex_decode(trimmed);
    }
    // Base64: try a few flavors.
    use base64::{
        engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD},
        Engine,
    };
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(trimmed) {
            return Ok(bytes);
        }
    }
    Err(EncryptionError::InvalidKey(
        "value is not 64-char hex or valid base64".into(),
    ))
}

fn hex_decode(s: &str) -> Result<Vec<u8>, EncryptionError> {
    let bytes = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| EncryptionError::InvalidKey("invalid hex".into()))?;
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Row-level encryption helpers
// ---------------------------------------------------------------------------

/// Encrypt every field in `row` named in `encrypted_fields`. Strings
/// and JSON values get encrypted in place; null/missing fields are
/// skipped. Already-encrypted values (with an `enc:` prefix) are
/// passed through — avoids double-wrapping when a row is re-written
/// without changing the encrypted field.
///
/// AAD binds (entity, field_name) per cell — see `EncryptionKey::encrypt`.
pub fn encrypt_row_fields(
    key: &EncryptionKey,
    entity: &str,
    row: &mut serde_json::Value,
    encrypted_fields: &[&str],
) -> Result<(), EncryptionError> {
    // Extract row id (when present) so AAD binds entity+field+row_id.
    // Encrypt happens at the runtime layer AFTER resolve_or_generate_id,
    // so data["id"] is guaranteed for inserts. For partial updates
    // the id may be absent — fall back to no binding.
    let row_id = row.get("id").and_then(|v| v.as_str()).map(String::from);
    let obj = match row.as_object_mut() {
        Some(o) => o,
        None => return Ok(()),
    };
    for field in encrypted_fields {
        match obj.get(*field) {
            None | Some(serde_json::Value::Null) => continue,
            Some(serde_json::Value::String(s)) => {
                if looks_encrypted(s) {
                    continue;
                }
                let ct = key.encrypt(s, entity, field, row_id.as_deref())?;
                obj.insert(field.to_string(), serde_json::Value::String(ct));
            }
            Some(v) => {
                let serialized = serde_json::to_string(v).unwrap_or_default();
                let ct = key.encrypt(&serialized, entity, field, row_id.as_deref())?;
                obj.insert(field.to_string(), serde_json::Value::String(ct));
            }
        }
    }
    Ok(())
}

/// Decrypt every field in `row` named in `encrypted_fields`.
/// Plaintext values (no `enc:v1:` prefix) pass through untouched —
/// rows written before the field gained `encrypted: true` stay
/// readable.
pub fn decrypt_row_fields(
    key: &EncryptionKey,
    entity: &str,
    row: &mut serde_json::Value,
    encrypted_fields: &[&str],
) -> Result<(), EncryptionError> {
    let row_id = row.get("id").and_then(|v| v.as_str()).map(String::from);
    let obj = match row.as_object_mut() {
        Some(o) => o,
        None => return Ok(()),
    };
    for field in encrypted_fields {
        match obj.get(*field) {
            None | Some(serde_json::Value::Null) => continue,
            Some(serde_json::Value::String(s)) => {
                if !looks_encrypted(s) {
                    continue;
                }
                let plain = key.decrypt(s, entity, field, row_id.as_deref())?;
                obj.insert(field.to_string(), serde_json::Value::String(plain));
            }
            _ => continue,
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> EncryptionKey {
        // Fixed 32-byte key for deterministic test setup. Not
        // cryptographically meaningful — only used to verify wire
        // format + round-trip.
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        EncryptionKey::from_raw(hex).unwrap()
    }

    #[test]
    fn round_trip_encrypts_and_decrypts() {
        let key = test_key();
        let plain = "hello, world";
        let ct = key.encrypt(plain, "Customer", "ssn", Some("row1")).unwrap();
        assert!(ct.starts_with(ENC_PREFIX));
        let recovered = key.decrypt(&ct, "Customer", "ssn", Some("row1")).unwrap();
        assert_eq!(recovered, plain);
    }

    #[test]
    fn unique_nonce_produces_different_ciphertext_for_same_plaintext() {
        let key = test_key();
        let a = key.encrypt("identical", "E", "f", Some("row1")).unwrap();
        let b = key.encrypt("identical", "E", "f", Some("row1")).unwrap();
        assert_ne!(a, b);
        assert_eq!(
            key.decrypt(&a, "E", "f", Some("row1")).unwrap(),
            "identical"
        );
        assert_eq!(
            key.decrypt(&b, "E", "f", Some("row1")).unwrap(),
            "identical"
        );
    }

    #[test]
    fn decrypt_rejects_tampered_ciphertext() {
        let key = test_key();
        let ct = key.encrypt("secret value", "E", "f", Some("r")).unwrap();
        let mut bytes: Vec<char> = ct.chars().collect();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == 'a' { 'b' } else { 'a' };
        let tampered: String = bytes.into_iter().collect();
        assert!(key.decrypt(&tampered, "E", "f", Some("r")).is_err());
    }

    #[test]
    fn decrypt_rejects_wrong_key() {
        let key_a = EncryptionKey::from_raw(
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap();
        let key_b = EncryptionKey::from_raw(
            "1111111111111111111111111111111111111111111111111111111111111111",
        )
        .unwrap();
        let ct = key_a.encrypt("classified", "E", "f", Some("r")).unwrap();
        assert!(key_b.decrypt(&ct, "E", "f", Some("r")).is_err());
    }

    #[test]
    fn decrypt_rejects_wrong_aad_entity_or_field() {
        // Codex P1: a ciphertext encrypted under (Customer, ssn) MUST
        // NOT decrypt under (User, ssn) or (Customer, otherfield).
        // Otherwise an attacker who copies a cell between rows/columns
        // can bypass column boundaries.
        let key = test_key();
        let ct = key
            .encrypt("super secret", "Customer", "ssn", Some("row-1"))
            .unwrap();
        assert!(
            key.decrypt(&ct, "User", "ssn", Some("row-1")).is_err(),
            "decrypt must reject wrong entity AAD"
        );
        assert!(
            key.decrypt(&ct, "Customer", "phone", Some("row-1"))
                .is_err(),
            "decrypt must reject wrong field AAD"
        );
        // Codex P1: cross-row copy attack — same entity+field but
        // different row id must fail.
        assert!(
            key.decrypt(&ct, "Customer", "ssn", Some("row-2")).is_err(),
            "decrypt must reject wrong row_id AAD"
        );
        assert_eq!(
            key.decrypt(&ct, "Customer", "ssn", Some("row-1")).unwrap(),
            "super secret"
        );
    }

    #[test]
    fn looks_encrypted_distinguishes_plaintext() {
        assert!(looks_encrypted("enc:v1:abcd:efgh"));
        assert!(looks_encrypted("enc:v2:0011223344556677:abcd:efgh"));
        assert!(!looks_encrypted("hello world"));
        assert!(!looks_encrypted(""));
        assert!(!looks_encrypted("enc:v3:..."));
    }

    #[test]
    fn key_loads_from_hex_and_base64() {
        // Hex (64 chars).
        let k1 = EncryptionKey::from_raw(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        );
        assert!(k1.is_ok());
        // Base64 standard with padding.
        let k2 = EncryptionKey::from_raw("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=");
        assert!(k2.is_ok());
        // Base64 URL-safe no padding.
        let k3 = EncryptionKey::from_raw("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8");
        assert!(k3.is_ok());
    }

    #[test]
    fn key_rejects_wrong_length() {
        // 16 bytes — too short for ChaCha20-Poly1305.
        let k = EncryptionKey::from_raw("0123456789abcdef0123456789abcdef");
        assert!(k.is_err());
    }

    #[test]
    fn key_rejects_garbage() {
        let k = EncryptionKey::from_raw("not a valid key");
        assert!(k.is_err());
    }

    #[test]
    fn encrypt_row_skips_null_and_missing_fields() {
        let key = test_key();
        let mut row = serde_json::json!({
            "ssn": "111-22-3333",
            "phone": null,
            "name": "Alice"
        });
        encrypt_row_fields(&key, "Customer", &mut row, &["ssn", "phone", "missing"]).unwrap();
        let ssn = row["ssn"].as_str().unwrap();
        assert!(ssn.starts_with(ENC_PREFIX));
        assert!(row["phone"].is_null());
        assert_eq!(row["name"], "Alice");
        assert!(row.get("missing").is_none());

        decrypt_row_fields(&key, "Customer", &mut row, &["ssn", "phone"]).unwrap();
        assert_eq!(row["ssn"], "111-22-3333");
    }

    #[test]
    fn encrypt_row_skips_already_encrypted() {
        let key = test_key();
        let original_ct = key.encrypt("9999", "C", "pin", Some("r")).unwrap();
        let mut row = serde_json::json!({ "pin": original_ct.clone() });
        encrypt_row_fields(&key, "C", &mut row, &["pin"]).unwrap();
        assert_eq!(row["pin"], original_ct);
    }

    #[test]
    fn decrypt_row_passes_through_plaintext() {
        let key = test_key();
        let mut row = serde_json::json!({ "ssn": "legacy plaintext" });
        decrypt_row_fields(&key, "C", &mut row, &["ssn"]).unwrap();
        assert_eq!(row["ssn"], "legacy plaintext");
    }

    #[test]
    fn decrypt_returns_string_for_numeric_looking_plaintext() {
        // Codex P2: previous behavior auto-parsed JSON, so a SSN
        // value "12345" came back as integer 12345. Encrypted fields
        // must round-trip as STRINGS — type preservation lives in
        // the manifest, not in the value.
        let key = test_key();
        let mut row = serde_json::json!({ "ssn": "12345" });
        encrypt_row_fields(&key, "C", &mut row, &["ssn"]).unwrap();
        decrypt_row_fields(&key, "C", &mut row, &["ssn"]).unwrap();
        assert_eq!(row["ssn"], serde_json::Value::String("12345".into()));
    }

    const K1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const K2: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    /// Seal a value in the v1 format (no key id), as releases before key
    /// ids wrote it.
    fn seal_v1(raw_key: &str, plain: &str, entity: &str, field: &str, row: &str) -> String {
        use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
        let bytes = decode_key(raw_key).unwrap();
        let key = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &bytes).unwrap());
        let nonce = [7u8; NONCE_LEN];
        let mut buf = plain.as_bytes().to_vec();
        let aad = build_aad(entity, field, Some(row));
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad.as_slice()),
            &mut buf,
        )
        .unwrap();
        format!(
            "{ENC_PREFIX_V1}{}:{}",
            STANDARD_NO_PAD.encode(nonce),
            STANDARD_NO_PAD.encode(&buf)
        )
    }

    #[test]
    fn new_values_carry_the_current_key_id() {
        let ring = EncryptionKey::from_raw_keys(K2, &[K1]).unwrap();
        let ct = ring.encrypt("x", "E", "f", Some("r")).unwrap();
        let id = ct
            .strip_prefix(ENC_PREFIX)
            .unwrap()
            .split(':')
            .next()
            .unwrap();
        assert_eq!(id, ring.current_key_id());
        assert_eq!(id.len(), KEY_ID_LEN);
        assert!(!ring.needs_rotation(&ct));
    }

    #[test]
    fn previous_keys_open_old_values_and_mark_them_for_rotation() {
        let old = EncryptionKey::from_raw(K1).unwrap();
        let old_ct = old.encrypt("ssn-1", "Customer", "ssn", Some("r1")).unwrap();
        let legacy = seal_v1(K1, "ssn-legacy", "Customer", "ssn", "r2");

        let rotated = EncryptionKey::from_raw_keys(K2, &[K1]).unwrap();
        assert_eq!(
            rotated
                .decrypt(&old_ct, "Customer", "ssn", Some("r1"))
                .unwrap(),
            "ssn-1"
        );
        assert_eq!(
            rotated
                .decrypt(&legacy, "Customer", "ssn", Some("r2"))
                .unwrap(),
            "ssn-legacy"
        );
        assert!(rotated.needs_rotation(&old_ct));
        assert!(rotated.needs_rotation(&legacy));
        assert!(!rotated.needs_rotation("plain text"));
        assert_eq!(rotated.previous_key_ids(), vec![old.current_key_id()]);
    }

    #[test]
    fn a_value_from_a_removed_key_names_the_missing_key() {
        let old = EncryptionKey::from_raw(K1).unwrap();
        let ct = old.encrypt("x", "E", "f", Some("r")).unwrap();
        let only_new = EncryptionKey::from_raw(K2).unwrap();
        match only_new.decrypt(&ct, "E", "f", Some("r")) {
            Err(EncryptionError::UnknownKey(id)) => assert_eq!(id, old.current_key_id()),
            other => panic!("expected UnknownKey, got {other:?}"),
        }
        // A v1 value from a removed key fails the tag check.
        let legacy = seal_v1(K1, "x", "E", "f", "r");
        assert!(only_new.decrypt(&legacy, "E", "f", Some("r")).is_err());
    }

    #[test]
    fn a_forged_key_id_cannot_open_a_value() {
        // Relabel a K1 value with K2's id: K2 then fails the tag check.
        let ring = EncryptionKey::from_raw_keys(K2, &[K1]).unwrap();
        let k1 = EncryptionKey::from_raw(K1).unwrap();
        let ct = k1.encrypt("x", "E", "f", Some("r")).unwrap();
        let forged = ct.replacen(k1.current_key_id(), ring.current_key_id(), 1);
        assert!(ring.decrypt(&forged, "E", "f", Some("r")).is_err());
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        assert!(EncryptionKey::from_raw_keys(K1, &[K1]).is_err());
        assert!(EncryptionKey::from_raw_keys(K1, &[K2, K2]).is_err());
    }

    #[test]
    fn key_ids_are_stable_and_distinct() {
        let a = EncryptionKey::from_raw(K1).unwrap();
        let b = EncryptionKey::from_raw(K1).unwrap();
        let c = EncryptionKey::from_raw(K2).unwrap();
        assert_eq!(a.current_key_id(), b.current_key_id());
        assert_ne!(a.current_key_id(), c.current_key_id());
    }
}
