# Auth: session tokens for native apps

Pylon's web flow is cookie-based — `/api/auth/password/login`,
`/api/auth/login/<provider>`, etc. set a `pylon_session` HTTP-only
cookie that proxies as identity for HTTP and the WebSocket
subprotocol bearer.

Native apps (desktop, mobile, CLI) can't use cookies — they need a
bearer token. Pylon ships **two** mint endpoints; pick the one that
matches your transport.

## Quick reference

| Endpoint | Use for | Works on HTTP? | Works on WebSocket? |
| --- | --- | --- | --- |
| `POST /api/auth/native-session` | Native apps with realtime sync | Yes | **Yes** |
| `POST /api/auth/jwt` | Edge runtimes, signed-claim consumers | Yes | No |

If your app uses `SyncEngine` / `useQuery` / `db.useQuery` (anything
that opens a WebSocket back to Pylon), **use
`/api/auth/native-session`**. JWTs from `/api/auth/jwt` aren't in
`SessionStore`, and the WS auth path resolves bearer tokens by
SessionStore lookup — so a JWT-only client passes HTTP fine but the
WebSocket reconnect-loops with `unauthorized: bearer token required`.

## `POST /api/auth/native-session`

Mints a fresh server-stored session token for the currently-
authenticated user.

**Auth**: cookie session must already be valid. No admin token
required (unlike `POST /api/auth/session`, which can mint a session
for ANY `user_id` and is therefore admin-gated). Here we mint a
session for the *same user* the cookie already represents — no
privilege escalation.

**Request**:

```http
POST /api/auth/native-session HTTP/1.1
Cookie: pylon_session=<existing cookie>
```

(No body — the user identity comes from the cookie.)

**Response**:

```json
HTTP/1.1 201 Created
Content-Type: application/json

{
  "token": "<random url-safe token>",
  "user_id": "<the cookie user's id>",
  "expires_at": 1735689600
}
```

**Errors**:

| Status | Code | When |
| --- | --- | --- |
| 401 | `AUTH_REQUIRED` | Cookie missing, expired, or anonymous |

The minted token has its own SessionStore row — revokable
independently of the browser cookie that minted it
(`POST /api/auth/logout` with the bearer drops it without affecting
the cookie session). Lifetime matches the framework's session
lifetime (default 14 days, override via `auth.session.expires_in` in
the manifest).

## Desktop handoff pattern

Recommended flow for "the user signs in via the browser, the desktop
app gets a token":

1. Native app generates a random `state` nonce + opens the system
   browser to `<your-web-host>/auth/desktop?state=<state>&return=<your-app>://auth/callback`.
2. Web `/auth/desktop` page checks the cookie. If signed-out, it
   redirects to `/login?next=<auth/desktop with the same state +
   return>`. After sign-in `/login` 302s back here.
3. Signed-in `/auth/desktop` POSTs `/api/auth/native-session`,
   gets back `{ token, user_id, expires_at }`, then redirects to
   `<your-app>://auth/callback?state=<state>&token=<token>`.
4. Native app receives the deep-link, validates `state` against what
   it generated in step 1, and calls
   `PylonClient.setSession(token: token)`. From then on every HTTP
   request carries the bearer header AND the WebSocket reconnect
   uses `Sec-WebSocket-Protocol: bearer.<urlencode(token)>`.

The token persists in the framework's session store, so killing
and relaunching the native app keeps the user signed in until the
session expires or they explicitly sign out.

## `POST /api/auth/jwt` (legacy / edge use case)

Same shape as `/api/auth/native-session` but mints an HS256-signed
JWT instead of a SessionStore entry. Use when:

- The consumer can't afford a SessionStore round-trip per request
  (Cloudflare Workers, edge functions where Pylon isn't co-located).
- You're integrating with an existing JWT-based system that wants
  signed claims to carry forward.

Requires `PYLON_JWT_SECRET` to be set on the server; returns 501
otherwise.

JWTs **don't work on the WebSocket path**. The WS handshake's bearer
subprotocol resolves through `SessionStore`, which doesn't know about
JWT-only tokens. If your app uses sync, use
`/api/auth/native-session` instead.

## SDK guidance

- **Swift (`PylonClient`)**: `client.setSession(token:)` works with
  either token type. The Mac and iOS Yapless app uses
  `/api/auth/native-session` because it depends on SyncEngine.
- **TypeScript (`@pylonsync/sdk`)**: pass the token as
  `client.setSession(token)` after the redirect.
- **CLI / scripts**: store the response token, use as
  `Authorization: Bearer <token>`.

## Revoking a token

`POST /api/auth/logout` with the bearer drops the SessionStore row.
The cookie session that originally minted it is unaffected.

```sh
curl -X POST https://your-host/api/auth/logout \
  -H "Authorization: Bearer <token>"
```

For bulk revocation (force every device to re-sign-in):
`POST /api/admin/users/:id/sessions/purge` with the admin token.
