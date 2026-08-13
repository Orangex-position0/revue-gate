// Command 薄封装：全项目唯一的 invoke 调用点，按域分组。类型与后端 Command 签名对齐。
import { invoke } from "@tauri-apps/api/core";
import type {
  ApiKey,
  ApiKeyInput,
  Channel,
  ChannelInput,
  ChannelTestResult,
  ServerStatus,
} from "@/types";

/** 统一错误转换：Tauri Command 返回 Result<T, String>，invoke 拒绝值可能是 String / Error。 */
export function invokeErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export const serverApi = {
  /** 查询服务当前运行状态（webview 首挂载校准，常规刷新依赖事件）。 */
  status: () => invoke<ServerStatus>("get_server_status"),
};

export const channelApi = {
  /** 列出全部渠道（按优先级升序；apiKey 已被后端遮蔽）。 */
  list: () => invoke<Channel[]>("list_channels"),
  /** 创建渠道并返回（apiKey 被遮蔽）。 */
  create: (input: ChannelInput) => invoke<Channel>("create_channel", { input }),
  /** 更新渠道并返回（input.apiKey 为 null = 保持原密钥）。 */
  update: (id: string, input: ChannelInput) =>
    invoke<Channel>("update_channel", { id, input }),
  /** 删除渠道。 */
  remove: (id: string) => invoke<void>("delete_channel", { id }),
  /** 启停渠道并返回（状态正确持久化）。 */
  setEnabled: (id: string, enabled: boolean) =>
    invoke<Channel>("set_channel_enabled", { id, enabled }),
  /** 渠道连通性测试：调用上游模型列表端点，返回结果并落库（lastTestAt / lastTestOk）。 */
  test: (id: string) => invoke<ChannelTestResult>("test_channel", { id }),
};

export const apiKeyApi = {
  /** 列出全部密钥（按名称升序；key 已被后端遮蔽为占位符）。 */
  list: () => invoke<ApiKey[]>("list_api_keys"),
  /** 创建密钥并返回完整明文（仅此路径下发明文，调用方需一次性展示）。 */
  create: (input: ApiKeyInput) => invoke<ApiKey>("create_api_key", { input }),
  /** 更新密钥并返回（key 被遮蔽；密钥本身不可更新）。 */
  update: (id: string, input: ApiKeyInput) =>
    invoke<ApiKey>("update_api_key", { id, input }),
  /** 删除密钥。 */
  remove: (id: string) => invoke<void>("delete_api_key", { id }),
  /** 启停密钥并返回（状态正确持久化）。 */
  setEnabled: (id: string, enabled: boolean) =>
    invoke<ApiKey>("set_api_key_enabled", { id, enabled }),
};
