//! Shard ticket signing and verification for this host (see
//! `pylon_realtime::ticket`).
//!
//! The ticket secret is `PYLON_SHARD_TICKET_SECRET` when set, else derived
//! from the host's signing secret (`PYLON_JWT_SECRET`, then
//! `PYLON_ENCRYPTION_KEY`, then a key kept in `.pylon/`). Machines that
//! verify each other's tickets need the same value: set one of the two
//! environment variables on all of them.

use hmac::{Hmac, Mac};
use pylon_auth::AuthContext;
use pylon_realtime::{ShardAuth, ShardTicket, TicketError};
use sha2::Sha256;

/// The longest lifetime a ticket can be minted with.
pub const MAX_TTL_SECS: u64 = 3600;
/// Lifetime when the caller does not pass one.
pub const DEFAULT_TTL_SECS: u64 = 60;

pub fn ticket_secret() -> &'static [u8] {
    static CELL: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| {
        if let Ok(v) = std::env::var("PYLON_SHARD_TICKET_SECRET") {
            if !v.is_empty() {
                return v.into_bytes();
            }
        }
        // Domain separation: a ticket signature is never valid as a file-URL
        // signature, or the reverse.
        let mut mac = Hmac::<Sha256>::new_from_slice(crate::file_urls::signing_secret())
            .expect("HMAC takes any key length");
        mac.update(b"pylon-shard-ticket-v1");
        mac.finalize().into_bytes().to_vec()
    })
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Mint a ticket for `shard` and `sid`, valid for `ttl_secs` (default 60,
/// at most one hour).
pub fn mint(
    shard: &str,
    sid: &str,
    user_id: Option<String>,
    claims: serde_json::Value,
    ttl_secs: Option<u64>,
) -> String {
    let ttl = ttl_secs.unwrap_or(DEFAULT_TTL_SECS).clamp(1, MAX_TTL_SECS);
    pylon_realtime::sign_ticket(
        ticket_secret(),
        &ShardTicket {
            shard: shard.to_string(),
            sid: sid.to_string(),
            user_id,
            exp: unix_now() + ttl,
            claims,
        },
    )
}

pub fn verify(token: &str) -> Result<ShardTicket, TicketError> {
    pylon_realtime::verify_ticket(ticket_secret(), token, unix_now())
}

/// The shard auth for a session, with a verified ticket when one was sent.
/// A ticket that fails verification is an error, not a silent downgrade.
pub fn shard_auth(auth: &AuthContext, ticket: Option<&str>) -> Result<ShardAuth, TicketError> {
    let ticket = match ticket.filter(|t| !t.is_empty()) {
        Some(t) => Some(verify(t)?),
        None => None,
    };
    Ok(ShardAuth {
        user_id: auth.user_id.clone(),
        is_admin: auth.is_admin,
        roles: auth.roles.clone(),
        tenant_id: auth.tenant_id.clone(),
        ticket,
    })
}
