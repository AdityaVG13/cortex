import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles/index.css";
import "./live-surface.css";
createRoot(document.getElementById("root")).render(
  React.createElement(React.StrictMode, null, React.createElement(App, null)),
);
