// 前端入口：React 挂载 + StrictMode（脚手架阶段，后续追加 server 事件桥接）
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
