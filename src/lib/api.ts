// Command 薄封装：全项目唯一的 invoke 调用点，按域分组。类型与后端 Command 签名对齐。
import { invoke } from "@tauri-apps/api/core";
import type { ServerStatus } from "@/types";

export const serverApi = {
  /** 查询服务当前运行状态（webview 首挂载校准，常规刷新依赖事件）。 */
  status: () => invoke<ServerStatus>("get_server_status"),
};
