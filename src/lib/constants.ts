// 应用级常量：应用名 + 渠道类型 label 映射（值随需求追加）。
import type { ChannelType } from "@/types";

export const APP_NAME = "revue-gate";

/** 渠道类型显示名：与 types/index.ts 的 ChannelType 一一对应。 */
export const CHANNEL_TYPE_LABELS: Record<ChannelType, string> = {
  openai: "OpenAI",
  deepseek: "DeepSeek",
  custom: "Custom",
  claude: "Claude",
  gemini: "Gemini",
};
