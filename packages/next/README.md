# @pylonsync/next

Next.js 16 helpers for Pylon. Cookie-based auth, server-side data
loading, edge proxy gate, OAuth provider rendering — all designed
around App Router conventions.

## Install

```sh
bun add @pylonsync/next
# or: npm i @pylonsync/next
```

## Setup

### 1. Wire the API origin

Set `PYLON_TARGET` on your Next host (Vercel project, fly.toml, etc.)
to your Pylon control-plane origin.

```env
PYLON_TARGET=https://api.example.com
```

In dev `next dev` defaults to `http://localhost:4321` (the `pylon dev`
default port) so no env tweaking required.

### 2. Build a server helper

```ts
// src/lib/pylon.ts
import { createPylonServer } from "@pylonsync/next/server";

export const pylon = createPylonServer({
  // Pylon emits `${app_name}_session` — pass that exact name. There's
  // no "default" because the package can't know your app name; passing
  // it explicitly here also kills a class of silent-breakage bugs
  // where a wrong env var quietly breaks auth in production.
  cookieName: "myapp_session",
  // Optional: name of your "current user" function. Default "getMe".
  getMeFn: "getMe",
});
```

### 3. Define the `getMe` function on Pylon

```ts
// apps/control-plane/functions/getMe.ts
import { query } from "@pylonsync/functions";

export default query({
  args: {},
  async handler(ctx) {
    if (!ctx.auth.userId) return null;
    const user = await ctx.db.get("User", ctx.auth.userId);
    if (!user) return null;
    // Project to safe-to-display fields. Bypasses the User entity's
    // read policy (which typically denies everything to keep the
    // password hash from leaving the server).
    return {
      id: user.id,
      email: user.email,
      displayName: user.displayName,
    };
  },
});
```

### 4. Gate dashboard routes with a proxy

```ts
// src/proxy.ts
import { createPylonProxy } from "@pylonsync/next/proxy";

// Next 16 statically extracts `config.matcher` at build time — it has
// to be an inline literal. We pass the same array to createPylonProxy
// so the runtime matches; tsc catches drift.
export const config = {
  matcher: ["/dashboard/:path*", "/onboarding/:path*"],
};

export const proxy = createPylonProxy({
  cookieName: "myapp_session",
  matcher: ["/dashboard/:path*", "/onboarding/:path*"],
}).proxy;
```

### 5. Use it in pages

```tsx
// src/app/dashboard/layout.tsx
import { pylon } from "@/lib/pylon";
import type { User } from "@/lib/types";

export default async function DashboardLayout({ children }) {
  const me = await pylon.requireMe<User>();
  return <Chrome user={me.user}>{children}</Chrome>;
}
```

```tsx
// src/app/dashboard/page.tsx
import { redirect } from "next/navigation";
import { pylon } from "@/lib/pylon";

type Org = { id: string; name: string; slug: string };

export default async function DashboardPage() {
  const orgs = await pylon.json<Org[]>("/api/entities/Organization");
  if (orgs.length === 0) redirect("/onboarding");
  return <OrgsList orgs={orgs} />;
}
```

```tsx
// src/app/login/page.tsx — no client-mount flicker for OAuth row
import { pylon } from "@/lib/pylon";
import { LoginForm } from "./login-form"; // "use client"

export default async function LoginPage() {
  const providers = await pylon.getOAuthProviders();
  return <LoginForm providers={providers} />;
}
```

## Server helper API

`createPylonServer(config)` returns a `PylonServer` with:

| Method | Returns | Notes |
|---|---|---|
| `fetch(path, init?)` | `Response` | Forwards the session cookie. Caller handles status. |
| `json<T>(path, init?)` | `T` | Parses + status-checks. Throws `ApiError` on non-2xx. |
| `getAuth()` | `PylonAuth \| null` | userId, tenantId, isAdmin, cookieHeader. |
| `requireAuth()` | `PylonAuth` | Redirects to `loginUrl` on null. |
| `getMe<U>()` | `{auth, user: U} \| null` | Calls `/api/fn/${getMeFn}`. |
| `requireMe<U>()` | `{auth, user: U}` | Redirects on null. |
| `getOAuthProviders()` | `OAuthProvider[]` | Empty array on failure. |

## Client helpers

```ts
// src/lib/api.ts
import { createPylonClient } from "@pylonsync/next/client";
export const api = createPylonClient(); // same-origin via Next rewrite

// usage from a client component
const me = await api<Me>("/api/auth/me");
```

```tsx
// "use client" form
import { useAuthSubmit, loginWithPassword } from "@pylonsync/next/auth";

const { submit, error, busy } = useAuthSubmit(loginWithPassword);
```

`@pylonsync/next/auth` provides: `signupWithPassword`, `loginWithPassword`,
`logout`, `startOAuthLogin`, `verifyEmail`, `useOAuthProviders`,
`useAuthSubmit`.

## CORS / cookies across subdomains

Most apps deploy the dashboard at `app.example.com` and the Pylon
control plane at `api.example.com`. To make the session cookie visible
on both:

```sh
fly secrets set -a my-pylon-app \
  PYLON_DASHBOARD_URL=https://app.example.com \
  PYLON_COOKIE_DOMAIN=.example.com \
  PYLON_CORS_ORIGIN=https://app.example.com
```

The dashboard's `next.config.ts` should rewrite `/api/*` to the API
origin so the browser sees same-origin (no CORS preflights). The
package's server-side helpers hit `PYLON_TARGET` directly and skip
the rewrite.

## Versioning

Tracks the rest of `@pylonsync/*`. Pylon binary 0.2.x → package 0.2.x.

License: MIT OR Apache-2.0.
