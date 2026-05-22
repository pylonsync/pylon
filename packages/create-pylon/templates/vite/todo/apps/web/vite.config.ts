import { defineConfig, type ProxyOptions } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Pylon dev exposes TWO ports:
//   :4321  → HTTP (functions, entity CRUD, /api/sync/pull, /api/auth/*)
//   :4322  → dedicated WebSocket listener with TCP read timeouts set
//            on the raw socket. Broadcasts flow immediately because
//            the reader thread's mutex is released every 200ms by the
//            kernel-level read timeout — no client keepalive ping
//            needed to break the wedge.
//
// We route the WS upgrade to :4322 and everything else to :4321. The
// HTTP-multiplexed `/api/sync/ws` on :4321 also works (and is the
// production fallback for proxies that can't forward to a secondary
// port), but it can't set stream-level timeouts because tiny_http's
// `CustomStream` hides the underlying TcpStream — so broadcasts there
// are latency-bounded by the client SDK's 200ms keepalive ping.
const PYLON_HTTP_TARGET = process.env.PYLON_TARGET ?? "http://localhost:4321";
const PYLON_WS_TARGET = process.env.PYLON_WS_TARGET ?? "ws://localhost:4322";

const proxyConfig: Record<string, string | ProxyOptions> = {
	// WebSocket sync: forward upgrade to the dedicated :4322 listener.
	// Must come BEFORE the catch-all /api entry so this more-specific
	// path wins for WS connections. The dedicated listener handshakes
	// on any path so we don't bother rewriting.
	"/api/sync/ws": {
		target: PYLON_WS_TARGET,
		ws: true,
		changeOrigin: true,
	},
	// HTTP — /api/auth/*, /api/fn/*, /api/sync/pull, /api/entities/*, etc.
	"/api": {
		target: PYLON_HTTP_TARGET,
		changeOrigin: true,
	},
};

export default defineConfig({
	plugins: [react(), tailwindcss()],
	server: {
		port: 3000,
		proxy: proxyConfig,
	},
});
