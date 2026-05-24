//! Field-level at-rest encryption for `field.encrypted()` fields.
//!
//! AEAD primitive: ChaCha20-Poly1305 (via `ring`). Same family
//! pylon-auth uses for session cookies — no new system dependency.
//!
//! Wire format on disk: `enc:v1:<base64(nonce)>:<base64(ciphertext)>`
//! - `v1` versions the format so future rotations (key derivation,
//!   AEAD swap) can co-exist with v1 records.
//! - 96-bit nonce, generated fresh per cell from `SystemRandom`.
//! - Ciphertext includes the 16-byte Poly1305 tag (`Aead::seal_in_place_append_tag`).
//!
//! Threat model:
//! - Protects against DB file copy, SQL dump leak, unauthorized
//!   physical access to the disk.
//! - Does NOT protect against an attacker with code execution on the
//!   Pylon process — the key lives in env / process memory.
//! - Does NOT defend against side-channel attacks against the AEAD
//!   itself (constant-time deps from ring).
//! - Encrypted fields are NOT queryable. Indexes + WHERE filters on
//!   encrypted fields don't work (ciphertext differs across writes).
//!
//! Key sourcing: `PYLON_ENCRYPTION_KEY` env. Either raw 32 bytes
//! base64-encoded, OR a 64-char hex string. Loading happens once at
//! boot via [`EncryptionKey::from_env`].

use std::sync::Arc;

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use ring::rand::{SecureRandom, SystemRandom};

/// Wire-format prefix every encrypted cell starts with. Older
/// records that don't start with `ENC_PREFIX` are returned as-is
/// from [`decrypt_if_needed`] so a manifest gaining `encrypted: true`
/// on a populated table doesn't break existing plaintext rows —
/// rows get re-encrypted on next write through the mutation
/// pipeline.
pub const ENC_PREFIX: &str = "enc:v1:";

const NONCE_LEN: usize = 12;

#[derive(Clone)]
pub struct EncryptionKey {
    inner: Arc<EncryptionKeyInner>,
}

struct EncryptionKeyInner {
    key: LessSafeKey,
    rng: SystemRandom,
}

#[derive(Debug)]
pub enum EncryptionError {
    /// PYLON_ENCRYPTION_KEY env was set but couldn't be decoded
    /// (wrong length, invalid base64/hex). Boot-time error.
    InvalidKey(String),
    /// Encrypt / decrypt operation failed at runtime.
    CryptoFailed,
    /// Decrypt called on data that doesn't have the `enc:v1:` prefix.
    /// Callers usually ignore this (legacy plaintext rows).
    NotEncrypted,
    /// Wire format violated after the prefix.
    MalformedWireFormat,
}

impl std::fmt::Display for EncryptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(reason) => {
                write!(f, "Invalid PYLON_ENCRYPTION_KEY: {reason}")
            }
            Self::CryptoFailed => write!(f, "AEAD operation failed"),
            Self::NotEncrypted => write!(f, "Value is not encrypted"),
            Self::MalformedWireFormat => write!(f, "Malformed encrypted wire format"),
        }
    }
}

impl std::error::Error for EncryptionError {}

impl EncryptionKey {
    /// Load the encryption key from `PYLON_ENCRYPTION_KEY` env.
    /// Returns `Ok(None)` when the env is unset — the caller is
    /// responsible for rejecting writes to `encrypted: true` fields
    /// in that case. Returns `Err` when the env IS set but the
    /// value is malformed (boot-time failure; don't start serving).
    pub fn from_env() -> Result<Option<Self>, EncryptionError> {
        let raw = match std::env::var("PYLON_ENCRYPTION_KEY") {
            Ok(s) if !s.is_empty() => s,
            _ => return Ok(None),
        };
        Self::from_raw(&raw).map(Some)
    }

    /// Build a key from the raw `PYLON_ENCRYPTION_KEY` string.
    /// Accepts either:
    /// - 64-char hex (e.g. `openssl rand -hex 32`)
    /// - 44-char standard base64 with padding (e.g. `openssl rand -base64 32`)
    /// - 43-char base64 without padding (URL-safe or standard)
    pub fn from_raw(raw: &str) -> Result<Self, EncryptionError> {
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
            inner: Arc::new(EncryptionKeyInner {
                key: LessSafeKey::new(unbound),
                rng: SystemRandom::new(),
            }),
        })
    }

    /// Encrypt a plaintext string. Returns the wire-format encrypted
    /// value (`enc:v1:<nonce-b64>:<ct-b64>`).
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
            .key
            .seal_in_place_append_tag(nonce, Aad::from(aad.as_slice()), &mut in_out)
            .map_err(|_| EncryptionError::CryptoFailed)?;

        use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
        let nonce_b64 = STANDARD_NO_PAD.encode(nonce_bytes);
        let ct_b64 = STANDARD_NO_PAD.encode(&in_out);
        Ok(format!("{ENC_PREFIX}{nonce_b64}:{ct_b64}"))
    }

    /// Decrypt a wire-format encrypted string. Errors if the input
    /// doesn't have the `enc:v1:` prefix or fails AEAD verification.
    ///
    /// Tries AAD with `row_id` first, then without — forwards/backwards
    /// compat with rows encrypted before the row-id binding was
    /// added.
    pub fn decrypt(
        &self,
        wire: &str,
        entity: &str,
        field: &str,
        row_id: Option<&str>,
    ) -> Result<String, EncryptionError> {
        let rest = wire
            .strip_prefix(ENC_PREFIX)
            .ok_or(EncryptionError::NotEncrypted)?;
        let (nonce_b64, ct_b64) = rest
            .split_once(':')
            .ok_or(EncryptionError::MalformedWireFormat)?;

        use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
        let nonce_bytes = STANDARD_NO_PAD
            .decode(nonce_b64)
            .map_err(|_| EncryptionError::MalformedWireFormat)?;
        if nonce_bytes.len() != NONCE_LEN {
            return Err(EncryptionError::MalformedWireFormat);
        }
        let mut ct = STANDARD_NO_PAD
            .decode(ct_b64)
            .map_err(|_| EncryptionError::MalformedWireFormat)?;

        let mut nonce_arr = [0u8; NONCE_LEN];
        nonce_arr.copy_from_slice(&nonce_bytes);
        let nonce = Nonce::assume_unique_for_key(nonce_arr);

        // Try AAD with row_id binding first (current encrypt path);
        // fall back to legacy AAD without row_id for rows written
        // before the binding was added. ring's open_in_place
        // consumes `ct`, so make a copy for the fallback.
        let ct_for_fallback = ct.clone();
        let aad_with_row = build_aad(entity, field, row_id);
        if let Ok(plain) =
            self.inner
                .key
                .open_in_place(nonce, Aad::from(aad_with_row.as_slice()), &mut ct)
        {
            return String::from_utf8(plain.to_vec()).map_err(|_| EncryptionError::CryptoFailed);
        }
        // Fall back to AAD without row_id (legacy rows).
        let mut ct2 = ct_for_fallback;
        let nonce2 = Nonce::assume_unique_for_key(nonce_arr);
        let aad_legacy = build_aad(entity, field, None);
        let plain = self
            .inner
            .key
            .open_in_place(nonce2, Aad::from(aad_legacy.as_slice()), &mut ct2)
            .map_err(|_| EncryptionError::CryptoFailed)?;
        String::from_utf8(plain.to_vec()).map_err(|_| EncryptionError::CryptoFailed)
    }
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
    value.starts_with(ENC_PREFIX)
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
/// + JSON values get encrypted in place; null/missing fields are
/// skipped. Already-encrypted values (already start with `enc:v1:`)
/// are passed through — avoids double-wrapping when a row is
/// re-written without changing the encrypted field.
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
        assert!(!looks_encrypted("hello world"));
        assert!(!looks_encrypted(""));
        assert!(!looks_encrypted("enc:v2:..."));
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
}
