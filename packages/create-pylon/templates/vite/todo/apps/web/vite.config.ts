import { defineConfig, type ProxyOptions } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Backend origin. `pylon dev` defaults to :4321 — override via the
// PYLON_TARGET env if you're running it elsewhere. In a production
// build (`vite build` → static hosting + a separate API origin),
// configure your reverse proxy / CDN to route /api/* to the same
// place this proxy points at.
const PYLON_TARGET = process.env.PYLON_TARGET ?? "http://localhost:4321";

const proxyConfig: Record<string, string | ProxyOptions> = {
	// HTTP — /api/auth/*, /api/fn/*, /api/sync/pull, etc.
	"/api": {
		target: PYLON_TARGET,
		changeOrigin: true,
		// Vite's http-proxy needs ws:true for the WebSocket upgrade
		// to propagate. Without it `/api/sync/ws` 404s and db.useQuery
		// hangs in a "connecting" state forever.
		ws: true,
	},
};

export default defineConfig({
	plugins: [react(), tailwindcss()],
	server: {
		port: 3000,
		proxy: proxyConfig,
	},
});
