import React from "react";
import { createRoot } from "react-dom/client";
import { ErpApp } from "../../client/ErpApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <ErpApp />
  </React.StrictMode>,
);
