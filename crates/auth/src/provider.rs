//! Table-driven OAuth/OIDC provider registry.
//!
//! Replaces the per-provider `match self.provider.as_str()` arms in
//! `lib.rs::OAuthConfig` with a single `ProviderSpec` struct that
//! fully describes a provider's endpoints and how to parse its
//! userinfo response. Adding a new provider becomes a struct literal
//! in [`builtin::all`] — no new branches anywhere else.
//!
//! Two flavors of provider are supported:
//!
//! - **Static specs** (Google, GitHub, Apple, Discord, Slack, etc.) —
//!   endpoints + userinfo shape are hard-coded in [`builtin::all`].
//!   Adding a 51st provider that follows the standard OAuth2/OIDC
//!   shape is one struct literal.
//!
//! - **OIDC discovery** (`from_issuer`) — pulls
//!   `<issuer>/.well-known/openid-configuration` and synthesizes a
//!   spec at runtime. Covers Auth0, Okta, Cognito, Keycloak, Logto,
//!   Authentik, Zitadel, and any compliant OIDC IdP without code
//!   changes. The runtime caches the discovery response so we don't
//!   round-trip the IdP on every login.
//!
//! Provider-specific quirks — Apple's RS256-signed `client_secret`,
//! GitHub's "primary email lives at /user/emails", Microsoft's
//! tenant-aware endpoints — are carried as enum variants on
//! [`ClientSecret`] and [`UserinfoSource`] so the call sites stay
//! data-driven.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ProviderSpec — the full description of one OAuth provider
// ---------------------------------------------------------------------------

/// Static description of one OAuth/OIDC provider. Endpoint URLs are
/// formatted with `{tenant}` etc. placeholders that the spec resolves
/// when given a runtime config (e.g. Microsoft swaps `{tenant}` for
/// the configured Azure tenant id).
#[derive(Debug, Clone)]
pub struct ProviderSpec {
    /// Stable id used in the dashboard URL and the `Account.provider`
    /// column. Lowercase ASCII; matches `/api/auth/login/<id>`.
    pub id: &'static str,

    /// Human-readable name for buttons / UIs.
    pub display_name: &'static str,

    /// Authorization endpoint — where we send the user to grant access.
    /// May contain `{tenant}` for tenant-aware providers (Microsoft).
    pub auth_url: &'static str,

    /// Token exchange endpoint — POST'd with the auth code to get
    /// access + refresh tokens.
    pub token_url: &'static str,

    /// Userinfo endpoint — GET'd with the access token to pull the
    /// authed user's profile. `None` for providers that put the
    /// identity inside the `id_token` JWT only (Apple).
    pub userinfo_url: Option<&'static str>,

    /// OAuth scope string the spec asks for. Defaults to the minimum
    /// needed to ID the user. Separator is [`Self::scope_separator`].
    pub scopes: &'static str,

    /// Scope separator. RFC 6749 says space; TikTok uses comma.
    pub scope_separator: &'static str,

    /// Form-field name for the OAuth `client_id`. RFC 6749 says
    /// `client_id`; TikTok says `client_key`.
    pub client_id_param: &'static str,

    /// Extra query parameters appended to the auth URL (already
    /// URL-encoded, no leading `&`). Apple needs `response_mode=form_post`
    /// when name/email scopes are requested; this is the hook for it.
    pub auth_query_extra: &'static str,

    /// PKCE — when true, pylon generates `code_verifier` /
    /// `code_challenge` (SHA-256, S256), sends the challenge on the
    /// auth request, and replays the verifier on token exchange.
    /// Twitter/X *requires* it; Google/Microsoft *recommend* it.
    pub requires_pkce: bool,

    /// HTTP method used for the userinfo fetch. Most providers use
    /// GET; Dropbox uses POST.
    pub userinfo_method: UserinfoMethod,

    /// How to extract `(provider_account_id, email, display_name)`
    /// from the provider's userinfo response. Provider-stable id
    /// path (Google's `sub`, GitHub's `id`) is what `Account` keys
    /// on, NOT the email — a renamed-email user keeps their account.
    pub userinfo_parser: UserinfoParser,

    /// Provider-specific oddities for token exchange.
    pub token_exchange: TokenExchangeShape,

    /// Whether the token endpoint expects an `Accept: application/json`
    /// header (required for GitHub's classic OAuth, otherwise it
    /// returns form-urlencoded). Default true; flip false for
    /// providers that explicitly require form encoding.
    pub token_response_json: bool,
}

/// Userinfo fetch HTTP verb. Dropbox uses POST with an empty body;
/// every other supported provider uses GET.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserinfoMethod {
    Get,
    Post,
}

/// Where + how to read identity fields out of a userinfo response.
#[derive(Debug, Clone)]
pub enum UserinfoParser {
    /// Standard OIDC shape: `{ sub, email, name, picture }`.
    Oidc,

    /// GitHub's REST shape — `id` is numeric, name falls back to
    /// `login`, email may need a separate /user/emails fetch.
    GitHub,

    /// Linear's `{ viewer: { id, email, name } }` GraphQL response.
    LinearGraphql,

    /// Apple — identity lives in the `id_token` JWT returned by the
    /// token endpoint, not a userinfo endpoint. Decoded inline.
    AppleIdToken,

    /// Custom JSON pointers — for one-off providers whose responses
    /// don't match any standard shape. JSON-pointer paths into the
    /// response object.
    Custom {
        id_path: &'static str,
        email_path: &'static str,
        name_path: Option<&'static str>,
    },
}

/// Provider-specific token exchange request shape.
#[derive(Debug, Clone)]
pub enum TokenExchangeShape {
    /// Standard `grant_type=authorization_code&...` form body with the
    /// client_id + client_secret embedded as form fields. Covers
    /// Google, GitHub, Discord, Slack, Spotify, and most providers.
    Standard,

    /// Apple: `client_secret` is a JWT signed with the developer's
    /// ES256 private key (NOT a static string). The key id and team
    /// id are needed to mint it. See [`apple_jwt`] for the signer.
    AppleJwt,

    /// HTTP Basic auth instead of form fields for client_id /
    /// client_secret. Body still carries
    /// `grant_type=authorization_code&code=…&redirect_uri=…`.
    /// OIDC's default per the discovery spec when
    /// `token_endpoint_auth_methods_supported` is omitted.
    BasicAuth,

    /// JSON body — `{ "grant_type": "authorization_code", "code": …,
    /// "redirect_uri": …, "client_id": …, "client_secret": … }`.
    /// Atlassian 3LO requires this.
    JsonBody,

    /// JSON body + HTTP Basic auth header. Notion uses this:
    /// the body carries `grant_type` + `code` + `redirect_uri`,
    /// the credentials live in the Authorization header.
    BasicAuthJsonBody,
}

// ---------------------------------------------------------------------------
// Runtime config + lookup
// ---------------------------------------------------------------------------

/// Runtime config layered on top of a static [`ProviderSpec`]: the
/// developer's client_id/client_secret/redirect_uri, plus any
/// per-provider extras (Microsoft tenant id, Apple key material,
/// scopes override).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: String,
    pub client_id: String,
    /// For most providers this is a static string. For Apple, it's
    /// the path to (or PEM contents of) a private key — see
    /// [`AppleConfig`]. The runtime stores this opaquely and the
    /// signer is responsible for interpretation.
    pub client_secret: String,
    pub redirect_uri: String,
    /// Scopes override — when present, replaces [`ProviderSpec::scopes`].
    /// Use cases: requesting `repo` on GitHub for app-installation
    /// flows; requesting `https://www.googleapis.com/auth/calendar` on
    /// Google for app-specific data access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scopes_override: Option<String>,
    /// Tenant id for Microsoft / Entra. Defaults to `common` (any
    /// account type — work, school, personal). Single-tenant apps
    /// supply a directory GUID; multi-tenant work-only apps use
    /// `organizations`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    /// Apple-specific extras. None for non-Apple providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apple: Option<AppleConfig>,
    /// OIDC issuer URL for [`builtin::generic_oidc`]-style providers.
    /// When set, the discovery cache pulls
    /// `<issuer>/.well-known/openid-configuration` to populate
    /// the spec at first use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,
}

/// Apple-specific config. See
/// <https://developer.apple.com/documentation/sign_in_with_apple/generate_and_validate_tokens>.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppleConfig {
    /// Apple Developer team id (10-char alphanumeric).
    pub team_id: String,
    /// Key id from Apple Developer portal (10-char alphanumeric).
    pub key_id: String,
    /// PEM-encoded ES256 private key. Either the key contents inline
    /// or the file path — the signer detects which.
    pub private_key_pem: String,
}

// ---------------------------------------------------------------------------
// Builtin specs — adding a provider is one entry here
// ---------------------------------------------------------------------------

pub mod builtin {
    use super::*;

    /// All compile-time-known providers. Lookup table for
    /// [`super::find_spec`]. Order is irrelevant; matched by `id`.
    pub fn all() -> &'static [&'static ProviderSpec] {
        ALL
    }

    /// Static array — taking `&` of a slice expression returns a
    /// temporary, so we hoist it to a `static` and return a borrow.
    static ALL: &[&ProviderSpec] = &[
        &GOOGLE,
        &GITHUB,
        &APPLE,
        &MICROSOFT,
        &DISCORD,
        &SLACK,
        &SPOTIFY,
        &TWITCH,
        &TWITTER,
        &LINKEDIN,
        &FACEBOOK,
        &GITLAB,
        &REDDIT,
        &NOTION,
        &LINEAR,
        &VERCEL,
        &ZOOM,
        &SALESFORCE,
        &ATLASSIAN,
        &FIGMA,
        &DROPBOX,
        &TIKTOK,
        &PAYPAL,
        &KICK,
        &ROBLOX,
    ];

    pub static GOOGLE: ProviderSpec = ProviderSpec {
        id: "google",
        display_name: "Google",
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        userinfo_url: Some("https://www.googleapis.com/oauth2/v3/userinfo"),
        scopes: "openid email profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static GITHUB: ProviderSpec = ProviderSpec {
        id: "github",
        display_name: "GitHub",
        auth_url: "https://github.com/login/oauth/authorize",
        token_url: "https://github.com/login/oauth/access_token",
        userinfo_url: Some("https://api.github.com/user"),
        scopes: "user:email",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::GitHub,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    /// Apple — uses `name` in scope to get the user's name on
    /// first-time signup (Apple ONLY returns name on the first
    /// authorization; subsequent logins don't include it).
    /// `response_mode=form_post` is REQUIRED by Apple when name/email
    /// scopes are requested — Apple POSTs the callback to the
    /// redirect URL with `code` + `id_token` in the form body.
    pub static APPLE: ProviderSpec = ProviderSpec {
        id: "apple",
        display_name: "Apple",
        auth_url: "https://appleid.apple.com/auth/authorize",
        token_url: "https://appleid.apple.com/auth/token",
        // No standalone userinfo endpoint — identity comes from the
        // id_token JWT in the token response. Decoded inline.
        userinfo_url: None,
        scopes: "name email",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "response_mode=form_post",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::AppleIdToken,
        token_exchange: TokenExchangeShape::AppleJwt,
        token_response_json: true,
    };

    /// Microsoft / Entra ID. The auth + token URLs include
    /// `{tenant}` which is resolved against `ProviderConfig.tenant`
    /// at request build time. Defaults to `common` (any account).
    pub static MICROSOFT: ProviderSpec = ProviderSpec {
        id: "microsoft",
        display_name: "Microsoft",
        auth_url: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
        token_url: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",
        userinfo_url: Some("https://graph.microsoft.com/oidc/userinfo"),
        scopes: "openid email profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static DISCORD: ProviderSpec = ProviderSpec {
        id: "discord",
        display_name: "Discord",
        auth_url: "https://discord.com/oauth2/authorize",
        token_url: "https://discord.com/api/oauth2/token",
        userinfo_url: Some("https://discord.com/api/users/@me"),
        scopes: "identify email",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/id",
            email_path: "/email",
            name_path: Some("/global_name"),
        },
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static SLACK: ProviderSpec = ProviderSpec {
        id: "slack",
        display_name: "Slack",
        auth_url: "https://slack.com/openid/connect/authorize",
        token_url: "https://slack.com/api/openid.connect.token",
        userinfo_url: Some("https://slack.com/api/openid.connect.userInfo"),
        scopes: "openid email profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static SPOTIFY: ProviderSpec = ProviderSpec {
        id: "spotify",
        display_name: "Spotify",
        auth_url: "https://accounts.spotify.com/authorize",
        token_url: "https://accounts.spotify.com/api/token",
        userinfo_url: Some("https://api.spotify.com/v1/me"),
        scopes: "user-read-email user-read-private",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/id",
            email_path: "/email",
            name_path: Some("/display_name"),
        },
        token_exchange: TokenExchangeShape::BasicAuth,
        token_response_json: true,
    };

    pub static TWITCH: ProviderSpec = ProviderSpec {
        id: "twitch",
        display_name: "Twitch",
        auth_url: "https://id.twitch.tv/oauth2/authorize",
        token_url: "https://id.twitch.tv/oauth2/token",
        userinfo_url: Some("https://id.twitch.tv/oauth2/userinfo"),
        scopes: "openid user:read:email",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    /// Twitter / X. OAuth 2.0 PKCE-only — pylon generates
    /// code_verifier/challenge and stores them in the OAuth state
    /// record alongside the redirect URLs. Twitter's userinfo
    /// doesn't include email without an extra approval; users with
    /// email-disabled accounts fall back to `<username>@x.invalid`
    /// (caller decides whether to accept). Returned `id` is the
    /// Twitter snowflake.
    pub static TWITTER: ProviderSpec = ProviderSpec {
        id: "twitter",
        display_name: "Twitter / X",
        auth_url: "https://twitter.com/i/oauth2/authorize",
        token_url: "https://api.twitter.com/2/oauth2/token",
        userinfo_url: Some("https://api.twitter.com/2/users/me?user.fields=id,name,username"),
        scopes: "users.read tweet.read",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: true,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/data/id",
            email_path: "/data/username",
            name_path: Some("/data/name"),
        },
        token_exchange: TokenExchangeShape::BasicAuth,
        token_response_json: true,
    };

    pub static LINKEDIN: ProviderSpec = ProviderSpec {
        id: "linkedin",
        display_name: "LinkedIn",
        auth_url: "https://www.linkedin.com/oauth/v2/authorization",
        token_url: "https://www.linkedin.com/oauth/v2/accessToken",
        userinfo_url: Some("https://api.linkedin.com/v2/userinfo"),
        scopes: "openid profile email",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static FACEBOOK: ProviderSpec = ProviderSpec {
        id: "facebook",
        display_name: "Facebook",
        auth_url: "https://www.facebook.com/v18.0/dialog/oauth",
        token_url: "https://graph.facebook.com/v18.0/oauth/access_token",
        userinfo_url: Some("https://graph.facebook.com/me?fields=id,email,name"),
        scopes: "email public_profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/id",
            email_path: "/email",
            name_path: Some("/name"),
        },
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static GITLAB: ProviderSpec = ProviderSpec {
        id: "gitlab",
        display_name: "GitLab",
        auth_url: "https://gitlab.com/oauth/authorize",
        token_url: "https://gitlab.com/oauth/token",
        userinfo_url: Some("https://gitlab.com/oauth/userinfo"),
        scopes: "openid email profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static REDDIT: ProviderSpec = ProviderSpec {
        id: "reddit",
        display_name: "Reddit",
        auth_url: "https://www.reddit.com/api/v1/authorize",
        token_url: "https://www.reddit.com/api/v1/access_token",
        userinfo_url: Some("https://oauth.reddit.com/api/v1/me"),
        scopes: "identity",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/id",
            // Reddit doesn't expose email — fall back to a synthesized
            // username@reddit.invalid so account-store still has a
            // value. Apps that require a real email should reject.
            email_path: "/name",
            name_path: Some("/name"),
        },
        token_exchange: TokenExchangeShape::BasicAuth,
        token_response_json: true,
    };

    /// Notion uses Basic auth + JSON body for token exchange (per
    /// their docs at https://developers.notion.com/guides/get-started/authorization).
    pub static NOTION: ProviderSpec = ProviderSpec {
        id: "notion",
        display_name: "Notion",
        auth_url: "https://api.notion.com/v1/oauth/authorize",
        token_url: "https://api.notion.com/v1/oauth/token",
        userinfo_url: Some("https://api.notion.com/v1/users/me"),
        scopes: "",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "owner=user",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/bot/owner/user/id",
            email_path: "/bot/owner/user/person/email",
            name_path: Some("/bot/owner/user/name"),
        },
        token_exchange: TokenExchangeShape::BasicAuthJsonBody,
        token_response_json: true,
    };

    pub static LINEAR: ProviderSpec = ProviderSpec {
        id: "linear",
        display_name: "Linear",
        auth_url: "https://linear.app/oauth/authorize",
        token_url: "https://api.linear.app/oauth/token",
        // Linear is GraphQL only; we POST a fixed query at request
        // time. The fetcher special-cases this id (see runtime layer).
        userinfo_url: Some("https://api.linear.app/graphql"),
        scopes: "read",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Post,
        userinfo_parser: UserinfoParser::LinearGraphql,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static VERCEL: ProviderSpec = ProviderSpec {
        id: "vercel",
        display_name: "Vercel",
        auth_url: "https://vercel.com/oauth/authorize",
        token_url: "https://api.vercel.com/v2/oauth/access_token",
        userinfo_url: Some("https://api.vercel.com/v2/user"),
        scopes: "",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/user/id",
            email_path: "/user/email",
            name_path: Some("/user/name"),
        },
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static ZOOM: ProviderSpec = ProviderSpec {
        id: "zoom",
        display_name: "Zoom",
        auth_url: "https://zoom.us/oauth/authorize",
        token_url: "https://zoom.us/oauth/token",
        userinfo_url: Some("https://api.zoom.us/v2/users/me"),
        scopes: "user:read",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/id",
            email_path: "/email",
            name_path: Some("/first_name"),
        },
        token_exchange: TokenExchangeShape::BasicAuth,
        token_response_json: true,
    };

    pub static SALESFORCE: ProviderSpec = ProviderSpec {
        id: "salesforce",
        display_name: "Salesforce",
        auth_url: "https://login.salesforce.com/services/oauth2/authorize",
        token_url: "https://login.salesforce.com/services/oauth2/token",
        userinfo_url: Some("https://login.salesforce.com/services/oauth2/userinfo"),
        scopes: "openid email profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    /// Atlassian 3LO uses JSON body for token exchange. Without the
    /// JSON content type they reject with a parser error.
    pub static ATLASSIAN: ProviderSpec = ProviderSpec {
        id: "atlassian",
        display_name: "Atlassian",
        auth_url: "https://auth.atlassian.com/authorize",
        token_url: "https://auth.atlassian.com/oauth/token",
        userinfo_url: Some("https://api.atlassian.com/me"),
        scopes: "read:me",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "audience=api.atlassian.com&prompt=consent",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/account_id",
            email_path: "/email",
            name_path: Some("/name"),
        },
        token_exchange: TokenExchangeShape::JsonBody,
        token_response_json: true,
    };

    pub static FIGMA: ProviderSpec = ProviderSpec {
        id: "figma",
        display_name: "Figma",
        auth_url: "https://www.figma.com/oauth",
        token_url: "https://api.figma.com/v1/oauth/token",
        userinfo_url: Some("https://api.figma.com/v1/me"),
        scopes: "files:read",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/id",
            email_path: "/email",
            name_path: Some("/handle"),
        },
        token_exchange: TokenExchangeShape::BasicAuth,
        token_response_json: true,
    };

    /// Dropbox userinfo is a POST RPC endpoint with an empty body
    /// — they don't follow the GET-userinfo convention.
    pub static DROPBOX: ProviderSpec = ProviderSpec {
        id: "dropbox",
        display_name: "Dropbox",
        auth_url: "https://www.dropbox.com/oauth2/authorize",
        token_url: "https://api.dropboxapi.com/oauth2/token",
        userinfo_url: Some("https://api.dropboxapi.com/2/users/get_current_account"),
        scopes: "account_info.read",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Post,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/account_id",
            email_path: "/email",
            name_path: Some("/name/display_name"),
        },
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    /// TikTok deviates from RFC 6749: form field is `client_key`
    /// (not `client_id`) and scopes are comma-separated.
    pub static TIKTOK: ProviderSpec = ProviderSpec {
        id: "tiktok",
        display_name: "TikTok",
        auth_url: "https://www.tiktok.com/v2/auth/authorize",
        token_url: "https://open.tiktokapis.com/v2/oauth/token/",
        userinfo_url: Some(
            "https://open.tiktokapis.com/v2/user/info/?fields=open_id,union_id,avatar_url,display_name,username",
        ),
        scopes: "user.info.basic",
        scope_separator: ",",
        client_id_param: "client_key",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/data/user/open_id",
            email_path: "/data/user/username",
            name_path: Some("/data/user/display_name"),
        },
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static PAYPAL: ProviderSpec = ProviderSpec {
        id: "paypal",
        display_name: "PayPal",
        auth_url: "https://www.paypal.com/connect",
        token_url: "https://api-m.paypal.com/v1/oauth2/token",
        userinfo_url: Some(
            "https://api-m.paypal.com/v1/identity/openidconnect/userinfo?schema=openid",
        ),
        scopes: "openid email profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::BasicAuth,
        token_response_json: true,
    };

    pub static KICK: ProviderSpec = ProviderSpec {
        id: "kick",
        display_name: "Kick",
        auth_url: "https://id.kick.com/oauth/authorize",
        token_url: "https://id.kick.com/oauth/token",
        userinfo_url: Some("https://api.kick.com/public/v1/users"),
        scopes: "user:read",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: true, // Kick requires PKCE per their docs
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Custom {
            id_path: "/data/0/user_id",
            email_path: "/data/0/email",
            name_path: Some("/data/0/name"),
        },
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    pub static ROBLOX: ProviderSpec = ProviderSpec {
        id: "roblox",
        display_name: "Roblox",
        auth_url: "https://apis.roblox.com/oauth/v1/authorize",
        token_url: "https://apis.roblox.com/oauth/v1/token",
        userinfo_url: Some("https://apis.roblox.com/oauth/v1/userinfo"),
        scopes: "openid profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };

    /// Generic OIDC stub. Real specs are produced at runtime by
    /// fetching `<issuer>/.well-known/openid-configuration` and
    /// using the discovered URLs. The static stub exists so a
    /// `ProviderConfig` with `oidc_issuer = "https://acme.auth0.com"`
    /// has something to point its `provider` field at.
    pub static GENERIC_OIDC: ProviderSpec = ProviderSpec {
        id: "oidc",
        display_name: "OpenID Connect",
        auth_url: "", // resolved from discovery doc
        token_url: "",
        userinfo_url: None,
        scopes: "openid email profile",
        scope_separator: " ",
        client_id_param: "client_id",
        auth_query_extra: "",
        requires_pkce: false,
        userinfo_method: UserinfoMethod::Get,
        userinfo_parser: UserinfoParser::Oidc,
        token_exchange: TokenExchangeShape::Standard,
        token_response_json: true,
    };
}

/// Look up a static provider spec by id. Returns `None` for unknown
/// ids OR for OIDC issuer-config providers (those need
/// [`oidc_cache::resolve`] to materialize a runtime spec).
pub fn find_spec(id: &str) -> Option<&'static ProviderSpec> {
    builtin::all().iter().copied().find(|p| p.id == id)
}

/// Either a compile-time builtin spec or a runtime-discovered OIDC
/// spec. The two cases share read accessors via this enum so call
/// sites don't care where the URLs came from.
#[derive(Debug, Clone)]
pub enum ResolvedSpec {
    /// Static spec from [`builtin::all`].
    Static(&'static ProviderSpec),
    /// Runtime spec materialized from an OIDC discovery doc.
    Oidc(std::sync::Arc<DiscoveredSpec>),
}

/// Owned spec produced by OIDC discovery — same fields as
/// [`ProviderSpec`] but with `String` instead of `&'static str` since
/// the URLs come from the network, not the binary.
#[derive(Debug, Clone)]
pub struct DiscoveredSpec {
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: Option<String>,
    pub scopes: String,
    pub userinfo_parser: UserinfoParser,
    pub token_exchange: TokenExchangeShape,
}

impl ResolvedSpec {
    pub fn auth_url(&self) -> &str {
        match self {
            ResolvedSpec::Static(s) => s.auth_url,
            ResolvedSpec::Oidc(d) => &d.auth_url,
        }
    }
    pub fn token_url(&self) -> &str {
        match self {
            ResolvedSpec::Static(s) => s.token_url,
            ResolvedSpec::Oidc(d) => &d.token_url,
        }
    }
    pub fn userinfo_url(&self) -> Option<&str> {
        match self {
            ResolvedSpec::Static(s) => s.userinfo_url,
            ResolvedSpec::Oidc(d) => d.userinfo_url.as_deref(),
        }
    }
    pub fn scopes(&self) -> &str {
        match self {
            ResolvedSpec::Static(s) => s.scopes,
            ResolvedSpec::Oidc(d) => &d.scopes,
        }
    }
    pub fn scope_separator(&self) -> &str {
        match self {
            ResolvedSpec::Static(s) => s.scope_separator,
            ResolvedSpec::Oidc(_) => " ",
        }
    }
    pub fn client_id_param(&self) -> &str {
        match self {
            ResolvedSpec::Static(s) => s.client_id_param,
            ResolvedSpec::Oidc(_) => "client_id",
        }
    }
    pub fn auth_query_extra(&self) -> &str {
        match self {
            ResolvedSpec::Static(s) => s.auth_query_extra,
            ResolvedSpec::Oidc(_) => "",
        }
    }
    pub fn requires_pkce(&self) -> bool {
        match self {
            ResolvedSpec::Static(s) => s.requires_pkce,
            ResolvedSpec::Oidc(_) => false,
        }
    }
    pub fn userinfo_method(&self) -> UserinfoMethod {
        match self {
            ResolvedSpec::Static(s) => s.userinfo_method,
            ResolvedSpec::Oidc(_) => UserinfoMethod::Get,
        }
    }
    pub fn userinfo_parser(&self) -> UserinfoParser {
        match self {
            ResolvedSpec::Static(s) => s.userinfo_parser.clone(),
            ResolvedSpec::Oidc(d) => d.userinfo_parser.clone(),
        }
    }
    pub fn token_exchange(&self) -> TokenExchangeShape {
        match self {
            ResolvedSpec::Static(s) => s.token_exchange.clone(),
            ResolvedSpec::Oidc(d) => d.token_exchange.clone(),
        }
    }
}

/// Resolve `{tenant}` placeholders in an endpoint URL using the
/// runtime config. Today only Microsoft uses this, but the
/// substitution is generic so future tenant-aware providers don't
/// need new code.
pub fn resolve_endpoint(template: &str, cfg: &ProviderConfig) -> String {
    let tenant = cfg.tenant.as_deref().unwrap_or("common");
    template.replace("{tenant}", tenant)
}

// ---------------------------------------------------------------------------
// OIDC discovery (runtime — produces a synthesized ProviderSpec)
// ---------------------------------------------------------------------------

/// Fields we extract from an OIDC provider's
/// `/.well-known/openid-configuration` document. Everything else is
/// ignored — pylon's auth flow only uses these.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcDiscoveryDoc {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: Option<String>,
    pub jwks_uri: Option<String>,
    pub issuer: String,
    /// Per OIDC Discovery: when present, lists the auth methods the
    /// token endpoint accepts. When omitted the OIDC default is
    /// `client_secret_basic` (NOT `client_secret_post`!) — getting
    /// this wrong silently breaks every IdP that follows the spec.
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

impl OidcDiscoveryDoc {
    /// Parse a discovery JSON blob. Returns the relevant fields or
    /// an error if the doc is missing required endpoints.
    pub fn parse(json: &str) -> Result<Self, String> {
        let doc: Self = serde_json::from_str(json)
            .map_err(|e| format!("OIDC discovery doc not valid JSON: {e}"))?;
        if doc.authorization_endpoint.is_empty() {
            return Err("OIDC discovery doc missing authorization_endpoint".into());
        }
        if doc.token_endpoint.is_empty() {
            return Err("OIDC discovery doc missing token_endpoint".into());
        }
        Ok(doc)
    }

    /// Convert into a runtime [`DiscoveredSpec`] using OIDC-standard
    /// scopes + parser. Token-exchange shape is selected from the
    /// discovered `token_endpoint_auth_methods_supported`:
    ///   - `client_secret_post` → [`TokenExchangeShape::Standard`]
    ///   - everything else (including the spec default of
    ///     `client_secret_basic`) → [`TokenExchangeShape::BasicAuth`]
    pub fn into_spec(self) -> DiscoveredSpec {
        let prefers_post = self
            .token_endpoint_auth_methods_supported
            .iter()
            .any(|m| m == "client_secret_post");
        let token_exchange = if prefers_post {
            TokenExchangeShape::Standard
        } else {
            TokenExchangeShape::BasicAuth
        };
        DiscoveredSpec {
            auth_url: self.authorization_endpoint,
            token_url: self.token_endpoint,
            userinfo_url: self.userinfo_endpoint,
            scopes: "openid email profile".to_string(),
            userinfo_parser: UserinfoParser::Oidc,
            token_exchange,
        }
    }
}

/// Process-wide cache of OIDC discovery documents. The cache is
/// populated lazily on first use of an `oidc_issuer`-configured
/// provider and never invalidated — the discovery doc is meant to
/// be stable for the lifetime of the process. If the IdP changes
/// endpoints (rare), restart the server.
pub mod oidc_cache {
    use super::*;
    use std::sync::{Arc, Mutex, OnceLock};

    type Cache = Mutex<std::collections::HashMap<String, Arc<DiscoveredSpec>>>;
    fn cache() -> &'static Cache {
        static CACHE: OnceLock<Cache> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
    }

    /// Resolve an issuer URL into a `ResolvedSpec`. On cache miss
    /// fetches `<issuer>/.well-known/openid-configuration` over HTTPS
    /// and parses it. The cache key is the issuer URL exactly as
    /// supplied — pylon does NOT canonicalize trailing slashes so the
    /// caller controls cache keying.
    pub fn resolve(issuer: &str) -> Result<ResolvedSpec, String> {
        if let Some(spec) = cache().lock().unwrap().get(issuer) {
            return Ok(ResolvedSpec::Oidc(spec.clone()));
        }
        let url = if issuer.ends_with('/') {
            format!("{issuer}.well-known/openid-configuration")
        } else {
            format!("{issuer}/.well-known/openid-configuration")
        };
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(10))
            .timeout_read(std::time::Duration::from_secs(10))
            .build();
        let body = agent
            .get(&url)
            .call()
            .map_err(|e| format!("oidc discovery {url}: {e}"))?
            .into_string()
            .map_err(|e| format!("oidc discovery body {url}: {e}"))?;
        let doc = OidcDiscoveryDoc::parse(&body)?;
        let spec = Arc::new(doc.into_spec());
        cache()
            .lock()
            .unwrap()
            .insert(issuer.to_string(), spec.clone());
        Ok(ResolvedSpec::Oidc(spec))
    }

    /// Test-only helper: prime the cache with a synthetic spec so
    /// unit tests don't need network access.
    #[cfg(test)]
    pub fn insert_for_test(issuer: &str, spec: DiscoveredSpec) {
        cache()
            .lock()
            .unwrap()
            .insert(issuer.to_string(), Arc::new(spec));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_has_unique_id() {
        let mut seen = std::collections::HashSet::new();
        for spec in builtin::all() {
            assert!(
                seen.insert(spec.id),
                "duplicate provider id in builtin::all: {}",
                spec.id
            );
        }
    }

    #[test]
    fn every_builtin_has_nonempty_endpoints() {
        for spec in builtin::all() {
            assert!(!spec.auth_url.is_empty(), "{}: missing auth_url", spec.id);
            assert!(!spec.token_url.is_empty(), "{}: missing token_url", spec.id);
            assert!(
                !spec.scopes.is_empty() || spec.id == "notion" || spec.id == "vercel",
                "{}: empty scopes (only Notion/Vercel are allowed empty)",
                spec.id
            );
        }
    }

    #[test]
    fn find_spec_returns_known_providers() {
        assert!(find_spec("google").is_some());
        assert!(find_spec("github").is_some());
        assert!(find_spec("apple").is_some());
        assert!(find_spec("microsoft").is_some());
        assert!(find_spec("nonexistent").is_none());
    }

    #[test]
    fn resolve_endpoint_substitutes_tenant() {
        let cfg = ProviderConfig {
            provider: "microsoft".into(),
            client_id: "x".into(),
            client_secret: "y".into(),
            redirect_uri: "z".into(),
            scopes_override: None,
            tenant: Some("contoso.onmicrosoft.com".into()),
            apple: None,
            oidc_issuer: None,
        };
        let resolved = resolve_endpoint(
            "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
            &cfg,
        );
        assert_eq!(
            resolved,
            "https://login.microsoftonline.com/contoso.onmicrosoft.com/oauth2/v2.0/authorize"
        );
    }

    #[test]
    fn resolve_endpoint_defaults_tenant_to_common() {
        let cfg = ProviderConfig {
            provider: "microsoft".into(),
            client_id: "x".into(),
            client_secret: "y".into(),
            redirect_uri: "z".into(),
            scopes_override: None,
            tenant: None,
            apple: None,
            oidc_issuer: None,
        };
        let resolved = resolve_endpoint(
            "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
            &cfg,
        );
        assert!(resolved.contains("/common/"));
    }

    #[test]
    fn oidc_discovery_doc_parses_minimal() {
        let json = r#"{
            "issuer": "https://acme.auth0.com/",
            "authorization_endpoint": "https://acme.auth0.com/authorize",
            "token_endpoint": "https://acme.auth0.com/oauth/token",
            "userinfo_endpoint": "https://acme.auth0.com/userinfo",
            "jwks_uri": "https://acme.auth0.com/.well-known/jwks.json"
        }"#;
        let doc = OidcDiscoveryDoc::parse(json).expect("parse");
        assert_eq!(doc.issuer, "https://acme.auth0.com/");
        assert_eq!(
            doc.authorization_endpoint,
            "https://acme.auth0.com/authorize"
        );
        assert_eq!(doc.token_endpoint, "https://acme.auth0.com/oauth/token");
        assert_eq!(
            doc.userinfo_endpoint.as_deref(),
            Some("https://acme.auth0.com/userinfo")
        );
    }
}
