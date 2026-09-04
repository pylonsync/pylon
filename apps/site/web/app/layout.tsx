import React from "react";
import { ThemeProvider } from "@pylon-site/ui/components/theme-provider";

// Root layout for pylonsync.com. Pylon compiles Tailwind from
// web/app/globals.css and injects the <link>; ThemeProvider (next-themes) runs
// as a client island because the marketing nav carries a theme switcher.
//
// No auth and no Toaster: this app has no session and nothing that toasts.
//
// The control-plane layout still carries a `globalThis.process` head shim. It
// is not repeated here: the client bundler now statically replaces
// `process.env.NODE_ENV` and collapses every other `process.env.*`, so no
// `process` reference reaches the browser for the shim to rescue.
interface LayoutProps {
	children: React.ReactNode;
	url?: string;
}

// Routes whose page component exports its own metadata.title. For these the
// layout must NOT emit the fallback <title>: the SSR head renders the layout's
// tag first and the page's second, and browsers + crawlers use the FIRST
// <title> in the document. With the fallback present, every page would show
// the same generic title in the tab and in search results.
const ROUTE_OWNS_TITLE =
	/^\/(?:$|product(?:\/|$)|solutions(?:\/|$)|vs(?:\/|$)|skill(?:\/|$)|developers(?:\/|$)|about(?:\/|$)|contact(?:\/|$))/;

function pathnameOf(url?: string): string {
	if (!url) return "/";
	try {
		// `url` may arrive as a bare path or a full URL depending on caller.
		return url.startsWith("/") ? url.split("?")[0] : new URL(url).pathname;
	} catch {
		return "/";
	}
}

export default function RootLayout({ children, url }: LayoutProps) {
	const ownsTitle = ROUTE_OWNS_TITLE.test(pathnameOf(url));
	return (
		<html lang="en" suppressHydrationWarning>
			<head>
				<meta charSet="utf-8" />
				<meta name="viewport" content="width=device-width, initial-scale=1" />
				{/*
				  Fallback <title> for pages without their own metadata export
				  (the legal pages). Suppressed on routes that own their title —
				  see ROUTE_OWNS_TITLE. The description is deliberately NOT set
				  here: React doesn't dedupe <meta>, so a layout description plus a
				  page description would ship as two tags.
				*/}
				{!ownsTitle && (
					<title>Pylon — full-stack framework for coding agents</title>
				)}
				<link rel="icon" href="/brand/pylon-icon.svg" type="image/svg+xml" />
				{/*
				  Revtrail analytics, served FIRST-PARTY for ad-blocker resistance:
				  `/rt/track.js` is a vendored, byte-identical copy of
				  https://revtrail.pyln.dev/track.js (refresh with
				  `curl -fsS https://revtrail.pyln.dev/track.js -o
				  apps/pylonsync-site/public/rt/track.js`). The script derives its
				  beacon endpoint from its own origin, so it POSTs to this app's own
				  /api/fn/ingestEvent, which relays server-side to Revtrail. Same
				  site id as before the split, so the history stays continuous.
				  Cookieless daily identity — no cookie is set, so this stays out of
				  the consent surface and does not opt pages out of the SSR cache.
				*/}
				<script defer src="/rt/track.js" data-site="ef763fd3c6ef3a79" />
			</head>
			<body className="bg-background text-foreground antialiased">
				<ThemeProvider
					attribute="class"
					defaultTheme="light"
					enableSystem
					disableTransitionOnChange
				>
					{children}
				</ThemeProvider>
			</body>
		</html>
	);
}
