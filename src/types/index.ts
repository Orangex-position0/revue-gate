// Central exports of cross-page shared types: one-to-one with the backend Command signatures (see Architecture-frontend.md).
// Field names align with the backend's serde camelCase (Channel / ChannelInput / ModelMapping see domain/channel.rs).

export interface ServerStatus {
  running: boolean;
  host: string | null;
  port: number | null;
}

/** Channel type: serialized lowercase, consistent with the backend ChannelType enum. */
export type ChannelType = "openai" | "deepseek" | "custom" | "claude" | "gemini";

/** Model mapping value object: client unified model name ↔ upstream actual model name. */
export interface ModelMapping {
  clientModel: string;
  upstreamModel: string;
}

/** Create / edit channel input (shared by create and update; apiKey null on update = keep the original key). */
export interface ChannelInput {
  name: string;
  channelType: ChannelType;
  baseUrl: string | null;
  apiKey: string | null;
  models: string[];
  priority: number;
  weight: number;
  modelMappings: ModelMapping[];
  enabled: boolean;
}

/** Channel entity: returned by Commands; apiKey is always masked to null by the backend, lastTest* is filled in by the connectivity test (phase 06). */
export interface Channel extends ChannelInput {
  id: string;
  lastTestAt: string | null;
  lastTestOk: boolean | null;
  createdAt: string;
  updatedAt: string;
}

/** Channel connectivity test result: returned by the test_channel command and persisted to the channel's lastTestAt / lastTestOk. */
export interface ChannelTestResult {
  ok: boolean;
  latencyMs: number;
  testedAt: string;
  error: string | null;
}

/** Quota value object: limit is the cap (null = unlimited), used is the consumed amount. Aligned with the backend Quota. */
export interface Quota {
  limit: number | null;
  used: number;
}

/** Create / edit API key input (shared by create and update; the key itself cannot be set via input). */
export interface ApiKeyInput {
  name: string;
  quotaLimit: number | null;
  enabled: boolean;
}

/** API key entity: returned by Commands. key carries plaintext only on create; the backend masks it to a placeholder on all other paths. */
export interface ApiKey {
  id: string;
  name: string;
  key: string;
  enabled: boolean;
  quota: Quota;
  createdAt: string;
  updatedAt: string;
}

/** Log query filters: all optional, null = do not filter that dimension. Aligned with the backend LogQuery (Option + serde default). */
export interface LogQuery {
  /** Keyword: case-insensitive substring match against model / upstream model / trace / error. */
  keyword?: string | null;
  /** Filter by the requesting key (exact api_key_id match). */
  apiKeyId?: string | null;
  /** Filter by channel (exact channel_id match). */
  channelId?: string | null;
  /** Filter by request model name (case-insensitive substring match). */
  model?: string | null;
  /** Lower date bound (inclusive): created_at >= startAt. */
  startAt?: string | null;
  /** Upper date bound (exclusive): created_at < endAt. */
  endAt?: string | null;
}

/** Security audit projection types: aligned with the backend security_audit domain module. */
export type RiskLevel = "clean" | "info" | "low" | "medium" | "high" | "critical";
export type AuditAction = "allow" | "logOnly" | "warn" | "redact" | "confirm" | "block";
export type AuditMode = "observe" | "enforce";
export type AuditEvidenceLevel = "summary" | "detailed";
export type AuditConfidence = "low" | "medium" | "high";
export type AuditScopeKind =
  | "messageContent"
  | "systemMessageContent"
  | "toolCallArguments"
  | "toolName"
  | "toolDescription"
  | "toolSchemaString"
  | "toolSchemaKey"
  | "topLevelParam";

export interface AuditFinding {
  ruleId: string;
  category: string;
  riskLevel: RiskLevel;
  action: AuditAction;
  confidence: AuditConfidence;
  scopeKind: AuditScopeKind;
  path: string;
  redactedExcerpt: string;
  matchHash: string;
  suggestedAction: string;
  evidence?: string | null;
}

export interface AuditReport {
  mode: AuditMode;
  riskLevel: RiskLevel;
  riskScore: number;
  action: AuditAction;
  findings: AuditFinding[];
  totalFindings: number;
  findingsTruncated: boolean;
  scannedBytes: number;
  candidateBytes: number;
  scanByteLimit: number;
  truncated: boolean;
  evidenceLevel: AuditEvidenceLevel;
  upstreamForwarded: boolean;
  plannedChannelId: string | null;
  plannedUpstreamModel: string | null;
}

/** Request log entity: the full audit record of one request (aligned with the backend RequestLog). */
export interface RequestLog {
  id: string;
  apiKeyId: string | null;
  channelId: string | null;
  model: string;
  upstreamModel: string | null;
  statusCode: number;
  promptTokens: number | null;
  completionTokens: number | null;
  totalTokens: number | null;
  durationMs: number;
  errorMessage: string | null;
  isStream: boolean;
  isRetry: boolean;
  traceId: string;
  requestBody: string | null;
  riskLevel: RiskLevel | null;
  auditAction: AuditAction | null;
  auditReport: AuditReport | null;
  createdAt: string;
}

/** Paginated query result: current-page logs + the total count matching the filters (for computing total pages). */
export interface LogPage {
  items: RequestLog[];
  total: number;
}

/** A single conversation message (shown in the log detail; content is text or null). */
export interface ConversationMessage {
  role: string;
  content: string | null;
  /** Names of the tool calls triggered by this message. */
  toolNames: string[];
}

/** Log detail: the log (flat fields) + conversation / params / tool tags parsed from the request body. */
export interface LogDetail extends RequestLog {
  conversation: ConversationMessage[];
  /** Request params: request_body top-level fields other than messages / tools / stream. */
  requestParams: unknown;
  /** Tool tags: declarations and calls merged with deduplication. */
  toolNames: string[];
}

/** A single data point on the 7-day trend line (date is a local-timezone date, YYYY-MM-DD). */
export interface DailyStat {
  date: string;
  requests: number;
  tokens: number;
}

/** Dashboard stats snapshot: card metrics + 7-day trend (consistent with request log data). */
export interface StatsSnapshot {
  /** Today's (local timezone) request count. */
  todayRequests: number;
  /** Today's (local timezone) total tokens. */
  todayTokens: number;
  /** Cumulative request count. */
  totalRequests: number;
  /** Cumulative total tokens. */
  totalTokens: number;
  /** Average latency (ms): mean of duration_ms across all requests; 0 when no logs. */
  avgLatencyMs: number;
  /** Channel availability (0..=1): share of requests with status_code < 400; 0 when no logs. */
  channelAvailability: number;
  /** Last 7 days (including today, oldest → newest) aggregated by local day. */
  trend: DailyStat[];
}

/** UI theme tri-state: serialized lowercase, consistent with the backend Theme enum. */
export type Theme = "light" | "dark" | "system";

/** Failure retry policy: enabled toggle; max_retries = extra attempts after the first (null = unlimited).
 *  Fields are snake_case: the backend RetryPolicy has no rename_all, only GatewaySettings is camelCase as a whole. */
export interface RetryPolicy {
  enabled: boolean;
  max_retries: number | null;
}

export interface AuditSettings {
  enabled: boolean;
  mode: AuditMode;
  blockCritical: boolean;
  scanSystemMessages: boolean;
  scanByteLimit: number;
  storePayload: boolean;
  evidenceLevel: AuditEvidenceLevel;
}

/** Gateway settings snapshot: aligned with the backend GatewaySettings serde camelCase; port 0 = random port. */
export interface GatewaySettings {
  host: string;
  port: number;
  theme: Theme;
  minimizeToTray: boolean;
  closeToTray: boolean;
  autostart: boolean;
  retry: RetryPolicy;
  audit: AuditSettings;
}
