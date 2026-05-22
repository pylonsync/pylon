import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { init } from "@pylonsync/react";
import { App } from "./App";
import "./index.css";

// Same-origin baseUrl — the Vite dev server proxies /api/* to the
// Pylon backend (see vite.config.ts). In production, configure your
// CDN / reverse proxy the same way and this works unchanged.
init({ baseUrl: "", appName: "__APP_NAME_SNAKE__" });

const root = document.getElementById("root");
if (!root) throw new Error("#root not found");
createRoot(root).render(
	<StrictMode>
		<App />
	</StrictMode>,
);
