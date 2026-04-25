import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: { port: 5178 },
  // See chat/web/vite.config.ts for the rationale.
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
