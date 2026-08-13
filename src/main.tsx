// 前端入口：React 挂载 + StrictMode + 渲染前预应用主题 + 启动 server 事件桥接。
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@/app/App";
import { setupServerEvents } from "@/lib/server-events";
import { applyTheme, getStoredTheme } from "@/lib/theme";
import "@/styles/index.css";

// 渲染前预应用主题，避免首帧闪色（system 不写 data-theme，由 CSS 媒体查询响应）。
applyTheme(getStoredTheme());

// 启动 server 事件桥接：幂等注册（StrictMode 双挂载安全），浏览器 dev 环境静默忽略。
setupServerEvents();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
