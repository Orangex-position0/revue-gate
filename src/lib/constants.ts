// App-level constants: app name + channel type label mapping (values grow with requirements).
import type { ChannelType } from "@/types";

export const APP_NAME = "revue-gate";

/** Channel type display names: one-to-one with ChannelType in types/index.ts. */
export const CHANNEL_TYPE_LABELS: Record<ChannelType, string> = {
  openai: "OpenAI",
  deepseek: "DeepSeek",
  custom: "Custom",
  claude: "Claude",
  gemini: "Gemini",
};
