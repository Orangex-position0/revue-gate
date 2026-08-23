// Thin Command wrapper: the only invoke call site in the project, grouped by domain. Types align with the backend Command signatures.
import { invoke } from "@tauri-apps/api/core";
import type {
  ApiKey,
  ApiKeyInput,
  Channel,
  ChannelInput,
  ChannelTestResult,
  GatewaySettings,
  LogDetail,
  LogPage,
  LogQuery,
  ServerStatus,
  StatsSnapshot,
  UsageStats,
} from "@/types";

/** Unified error conversion: Tauri Commands return Result<T, String>; invoke rejection values may be String / Error. */
export function invokeErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export const serverApi = {
  /** Query the current server running status (reconciled on first webview mount; regular refreshes rely on events). */
  status: () => invoke<ServerStatus>("get_server_status"),
  /** Start the service: when host/port are omitted, read the saved shared settings (0 = random port).
   *  Returns the new status; the running state is governed by the server-started event (event bridge). */
  start: (host?: string, port?: number) =>
    invoke<ServerStatus>("start_server", { host, port }),
  /** Stop the service: returns the post-stop status; the running state is governed by the server-stopped event. */
  stop: () => invoke<ServerStatus>("stop_server"),
};

export const settingsApi = {
  /** Read the settings snapshot (returns defaults when nothing has been persisted). */
  get: () => invoke<GatewaySettings>("get_settings"),
  /** Save the full settings snapshot: validate → apply autostart → persist → update shared settings (effective immediately). */
  save: (settings: GatewaySettings) =>
    invoke<void>("save_settings", { settings }),
};

export const channelApi = {
  /** List all channels (ascending by priority; apiKey is masked by the backend). */
  list: () => invoke<Channel[]>("list_channels"),
  /** Create a channel and return it (apiKey is masked). */
  create: (input: ChannelInput) => invoke<Channel>("create_channel", { input }),
  /** Update a channel and return it (input.apiKey null = keep the original key). */
  update: (id: string, input: ChannelInput) =>
    invoke<Channel>("update_channel", { id, input }),
  /** Delete a channel. */
  remove: (id: string) => invoke<void>("delete_channel", { id }),
  /** Enable/disable a channel and return it (state is persisted correctly). */
  setEnabled: (id: string, enabled: boolean) =>
    invoke<Channel>("set_channel_enabled", { id, enabled }),
  /** Channel connectivity test: calls the upstream model-list endpoint, returns the result, and persists it (lastTestAt / lastTestOk). */
  test: (id: string) => invoke<ChannelTestResult>("test_channel", { id }),
  /** Manually fetch provider-reported models for the current form; failures do not mutate the saved channel. */
  fetchModels: (id: string | null, input: ChannelInput) =>
    invoke<string[]>("fetch_channel_models", { id, input }),
};

export const apiKeyApi = {
  /** List all API keys (ascending by name; key is masked to a placeholder by the backend). */
  list: () => invoke<ApiKey[]>("list_api_keys"),
  /** Create an API key and return the full plaintext (the only path that reveals it; the caller must display it once). */
  create: (input: ApiKeyInput) => invoke<ApiKey>("create_api_key", { input }),
  /** Update an API key and return it (key is masked; the key itself cannot be updated). */
  update: (id: string, input: ApiKeyInput) =>
    invoke<ApiKey>("update_api_key", { id, input }),
  /** Delete an API key. */
  remove: (id: string) => invoke<void>("delete_api_key", { id }),
  /** Enable/disable an API key and return it (state is persisted correctly). */
  setEnabled: (id: string, enabled: boolean) =>
    invoke<ApiKey>("set_api_key_enabled", { id, enabled }),
};

export const logApi = {
  /** Paginated log query: multi-criteria filters (keyword / key / channel / model / date range), newest first by creation time. */
  list: (query: LogQuery, page: number, pageSize: number) =>
    invoke<LogPage>("list_logs", { query, page, pageSize }),
  /** Log detail: request log + conversation / parameters / tool tags parsed from the request body. */
  detail: (id: string) => invoke<LogDetail>("get_log_detail", { id }),
  /** Delete logs created strictly before before (ISO-8601); returns the number deleted. */
  deleteBefore: (before: string) =>
    invoke<number>("delete_logs_before", { before }),
  /** Clear all request logs; returns the number deleted. */
  clear: () => invoke<number>("clear_logs"),
};

export const statsApi = {
  /** Dashboard stats snapshot: card metrics + 7-day trend.
   *  timezoneOffsetMinutes is the frontend local timezone offset (JS `Date.getTimezoneOffset()`: positive west, negative east). */
  get: (timezoneOffsetMinutes: number) =>
    invoke<StatsSnapshot>("get_stats", { timezoneOffsetMinutes }),
  usage: (startAt: string, endAt: string, timezoneOffsetMinutes: number) =>
    invoke<UsageStats>("usage_stats", {
      startAt,
      endAt,
      timezoneOffsetMinutes,
    }),
};
