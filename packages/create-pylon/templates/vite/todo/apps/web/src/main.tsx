import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { init } from "@pylonsync/react";
import { App } from "./App";
import "./index.css";

// Omit baseUrl — Pylon defaults to window.location.origin, which the
// Vite dev server proxies to the backend (see vite.config.ts). The
// same setup ships to production unchanged: configure your CDN /
// reverse proxy to forward /api/* (including the WebSocket upgrade
// for /api/sync/ws) to your Pylon server.
init({ appName: "__APP_NAME_SNAKE__" });

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");
createRoot(root).render(
	<StrictMode>
		<App />
	</StrictMode>,
);
