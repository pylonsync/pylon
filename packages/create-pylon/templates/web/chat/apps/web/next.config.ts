import type { NextConfig } from "next";

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
