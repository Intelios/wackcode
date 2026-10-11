import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyCachedTheme } from "./theme";
import { installForceReducedMotion } from "./reducedMotion";
import "./styles.css";

// Before the first render, so the boot screen already wears the user's theme.
applyCachedTheme();
installForceReducedMotion();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
