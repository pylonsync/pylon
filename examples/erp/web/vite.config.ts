import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: { port: 5174 },
  optimizeDeps: {
    exclude: [
      "@statecraft/sync",
      "@statecraft/react",
      "@statecraft/sdk",
    ],
  },
});
