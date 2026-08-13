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
