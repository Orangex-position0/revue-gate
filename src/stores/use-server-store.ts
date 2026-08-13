// 唯一全局 store：服务运行状态。只由 server 事件（lib/server-events.ts）写入，页面只读。
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
