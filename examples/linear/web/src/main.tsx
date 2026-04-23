import React from "react";
import { createRoot } from "react-dom/client";
import { LinearApp } from "../../client/LinearApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <LinearApp />
  </React.StrictMode>,
);
