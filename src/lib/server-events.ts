// server 事件桥接：把 server-started / server-stopped 事件写入 useServerStore。
// 事件是运行状态的唯一权威源；模块级幂等标志防 StrictMode 双挂载重复注册。
import { listen } from "@tauri-apps/api/event";
import { serverApi } from "@/lib/api";
import { useServerStore } from "@/stores/use-server-store";
import type { ServerStatus } from "@/types";

let started = false;

export function setupServerEvents(): void {
  if (started) return;
  started = true;

  // 监听事件写 store；非 Tauri 环境（纯浏览器 dev）无事件系统，静默忽略。
  void listen<ServerStatus>("server-started", (event) => {
    useServerStore.getState().applyStatus(event.payload);
  }).catch(() => {});

  void listen<ServerStatus>("server-stopped", (event) => {
    useServerStore.getState().applyStatus(event.payload);
  }).catch(() => {});

  // 首次校准：webview 可能晚于启动事件加载而错过 boot 事件（见 commands/server.rs）。
  void serverApi
    .status()
    .then((status) => useServerStore.getState().applyStatus(status))
    .catch(() => {});
}
