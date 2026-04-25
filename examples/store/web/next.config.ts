import type { NextConfig } from "next";

const PYLON_BASE_URL = process.env.PYLON_BASE_URL ?? "http://localhost:4321";

const config: NextConfig = {
  // Server components fetch from Pylon over the public REST surface.
  // Expose the same URL to client components via NEXT_PUBLIC_*.
  env: {
    NEXT_PUBLIC_PYLON_URL: PYLON_BASE_URL,
  },

  // Pylon's workspace packages are TypeScript source — let Next compile
  // them rather than expecting prebuilt dist output.
  transpilePackages: [
    "@pylonsync/react",
    "@pylonsync/sync",
    "@pylonsync/sdk",
    "@pylonsync/functions",
    "@pylonsync/example-ui",
  ],

  // Disable image optimization for the demo; the gradient placeholders
  // we use don't need it.
  images: { unoptimized: true },

  experimental: {
    // Server actions size cap is fine at default; explicitly set so
    // the cart-clear path doesn't run into the 1MB default.
    serverActions: { bodySizeLimit: "1mb" },
  },
};

export default config;
