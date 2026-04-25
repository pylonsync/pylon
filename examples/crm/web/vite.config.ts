import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: { port: 5175 },
  // Force a single React + react-dom across the workspace to dodge the
  // duplicate-React crash where radix elements look like opaque objects
  // to the app's React copy. See chat/web/vite.config.ts for the full
  // story.
  resolve: {
    dedupe: ["react", "react-dom"],
  },
  optimizeDeps: {
    exclude: [
      "@pylonsync/sync",
      "@pylonsync/react",
      "@pylonsync/sdk",
      "@pylonsync/functions",
    ],
  },
});
