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

    /// 合并两次用量：各字段按「两侧都有则求和、否则保留有的一侧」累加（饱和加法防溢出）。
    /// 用于流式响应逐帧用量聚合（OpenAI 兼容流通常仅末帧携带，聚合结果即等价于取末帧）。
    pub fn accumulate(self, other: Usage) -> Usage {
        Usage {
            prompt_tokens: opt_add(self.prompt_tokens, other.prompt_tokens),
            completion_tokens: opt_add(self.completion_tokens, other.completion_tokens),
            total_tokens: opt_add(self.total_tokens, other.total_tokens),
        }
    }
}

/// 两侧都有则饱和求和；仅一侧有则保留该侧；都无则 None。
fn opt_add(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.saturating_add(y)),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
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
#[derive(Debug, Clone, thiserror::Error)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(prompt: Option<u64>, completion: Option<u64>, total: Option<u64>) -> Usage {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
        }
    }

    /// 两侧都有时按字段饱和求和（流式多帧用量累加）。
    #[test]
    fn accumulate_sums_fields_present_on_both_sides() {
        let a = usage(Some(10), Some(5), Some(15));
        let b = usage(Some(3), Some(2), Some(5));
        assert_eq!(a.accumulate(b), usage(Some(13), Some(7), Some(20)));
    }

    /// 一侧缺失时保留有的一侧（各供应商用量字段不全时仍能聚合）。
    #[test]
    fn accumulate_keeps_present_side_when_other_missing() {
        let a = usage(Some(10), None, None);
        let b = usage(None, Some(2), Some(5));
        assert_eq!(a.accumulate(b), usage(Some(10), Some(2), Some(5)));
    }

    /// 两侧都缺失时保持 None；空对象是恒等元。
    #[test]
    fn accumulate_none_fields_stay_none() {
        let a = usage(None, None, None);
        let b = usage(Some(1), None, None);
        assert_eq!(a.accumulate(b), usage(Some(1), None, None));
        assert_eq!(a.accumulate(a), usage(None, None, None));
    }

    /// total 缺失时由 prompt + completion 推导；已存在时不被覆盖。
    #[test]
    fn normalized_derives_total_from_parts_when_missing() {
        assert_eq!(
            usage(Some(10), Some(5), None).normalized(),
            usage(Some(10), Some(5), Some(15))
        );
        assert_eq!(
            usage(Some(10), Some(5), Some(20)).normalized(),
            usage(Some(10), Some(5), Some(20))
        );
    }
}
