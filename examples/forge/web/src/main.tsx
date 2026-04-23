import React from "react";
import { createRoot } from "react-dom/client";
import { ForgeApp } from "../../client/ForgeApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <ForgeApp />
  </React.StrictMode>,
);
