import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: { port: 5176 },
  // Workers and the workspace packages change often; serve from source.
  worker: { format: "es" },
  optimizeDeps: {
    exclude: [
      "@pylonsync/sync",
      "@pylonsync/react",
      "@pylonsync/sdk",
      "@pylonsync/functions",
    ],
  },
});
