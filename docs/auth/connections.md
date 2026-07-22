# Pylon: `ctx.connections` for per-user OAuth integrations

`ctx.connections.*` lets server-side actions act on behalf of the
signed-in user against external services (Google Calendar, GitHub,
Slack, Notion, etc.). The framework handles the OAuth dance,
encrypted token storage, and silent refresh. Your handler
asks for a fresh access token and makes the API call.

> Distinct from the framework's "sign in with Google" flow. That
> one establishes the user's Pylon identity. `ctx.connections.*`
> links additional accounts after the user is already signed in.

## Quick start

```ts
// app.ts
import { defineConnection } from "@pylonsync/sdk";

export const googleCalendar = defineConnection({
  name: "google-calendar",
  provider: "google",
  scopes: "https://www.googleapis.com/auth/calendar.readonly",
});

export default {
  entities: [/* ... */],
  connections: [googleCalendar],
};
```

```bash
# .env
PYLON_PUBLIC_URL=https://app.example.com
PYLON_ENCRYPTION_KEY=$(openssl rand -hex 32)
PYLON_OAUTH_GOOGLE_CLIENT_ID=xxx.apps.googleusercontent.com
PYLON_OAUTH_GOOGLE_CLIENT_SECRET=yyy
```

```ts
// functions/listEvents.ts: read the user's calendar
import { action } from "@pylonsync/functions";

export default action({
  async handler(ctx) {
    const { accessToken } = await ctx.connections.get("google-calendar");
    const resp = await fetch(
      "https://www.googleapis.com/calendar/v3/calendars/primary/events?maxResults=10",
      { headers: { Authorization: `Bearer ${accessToken}` } },
    );
    return await resp.json();
  },
});

// functions/connectGoogle.ts: start the OAuth flow
import { action } from "@pylonsync/functions";

export default action({
  async handler(ctx) {
    const { url } = await ctx.connections.authorizeUrl("google-calendar", {
      postRedirect: "/dashboard?connected=google",
    });
    return { url };
  },
});
```

The browser navigates to the returned URL → user consents → Google
redirects to `/api/connections/google-calendar/callback?code=...&state=...`
→ Pylon stores the encrypted token pair → browser lands at
`postRedirect`.

## Why a primitive

The Pylon primitive provides:

- **One declaration** instead of three (auth URL builder, callback
  handler, refresh logic) per provider.
- **Encrypted token storage** automatically. The `_Connection`
  entity stores `accessToken` + `refreshToken` with
  `field.string().serverOnly().encrypted()`. SQL dump leak →
  attacker sees only ciphertext.
- **Silent refresh.** `ctx.connections.get(name)` checks expiry
  and refreshes via the provider's refresh-token grant when
  needed. Refresh-token rotation (Google, Auth0) is handled.
- **Per-user rate limit** on refresh (1/sec per user+connection)
  so a buggy handler can't batter the provider's refresh endpoint.
- **CSRF defense.** State tokens are single-use and
  expire in 10 minutes.

## Configuration

### Required env

| Env                                       | Purpose                                                |
| ----------------------------------------- | ------------------------------------------------------ |
| `PYLON_PUBLIC_URL`                        | Base URL for the callback (e.g. `https://app.com`).    |
| `PYLON_ENCRYPTION_KEY`                    | 32-byte AEAD key; refresh tokens must be encrypted.    |
| `PYLON_OAUTH_<PROVIDER>_CLIENT_ID`        | OAuth client id from the provider's developer console. |
| `PYLON_OAUTH_<PROVIDER>_CLIENT_SECRET`    | OAuth client secret.                                   |

`<PROVIDER>` is uppercased: `google` → `PYLON_OAUTH_GOOGLE_CLIENT_ID`.

### Optional env

| Env                                | Purpose                                                |
| ---------------------------------- | ------------------------------------------------------ |
| `PYLON_CONNECTIONS_CALLBACK_BASE`  | Override the callback base URL (otherwise PUBLIC_URL). |

### Built-in providers

The built-in OAuth client supports Google, GitHub, Slack,
Microsoft, Notion, GitLab, LinkedIn, Discord, Twitter/X, Apple,
Reddit, Spotify, Twitch, Auth0, Okta (via `oidc:` prefix), and
more. See `crates/auth/src/provider.rs` for the full list.

## API

### `defineConnection({ name, provider, scopes? })`

Returns a `ManifestConnection`. Pass via `buildManifest` or include in
your default-export config:

```ts
export default {
  connections: [
    defineConnection({ name: "google", provider: "google", scopes: "email" }),
    defineConnection({ name: "github", provider: "github", scopes: "repo" }),
  ],
};
```

Different connections can target the same provider with different
scopes (e.g. `google-mail` vs `google-calendar`).

### `ctx.connections.authorizeUrl(name, opts?)`

Returns `{ url: string }`. Redirect the browser to that URL.
`opts.postRedirect` (optional) is where the browser lands after
the OAuth callback succeeds; it defaults to `/`.

Throws on:

- `CONNECTIONS_NOT_CONFIGURED`: no `defineConnection(...)` in the manifest
- `CONNECTION_UNKNOWN`: `name` doesn't match any declared connection
- `PROVIDER_NOT_CONFIGURED`: missing `PYLON_OAUTH_<PROVIDER>_*` env
- `ENCRYPTION_REQUIRED`: `PYLON_ENCRYPTION_KEY` is unset
- `CONNECTION_REQUIRES_AUTH`: caller is anonymous

### `ctx.connections.get(name)`

Returns `{ accessToken: string; scope: string | null; expiresAt: number | null }`.

Behavior:

- Loads the stored `_Connection` row for `(ctx.auth.userId, name)`.
- If the access token expires within 60 seconds, the framework
  calls `exchange_refresh_token`, persists the new token pair
  (and the rotated refresh token, if the provider sent one),
  and returns the new access token.
- If there's no refresh token (some providers don't return one
  on second-link without `prompt=consent`), `get` throws
  `REFRESH_FAILED`; the user must re-link.

Throws on:

- `CONNECTION_NOT_LINKED`: user hasn't started the OAuth flow yet
- `REFRESH_FAILED`: provider rejected the refresh token
- `PROVIDER_NOT_CONFIGURED`: missing env

### `ctx.connections.list()`

Returns `{ connections: Array<{ name, provider, scope, expiresAt, updatedAt }> }`.
Tokens are not included. Call `get(name)` for those.

### `ctx.connections.disconnect(name)`

Removes the `_Connection` row. Provider-side revocation is the
caller's responsibility. Most providers expose a separate `/revoke`
endpoint that this surface intentionally doesn't call (revoke vs
unlink semantics differ per provider).

## HTTP surface

These two routes exist regardless of whether your app declares
connections. They return 503 with `CONNECTIONS_NOT_CONFIGURED` when no
`defineConnection(...)` exists.

### `POST /api/connections/<name>/auth-url`

Authed (cookie session). Body: `{ "post_redirect": "/dashboard" }` (optional).
Returns: `{ "url": "https://accounts.google.com/o/oauth2/v2/auth?..." }`.

### `GET /api/connections/<name>/callback?code=&state=`

Unauthed (OAuth provider hits this after consent). On success,
302-redirects to `post_redirect` (or `/` if none was supplied).
On failure, returns a JSON error envelope.

## Threat model + restrictions

- **Refresh tokens MUST be encrypted at rest.** The framework
  refuses to build an authorize URL when `PYLON_ENCRYPTION_KEY`
  is unset. A refresh token that lives in plaintext is a long-lived
  bearer credential for the user's external account.
- **CSRF defense via single-use state tokens.** Each `authorizeUrl`
  mints a 192-bit random state token, persists it for 10 minutes,
  consumes it on the callback. Replay attacks fail.
- **`accessToken` + `refreshToken` are `serverOnly`.** They never
  appear in HTTP responses, WS broadcasts, or sync push payloads.
  Only `ctx.connections.get` returns the plaintext, and only inside
  a server-side handler.
- **Per-user refresh rate limit (1/sec).** A buggy handler calling
  `ctx.connections.get` in a tight loop can't burn through the
  provider's refresh quota.
- **No queries against encrypted fields.** Random nonces mean two
  identical tokens write different ciphertext; `ctx.db.lookup` on
  `accessToken` always returns `null`. Use the `(userId, name)`
  unique index instead.

## Provider gotchas

- **Google**: the framework forces `prompt=consent&access_type=offline`
  so a refresh token is issued on every link. Without this, Google
  omits the refresh token on subsequent re-links and you can't
  refresh expired access tokens; users would have to fully re-consent.
- **GitHub**: classic personal-OAuth-app tokens don't expire and
  don't return refresh tokens. `ctx.connections.get` returns the
  stored access token forever (no refresh path). GitHub Apps + the
  newer GitHub OAuth flows DO return refresh tokens.
- **Slack**: returns a workspace-scoped token. The connection is
  per-(user, workspace), so multiple workspaces need multiple
  `defineConnection` entries.
- **Custom OIDC** (Auth0, Okta, Keycloak): not yet supported in
  `ctx.connections.*`. Use the framework's general OAuth surface
  for now.

## Errors

| Code                        | Where                       | Meaning                                                  |
| --------------------------- | --------------------------- | -------------------------------------------------------- |
| `CONNECTIONS_NOT_CONFIGURED`| All ops                     | No `defineConnection(...)` in the manifest.              |
| `CONNECTION_UNKNOWN`        | All ops                     | `name` doesn't match a declared connection.              |
| `CONNECTION_REQUIRES_AUTH`  | All ops                     | Caller is anonymous. Use `ctx.auth.elevate` to bypass.   |
| `CONNECTION_NOT_LINKED`     | `get`, `disconnect`         | User hasn't completed the OAuth flow for this connection.|
| `PROVIDER_NOT_CONFIGURED`   | All ops                     | `PYLON_OAUTH_<PROVIDER>_*` env is missing.               |
| `ENCRYPTION_REQUIRED`       | `authorize_url`             | `PYLON_ENCRYPTION_KEY` is unset.                         |
| `AUTH_FAILED`               | `callback`                  | State token invalid/expired, or provider rejected code.  |
| `REFRESH_FAILED`            | `get`                       | Provider rejected the refresh token (user must re-link). |

## Limits

- Call the provider SDK or `fetch("https://googleapis.com/...")` yourself;
  Pylon supplies the bearer token.
- Connections link users to external OAuth providers, not two Pylon apps.
- Query handlers cannot use `ctx.connections`, because reactive re-runs could
  loop the refresh and database write.
- A connection belongs to a `userId`, not a specific tenant. Multi-tenant apps
  that want per-tenant connections should
  include the tenant id in the connection `name` (e.g.
  `slack-workspace-${tenantId}`).
