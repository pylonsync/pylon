import type { NextConfig } from "next";

const PYLON_API_URL = process.env.PYLON_API_URL ?? "http://localhost:4321";

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
			{ source: "/api/fn/:path*", destination: `${PYLON_API_URL}/api/fn/:path*` },
			{ source: "/api/auth/:path*", destination: `${PYLON_API_URL}/api/auth/:path*` },
			{ source: "/api/sync/:path*", destination: `${PYLON_API_URL}/api/sync/:path*` },
			{ source: "/api/:path*", destination: `${PYLON_API_URL}/api/:path*` },
		];
	},
};

export default config;
