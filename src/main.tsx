// Frontend entry: React mount + StrictMode + pre-apply theme before render + start the server event bridge.
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@/app/App";
import { setupServerEvents } from "@/lib/server-events";
import { applyTheme, getStoredTheme } from "@/lib/theme";
import "@/styles/index.css";

// Pre-apply the theme before render to avoid first-frame flash (system writes no data-theme; the CSS media query responds).
applyTheme(getStoredTheme());

// Start the server event bridge: idempotent registration (StrictMode double-mount safe), silently ignored in browser dev.
setupServerEvents();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
