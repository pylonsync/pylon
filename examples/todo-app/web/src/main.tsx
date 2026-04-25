import "./index.css";
import React from "react";
import { createRoot } from "react-dom/client";
import { TodoApp } from "../../client/TodoApp";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");
createRoot(root).render(
  <React.StrictMode>
    <TodoApp />
  </React.StrictMode>,
);
