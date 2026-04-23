import React from "react";
import { createRoot } from "react-dom/client";
import { CrmApp } from "../../client/CrmApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <CrmApp />
  </React.StrictMode>,
);
