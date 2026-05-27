# @pylonsync/client

Drop-in React components for Pylon auth. Clerk-style API, wired to the
existing `/api/auth/*` surface — magic link, password, and OAuth all
work with zero config.

```bash
pnpm add @pylonsync/client
```

```tsx
import { init } from "@pylonsync/react";
import {
	SignIn,
	SignedIn,
	SignedOut,
	UserButton,
} from "@pylonsync/client";
import "@pylonsync/client/theme.css"; // optional default theme

init(); // once at app boot

export function App() {
	return (
		<header>
			<SignedIn>
				<UserButton showName afterSignOutUrl="/" />
			</SignedIn>
			<SignedOut>
				<SignIn afterSignInUrl="/dashboard" />
			</SignedOut>
		</header>
	);
}
```

## Components

**Auth surfaces**
- `<SignIn />` — magic link + password + OAuth, two-step code flow.
- `<SignUp />` — password registration with email validation.
- `<UserButton />` — avatar dropdown with sign-out.
- `<SignOutButton />` — bare sign-out trigger.
- `<UserProfile />` — account management: identity, password, sessions, API keys.

**Org surfaces**
- `<OrganizationSwitcher />` — list + switch + inline create.
- `<CreateOrganization />` — standalone create form.
- `<InviteMembers />` — invite by email, pending list, role mgmt.
- `<AcceptInvite token={...} />` — landing page for invite emails.

**Integrations**
- `<ConnectAccount name="slack" />` — start OAuth for a `defineConnection(...)`.

**Files**
- `<FileUpload />` — drag-and-drop, multi-file, server-side via `/api/files/upload`.

**Control components**
- `<SignedIn>` / `<SignedOut>` — auth render-gates.
- `<InOrg>` / `<NoOrg>` — active-org render-gates.
- `<HasRole role="owner">` — role render-gate (single or array).
- `<Protect admin>` / `<Protect predicate={...}>` — predicate gate.
- `<RedirectToSignIn signInUrl="/sign-in" />` — client redirect.

**Hooks**
- `useAuth()` — `{isSignedIn, userId, tenantId, isAdmin, session, signOut, ...}`.

## Orgs

Pylon ships org/membership endpoints (`/api/auth/orgs`) when your
manifest declares `Org` and `OrgMember` entities. `<OrganizationSwitcher />`
reads + writes through those endpoints — no app-side glue needed.

```tsx
<OrganizationSwitcher
	onSwitched={(orgId) => router.push("/")}
	onCreated={(org) => analytics.track("org_created", org)}
/>
```

## Theming

All styling references CSS variables namespaced under `--pylon-*`. Import
`@pylonsync/client/theme.css` for sensible light/dark defaults, or define
the variables yourself.

```css
:root {
	--pylon-ink: #1d4ed8;
	--pylon-paper: #f8fafc;
}
```

## OAuth providers

`<SignIn />` queries `/api/auth/providers` and renders a button per
enabled provider. Configure providers via env vars on the Pylon process
(e.g. `PYLON_OAUTH_GOOGLE_CLIENT_ID` / `PYLON_OAUTH_GOOGLE_CLIENT_SECRET`).
