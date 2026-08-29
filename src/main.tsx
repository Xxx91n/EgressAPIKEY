import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./i18n";
import "./styles.css";
import { attachLogger } from "@fltsci/tauri-plugin-tracing";
import { useLogStore } from "./store/logStore";
import { purgeLegacyDeadKeys } from "./lib/settings";

// Bridge the Rust tracing WebviewLayer into the frontend log panel. Outside a
// Tauri context (vitest / plain browser) attachLogger resolves to a no-op
// listener promise we can safely ignore. The plugin emits `tracing://log`
// events; attachLogger subscribes and calls us per {level, message}.
// `entry.message` is a LogMessage = rest-tuple of console.log args; serialise
// it to one display string (strings joined, objects JSON-stringified).
const fmt = (parts: unknown[]) =>
  parts.map((p) => (typeof p === "string" ? p : JSON.stringify(p))).join(" ");
void attachLogger((entry) => {
  useLogStore.getState().append({
    level: entry.level,
    message: fmt(entry.message as unknown[]),
    ts: Date.now(),
  });
}).catch(() => { /* no-op outside tauri */ });

// arch/02: drop legacy dead settings keys (if any residue exists) before the
// UI mounts, so settings.json only ever carries live keys.
void purgeLegacyDeadKeys();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
