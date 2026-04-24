import React from "react";
import { createRoot } from "react-dom/client";
import { StoreApp } from "../../client/StoreApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <StoreApp />
  </React.StrictMode>,
);
