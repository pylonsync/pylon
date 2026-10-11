//! Providers for `ctx.connections` that are not sign-in providers.
//!
//! Square, Stripe, Clover, Calendly, and Mailchimp let an app act on a
//! business's account. None of them is a way to sign in, and each one
//! departs from the plain OAuth 2 shape that [`crate::provider`] covers:
//!
//! - Square and Clover take JSON token requests.
//! - Square returns `expires_at` as an RFC 3339 time, Clover as Unix
//!   seconds in `access_token_expiration`.
//! - Clover refreshes at a separate endpoint and takes no client secret
//!   there.
//! - Stripe's `client_secret` is the platform's secret API key.
//! - Mailchimp tokens never expire (`expires_in: 0`) and the account's
//!   data center comes from a separate metadata call.
//! - The account id is a token field (Square `merchant_id`, Stripe
//!   `stripe_user_id`, Calendly `owner`), a callback query parameter
//!   (Clover `merchant_id`), or a metadata field (Mailchimp `user_id`).
//!
//! Keeping them out of the sign-in table means `/api/auth/login/square`
//! never exists. A spec here is data; the request builders and the
//! response parser are pure so they are tested without a network.
//!
//! API-key providers ([`ApiKeySpec`]) cover services that hand the
//! business a key instead of an OAuth consent screen (Toast's Standard
//! API access, a Stripe restricted key, a Square access token). A check
//! call confirms the key works before it is stored.

use std::collections::HashMap;

/// One field of a token request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    GrantType,
    ClientId,
    ClientSecret,
    Code,
    RedirectUri,
    RefreshToken,
    /// Sent only when the flow used PKCE.
    CodeVerifier,
}

/// How a token request body is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    Form,
    Json,
}

/// Where the access token's expiry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// `expires_in` seconds. `0` or absent means the token does not expire.
    ExpiresIn,
    /// An RFC 3339 time in the named field.
    Rfc3339(&'static str),
    /// Unix seconds in the named field.
    UnixSeconds(&'static str),
}

/// Where the connected account's id comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountId {
    /// A field of the token response.
    TokenField(&'static str),
    /// A query parameter on the callback URL.
    CallbackParam(&'static str),
    /// A field of the metadata response (see [`Metadata`]).
    MetadataField(&'static str),
}

/// A GET made with the new access token right after the code exchange,
/// whose listed fields are kept with the connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metadata {
    pub url: &'static str,
    /// The Authorization scheme: `Bearer`, or Mailchimp's `OAuth`.
    pub scheme: &'static str,
    pub keep: &'static [&'static str],
}

#[derive(Debug, Clone)]
pub struct IntegrationSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub auth_url: &'static str,
    pub token_url: &'static str,
    pub refresh_url: &'static str,
    /// Used when the connection declares no scopes. Empty sends no `scope`.
    pub default_scopes: &'static str,
    /// Whether the authorize request carries `redirect_uri`. Square uses
    /// the one registered for the app and rejects a mismatch.
    pub auth_sends_redirect_uri: bool,
    /// Extra authorize parameters, already URL-safe.
    pub auth_extra: &'static [(&'static str, &'static str)],
    /// Whether the authorize request carries a PKCE challenge.
    pub pkce: bool,
    pub body: Body,
    pub code_fields: &'static [Field],
    pub refresh_fields: &'static [Field],
    /// False when the provider issues no refresh token.
    pub refreshes: bool,
    pub expiry: Expiry,
    pub account_id: Option<AccountId>,
    pub metadata: Option<Metadata>,
}

use Field::*;

pub static SQUARE: IntegrationSpec = IntegrationSpec {
    id: "square",
    display_name: "Square",
    auth_url: "https://connect.squareup.com/oauth2/authorize",
    token_url: "https://connect.squareup.com/oauth2/token",
    refresh_url: "https://connect.squareup.com/oauth2/token",
    default_scopes: "MERCHANT_PROFILE_READ",
    auth_sends_redirect_uri: false,
    // Square signs the browser into the seller account it already has a
    // session for unless told not to; a business owner on a shared
    // computer would connect the wrong account.
    auth_extra: &[("session", "false")],
    pkce: false,
    body: Body::Json,
    code_fields: &[ClientId, ClientSecret, Code, GrantType],
    refresh_fields: &[ClientId, ClientSecret, RefreshToken, GrantType],
    refreshes: true,
    expiry: Expiry::Rfc3339("expires_at"),
    account_id: Some(AccountId::TokenField("merchant_id")),
    metadata: None,
};

pub static STRIPE: IntegrationSpec = IntegrationSpec {
    id: "stripe",
    display_name: "Stripe",
    auth_url: "https://connect.stripe.com/oauth/authorize",
    token_url: "https://connect.stripe.com/oauth/token",
    refresh_url: "https://connect.stripe.com/oauth/token",
    default_scopes: "read_write",
    auth_sends_redirect_uri: true,
    auth_extra: &[],
    pkce: false,
    body: Body::Form,
    // The client secret is the platform's secret API key.
    code_fields: &[ClientSecret, Code, GrantType],
    refresh_fields: &[ClientSecret, RefreshToken, GrantType],
    refreshes: true,
    expiry: Expiry::ExpiresIn,
    account_id: Some(AccountId::TokenField("stripe_user_id")),
    metadata: None,
};

pub static CLOVER: IntegrationSpec = IntegrationSpec {
    id: "clover",
    display_name: "Clover",
    auth_url: "https://www.clover.com/oauth/v2/authorize",
    token_url: "https://api.clover.com/oauth/v2/token",
    refresh_url: "https://api.clover.com/oauth/v2/refresh",
    // Clover permissions are set on the app in the Clover dashboard.
    default_scopes: "",
    auth_sends_redirect_uri: true,
    auth_extra: &[],
    pkce: false,
    body: Body::Json,
    code_fields: &[ClientId, ClientSecret, Code],
    refresh_fields: &[ClientId, RefreshToken],
    refreshes: true,
    expiry: Expiry::UnixSeconds("access_token_expiration"),
    account_id: Some(AccountId::CallbackParam("merchant_id")),
    metadata: None,
};

pub static CALENDLY: IntegrationSpec = IntegrationSpec {
    id: "calendly",
    display_name: "Calendly",
    auth_url: "https://auth.calendly.com/oauth/authorize",
    token_url: "https://auth.calendly.com/oauth/token",
    refresh_url: "https://auth.calendly.com/oauth/token",
    default_scopes: "",
    auth_sends_redirect_uri: true,
    auth_extra: &[],
    pkce: true,
    body: Body::Form,
    code_fields: &[
        GrantType,
        Code,
        RedirectUri,
        ClientId,
        ClientSecret,
        CodeVerifier,
    ],
    refresh_fields: &[GrantType, RefreshToken, ClientId, ClientSecret],
    refreshes: true,
    expiry: Expiry::ExpiresIn,
    account_id: Some(AccountId::TokenField("owner")),
    metadata: None,
};

pub static MAILCHIMP: IntegrationSpec = IntegrationSpec {
    id: "mailchimp",
    display_name: "Mailchimp",
    auth_url: "https://login.mailchimp.com/oauth2/authorize",
    token_url: "https://login.mailchimp.com/oauth2/token",
    refresh_url: "https://login.mailchimp.com/oauth2/token",
    default_scopes: "",
    auth_sends_redirect_uri: true,
    auth_extra: &[],
    pkce: false,
    body: Body::Form,
    code_fields: &[GrantType, ClientId, ClientSecret, RedirectUri, Code],
    refresh_fields: &[],
    refreshes: false,
    expiry: Expiry::ExpiresIn,
    account_id: Some(AccountId::MetadataField("user_id")),
    // API calls go to https://<dc>.api.mailchimp.com; the dc is only
    // available from this call.
    metadata: Some(Metadata {
        url: "https://login.mailchimp.com/oauth2/metadata",
        scheme: "OAuth",
        keep: &["dc", "api_endpoint", "accountname", "user_id"],
    }),
};

static ALL: [&IntegrationSpec; 5] = [&SQUARE, &STRIPE, &CLOVER, &CALENDLY, &MAILCHIMP];

pub fn all() -> &'static [&'static IntegrationSpec] {
    &ALL
}

pub fn find(id: &str) -> Option<&'static IntegrationSpec> {
    all().iter().copied().find(|s| s.id == id)
}

// ---------------------------------------------------------------------------
// Requests (pure)
// ---------------------------------------------------------------------------

/// A request ready to send. `Debug` leaves out the body, which holds
/// secrets.
#[derive(Clone, PartialEq, Eq)]
pub struct TokenRequest {
    pub url: String,
    pub body: Body,
    /// Form-encoded or JSON text, matching `body`.
    pub payload: String,
}

impl std::fmt::Debug for TokenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenRequest")
            .field("url", &self.url)
            .field("body", &self.body)
            .finish_non_exhaustive()
    }
}

/// The app's registration with the provider.
#[derive(Clone)]
pub struct Client<'a> {
    pub client_id: &'a str,
    pub client_secret: &'a str,
    pub redirect_uri: &'a str,
}

/// The URL that sends the browser to the provider's consent screen.
/// `scopes` replaces the spec's default when non-empty.
pub fn authorize_url(
    spec: &IntegrationSpec,
    client: &Client<'_>,
    scopes: &str,
    state: &str,
    pkce_challenge: Option<&str>,
) -> String {
    let mut params: Vec<(String, String)> = vec![
        ("client_id".into(), client.client_id.into()),
        ("response_type".into(), "code".into()),
    ];
    if spec.auth_sends_redirect_uri {
        params.push(("redirect_uri".into(), client.redirect_uri.into()));
    }
    let scopes = if scopes.trim().is_empty() {
        spec.default_scopes
    } else {
        scopes
    };
    let scopes = scopes.split_whitespace().collect::<Vec<_>>().join(" ");
    if !scopes.is_empty() {
        params.push(("scope".into(), scopes));
    }
    params.push(("state".into(), state.into()));
    if spec.pkce {
        if let Some(challenge) = pkce_challenge {
            params.push(("code_challenge".into(), challenge.into()));
            params.push(("code_challenge_method".into(), "S256".into()));
        }
    }
    for (k, v) in spec.auth_extra {
        params.push(((*k).into(), (*v).into()));
    }
    format!("{}?{}", spec.auth_url, form_encode(&params))
}

pub fn code_request(
    spec: &IntegrationSpec,
    client: &Client<'_>,
    code: &str,
    code_verifier: Option<&str>,
) -> TokenRequest {
    let pairs = spec
        .code_fields
        .iter()
        .filter_map(|f| {
            let value = match f {
                GrantType => "authorization_code",
                ClientId => client.client_id,
                ClientSecret => client.client_secret,
                Code => code,
                RedirectUri => client.redirect_uri,
                CodeVerifier => code_verifier?,
                RefreshToken => return None,
            };
            Some((field_name(*f), value.to_string()))
        })
        .collect::<Vec<_>>();
    request(spec.token_url, spec.body, &pairs)
}

/// `None` when the provider issues no refresh token.
pub fn refresh_request(
    spec: &IntegrationSpec,
    client: &Client<'_>,
    refresh_token: &str,
) -> Option<TokenRequest> {
    if !spec.refreshes {
        return None;
    }
    let pairs = spec
        .refresh_fields
        .iter()
        .filter_map(|f| {
            let value = match f {
                GrantType => "refresh_token",
                ClientId => client.client_id,
                ClientSecret => client.client_secret,
                RefreshToken => refresh_token,
                RedirectUri => client.redirect_uri,
                Code | CodeVerifier => return None,
            };
            Some((field_name(*f), value.to_string()))
        })
        .collect::<Vec<_>>();
    Some(request(spec.refresh_url, spec.body, &pairs))
}

fn field_name(f: Field) -> &'static str {
    match f {
        GrantType => "grant_type",
        ClientId => "client_id",
        ClientSecret => "client_secret",
        Code => "code",
        RedirectUri => "redirect_uri",
        RefreshToken => "refresh_token",
        CodeVerifier => "code_verifier",
    }
}

fn request(url: &str, body: Body, pairs: &[(&str, String)]) -> TokenRequest {
    let payload = match body {
        Body::Form => form_encode(
            &pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect::<Vec<_>>(),
        ),
        Body::Json => {
            let mut map = serde_json::Map::new();
            for (k, v) in pairs {
                map.insert((*k).into(), serde_json::Value::String(v.clone()));
            }
            serde_json::Value::Object(map).to_string()
        }
    };
    TokenRequest {
        url: url.into(),
        body,
        payload,
    }
}

fn form_encode(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", super::url_encode(k), super::url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

// ---------------------------------------------------------------------------
// Responses (pure)
// ---------------------------------------------------------------------------

/// What a code exchange or a refresh returns. `Debug` leaves out the
/// tokens.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds. `None`: the token does not expire.
    pub expires_at: Option<u64>,
    pub scope: Option<String>,
    pub account_id: Option<String>,
    /// The kept metadata fields, as a JSON object.
    pub metadata: Option<serde_json::Value>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens")
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .field("account_id", &self.account_id)
            .finish_non_exhaustive()
    }
}

/// Read a token response. `callback` is the callback URL's query
/// parameters (empty for a refresh); `now` is Unix seconds.
pub fn parse_tokens(
    spec: &IntegrationSpec,
    body: &str,
    callback: &HashMap<String, String>,
    now: u64,
) -> Result<Tokens, String> {
    let json: serde_json::Value = serde_json::from_str(body).map_err(|_| {
        format!(
            "{} returned a token response that is not JSON",
            spec.display_name
        )
    })?;
    let str_field = |name: &str| -> Option<String> {
        match json.get(name)? {
            serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    };
    let access_token = str_field("access_token").ok_or_else(|| {
        let err = str_field("error_description")
            .or_else(|| str_field("error"))
            .or_else(|| str_field("message"))
            .unwrap_or_else(|| "no access_token".into());
        format!("{}: {}", spec.display_name, err)
    })?;
    let expires_at = match spec.expiry {
        Expiry::ExpiresIn => json
            .get("expires_in")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .filter(|secs| *secs > 0)
            .map(|secs| now.saturating_add(secs)),
        Expiry::Rfc3339(field) => match str_field(field) {
            Some(s) => Some(
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map_err(|_| {
                        format!("{}: `{field}` is not an RFC 3339 time", spec.display_name)
                    })?
                    .timestamp()
                    .max(0) as u64,
            ),
            None => None,
        },
        Expiry::UnixSeconds(field) => json.get(field).and_then(|v| v.as_u64()),
    };
    let account_id = match spec.account_id {
        Some(AccountId::TokenField(f)) => str_field(f),
        Some(AccountId::CallbackParam(p)) => callback.get(p).filter(|v| !v.is_empty()).cloned(),
        Some(AccountId::MetadataField(_)) | None => None,
    };
    Ok(Tokens {
        access_token,
        refresh_token: str_field("refresh_token"),
        expires_at,
        scope: str_field("scope"),
        account_id,
        metadata: None,
    })
}

/// Keep the spec's listed fields of a metadata response, and fill the
/// account id when the spec reads it from there.
pub fn apply_metadata(
    spec: &IntegrationSpec,
    tokens: &mut Tokens,
    body: &str,
) -> Result<(), String> {
    let Some(meta) = spec.metadata else {
        return Ok(());
    };
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| format!("{} returned metadata that is not JSON", spec.display_name))?;
    let mut kept = serde_json::Map::new();
    for key in meta.keep {
        if let Some(v) = json.get(*key) {
            if !v.is_null() {
                kept.insert((*key).into(), v.clone());
            }
        }
    }
    if let Some(AccountId::MetadataField(f)) = spec.account_id {
        tokens.account_id = match kept.get(f) {
            Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
    }
    tokens.metadata = Some(serde_json::Value::Object(kept));
    Ok(())
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
fn send(req: &TokenRequest) -> Result<String, String> {
    let agent = super::ureq_agent();
    let content_type = match req.body {
        Body::Form => "application/x-www-form-urlencoded",
        Body::Json => "application/json",
    };
    let result = agent
        .post(&req.url)
        .set("Content-Type", content_type)
        .set("Accept", "application/json")
        .send_string(&req.payload);
    read_response(result)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_response(result: Result<ureq::Response, ureq::Error>) -> Result<String, String> {
    match result {
        Ok(resp) => resp.into_string().map_err(|e| format!("read body: {e}")),
        // Error bodies are still read: the parser pulls `error_description`
        // out of them. sanitize_token_error strips echoed secrets.
        Err(ureq::Error::Status(_, resp)) => Ok(resp.into_string().unwrap_or_default()),
        Err(e) => Err(format!("HTTP error: {e}")),
    }
}

/// Exchange the callback's code and, when the spec has one, make the
/// metadata call.
#[cfg(not(target_arch = "wasm32"))]
pub fn exchange_code(
    spec: &IntegrationSpec,
    client: &Client<'_>,
    code: &str,
    code_verifier: Option<&str>,
    callback: &HashMap<String, String>,
) -> Result<Tokens, String> {
    let body = send(&code_request(spec, client, code, code_verifier))
        .map_err(super::sanitize_token_error)?;
    let mut tokens = parse_tokens(spec, &body, callback, super::now_secs())
        .map_err(super::sanitize_token_error)?;
    if let Some(meta) = spec.metadata {
        let agent = super::ureq_agent();
        let result = agent
            .get(meta.url)
            .set(
                "Authorization",
                &format!("{} {}", meta.scheme, tokens.access_token),
            )
            .set("Accept", "application/json")
            .call();
        let body = match result {
            Ok(resp) => resp.into_string().map_err(|e| format!("read body: {e}"))?,
            Err(ureq::Error::Status(code, _)) => {
                return Err(format!(
                    "{} metadata call failed with HTTP {code}",
                    spec.display_name
                ))
            }
            Err(e) => return Err(format!("HTTP error: {e}")),
        };
        apply_metadata(spec, &mut tokens, &body)?;
    }
    Ok(tokens)
}

/// Refresh. A response without a new refresh token keeps the old one;
/// one without an account id keeps none (the caller keeps the stored one).
#[cfg(not(target_arch = "wasm32"))]
pub fn refresh(
    spec: &IntegrationSpec,
    client: &Client<'_>,
    refresh_token: &str,
) -> Result<Tokens, String> {
    let req = refresh_request(spec, client, refresh_token)
        .ok_or_else(|| format!("{} tokens do not refresh", spec.display_name))?;
    let body = send(&req).map_err(super::sanitize_token_error)?;
    let mut tokens = parse_tokens(spec, &body, &HashMap::new(), super::now_secs())
        .map_err(super::sanitize_token_error)?;
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(refresh_token.to_string());
    }
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// API-key providers
// ---------------------------------------------------------------------------

/// A provider whose connection is a key, not an OAuth grant.
#[derive(Debug, Clone)]
pub struct ApiKeySpec {
    pub id: &'static str,
    pub display_name: &'static str,
    /// The credential fields, in the order a form shows them. The first
    /// one is the value `ctx.connections.get(name).accessToken` returns.
    pub fields: &'static [&'static str],
}

pub static STRIPE_KEY: ApiKeySpec = ApiKeySpec {
    id: "stripe",
    display_name: "Stripe",
    fields: &["secretKey"],
};

pub static SQUARE_KEY: ApiKeySpec = ApiKeySpec {
    id: "square",
    display_name: "Square",
    fields: &["accessToken"],
};

/// Toast's Standard API access: a restaurant creates the credentials in
/// Toast Web. Toast has no consent screen for this.
pub static TOAST_KEY: ApiKeySpec = ApiKeySpec {
    id: "toast",
    display_name: "Toast",
    fields: &["clientId", "clientSecret", "restaurantGuid"],
};

pub fn api_key_spec(id: &str) -> Option<&'static ApiKeySpec> {
    [&STRIPE_KEY, &SQUARE_KEY, &TOAST_KEY]
        .into_iter()
        .find(|s| s.id == id)
}

/// Check a credential's shape: every field present, no others, no
/// whitespace or control characters, at most 4 KB each. `fields` is the
/// known provider's list, or the connection's own declared list.
pub fn validate_credential_shape(
    fields: &[&str],
    creds: &HashMap<String, String>,
) -> Result<(), String> {
    for key in creds.keys() {
        if !fields.contains(&key.as_str()) {
            return Err(format!("unknown field `{key}`"));
        }
    }
    for f in fields {
        let v = creds.get(*f).map(String::as_str).unwrap_or("");
        if v.is_empty() {
            return Err(format!("`{f}` is required"));
        }
        if v.len() > 4096 {
            return Err(format!("`{f}` is too long"));
        }
        if v.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("`{f}` contains spaces or control characters"));
        }
    }
    Ok(())
}

/// Shape rules a provider adds on top of [`validate_credential_shape`].
pub fn check_known_prefix(
    spec: &ApiKeySpec,
    creds: &HashMap<String, String>,
) -> Result<(), String> {
    if spec.id == "stripe" {
        let key = creds.get("secretKey").map(String::as_str).unwrap_or("");
        // A publishable key (pk_) cannot act on the account; reject it
        // before the check call so the message says why.
        if !(key.starts_with("sk_") || key.starts_with("rk_")) {
            return Err(
                "`secretKey` must be a Stripe secret key (sk_) or restricted key (rk_)".into(),
            );
        }
    }
    if spec.id == "toast" {
        let guid = creds
            .get("restaurantGuid")
            .map(String::as_str)
            .unwrap_or("");
        let ok = guid.len() == 36
            && guid.chars().enumerate().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => c == '-',
                _ => c.is_ascii_hexdigit(),
            });
        if !ok {
            return Err("`restaurantGuid` must be the restaurant's GUID from Toast Web".into());
        }
    }
    Ok(())
}

/// Make the provider's check call. Returns the account id when the
/// provider reports one.
#[cfg(not(target_arch = "wasm32"))]
pub fn check_api_key(
    spec: &ApiKeySpec,
    creds: &HashMap<String, String>,
) -> Result<Option<String>, String> {
    let agent = super::ureq_agent();
    let field = |k: &str| creds.get(k).map(String::as_str).unwrap_or("");
    let read_json =
        |result: Result<ureq::Response, ureq::Error>| -> Result<serde_json::Value, String> {
            match result {
                Ok(resp) => {
                    let body = resp.into_string().map_err(|e| format!("read body: {e}"))?;
                    serde_json::from_str(&body).map_err(|_| {
                        format!("{} returned a response that is not JSON", spec.display_name)
                    })
                }
                Err(ureq::Error::Status(401 | 403, _)) => {
                    Err(format!("{} did not accept the key", spec.display_name))
                }
                Err(ureq::Error::Status(code, _)) => Err(format!(
                    "{} check failed with HTTP {code}",
                    spec.display_name
                )),
                Err(e) => Err(format!("HTTP error: {e}")),
            }
        };
    match spec.id {
        "stripe" => {
            let json = read_json(
                agent
                    .get("https://api.stripe.com/v1/account")
                    .set("Authorization", &format!("Bearer {}", field("secretKey")))
                    .call(),
            )?;
            Ok(json.get("id").and_then(|v| v.as_str()).map(String::from))
        }
        "square" => {
            let json = read_json(
                agent
                    .get("https://connect.squareup.com/v2/merchants/me")
                    .set("Authorization", &format!("Bearer {}", field("accessToken")))
                    .call(),
            )?;
            Ok(json
                .pointer("/merchant/id")
                .and_then(|v| v.as_str())
                .map(String::from))
        }
        "toast" => {
            let body = serde_json::json!({
                "clientId": field("clientId"),
                "clientSecret": field("clientSecret"),
                "userAccessType": "TOAST_MACHINE_CLIENT",
            })
            .to_string();
            let json = read_json(
                agent
                    .post("https://ws-api.toasttab.com/authentication/v1/authentication/login")
                    .set("Content-Type", "application/json")
                    .send_string(&body),
            )?;
            if json
                .pointer("/token/accessToken")
                .and_then(|v| v.as_str())
                .is_none()
            {
                return Err("Toast did not return a token for these credentials".into());
            }
            Ok(Some(field("restaurantGuid").to_string()))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> Client<'static> {
        Client {
            client_id: "cid",
            client_secret: "secret value",
            redirect_uri: "https://hub.example/cb",
        }
    }

    fn query(url: &str) -> HashMap<String, String> {
        let q = url.split_once('?').unwrap().1;
        q.split('&')
            .map(|p| {
                let (k, v) = p.split_once('=').unwrap();
                (k.to_string(), percent(v))
            })
            .collect()
    }

    fn percent(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn integrations_are_not_sign_in_providers() {
        for spec in all() {
            assert!(
                crate::provider::find_spec(spec.id).is_none(),
                "{} must not also be a sign-in provider",
                spec.id
            );
        }
    }

    #[test]
    fn square_authorize_uses_registered_redirect_and_no_session() {
        let url = authorize_url(
            &SQUARE,
            &client(),
            "ITEMS_READ  INVENTORY_WRITE",
            "st",
            None,
        );
        assert!(url.starts_with("https://connect.squareup.com/oauth2/authorize?"));
        let q = query(&url);
        assert_eq!(q["client_id"], "cid");
        assert_eq!(q["scope"], "ITEMS_READ INVENTORY_WRITE");
        assert_eq!(q["state"], "st");
        assert_eq!(q["session"], "false");
        assert!(!q.contains_key("redirect_uri"));
    }

    #[test]
    fn default_scopes_apply_and_empty_sends_none() {
        let q = query(&authorize_url(&STRIPE, &client(), "", "st", None));
        assert_eq!(q["scope"], "read_write");
        assert_eq!(q["redirect_uri"], "https://hub.example/cb");
        let q = query(&authorize_url(&CLOVER, &client(), "", "st", None));
        assert!(!q.contains_key("scope"));
    }

    #[test]
    fn calendly_sends_pkce_challenge() {
        let q = query(&authorize_url(&CALENDLY, &client(), "", "st", Some("chal")));
        assert_eq!(q["code_challenge"], "chal");
        assert_eq!(q["code_challenge_method"], "S256");
        // A spec without PKCE ignores a challenge.
        let q = query(&authorize_url(
            &MAILCHIMP,
            &client(),
            "",
            "st",
            Some("chal"),
        ));
        assert!(!q.contains_key("code_challenge"));
    }

    #[test]
    fn square_code_request_is_json_without_redirect() {
        let req = code_request(&SQUARE, &client(), "the-code", None);
        assert_eq!(req.body, Body::Json);
        let json: serde_json::Value = serde_json::from_str(&req.payload).unwrap();
        assert_eq!(json["grant_type"], "authorization_code");
        assert_eq!(json["client_secret"], "secret value");
        assert_eq!(json["code"], "the-code");
        assert!(json.get("redirect_uri").is_none());
    }

    #[test]
    fn clover_refresh_goes_to_refresh_endpoint_without_secret() {
        let req = refresh_request(&CLOVER, &client(), "rt").unwrap();
        assert_eq!(req.url, "https://api.clover.com/oauth/v2/refresh");
        let json: serde_json::Value = serde_json::from_str(&req.payload).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "client_id": "cid", "refresh_token": "rt" })
        );
    }

    #[test]
    fn stripe_code_request_is_form_with_platform_secret() {
        let req = code_request(&STRIPE, &client(), "ac_1", None);
        assert_eq!(req.body, Body::Form);
        assert_eq!(
            req.payload,
            "client_secret=secret%20value&code=ac_1&grant_type=authorization_code"
        );
    }

    #[test]
    fn calendly_code_request_carries_verifier_only_when_present() {
        let with = code_request(&CALENDLY, &client(), "c", Some("v"));
        assert!(with.payload.contains("code_verifier=v"));
        let without = code_request(&CALENDLY, &client(), "c", None);
        assert!(!without.payload.contains("code_verifier"));
    }

    #[test]
    fn mailchimp_does_not_refresh() {
        assert!(refresh_request(&MAILCHIMP, &client(), "rt").is_none());
    }

    #[test]
    fn token_request_debug_hides_payload() {
        let req = code_request(&SQUARE, &client(), "c", None);
        assert!(!format!("{req:?}").contains("secret value"));
    }

    #[test]
    fn parses_square_rfc3339_expiry_and_merchant() {
        let body = r#"{"access_token":"EAAA","token_type":"bearer","expires_at":"2026-11-09T00:00:00Z","merchant_id":"M1","refresh_token":"R1"}"#;
        let t = parse_tokens(&SQUARE, body, &HashMap::new(), 0).unwrap();
        assert_eq!(t.access_token, "EAAA");
        assert_eq!(t.expires_at, Some(1_794_182_400));
        assert_eq!(t.account_id.as_deref(), Some("M1"));
        assert_eq!(t.refresh_token.as_deref(), Some("R1"));
        assert!(!format!("{t:?}").contains("EAAA"));
    }

    #[test]
    fn parses_clover_unix_expiry_and_callback_merchant() {
        let body = r#"{"access_token":"a","access_token_expiration":1800000000,"refresh_token":"r","refresh_token_expiration":1900000000}"#;
        let cb = HashMap::from([("merchant_id".to_string(), "MID".to_string())]);
        let t = parse_tokens(&CLOVER, body, &cb, 0).unwrap();
        assert_eq!(t.expires_at, Some(1_800_000_000));
        assert_eq!(t.account_id.as_deref(), Some("MID"));
    }

    #[test]
    fn expires_in_zero_means_no_expiry() {
        let t = parse_tokens(
            &MAILCHIMP,
            r#"{"access_token":"a","expires_in":0}"#,
            &HashMap::new(),
            100,
        )
        .unwrap();
        assert_eq!(t.expires_at, None);
        let t = parse_tokens(
            &CALENDLY,
            r#"{"access_token":"a","expires_in":7200,"owner":"https://api.calendly.com/users/U"}"#,
            &HashMap::new(),
            100,
        )
        .unwrap();
        assert_eq!(t.expires_at, Some(7300));
        assert_eq!(
            t.account_id.as_deref(),
            Some("https://api.calendly.com/users/U")
        );
    }

    #[test]
    fn error_response_reports_the_provider_message() {
        let err = parse_tokens(
            &STRIPE,
            r#"{"error":"invalid_grant","error_description":"Authorization code expired"}"#,
            &HashMap::new(),
            0,
        )
        .unwrap_err();
        assert_eq!(err, "Stripe: Authorization code expired");
        assert!(parse_tokens(&STRIPE, "<html>", &HashMap::new(), 0).is_err());
    }

    #[test]
    fn mailchimp_metadata_fills_account_and_keeps_listed_fields() {
        let mut t = parse_tokens(
            &MAILCHIMP,
            r#"{"access_token":"a","expires_in":0}"#,
            &HashMap::new(),
            0,
        )
        .unwrap();
        apply_metadata(&MAILCHIMP, &mut t, r#"{"dc":"us21","api_endpoint":"https://us21.api.mailchimp.com","user_id":12345,"login":{"email":"x@y.z"}}"#).unwrap();
        assert_eq!(t.account_id.as_deref(), Some("12345"));
        let m = t.metadata.unwrap();
        assert_eq!(m["dc"], "us21");
        assert!(m.get("login").is_none());
    }

    #[test]
    fn credential_shape_rules() {
        let creds = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        assert!(validate_credential_shape(&["apiKey"], &creds(&[("apiKey", "k1")])).is_ok());
        assert!(validate_credential_shape(&["apiKey"], &creds(&[])).is_err());
        assert!(
            validate_credential_shape(&["apiKey"], &creds(&[("apiKey", "k1"), ("x", "y")]))
                .is_err()
        );
        assert!(validate_credential_shape(&["apiKey"], &creds(&[("apiKey", "a b")])).is_err());
        assert!(check_known_prefix(&STRIPE_KEY, &creds(&[("secretKey", "pk_live_1")])).is_err());
        assert!(check_known_prefix(&STRIPE_KEY, &creds(&[("secretKey", "rk_live_1")])).is_ok());
        assert!(
            check_known_prefix(&TOAST_KEY, &creds(&[("restaurantGuid", "not-a-guid")])).is_err()
        );
        assert!(check_known_prefix(
            &TOAST_KEY,
            &creds(&[("restaurantGuid", "0a1b2c3d-0000-4000-8000-123456789abc")])
        )
        .is_ok());
    }
}
