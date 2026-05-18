import type { NextConfig } from "next";

/**
 * Pylon's typed client + functions packages re-export across the
 * server/client boundary AND the workspace UI package ships TSX.
 * `transpilePackages` makes Next bundle them cleanly.
 *
 * `rewrites` proxies every Pylon-owned path (`/api/fn/*`,
 * `/api/auth/*`, `/api/sync/*`, …) to the Pylon binary running on
 * `PYLON_TARGET` (default http://localhost:4321). Without this,
 * Next.js sees `/api/fn/createWidget` as a missing route and 404s
 * before the request reaches Pylon.
 *
 * In production set `PYLON_TARGET` to wherever you've deployed the
 * Pylon binary (Fly, Render, Railway, your own box). The browser
 * still hits same-origin paths under your Next deployment, and Next
 * forwards them server-side — no CORS, no extra DNS.
 */
const PYLON_TARGET = process.env.PYLON_TARGET ?? "http://localhost:4321";

const config: NextConfig = {
	transpilePackages: [
		"@__APP_NAME_KEBAB__/ui",
		"@pylonsync/sdk",
		"@pylonsync/react",
		"@pylonsync/next",
		"@pylonsync/functions",
		"@pylonsync/sync",
	],
	async rewrites() {
		return [
			{ source: "/api/fn/:path*", destination: `${PYLON_TARGET}/api/fn/:path*` },
			{ source: "/api/auth/:path*", destination: `${PYLON_TARGET}/api/auth/:path*` },
			{ source: "/api/sync/:path*", destination: `${PYLON_TARGET}/api/sync/:path*` },
			{ source: "/api/:path*", destination: `${PYLON_TARGET}/api/:path*` },
		];
	},
};

export default config;
