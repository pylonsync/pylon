import "./index.css";
import React from "react";
import { createRoot } from "react-dom/client";
import { ArenaApp } from "../../client/ArenaApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <ArenaApp />
  </React.StrictMode>,
);
