import React from "react";
import { createRoot } from "react-dom/client";
import { TradeApp } from "../../client/TradeApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <TradeApp />
  </React.StrictMode>,
);
