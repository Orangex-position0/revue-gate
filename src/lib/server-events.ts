// Server event bridge: writes server-started / server-stopped events into useServerStore.
// Events are the single source of truth for the running state; a module-level idempotent flag prevents duplicate registration on StrictMode double-mount.
import { listen } from "@tauri-apps/api/event";
import { serverApi } from "@/lib/api";
import { useServerStore } from "@/stores/use-server-store";
import type { ServerStatus } from "@/types";

let started = false;

export function setupServerEvents(): void {
  if (started) return;
  started = true;

  // Listen for events and write the store; non-Tauri environments (pure browser dev) have no event system, so ignore silently.
  void listen<ServerStatus>("server-started", (event) => {
    useServerStore.getState().applyStatus(event.payload);
  }).catch(() => {});

  void listen<ServerStatus>("server-stopped", (event) => {
    useServerStore.getState().applyStatus(event.payload);
  }).catch(() => {});

  // Initial reconciliation: the webview may load after the start event and miss the boot event (see commands/server.rs).
  void serverApi
    .status()
    .then((status) => useServerStore.getState().applyStatus(status))
    .catch(() => {});
}
