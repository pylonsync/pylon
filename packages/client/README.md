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

- `<SignIn />` — magic link + password + OAuth, two-step code flow.
- `<SignUp />` — password registration with email validation.
- `<UserButton />` — avatar dropdown with sign-out.
- `<SignOutButton />` — bare sign-out trigger.
- `<SignedIn>` / `<SignedOut>` — render-gates.
- `<Protect admin>` / `<Protect predicate={...}>` — predicate gate.
- `useAuth()` — `{isSignedIn, userId, isAdmin, signOut, ...}`.

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
