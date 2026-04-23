import React from "react";
import { createRoot } from "react-dom/client";
import { WorldApp } from "../../client/WorldApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <WorldApp />
  </React.StrictMode>,
);
