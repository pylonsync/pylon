import "./index.css";
import React from "react";
import { createRoot } from "react-dom/client";
import { configureClient } from "@pylonsync/react";
import { App } from "./App";
import { PYLON_URL } from "@/lib/pylon";

// Wire up the SDK before mounting so hooks see the configured baseUrl.
configureClient({ baseUrl: PYLON_URL });

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
