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

export const DEFAULT_CHANNEL_MODELS: Record<ChannelType, string[]> = {
  openai: ["gpt-4o", "gpt-4o-mini", "gpt-4.1", "gpt-4.1-mini"],
  deepseek: ["deepseek-chat", "deepseek-reasoner"],
  custom: [],
  claude: ["claude-opus-4-1", "claude-sonnet-4-5", "claude-haiku-4-5"],
  gemini: ["gemini-2.0-flash", "gemini-2.5-flash", "gemini-2.5-pro"],
};
