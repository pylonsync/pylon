import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import wasm from "vite-plugin-wasm";

// Pylon hosts this app from `localhost:4321` in dev (and in prod with
// CloudFlare in front). The Vite dev server still runs separately on
// a port Pylon assigns via env, and Pylon reverse-proxies non-API
// requests to it. The frontend never connects directly — every URL
// the user sees is `:4321`.
//
// `server.proxy` is still useful for direct `vite` runs (without
// Pylon) so the existing API routes work during isolated frontend
// development.
//
// `@pylonsync/*` workspace packages ship raw TypeScript source as
// their entry (`main: "src/index.ts"`) — there's no build step. Vite
// doesn't compile TS inside node_modules by default, so it would
// surface module-resolution errors at runtime. Force them into Vite's
// dep-optimizer via `optimizeDeps.include`; esbuild handles the TS
// transpile during pre-bundle. Same pattern @pylonsync/example-ui
// needs, plus loro-crdt (which uses sync XHR for WASM init and
// otherwise emits a "Synchronous XMLHttpRequest" console warning).
export default defineConfig({
  // wasm() lets Vite 7 handle loro-crdt's ESM `.wasm` import (Vite 7 no
  // longer transforms WebAssembly ESM imports out of the box). It emits
  // top-level await, which the `build.target: "esnext"` below keeps
  // natively — we used to need vite-plugin-top-level-await for that, but
  // that plugin crashes under Bun's `vite build` (the cloud runtime builds
  // the SPA with Bun): `virtualModule.require is not a function`.
  plugins: [wasm(), react(), tailwindcss()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: {
      "/api": "http://localhost:4321",
    },
  },
  optimizeDeps: {
    // Top-level workspace packages — Vite walks their import graph
    // and pre-bundles transitive deps (radix, clsx, tailwind-merge,
    // etc.) automatically. Subpath exports of `@pylonsync/example-ui`
    // (`/avatar`, `/button`, …) are listed via the parent include
    // and get pulled in via Vite's normal resolver; listing them
    // explicitly produces "Cannot optimize dependency" warnings.
    include: [
      "@pylonsync/client",
      "@pylonsync/react",
      "@pylonsync/sync",
      "@pylonsync/loro",
      "@pylonsync/example-ui",
    ],
    // loro-crdt ships a WASM blob that its own runtime loads via a
    // relative URL during `init()`. Vite's dep-optimizer rewrites the
    // module path during pre-bundling, which breaks that URL —
    // requests for the .wasm file land at the SPA fallback and the
    // browser tries to compile HTML as WebAssembly:
    //   CompileError: expected magic word 00 61 73 6d, found 3c 21 64 6f
    // (3c 21 64 6f === "<!do" === the start of index.html).
    // Excluding it from pre-bundling makes Vite serve the package's
    // own files as-is, so its WASM loader resolves correctly.
    exclude: ["loro-crdt"],
    esbuildOptions: {
      // example-ui ships .tsx — make sure esbuild picks the JSX
      // loader for these extensions when pre-bundling.
      loader: { ".ts": "tsx", ".tsx": "tsx" },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // loro-crdt + vite-plugin-wasm emit top-level await; esnext keeps it
    // natively so we don't need vite-plugin-top-level-await (which breaks
    // under Bun). All evergreen browsers support TLA.
    target: "esnext",
  },
});
