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
  // Enable WebAssembly module imports (needed by loro-crdt's bundler
  // entry which does `import * as wasm from './loro_wasm_bg.wasm'`).
  // Webpack 5 gates WASM behind an experiment flag; without it the
  // import errors out at build time.
  //
  // Why webpack and not Turbopack: Turbopack on Next 16 + Vercel
  // double-encodes the WASM URL when Vercel appends its `?dpl=` cache-
  // bust query, producing 404s on prod (the encoded `?` lands in the
  // chunk filename literally). Webpack emits a clean URL.
  webpack(config) {
    config.experiments = { ...config.experiments, asyncWebAssembly: true };
    return config;
  },
};

export default config;
