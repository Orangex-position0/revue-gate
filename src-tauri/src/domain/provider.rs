//! ProviderAdaptor trait：隔离各供应商协议差异（见 docs/Architecture-backend.md）。
//!
//! 领域层只定义契约与纯类型：OpenAI / DeepSeek / Custom 走 OpenAI-compatible 直通，
//! Claude / Gemini 做协议转换，实现全部位于 infrastructure/providers（转换逻辑不出该目录）。
//! 适配器方法返回领域类型，不暴露 reqwest/axum 等技术类型；`forward_stream` 返回
//! OpenAI 兼容 SSE 字节流，数据面只负责透传。`futures-util` 仅为流抽象，非技术依赖（见 Spec 决策 6）。

pub use futures_util::stream::BoxStream;
use serde_json::Value;

use crate::domain::channel::{Channel, ChannelType};

/// usage：一次上游调用的 token 统计。各供应商字段名不同（OpenAI `usage` / Claude `input|output_tokens`
/// / Gemini `usageMetadata`），由适配器归一为本类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl Usage {
    /// total 缺失时由 prompt + completion 推导（部分供应商不返回 total）。
    pub fn normalized(self) -> Usage {
        let total_tokens = self.total_tokens.or_else(|| {
            Some(self.prompt_tokens.unwrap_or(0) + self.completion_tokens.unwrap_or(0))
        });
        Usage {
            total_tokens,
            ..self
        }
    }
}

/// 转发到上游的请求：代理已应用模型映射并改写 `body["model"]`（见阶段 07 转发用例）。
/// `model` / `stream` 与 body 保持一致，便于适配器构造目标协议请求。
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// 应用模型映射后的上游模型名。
    pub model: String,
    /// 是否流式。
    pub stream: bool,
    /// 原始 OpenAI 兼容请求体（JSON）。
    pub body: Value,
}

/// 非流式上游响应：适配器已转换为 OpenAI 兼容格式。`body` 为原始字节，
/// 上游错误响应（HTML/非 JSON）也原样透传不丢失。
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub status_code: u16,
    /// 已转换 / 透传的响应体。
    pub body: Vec<u8>,
    /// 从响应解析出的 usage（错误响应或解析失败时为 None）。
    pub usage: Option<Usage>,
}

/// 流式响应事件：`data` 为透传给下游的 OpenAI 兼容 SSE 字节，`usage` 为该帧携带的用量（如有）。
#[derive(Debug, Clone)]
pub struct StreamEvent {
    pub data: Vec<u8>,
    pub usage: Option<Usage>,
}

/// 连通性测试结果：成功 / 失败、延迟、失败原因（适配器用各自模型列表端点探测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// 供应商适配器错误：配置缺失与传输 / 解析失败。上游业务错误（4xx/5xx）不落此枚举——
/// `forward` 原样返回状态码与 body，由转发用例决定重试。
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider not configured: {0}")]
    NotConfigured(String),
    #[error("request error: {0}")]
    Request(String),
    #[error("invalid upstream response: {0}")]
    InvalidResponse(String),
}

/// 供应商适配器 trait：隔离协议差异，核心代理只依赖本 trait（见 docs/Architecture-backend.md）。
#[async_trait::async_trait]
pub trait ProviderAdaptor: Send + Sync {
    fn channel_type(&self) -> ChannelType;
    /// 创建渠道时的默认模型列表（便于前端表单预填）。
    fn default_models(&self) -> Vec<String>;
    /// 默认 Base URL；Custom 无默认值返回 None（渠道必须显式配置）。
    fn default_base_url(&self) -> Option<&'static str>;
    /// 连通性测试：用各供应商模型列表端点探测，返回延迟与失败原因。
    async fn test(&self, channel: &Channel) -> Result<TestResult, ProviderError>;
    /// 非流式转发：构造上游请求、转换协议、解析 usage。
    async fn forward(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<ProviderResponse, ProviderError>;
    /// 流式转发：返回 OpenAI 兼容 SSE 字节流，逐帧附带解析出的 usage。
    async fn forward_stream(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError>;
}
