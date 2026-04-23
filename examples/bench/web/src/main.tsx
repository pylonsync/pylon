import React from "react";
import { createRoot } from "react-dom/client";
import { BenchApp } from "../../client/BenchApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <BenchApp />
  </React.StrictMode>,
);
