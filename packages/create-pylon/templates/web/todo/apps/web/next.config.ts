import type { NextConfig } from "next";

// Backend origin. In dev defaults to the `pylon dev` default port.
// In production (Vercel etc.), set PYLON_TARGET on your project's env.
// Matches the env var @pylonsync/next reads on the server side — keeping
// them aligned means one env var sets the URL for both the rewrite and
// the server-side fetch helpers.
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
		// /api/* → ${PYLON_TARGET}/api/* keeps the browser same-origin
		// (no CORS preflight, session cookie rides natively).
		return [{ source: "/api/:path*", destination: `${PYLON_TARGET}/api/:path*` }];
	},
};

export default config;
