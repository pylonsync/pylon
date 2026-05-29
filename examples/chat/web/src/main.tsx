// Vite entry point. Mounts the chat app into #root, wires up the
// Pylon client (sync engine + bearer/cookie auth + guest-session
// auto-mint), and that's the whole app shell.

import React from "react";
import ReactDOM from "react-dom/client";
import { ChatApp } from "./ChatApp";
import { PylonProvider } from "./pylon-client";
import "./styles.css";

// Same-origin in dev (Pylon serves the SPA on :4321 and proxies to
// Vite on the assigned port for HMR + assets) and in prod (Pylon
// serves the built dist). Falls back to a hard-coded localhost for
// the "vite-only" workflow where someone runs `vite` without Pylon
// in front, in which case the dev-server proxy in vite.config.ts
// kicks the API requests over to :4321 directly.
const BASE_URL = (() => {
  if (typeof window === "undefined") return "http://localhost:4321";
  const env = (import.meta as unknown as { env?: { VITE_PYLON_URL?: string } })
    .env;
  return env?.VITE_PYLON_URL ?? window.location.origin;
})();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <PylonProvider baseUrl={BASE_URL}>
      <ChatApp />
    </PylonProvider>
  </React.StrictMode>,
);
