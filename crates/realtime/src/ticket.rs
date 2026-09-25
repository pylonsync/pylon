//! Shard tickets: short-lived, signed permission to join one shard.
//!
//! A server function mints a ticket for the signed-in user (for example
//! with the character and realm it has checked in the database). The client
//! presents it when it connects. The transport verifies the signature and
//! puts the ticket in [`crate::ShardAuth::ticket`]; the shard then checks
//! that it names this shard, this subscriber, and has not expired, and the
//! game's `authorize_subscribe` / `authorize_input` read the claims with no
//! database call.
//!
//! Format: `v1.<payload>.<signature>`, where `payload` is the base64url
//! (no padding) JSON of [`ShardTicket`] and `signature` is the base64url
//! HMAC-SHA256 of `v1.<payload>` with the host's ticket secret.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const VERSION: &str = "v1";

/// The claims a ticket carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShardTicket {
    /// The shard the ticket admits to.
    pub shard: String,
    /// The subscriber id the holder may use (a user id, or a character id).
    pub sid: String,
    /// The user the ticket was issued to.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Expiry, in seconds since the Unix epoch.
    pub exp: u64,
    /// When it was issued, in seconds since the Unix epoch (0 for tickets
    /// from before this field).
    #[serde(default)]
    pub iat: u64,
    /// App claims, for example `{ "character": "c_12", "realm": "north" }`.
    #[serde(default)]
    pub claims: serde_json::Value,
}

impl ShardTicket {
    /// One app claim, by key.
    pub fn claim(&self, key: &str) -> Option<&serde_json::Value> {
        self.claims.get(key)
    }

    pub fn is_expired(&self, now_unix: u64) -> bool {
        now_unix >= self.exp
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketError {
    Malformed,
    BadSignature,
    Expired,
}

impl std::fmt::Display for TicketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Malformed => "malformed shard ticket",
            Self::BadSignature => "shard ticket signature does not verify",
            Self::Expired => "shard ticket has expired",
        })
    }
}

impl std::error::Error for TicketError {}

fn mac(secret: &[u8], signed: &str) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC takes any key length");
    m.update(signed.as_bytes());
    m
}

/// Sign `ticket` with `secret`.
pub fn sign_ticket(secret: &[u8], ticket: &ShardTicket) -> String {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(ticket).expect("ticket serializes"));
    let signed = format!("{VERSION}.{payload}");
    let sig = URL_SAFE_NO_PAD.encode(mac(secret, &signed).finalize().into_bytes());
    format!("{signed}.{sig}")
}

/// Check the signature and expiry of a ticket and return its claims.
pub fn verify_ticket(
    secret: &[u8],
    token: &str,
    now_unix: u64,
) -> Result<ShardTicket, TicketError> {
    let mut parts = token.splitn(3, '.');
    let (Some(version), Some(payload), Some(sig)) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(TicketError::Malformed);
    };
    if version != VERSION {
        return Err(TicketError::Malformed);
    }
    let sig = URL_SAFE_NO_PAD
        .decode(sig)
        .map_err(|_| TicketError::Malformed)?;
    // Constant-time comparison.
    mac(secret, &format!("{version}.{payload}"))
        .verify_slice(&sig)
        .map_err(|_| TicketError::BadSignature)?;
    let json = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| TicketError::Malformed)?;
    let ticket: ShardTicket = serde_json::from_slice(&json).map_err(|_| TicketError::Malformed)?;
    if ticket.is_expired(now_unix) {
        return Err(TicketError::Expired);
    }
    Ok(ticket)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(exp: u64) -> ShardTicket {
        ShardTicket {
            shard: "zone-3".into(),
            sid: "char_12".into(),
            user_id: Some("u_1".into()),
            exp,
            iat: 0,
            claims: serde_json::json!({ "realm": "north" }),
        }
    }

    #[test]
    fn a_signed_ticket_verifies_and_keeps_its_claims() {
        let t = ticket(1_000);
        let token = sign_ticket(b"secret", &t);
        let got = verify_ticket(b"secret", &token, 999).unwrap();
        assert_eq!(got, t);
        assert_eq!(got.claim("realm"), Some(&serde_json::json!("north")));
    }

    #[test]
    fn a_tampered_or_foreign_ticket_fails() {
        let token = sign_ticket(b"secret", &ticket(1_000));
        assert_eq!(
            verify_ticket(b"other", &token, 0),
            Err(TicketError::BadSignature)
        );

        // Swap the payload for one naming another shard, keep the signature.
        let mut forged = ticket(1_000);
        forged.shard = "zone-9".into();
        let forged_payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());
        let sig = token.rsplit('.').next().unwrap();
        let tampered = format!("v1.{forged_payload}.{sig}");
        assert_eq!(
            verify_ticket(b"secret", &tampered, 0),
            Err(TicketError::BadSignature)
        );

        assert_eq!(
            verify_ticket(b"secret", "nonsense", 0),
            Err(TicketError::Malformed)
        );
        assert_eq!(
            verify_ticket(b"secret", "v2.a.b", 0),
            Err(TicketError::Malformed)
        );
    }

    #[test]
    fn an_expired_ticket_fails() {
        let token = sign_ticket(b"secret", &ticket(1_000));
        assert_eq!(
            verify_ticket(b"secret", &token, 1_000),
            Err(TicketError::Expired)
        );
    }
}
