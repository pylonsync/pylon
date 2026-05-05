import type { NextConfig } from "next";

const PYLON_BASE_URL = process.env.PYLON_BASE_URL ?? "http://localhost:4321";

const config: NextConfig = {
  env: {
    NEXT_PUBLIC_PYLON_URL: PYLON_BASE_URL,
  },
  // Pylon's workspace packages are TypeScript source — Next compiles them.
  transpilePackages: [
    "@pylonsync/react",
    "@pylonsync/sync",
    "@pylonsync/sdk",
    "@pylonsync/functions",
    "@pylonsync/loro",
    "@pylonsync/example-ui",
  ],
  images: { unoptimized: true },
};

export default config;
