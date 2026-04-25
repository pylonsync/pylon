import "./index.css";
import React from "react";
import { createRoot } from "react-dom/client";
import { ChatApp } from "../../client/ChatApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <ChatApp />
  </React.StrictMode>,
);
