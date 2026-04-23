import React from "react";
import { createRoot } from "react-dom/client";
import { StageApp } from "../../client/StageApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <StageApp />
  </React.StrictMode>,
);
