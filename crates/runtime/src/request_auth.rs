//! Turning a connection's credentials into an identity.
//!
//! Every transport goes through here: HTTP requests, the sync WebSocket (on
//! the main port and on its own port), the SSE port, and shard connections
//! (WebSocket and WebTransport). A transport parses what the client sent
//! into [`Credentials`] and calls [`AuthResolver::identify`]. It decides two
//! things itself: which sources it reads (a transport that takes no cookie
//! passes no cookie names), and whether an ambient session cookie is trusted
//! on this request (the CSRF check for HTTP, the Origin check for a
//! handshake).
//!
//! The rules:
//!   1. An explicit token (`Authorization: Bearer`, the `bearer.<token>`
//!      WebSocket subprotocol, `?token=` on SSE, a shard hello) that
//!      resolves to a user or the admin wins.
//!   2. An explicit token the resolver rejects outright (a bad `pk.` API key,
//!      a bad JWT) is an error, even with a valid cookie: the cookie does not
//!      cover up a wrong credential.
//!   3. An explicit token that resolves to no one (an unknown, expired or
//!      revoked session, or an operator session on an app surface) gives way
//!      to the session cookie when the cookie is trusted and resolves to an
//!      identity. Otherwise the connection is anonymous.
//!   4. With no explicit token, a trusted cookie is used. An untrusted one is
//!      refused; the transport decides whether that rejects the connection.

use std::sync::Arc;

use pylon_auth::{api_key::ApiKeyStore, AccountStore, AuthContext, SessionStore};

/// The credentials a connection presented.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Credentials {
    /// An explicit token: `Authorization: Bearer`, the `bearer.<token>`
    /// WebSocket subprotocol, `?token=` (SSE), or a shard hello's token.
    pub explicit: Option<String>,
    /// The session cookie's token, when the transport reads cookies.
    pub cookie: Option<String>,
    /// The `bearer.<token>` subprotocol the client offered, to echo in a
    /// WebSocket handshake response (browsers refuse the socket otherwise).
    pub subprotocol: Option<String>,
}

impl Credentials {
    /// Parse request headers. `cookie_names` lists the session cookies to
    /// read, first match wins; pass none for a transport that takes no
    /// cookie. The `Authorization` header wins over the subprotocol.
    pub fn from_headers<'a>(
        headers: impl IntoIterator<Item = (&'a str, &'a str)>,
        cookie_names: &[&str],
    ) -> Self {
        let mut header_token: Option<String> = None;
        let mut proto_token: Option<String> = None;
        let mut subprotocol: Option<String> = None;
        let mut cookie_header: Option<&str> = None;
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("authorization") {
                if let Some(token) = bearer_from_authorization(value) {
                    header_token = Some(token);
                }
            } else if name.eq_ignore_ascii_case("sec-websocket-protocol") {
                if subprotocol.is_none() {
                    for proto in value.split(',').map(str::trim) {
                        if let Some(encoded) = proto.strip_prefix("bearer.") {
                            if let Some(decoded) = crate::ws::percent_decode_token(encoded) {
                                proto_token = Some(decoded);
                                subprotocol = Some(proto.to_string());
                                break;
                            }
                        }
                    }
                }
            } else if name.eq_ignore_ascii_case("cookie") {
                cookie_header = Some(value);
            }
        }
        let cookie = cookie_header.and_then(|cookies| {
            cookie_names
                .iter()
                .find_map(|name| pylon_auth::extract_session_cookie(cookies, name))
        });
        Self {
            explicit: header_token.or(proto_token),
            cookie,
            subprotocol,
        }
    }

    /// Only an explicit token (a shard hello, a native client).
    pub fn explicit(token: Option<String>) -> Self {
        Self {
            explicit: token,
            ..Self::default()
        }
    }

    /// Use `token` (a `?token=` query parameter) as the explicit token when
    /// the headers carried none.
    pub fn or_query_token(mut self, token: Option<String>) -> Self {
        if self.explicit.is_none() {
            self.explicit = token;
        }
        self
    }
}

/// `Bearer <token>`, scheme case-insensitive (RFC 7235). `None` for another
/// scheme or an empty token.
fn bearer_from_authorization(value: &str) -> Option<String> {
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then(|| token.to_string())
}

/// Where a request is going. Studio's surface accepts operator sessions;
/// on the app's own surface an operator session is no one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    App,
    Studio,
}

/// Whether an ambient session cookie is trusted on a connection that is not
/// an HTTP request (a handshake), from its `Origin` and `Host` headers. The
/// server builds it from its CORS allowlist (see `sse_cookie_trust`).
pub type CookieTrust = Arc<dyn Fn(Option<&str>, Option<&str>) -> bool + Send + Sync>;

/// Completes a resolved identity: per-user admin designation and the
/// caller's role in their active org. See [`AuthResolver::enrich`].
pub type AuthEnricher = Arc<dyn Fn(&mut AuthContext) + Send + Sync>;

/// The identity a connection acts as, and where it came from.
#[derive(Debug)]
pub struct Identity {
    pub ctx: AuthContext,
    /// The token the identity came from: the explicit token, or the cookie
    /// when the connection fell back to it or presented only a cookie.
    pub token: Option<String>,
    /// True when `token` is the session cookie.
    pub cookie_auth: bool,
    /// True when an explicit token was presented and resolved to no one.
    /// HTTP responses report it so a client clears a token it stored.
    pub explicit_rejected: bool,
    /// True when only a cookie was presented and it was not trusted on this
    /// request. The connection is anonymous; a transport that requires a
    /// trusted Origin for cookie auth rejects it instead.
    pub cookie_refused: bool,
}

/// Resolves tokens to identities the same way for every transport.
pub struct AuthResolver {
    pub sessions: Arc<SessionStore>,
    pub api_keys: Arc<ApiKeyStore>,
    pub admin_token: Option<String>,
    pub jwt_secret: Option<String>,
    pub jwt_issuer: Option<String>,
    /// Operator accounts, so an operator session counts as no one on the
    /// app's surface. `None` treats operator sessions like any session.
    pub accounts: Option<Arc<AccountStore>>,
    /// See [`AuthResolver::enrich`]. `None` leaves the resolved identity as
    /// it is, with no active-org role.
    pub enrich: Option<AuthEnricher>,
}

impl AuthResolver {
    /// A resolver that knows only sessions: no admin token, API keys, JWTs,
    /// operator accounts or enrichment. For tests and standalone servers.
    pub fn from_sessions(sessions: Arc<SessionStore>) -> Arc<Self> {
        Arc::new(Self {
            sessions,
            api_keys: Arc::new(ApiKeyStore::new()),
            admin_token: None,
            jwt_secret: None,
            jwt_issuer: None,
            accounts: None,
            enrich: None,
        })
    }

    /// Resolve one token: the admin token, a `pk.` API key, a JWT (when
    /// configured), or a session. Counts as a use of an API key.
    pub fn resolve_token(
        &self,
        token: Option<&str>,
        surface: Surface,
    ) -> Result<AuthContext, &'static str> {
        let mut ctx = pylon_auth::resolve_bearer_token(
            token,
            &self.sessions,
            &self.api_keys,
            self.admin_token.as_deref(),
            self.jwt_secret.as_deref(),
            self.jwt_issuer.as_deref(),
        )?;
        self.drop_operator(&mut ctx, surface);
        Ok(ctx)
    }

    /// [`resolve_token`](Self::resolve_token) for a credential checked again
    /// (a long-lived connection's periodic pass): does not count as a use of
    /// an API key.
    pub fn recheck_token(
        &self,
        token: Option<&str>,
        surface: Surface,
    ) -> Result<AuthContext, &'static str> {
        let mut ctx = pylon_auth::recheck_bearer_token(
            token,
            &self.sessions,
            &self.api_keys,
            self.admin_token.as_deref(),
            self.jwt_secret.as_deref(),
            self.jwt_issuer.as_deref(),
        )?;
        self.drop_operator(&mut ctx, surface);
        Ok(ctx)
    }

    fn drop_operator(&self, ctx: &mut AuthContext, surface: Surface) {
        if surface == Surface::App {
            if let Some(accounts) = &self.accounts {
                crate::server::drop_operator_identity(accounts, ctx);
            }
        }
    }

    /// Finish an identity the way every transport must before it uses it:
    /// admin designation and the active org's role. Without the role, a read
    /// policy that calls `auth.hasAnyRole(...)` denies everything.
    pub fn enrich(&self, ctx: &mut AuthContext) {
        if let Some(enrich) = &self.enrich {
            enrich(ctx);
        }
    }

    /// Apply the module's rules to `creds`. `cookie_trusted` runs at most
    /// once, and only when the cookie would be used.
    pub fn identify(
        &self,
        creds: &Credentials,
        surface: Surface,
        cookie_trusted: impl FnOnce() -> bool,
    ) -> Result<Identity, &'static str> {
        identify_with(creds, cookie_trusted, |token| {
            self.resolve_token(Some(token), surface)
        })
    }
}

/// [`AuthResolver::identify`] over any token resolver, so the rules are
/// testable without stores.
pub(crate) fn identify_with(
    creds: &Credentials,
    cookie_trusted: impl FnOnce() -> bool,
    resolve: impl Fn(&str) -> Result<AuthContext, &'static str>,
) -> Result<Identity, &'static str> {
    let cookie_identity = |cookie: &str| {
        resolve(cookie)
            .ok()
            .filter(has_identity)
            .map(|ctx| (ctx, cookie.to_string()))
    };
    let Some(explicit) = creds.explicit.as_deref() else {
        let Some(cookie) = creds.cookie.as_deref() else {
            return Ok(anonymous(None, false, false));
        };
        if !cookie_trusted() {
            return Ok(anonymous(None, false, true));
        }
        let ctx = resolve(cookie)?;
        return Ok(Identity {
            ctx,
            token: Some(cookie.to_string()),
            cookie_auth: true,
            explicit_rejected: false,
            cookie_refused: false,
        });
    };
    let ctx = resolve(explicit)?;
    if has_identity(&ctx) {
        return Ok(Identity {
            ctx,
            token: Some(explicit.to_string()),
            cookie_auth: false,
            explicit_rejected: false,
            cookie_refused: false,
        });
    }
    if let Some(cookie) = creds.cookie.as_deref() {
        if cookie_trusted() {
            if let Some((ctx, token)) = cookie_identity(cookie) {
                return Ok(Identity {
                    ctx,
                    token: Some(token),
                    cookie_auth: true,
                    explicit_rejected: true,
                    cookie_refused: false,
                });
            }
        }
    }
    Ok(Identity {
        ctx,
        token: Some(explicit.to_string()),
        cookie_auth: false,
        explicit_rejected: true,
        cookie_refused: false,
    })
}

fn anonymous(token: Option<String>, explicit_rejected: bool, cookie_refused: bool) -> Identity {
    Identity {
        ctx: AuthContext::anonymous(),
        token,
        cookie_auth: false,
        explicit_rejected,
        cookie_refused,
    }
}

fn has_identity(ctx: &AuthContext) -> bool {
    ctx.user_id.is_some() || ctx.is_admin
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "good-*" resolves to that user, "admin" to the admin, "pk.bad" is
    /// rejected, anything else is anonymous.
    fn resolve(token: &str) -> Result<AuthContext, &'static str> {
        match token {
            "admin" => Ok(AuthContext::admin()),
            "pk.bad" => Err("INVALID_API_KEY"),
            t if t.starts_with("good-") => Ok(AuthContext::authenticated(t.to_string())),
            _ => Ok(AuthContext::anonymous()),
        }
    }

    fn creds(explicit: Option<&str>, cookie: Option<&str>) -> Credentials {
        Credentials {
            explicit: explicit.map(String::from),
            cookie: cookie.map(String::from),
            subprotocol: None,
        }
    }

    fn user(id: &Identity) -> Option<&str> {
        id.ctx.user_id.as_deref()
    }

    #[test]
    fn a_valid_explicit_token_wins_and_the_cookie_is_not_checked() {
        let id = identify_with(
            &creds(Some("good-bearer"), Some("good-cookie")),
            || panic!("the trust check must not run"),
            resolve,
        )
        .unwrap();
        assert_eq!(user(&id), Some("good-bearer"));
        assert!(!id.cookie_auth && !id.explicit_rejected);
        let admin = identify_with(&creds(Some("admin"), Some("good-cookie")), || true, resolve);
        assert!(admin.unwrap().ctx.is_admin);
    }

    #[test]
    fn a_stale_explicit_token_gives_way_to_a_trusted_cookie() {
        let id =
            identify_with(&creds(Some("stale"), Some("good-cookie")), || true, resolve).unwrap();
        assert_eq!(user(&id), Some("good-cookie"));
        assert_eq!(id.token.as_deref(), Some("good-cookie"));
        assert!(id.cookie_auth && id.explicit_rejected);
    }

    #[test]
    fn a_stale_explicit_token_stays_anonymous_with_an_untrusted_or_stale_cookie() {
        for (cookie, trusted) in [
            (Some("good-cookie"), false),
            (Some("stale-cookie"), true),
            (None, true),
        ] {
            let id = identify_with(&creds(Some("stale"), cookie), || trusted, resolve).unwrap();
            assert_eq!(user(&id), None);
            assert_eq!(id.token.as_deref(), Some("stale"));
            assert!(id.explicit_rejected && !id.cookie_auth);
        }
    }

    #[test]
    fn a_rejected_explicit_token_is_an_error_even_with_a_valid_cookie() {
        let err = identify_with(
            &creds(Some("pk.bad"), Some("good-cookie")),
            || true,
            resolve,
        )
        .unwrap_err();
        assert_eq!(err, "INVALID_API_KEY");
    }

    #[test]
    fn a_cookie_alone_is_used_only_when_trusted() {
        let id = identify_with(&creds(None, Some("good-cookie")), || true, resolve).unwrap();
        assert_eq!(user(&id), Some("good-cookie"));
        assert!(id.cookie_auth && !id.cookie_refused);
        let id = identify_with(&creds(None, Some("good-cookie")), || false, resolve).unwrap();
        assert_eq!(user(&id), None);
        assert!(id.cookie_refused && !id.cookie_auth);
    }

    #[test]
    fn no_credentials_is_anonymous() {
        let id =
            identify_with(&creds(None, None), || panic!("no cookie to trust"), resolve).unwrap();
        assert_eq!(user(&id), None);
        assert!(!id.cookie_auth && !id.cookie_refused && !id.explicit_rejected);
    }

    #[test]
    fn headers_parse_into_one_set_of_credentials() {
        let c = Credentials::from_headers(
            [
                ("authorization", "bearer  tok-header "),
                ("Sec-WebSocket-Protocol", "pylon, bearer.tok%2Dproto"),
                ("Cookie", "other=1; app_studio=op; app_session=tok-cookie"),
            ],
            &["app_session"],
        );
        assert_eq!(c.explicit.as_deref(), Some("tok-header"));
        assert_eq!(c.subprotocol.as_deref(), Some("bearer.tok%2Dproto"));
        assert_eq!(c.cookie.as_deref(), Some("tok-cookie"));

        // The subprotocol is the explicit token without an Authorization
        // header; cookie names are read in order; no names, no cookie.
        let c = Credentials::from_headers(
            [
                ("Sec-WebSocket-Protocol", "bearer.tok-proto"),
                ("Cookie", "app_studio=op; app_session=tok-cookie"),
            ],
            &["app_studio", "app_session"],
        );
        assert_eq!(c.explicit.as_deref(), Some("tok-proto"));
        assert_eq!(c.cookie.as_deref(), Some("op"));
        let c = Credentials::from_headers([("Cookie", "app_session=tok-cookie")], &[]);
        assert_eq!(c.cookie, None);

        // Another scheme or an empty token is no token; ?token= fills in.
        let c = Credentials::from_headers([("Authorization", "Basic abc")], &[])
            .or_query_token(Some("tok-query".into()));
        assert_eq!(c.explicit.as_deref(), Some("tok-query"));
        let c = Credentials::from_headers([("Authorization", "Bearer ")], &[]);
        assert_eq!(c.explicit, None);
    }
}
