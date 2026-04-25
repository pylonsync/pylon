import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: { port: 5173 },
  // Force a single React + react-dom across the workspace. Without this,
  // hoisted shadcn primitives (which import React from the root
  // node_modules) and example app code (which imports React from the
  // example's local node_modules) end up with two parallel React
  // copies — duplicate context, broken hooks, and React elements from
  // one version look like opaque {$$typeof,...} objects to the other.
  resolve: {
    dedupe: ["react", "react-dom"],
  },
  optimizeDeps: {
    exclude: [
      "@pylonsync/sync",
      "@pylonsync/react",
      "@pylonsync/sdk",
      "@pylonsync/functions",
      // Loro ships WASM; let Vite serve it as an asset rather than
      // try to pre-bundle the .wasm import (which the dev pre-bundler
      // can't resolve through the workspace package layout).
      "loro-crdt",
      "@pylonsync/loro",
    ],
  },
  // Treat .wasm files as static assets so loro-crdt's `import wasm
  // from "./loro_wasm_bg.wasm?url"` resolves at build time.
  assetsInclude: ["**/*.wasm"],
});
