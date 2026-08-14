// 跨页共享类型集中导出：与后端 Command 签名一一对应（见 Architecture-frontend.md）。
// 字段名与后端 serde camelCase 对齐（Channel / ChannelInput / ModelMapping 见 domain/channel.rs）。

export interface ServerStatus {
  running: boolean;
  host: string | null;
  port: number | null;
}

/** 渠道类型：与后端 ChannelType 枚举 lowercase 序列化一致。 */
export type ChannelType = "openai" | "deepseek" | "custom" | "claude" | "gemini";

/** 模型映射值对象：客户端统一模型名 ↔ 上游实际模型名。 */
export interface ModelMapping {
  clientModel: string;
  upstreamModel: string;
}

/** 创建 / 编辑渠道入参（create 与 update 共用；update 时 apiKey 为 null = 保持原密钥）。 */
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

/** 渠道实体：Command 返回；apiKey 已被后端遮蔽恒为 null，lastTest* 待连通性测试（阶段 06）填充。 */
export interface Channel extends ChannelInput {
  id: string;
  lastTestAt: string | null;
  lastTestOk: boolean | null;
  createdAt: string;
  updatedAt: string;
}

/** 渠道连通性测试结果：由 test_channel 命令返回，并已持久化到渠道 lastTestAt / lastTestOk。 */
export interface ChannelTestResult {
  ok: boolean;
  latencyMs: number;
  testedAt: string;
  error: string | null;
}

/** 配额值对象：limit 为上限（null = 无上限），used 为已用额度。与后端 Quota 对齐。 */
export interface Quota {
  limit: number | null;
  used: number;
}

/** 创建 / 编辑密钥入参（create 与 update 共用；密钥本身不可由入参指定）。 */
export interface ApiKeyInput {
  name: string;
  quotaLimit: number | null;
  enabled: boolean;
}

/** 密钥实体：Command 返回。key 仅在 create 时携带明文，其余路径已被后端遮蔽为占位符。 */
export interface ApiKey {
  id: string;
  name: string;
  key: string;
  enabled: boolean;
  quota: Quota;
  createdAt: string;
  updatedAt: string;
}

/** 日志查询筛选条件：全部可选，null = 不筛该维度。与后端 LogQuery（Option + serde default）对齐。 */
export interface LogQuery {
  /** 关键词：对模型 / 上游模型 / trace / 错误做大小写不敏感子串匹配。 */
  keyword?: string | null;
  /** 按发起密钥筛选（api_key_id 精确匹配）。 */
  apiKeyId?: string | null;
  /** 按渠道筛选（channel_id 精确匹配）。 */
  channelId?: string | null;
  /** 按请求模型名筛选（大小写不敏感子串匹配）。 */
  model?: string | null;
  /** 日期下界（左闭）：created_at >= startAt。 */
  startAt?: string | null;
  /** 日期上界（右开）：created_at < endAt。 */
  endAt?: string | null;
}

/** 请求日志实体：一次请求的完整审计记录（与后端 RequestLog 对齐）。 */
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
  createdAt: string;
}

/** 分页查询结果：当前页日志 + 满足筛选的总条数（供计算总页数）。 */
export interface LogPage {
  items: RequestLog[];
  total: number;
}

/** 一条对话消息（日志详情展示；content 为文本或 null）。 */
export interface ConversationMessage {
  role: string;
  content: string | null;
  /** 该消息触发的工具调用名。 */
  toolNames: string[];
}

/** 日志详情：日志（扁平字段）+ 从请求体解析出的对话 / 参数 / 工具标签。 */
export interface LogDetail extends RequestLog {
  conversation: ConversationMessage[];
  /** 请求参数：request_body 顶层除 messages / tools / stream 之外的字段。 */
  requestParams: unknown;
  /** 工具标签：声明 + 调用去重合并。 */
  toolNames: string[];
}

/** 7 天趋势折线的单个数据点（date 为本地时区日期 YYYY-MM-DD）。 */
export interface DailyStat {
  date: string;
  requests: number;
  tokens: number;
}

/** 仪表盘统计快照：卡片指标 + 7 天趋势（与请求日志数据一致）。 */
export interface StatsSnapshot {
  /** 今日（本地时区）请求数。 */
  todayRequests: number;
  /** 今日（本地时区）总 token。 */
  todayTokens: number;
  /** 累计请求数。 */
  totalRequests: number;
  /** 累计总 token。 */
  totalTokens: number;
  /** 平均延迟（毫秒）：全部请求 duration_ms 的均值；无日志为 0。 */
  avgLatencyMs: number;
  /** 渠道可用率（0..=1）：status_code < 400 的请求占比；无日志为 0。 */
  channelAvailability: number;
  /** 最近 7 天（含今日，旧→新）按本地日聚合。 */
  trend: DailyStat[];
}

/** 界面主题三态：与后端 Theme 枚举 lowercase 序列化一致。 */
export type Theme = "light" | "dark" | "system";

/** 失败重试策略：enabled 开关；max_retries = 首次之后的额外尝试次数（null = 无上限）。
 *  字段为 snake_case：后端 RetryPolicy 无 rename_all，仅 GatewaySettings 整体 camelCase。 */
export interface RetryPolicy {
  enabled: boolean;
  max_retries: number | null;
}

/** 网关设置快照：与后端 GatewaySettings serde camelCase 对齐；port 0 = 随机端口。 */
export interface GatewaySettings {
  host: string;
  port: number;
  theme: Theme;
  minimizeToTray: boolean;
  closeToTray: boolean;
  autostart: boolean;
  retry: RetryPolicy;
}
