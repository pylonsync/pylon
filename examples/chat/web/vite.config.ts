import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Pylon dev server runs on 4321 (HTTP) + 4322 (WS) + 4323 (SSE).
// Vite serves the React UI on 5173. No proxy needed — the client calls the
// pylon API directly via CORS (dev mode allows any origin).
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
  },
  // Workspace packages change frequently during demo dev. Tell Vite to
  // serve them from source on every change rather than pre-bundling.
  // Without this, edits to packages/sync/src/index.ts take a cache clear
  // to show up.
  optimizeDeps: {
    exclude: [
      "@pylonsync/sync",
      "@pylonsync/react",
      "@pylonsync/sdk",
      "@pylonsync/functions",
    ],
  },
});
