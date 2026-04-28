import { defineConfig } from "@tanstack/react-start/config";
import tsConfigPaths from "vite-tsconfig-paths";

export default defineConfig({
  vite: {
    plugins: [tsConfigPaths({ projects: ["./tsconfig.json"] })],
    server: {
      // Proxy /api → Pylon dev server.
      proxy: {
        "/api": "http://localhost:4321",
      },
    },
  },
});
