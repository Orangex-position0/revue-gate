// The only global store: server running status. Written only by server events (lib/server-events.ts); pages read it.
import { create } from "zustand";
import type { ServerStatus } from "@/types";

interface ServerStore {
  running: boolean;
  host: string | null;
  port: number | null;
  applyStatus: (status: ServerStatus) => void;
}

export const useServerStore = create<ServerStore>((set) => ({
  running: false,
  host: null,
  port: null,
  applyStatus: (status) =>
    set({ running: status.running, host: status.host, port: status.port }),
}));
