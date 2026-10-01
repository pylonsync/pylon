//! Steam sign-in: check a session ticket a Steam game got from
//! `ISteamUser::GetAuthTicketForWebApi(identity)` with Steam's
//! `ISteamUserAuth/AuthenticateUserTicket/v1` Web API.
//!
//! Configuration (environment):
//!
//! - `PYLON_STEAM_WEB_API_KEY`: the publisher Web API key.
//! - `PYLON_STEAM_APP_ID`: the game's app id. Steam refuses a ticket made
//!   for another app.
//! - `PYLON_STEAM_IDENTITY`: the identity string the game passes to
//!   `GetAuthTicketForWebApi`. Steam refuses a ticket made for another
//!   identity, so a ticket a player gave some other service does not sign
//!   in here.
//! - `PYLON_STEAM_REFUSE_BANNED`: `1` refuses a player with a VAC ban or a
//!   publisher ban on the app.
//! - `PYLON_STEAM_API_BASE`: the Web API origin. Default
//!   `https://partner.steam-api.com`; tests point it at a local server.
//!
//! The account is the player's SteamID (`steamid`). With Family Sharing,
//! `ownersteamid` is the account that owns the game and differs from the
//! player; the player still signs in as themselves.

use std::time::Duration;

/// A Steam session ticket is a hex string. Steam's are a few hundred
/// bytes; anything this long is not one.
pub const MAX_TICKET_HEX_LEN: usize = 8192;

const DEFAULT_API_BASE: &str = "https://partner.steam-api.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamConfig {
    pub web_api_key: String,
    pub app_id: String,
    pub identity: String,
    pub refuse_banned: bool,
    pub api_base: String,
}

impl SteamConfig {
    /// The configuration from the environment, or None when Steam sign-in
    /// is not configured (the key, app id, or identity is missing).
    pub fn from_env() -> Option<Self> {
        let get = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let refuse_banned = matches!(
            get("PYLON_STEAM_REFUSE_BANNED").as_deref(),
            Some("1" | "true" | "yes")
        );
        Some(Self {
            web_api_key: get("PYLON_STEAM_WEB_API_KEY")?,
            app_id: get("PYLON_STEAM_APP_ID")?,
            identity: get("PYLON_STEAM_IDENTITY")?,
            refuse_banned,
            api_base: get("PYLON_STEAM_API_BASE")
                .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
                .trim_end_matches('/')
                .to_string(),
        })
    }
}

/// The player a ticket names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamIdentity {
    /// SteamID64 of the player.
    pub steam_id: String,
    /// SteamID64 of the account that owns the game (differs under Family Sharing).
    pub owner_steam_id: String,
    pub vac_banned: bool,
    pub publisher_banned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamError {
    /// The ticket is not a hex string of a plausible length.
    MalformedTicket,
    /// Steam refused the ticket (expired, another app, another identity, forged).
    Rejected(String),
    /// The player is banned and the app refuses banned players.
    Banned,
    /// Steam could not be reached or answered something unexpected.
    Unavailable(String),
}

impl std::fmt::Display for SteamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedTicket => write!(f, "the ticket is not a hex string of a valid length"),
            Self::Rejected(why) => write!(f, "Steam refused the ticket: {why}"),
            Self::Banned => write!(f, "the Steam account is banned"),
            Self::Unavailable(why) => write!(f, "Steam is unavailable: {why}"),
        }
    }
}

/// The email address of a new User row for a Steam player, who has none:
/// `steam-<steamid>-<random>@steam.invalid`. The `.invalid` TLD never
/// delivers, and the random part keeps anyone from registering the address
/// before the player first signs in.
pub fn placeholder_email(steam_id: &str) -> String {
    use rand::Rng;
    let suffix: [u8; 6] = rand::thread_rng().gen();
    let hex: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
    format!("steam-{steam_id}-{hex}@steam.invalid")
}

/// True for an even-length hex string within `MAX_TICKET_HEX_LEN`.
pub fn ticket_is_well_formed(ticket: &str) -> bool {
    !ticket.is_empty()
        && ticket.len() <= MAX_TICKET_HEX_LEN
        && ticket.len().is_multiple_of(2)
        && ticket.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A SteamID64: 17 decimal digits, starting with 7656 (individual accounts).
fn is_steam_id64(s: &str) -> bool {
    s.len() == 17 && s.starts_with("7656") && s.bytes().all(|b| b.is_ascii_digit())
}

/// Read an `AuthenticateUserTicket` response body. Anything other than a
/// well-formed `result: "OK"` with a SteamID is refused.
pub fn parse_response(body: &str, refuse_banned: bool) -> Result<SteamIdentity, SteamError> {
    let v: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| SteamError::Unavailable(format!("the response is not JSON: {e}")))?;
    let response = &v["response"];
    if let Some(err) = response.get("error") {
        let code = err["errorcode"].as_i64().unwrap_or(0);
        let desc = err["errordesc"].as_str().unwrap_or("");
        return Err(SteamError::Rejected(
            format!("{code} {desc}").trim().to_string(),
        ));
    }
    let params = response.get("params").ok_or_else(|| {
        SteamError::Unavailable("the response has neither params nor error".into())
    })?;
    if params["result"].as_str() != Some("OK") {
        return Err(SteamError::Rejected(format!(
            "result {}",
            params["result"].as_str().unwrap_or("missing")
        )));
    }
    let id_of = |key: &str| -> Option<String> {
        let raw = match &params[key] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => return None,
        };
        is_steam_id64(&raw).then_some(raw)
    };
    let steam_id = id_of("steamid")
        .ok_or_else(|| SteamError::Unavailable("the response has no valid steamid".into()))?;
    let owner_steam_id = id_of("ownersteamid").unwrap_or_else(|| steam_id.clone());
    let identity = SteamIdentity {
        steam_id,
        owner_steam_id,
        vac_banned: params["vacbanned"].as_bool().unwrap_or(false),
        publisher_banned: params["publisherbanned"].as_bool().unwrap_or(false),
    };
    if refuse_banned && (identity.vac_banned || identity.publisher_banned) {
        return Err(SteamError::Banned);
    }
    Ok(identity)
}

fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Check `ticket` with Steam. Fails closed: a network error, a timeout, a
/// non-200 status, or an unexpected body is `Unavailable`, never a sign-in.
pub fn verify_ticket(config: &SteamConfig, ticket: &str) -> Result<SteamIdentity, SteamError> {
    if !ticket_is_well_formed(ticket) {
        return Err(SteamError::MalformedTicket);
    }
    let url = format!(
        "{}/ISteamUserAuth/AuthenticateUserTicket/v1/?key={}&appid={}&ticket={}&identity={}",
        config.api_base,
        query_escape(&config.web_api_key),
        query_escape(&config.app_id),
        query_escape(ticket),
        query_escape(&config.identity),
    );
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .build();
    let body = match agent.get(&url).set("Accept", "application/json").call() {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| SteamError::Unavailable(format!("read body ({:?})", e.kind())))?,
        // Steam answers 403 for a bad key; any status other than 200 is a
        // server-side problem the player cannot fix.
        Err(ureq::Error::Status(code, _)) => {
            return Err(SteamError::Unavailable(format!("HTTP {code}")))
        }
        // The transport error's text carries the request URL, which holds
        // the Web API key and the ticket; keep only its kind.
        Err(ureq::Error::Transport(t)) => {
            return Err(SteamError::Unavailable(format!(
                "transport error ({:?})",
                t.kind()
            )))
        }
    };
    parse_response(&body, config.refuse_banned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER: &str = "76561197960287930";
    const OWNER: &str = "76561197960265728";

    fn ok(steamid: &str, owner: &str, vac: bool, publisher: bool) -> String {
        serde_json::json!({ "response": { "params": {
            "result": "OK", "steamid": steamid, "ownersteamid": owner,
            "vacbanned": vac, "publisherbanned": publisher
        }}})
        .to_string()
    }

    #[test]
    fn an_ok_response_names_the_player_not_the_owner() {
        let id = parse_response(&ok(PLAYER, OWNER, false, false), true).unwrap();
        assert_eq!(id.steam_id, PLAYER);
        assert_eq!(id.owner_steam_id, OWNER);
    }

    #[test]
    fn a_ban_is_refused_only_when_configured() {
        assert_eq!(
            parse_response(&ok(PLAYER, PLAYER, true, false), true),
            Err(SteamError::Banned)
        );
        assert_eq!(
            parse_response(&ok(PLAYER, PLAYER, false, true), true),
            Err(SteamError::Banned)
        );
        assert!(
            parse_response(&ok(PLAYER, PLAYER, true, true), false)
                .unwrap()
                .vac_banned
        );
    }

    #[test]
    fn errors_and_odd_bodies_never_sign_in() {
        let err = serde_json::json!({"response":{"error":{"errorcode":101,"errordesc":"Invalid ticket"}}});
        assert!(
            matches!(parse_response(&err.to_string(), false), Err(SteamError::Rejected(m)) if m.contains("101"))
        );
        assert!(matches!(
            parse_response("not json", false),
            Err(SteamError::Unavailable(_))
        ));
        assert!(matches!(
            parse_response(r#"{"response":{}}"#, false),
            Err(SteamError::Unavailable(_))
        ));
        let not_ok =
            serde_json::json!({"response":{"params":{"result":"Denied","steamid":PLAYER}}});
        assert!(matches!(
            parse_response(&not_ok.to_string(), false),
            Err(SteamError::Rejected(_))
        ));
        // A missing or malformed steamid is not a player.
        for bad in ["", "123", "76561197960287930x", "86561197960287930"] {
            let body = serde_json::json!({"response":{"params":{"result":"OK","steamid":bad}}});
            assert!(
                matches!(
                    parse_response(&body.to_string(), false),
                    Err(SteamError::Unavailable(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn tickets_are_bounded_hex() {
        assert!(ticket_is_well_formed("14000000a1b2c3D4"));
        assert!(!ticket_is_well_formed(""));
        assert!(!ticket_is_well_formed("abc"));
        assert!(!ticket_is_well_formed("zz"));
        assert!(!ticket_is_well_formed(&"a".repeat(MAX_TICKET_HEX_LEN + 2)));
        assert!(ticket_is_well_formed(&"a".repeat(MAX_TICKET_HEX_LEN)));
    }

    #[test]
    fn placeholder_emails_are_unique_and_undeliverable() {
        let a = placeholder_email(PLAYER);
        let b = placeholder_email(PLAYER);
        assert_ne!(a, b);
        assert!(a.starts_with(&format!("steam-{PLAYER}-")));
        assert!(a.ends_with("@steam.invalid"));
    }

    #[test]
    fn an_unreachable_steam_fails_closed_without_leaking_the_key_or_ticket() {
        // A port nothing listens on.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let config = SteamConfig {
            web_api_key: "SECRETKEY0123456789".into(),
            app_id: "480".into(),
            identity: "pylon".into(),
            refuse_banned: false,
            api_base: format!("http://127.0.0.1:{port}"),
        };
        let ticket = "14000000deadbeefcafe";
        let err = verify_ticket(&config, ticket).unwrap_err();
        let SteamError::Unavailable(message) = &err else {
            panic!("expected Unavailable, got {err:?}");
        };
        assert!(!message.contains("SECRETKEY"), "{message}");
        assert!(!message.contains(ticket), "{message}");
        assert!(!err.to_string().contains("SECRETKEY"));
    }

    #[test]
    fn query_values_are_escaped() {
        assert_eq!(query_escape("a b&c=d/é"), "a%20b%26c%3Dd%2F%C3%A9");
    }
}
