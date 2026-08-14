//! 转发用例引擎：网关核心调度闭环（认证 → 选渠道 → 模型映射 → 转发 → 解析 usage → 记账 → 写日志 → 失败按序重试）。
//!
//! `ProxyRequestUsecase` 不接真实 HTTP：三仓储以 `Arc<dyn Trait>` 注入、适配器经解析闭包注入
//! （seam A 可全 mock，见 Spec §Testing）。流程：
//! 1. 认证（复用 AuthenticateRequestUsecase，401 / 429）；
//! 2. `ChannelSelector` 选候选渠道（启用 → 模型匹配 → 优先级升序）；
//! 3. 应用模型映射改写 `body["model"]`，逐候选尝试：
//!    - 非流式成功（非可重试状态码）→ 记账 + 写成功日志后返回；
//!    - 失败（传输错误 / 429 / 5xx）→ 写一次失败日志后尝试下一候选，不超过候选渠道数；
//! 4. 流式：打开失败同样按序重试；打开成功后返回包装流，流结束（正常或出错）时聚合 usage 记账 + 写日志。
//!
//! 记账 / 日志写入为尽力而为：上游已成功处理时，记账失败不应让客户端收到 5xx 而重复计费。

use std::sync::{Arc, RwLock};
use std::time::Instant;

use chrono::Utc;
use futures_util::StreamExt;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::api_key::{ApiKey, ApiKeyRepository};
use crate::domain::channel::{Channel, ChannelRepository};
use crate::domain::dispatcher::ChannelSelector;
use crate::domain::error::RepositoryError;
use crate::domain::provider::{
    BoxStream, ChatRequest, ProviderAdaptor, ProviderError, ProviderResponse, StreamEvent, Usage,
};
use crate::domain::request_log::{RequestLog, RequestLogRepository};
use crate::domain::settings::GatewaySettings;
use crate::usecases::api_key::{AccumulateUsageUsecase, ApiKeyError};
use crate::usecases::auth::{AuthError, AuthenticateRequestUsecase};

/// 代理用例错误：分支与数据面 HTTP 状态码一一对应（401 / 429 / 404 / 502）。
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("invalid or missing api key")]
    /// 应返回 401。
    Unauthorized,
    #[error("quota exceeded")]
    /// 应返回 429。
    QuotaExceeded,
    #[error("no candidate channel supports model '{0}'")]
    /// 应返回 404（模型不可路由）。
    NoCandidateChannel(String),
    #[error("all {attempts} candidate channel(s) failed: {last_error}")]
    /// 全部候选失败，应返回 502。
    NoChannelAvailable {
        attempts: usize,
        last_status: Option<u16>,
        last_error: String,
    },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("repository error: {0}")]
    Repository(#[from] RepositoryError),
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
}

impl From<AuthError> for ProxyError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::Unauthorized => ProxyError::Unauthorized,
            AuthError::QuotaExceeded => ProxyError::QuotaExceeded,
            AuthError::Repository(e) => ProxyError::Repository(e),
        }
    }
}

impl From<ApiKeyError> for ProxyError {
    fn from(e: ApiKeyError) -> Self {
        match e {
            ApiKeyError::Repository(e) => ProxyError::Repository(e),
            ApiKeyError::NotFound => {
                ProxyError::InvalidRequest("api key disappeared during proxy".into())
            }
            ApiKeyError::Validation(m) => ProxyError::InvalidRequest(m),
        }
    }
}

/// 代理入参：数据面从 HTTP 请求解析后传入。
#[derive(Debug, Clone)]
pub struct ProxyRequest {
    /// Authorization Bearer 令牌（已剥离 `Bearer ` 前缀；None = 无有效头）。
    pub bearer_token: Option<String>,
    /// 客户端请求的模型名（`body["model"]`）。
    pub model: String,
    /// 是否流式（对应 `body["stream"]`）。
    pub stream: bool,
    /// 原始 OpenAI 兼容请求体（JSON）。
    pub body: Value,
    /// 贯穿请求的 trace id（写日志用）。
    pub trace_id: String,
}

/// 代理成功结果：非流式响应，或流式事件流（流结束时报账/写日志已内联）。
pub enum ProxySuccess {
    NonStream(ProviderResponse),
    Stream(BoxStream<'static, Result<StreamEvent, ProxyError>>),
}

impl std::fmt::Debug for ProxySuccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxySuccess::NonStream(r) => f.debug_tuple("NonStream").field(r).finish(),
            ProxySuccess::Stream(_) => f.write_str("Stream(..)"),
        }
    }
}

/// 按渠道解析适配器的解析器（生产注入 `infrastructure::providers::adaptor_for`）。
type AdaptorResolver = Box<dyn Fn(&Channel) -> Box<dyn ProviderAdaptor> + Send + Sync>;

/// 转发用例：编排认证 → 选渠道 → 映射 → 转发 → 记账 → 日志 → 重试。
pub struct ProxyRequestUsecase {
    api_key_repo: Arc<dyn ApiKeyRepository>,
    channel_repo: Arc<dyn ChannelRepository>,
    log_repo: Arc<dyn RequestLogRepository>,
    adaptor: AdaptorResolver,
    /// 共享网关设置：execute 时读取重试策略（保存设置后即时生效，无需重建用例）。
    settings: Arc<RwLock<GatewaySettings>>,
}

impl ProxyRequestUsecase {
    pub fn new(
        api_key_repo: Arc<dyn ApiKeyRepository>,
        channel_repo: Arc<dyn ChannelRepository>,
        log_repo: Arc<dyn RequestLogRepository>,
        adaptor: AdaptorResolver,
        settings: Arc<RwLock<GatewaySettings>>,
    ) -> Self {
        Self {
            api_key_repo,
            channel_repo,
            log_repo,
            adaptor,
            settings,
        }
    }

    /// 执行一次请求转发闭环。
    pub async fn execute(&self, request: ProxyRequest) -> Result<ProxySuccess, ProxyError> {
        let model = request.model.trim();
        if model.is_empty() {
            return Err(ProxyError::InvalidRequest("model must not be empty".into()));
        }

        // 1) 认证：无 / 无效 / 停用密钥 → 401；配额超限 → 429。
        let api_key = AuthenticateRequestUsecase
            .execute(self.api_key_repo.as_ref(), request.bearer_token.as_deref())
            .await?;

        // 2) 选候选渠道：启用 → 模型匹配 → 优先级升序。
        let candidates = ChannelSelector::select(&self.channel_repo.list().await?, model);
        if candidates.is_empty() {
            return Err(ProxyError::NoCandidateChannel(model.to_string()));
        }

        // 2b) 重试策略（共享设置，execute 时读取）决定本轮最多尝试次数：
        //     关闭 → 只试首个候选；开启且不限 → 全部候选（ticket 07 默认行为）；
        //     开启且限 n → 首个之后最多再试 n 次（总尝试 = n + 1），且不超过候选数。
        let retry = self
            .settings
            .read()
            .expect("settings lock poisoned")
            .retry
            .clone();
        let max_attempts = if retry.enabled {
            retry
                .max_retries
                .map(|n| (n as usize).saturating_add(1))
                .unwrap_or(candidates.len())
        } else {
            1
        }
        .min(candidates.len());

        // 3) 逐候选尝试：成功即返回；失败记录日志后尝试下一个（不超过 max_attempts）。
        let mut attempts = 0usize;
        let mut last_status: Option<u16> = None;
        let mut last_error = String::new();

        for channel in candidates.iter().take(max_attempts) {
            attempts += 1;
            let upstream_model = apply_mapping(channel, model);
            let mut body = request.body.clone();
            body["model"] = json!(upstream_model);
            let chat_request = ChatRequest {
                model: upstream_model.clone(),
                stream: request.stream,
                body,
            };
            let adaptor = (self.adaptor)(channel);
            let ctx = AttemptContext {
                request: request.clone(),
                api_key: api_key.clone(),
                channel: channel.clone(),
                upstream_model: upstream_model.clone(),
                is_retry: attempts > 1,
                started: Instant::now(),
            };

            if request.stream {
                match adaptor.forward_stream(channel, &chat_request).await {
                    Ok(stream) => {
                        let wrapped = wrap_stream_bookkeeping(
                            stream,
                            Arc::clone(&self.api_key_repo),
                            Arc::clone(&self.log_repo),
                            ctx,
                        );
                        return Ok(ProxySuccess::Stream(wrapped));
                    }
                    Err(e) => {
                        let reason = e.to_string();
                        record_failure(self.log_repo.as_ref(), &ctx, None, &reason).await;
                        last_status = None;
                        last_error = reason;
                    }
                }
            } else {
                match adaptor.forward(channel, &chat_request).await {
                    Ok(response) if !is_retryable(response.status_code) => {
                        record_success(
                            self.api_key_repo.as_ref(),
                            self.log_repo.as_ref(),
                            &ctx,
                            &response,
                        )
                        .await;
                        return Ok(ProxySuccess::NonStream(response));
                    }
                    Ok(response) => {
                        let reason = format!("upstream returned status {}", response.status_code);
                        record_failure(
                            self.log_repo.as_ref(),
                            &ctx,
                            Some(response.status_code),
                            &reason,
                        )
                        .await;
                        last_status = Some(response.status_code);
                        last_error = reason;
                    }
                    Err(e) => {
                        let reason = e.to_string();
                        record_failure(self.log_repo.as_ref(), &ctx, None, &reason).await;
                        last_status = None;
                        last_error = reason;
                    }
                }
            }
        }

        Err(ProxyError::NoChannelAvailable {
            attempts,
            last_status,
            last_error,
        })
    }
}

/// 应用模型映射：命中 `client_model` 用 `upstream_model`，未映射直传客户端模型名。
fn apply_mapping(channel: &Channel, client_model: &str) -> String {
    channel
        .model_mappings
        .iter()
        .find(|m| m.client_model == client_model)
        .map(|m| m.upstream_model.clone())
        .unwrap_or_else(|| client_model.to_string())
}

/// 可重试状态码：429（上游限流）与 5xx（上游服务器错误）换下一候选；4xx 属客户端/模型错误不重试。
fn is_retryable(status: u16) -> bool {
    status == 429 || (500..=599).contains(&status)
}

/// 记账：按 usage 归一后的 total tokens 累加密钥已用额度（无 usage 或 total 为 0 时不动作）。
async fn accumulate_usage(
    api_key_repo: &dyn ApiKeyRepository,
    api_key_id: Uuid,
    usage: Option<Usage>,
) -> Result<(), ProxyError> {
    let Some(usage) = usage else {
        return Ok(());
    };
    let total = usage.normalized().total_tokens.unwrap_or(0);
    if total == 0 {
        return Ok(());
    }
    AccumulateUsageUsecase
        .execute(api_key_repo, api_key_id, total)
        .await
        .map(|_| ())
        .map_err(ProxyError::from)
}

/// 一次上游尝试的共享上下文：请求 + 认证后的 key + 选中渠道 + 映射后模型 + 重试标记 + 起始时间。
#[derive(Clone)]
struct AttemptContext {
    request: ProxyRequest,
    api_key: ApiKey,
    channel: Channel,
    upstream_model: String,
    is_retry: bool,
    started: Instant,
}

/// 成功路径报账 + 写日志（尽力而为：上游已成功，记账失败仅告警不阻断响应）。
async fn record_success(
    api_key_repo: &dyn ApiKeyRepository,
    log_repo: &dyn RequestLogRepository,
    ctx: &AttemptContext,
    response: &ProviderResponse,
) {
    if let Err(e) = accumulate_usage(api_key_repo, ctx.api_key.id, response.usage).await {
        tracing::warn!(error = %e, trace_id = %ctx.request.trace_id, "failed to accumulate quota after successful forward");
    }
    let log = build_log(ctx, response.status_code, response.usage, None);
    if let Err(e) = log_repo.save(&log).await {
        tracing::warn!(error = %e, trace_id = %ctx.request.trace_id, "failed to write request log after successful forward");
    }
}

/// 失败路径写日志：状态码缺失（传输错误）记 502；重试循环不受日志失败影响。
async fn record_failure(
    log_repo: &dyn RequestLogRepository,
    ctx: &AttemptContext,
    status: Option<u16>,
    error_message: &str,
) {
    let log = build_log(
        ctx,
        status.unwrap_or(502),
        None,
        Some(error_message.to_string()),
    );
    if let Err(e) = log_repo.save(&log).await {
        tracing::warn!(error = %e, trace_id = %ctx.request.trace_id, "failed to write failure request log");
    }
}

/// 组装一条请求日志（token 字段由 u64 usage 收窄为 u32）。
fn build_log(
    ctx: &AttemptContext,
    status_code: u16,
    usage: Option<Usage>,
    error_message: Option<String>,
) -> RequestLog {
    RequestLog {
        id: Uuid::now_v7(),
        api_key_id: Some(ctx.api_key.id),
        channel_id: Some(ctx.channel.id),
        model: ctx.request.model.clone(),
        upstream_model: Some(ctx.upstream_model.clone()),
        status_code,
        prompt_tokens: usage.and_then(|u| u.prompt_tokens).map(|t| t as u32),
        completion_tokens: usage.and_then(|u| u.completion_tokens).map(|t| t as u32),
        total_tokens: usage.and_then(|u| u.total_tokens).map(|t| t as u32),
        duration_ms: ctx.started.elapsed().as_millis() as u64,
        error_message,
        is_stream: ctx.request.stream,
        is_retry: ctx.is_retry,
        trace_id: ctx.request.trace_id.clone(),
        request_body: Some(ctx.request.body.to_string()),
        created_at: Utc::now(),
    }
}

/// 流结束时的报账/写日志状态。
struct StreamState {
    inner: BoxStream<'static, Result<StreamEvent, ProviderError>>,
    done: bool,
    usage: Usage,
}

/// 包装上游流：逐帧聚合 usage，流正常结束（None）时记账 + 写成功日志，
/// 中途出错（Err）时按已聚合的增量 usage 记账 + 写失败日志后终止（US19「每次请求累加密钥配额」）。
/// 打开流本身失败的重试在 execute 内处理。
/// ponytail: 客户端中途断开导致流未被拉尽时不报账/写日志；v0.1 接受，升级需取消感知包装。
fn wrap_stream_bookkeeping(
    inner: BoxStream<'static, Result<StreamEvent, ProviderError>>,
    api_key_repo: Arc<dyn ApiKeyRepository>,
    log_repo: Arc<dyn RequestLogRepository>,
    ctx: AttemptContext,
) -> BoxStream<'static, Result<StreamEvent, ProxyError>> {
    Box::pin(futures_util::stream::unfold(
        StreamState {
            inner,
            done: false,
            usage: Usage::default(),
        },
        move |mut state: StreamState| {
            let api_key_repo = Arc::clone(&api_key_repo);
            let log_repo = Arc::clone(&log_repo);
            let ctx = ctx.clone();
            async move {
                if state.done {
                    return None;
                }
                match state.inner.next().await {
                    Some(Ok(event)) => {
                        if let Some(u) = event.usage {
                            state.usage = state.usage.accumulate(u);
                        }
                        Some((Ok(event), state))
                    }
                    Some(Err(err)) => {
                        state.done = true;
                        let proxy_err = ProxyError::Provider(err);
                        // 已下发的增量 usage 也累加配额（与失败日志记录的 total_tokens 一致；尽力而为）。
                        if let Err(e) = accumulate_usage(
                            api_key_repo.as_ref(),
                            ctx.api_key.id,
                            Some(state.usage),
                        )
                        .await
                        {
                            tracing::warn!(error = %e, trace_id = %ctx.request.trace_id, "failed to accumulate quota after stream error");
                        }
                        let log =
                            build_log(&ctx, 502, Some(state.usage), Some(proxy_err.to_string()));
                        if let Err(e) = log_repo.save(&log).await {
                            tracing::warn!(error = %e, trace_id = %ctx.request.trace_id, "failed to write stream failure log");
                        }
                        Some((Err(proxy_err), state))
                    }
                    None => {
                        state.done = true;
                        if let Err(e) = accumulate_usage(
                            api_key_repo.as_ref(),
                            ctx.api_key.id,
                            Some(state.usage),
                        )
                        .await
                        {
                            tracing::warn!(error = %e, trace_id = %ctx.request.trace_id, "failed to accumulate quota after stream ended");
                        }
                        let log = build_log(&ctx, 200, Some(state.usage), None);
                        if let Err(e) = log_repo.save(&log).await {
                            tracing::warn!(error = %e, trace_id = %ctx.request.trace_id, "failed to write stream success log");
                        }
                        None
                    }
                }
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channel::ModelMapping;
    use crate::domain::provider::Usage;
    use crate::domain::settings::RetryPolicy;
    use crate::test_support::{
        InMemoryApiKeyRepository, InMemoryChannelRepository, InMemoryRequestLogRepository,
        MockForwardAdaptor, all_request_logs, sample_api_key, sample_channel,
    };
    use std::sync::Mutex;

    fn usage(prompt: u64, completion: u64, total: u64) -> Usage {
        Usage {
            prompt_tokens: Some(prompt),
            completion_tokens: Some(completion),
            total_tokens: Some(total),
        }
    }

    fn ok_response(status: u16, usage: Option<Usage>) -> ProviderResponse {
        ProviderResponse {
            status_code: status,
            body: b"{\"ok\":true}".to_vec(),
            usage,
        }
    }

    async fn save_key(repo: &InMemoryApiKeyRepository) -> ApiKey {
        let mut k = sample_api_key();
        k.quota.limit = Some(1000);
        k.quota.used = 0;
        let saved = k.clone();
        repo.save(&k).await.expect("save key");
        saved
    }

    async fn save_channel(
        repo: &InMemoryChannelRepository,
        name: &str,
        models: &[&str],
        priority: i32,
    ) -> Channel {
        let mut c = sample_channel();
        c.name = name.to_string();
        c.models = models.iter().map(|m| m.to_string()).collect();
        c.priority = priority;
        let saved = c.clone();
        repo.save(&c).await.expect("save channel");
        saved
    }

    fn request(bearer: Option<&str>, model: &str, stream: bool) -> ProxyRequest {
        ProxyRequest {
            bearer_token: bearer.map(str::to_string),
            model: model.to_string(),
            stream,
            body: serde_json::json!({"model": model, "stream": stream, "messages": []}),
            trace_id: "trace-1".to_string(),
        }
    }

    /// 组装用例 + 共享仓储 + 指定重试策略的共享设置（测试侧持 Arc 事后断言日志 / 配额）。
    fn harness_with_retry(
        adaptor: impl Fn(&Channel) -> Box<dyn ProviderAdaptor> + Send + Sync + 'static,
        retry: RetryPolicy,
    ) -> (
        ProxyRequestUsecase,
        Arc<InMemoryApiKeyRepository>,
        Arc<InMemoryChannelRepository>,
        Arc<InMemoryRequestLogRepository>,
    ) {
        let keys = Arc::new(InMemoryApiKeyRepository::new());
        let channels = Arc::new(InMemoryChannelRepository::new());
        let logs = Arc::new(InMemoryRequestLogRepository::new());
        let settings = Arc::new(RwLock::new(GatewaySettings {
            retry,
            ..GatewaySettings::default()
        }));
        let uc = ProxyRequestUsecase::new(
            Arc::clone(&keys) as Arc<dyn ApiKeyRepository>,
            Arc::clone(&channels) as Arc<dyn ChannelRepository>,
            Arc::clone(&logs) as Arc<dyn RequestLogRepository>,
            Box::new(adaptor),
            settings,
        );
        (uc, keys, channels, logs)
    }

    /// 默认重试策略（开启、不限）的组装：既有测试语义不变（ticket 07 逐个尝试）。
    fn harness(
        adaptor: impl Fn(&Channel) -> Box<dyn ProviderAdaptor> + Send + Sync + 'static,
    ) -> (
        ProxyRequestUsecase,
        Arc<InMemoryApiKeyRepository>,
        Arc<InMemoryChannelRepository>,
        Arc<InMemoryRequestLogRepository>,
    ) {
        harness_with_retry(adaptor, RetryPolicy::default())
    }

    fn single_ok_adaptor(
        response: ProviderResponse,
    ) -> impl Fn(&Channel) -> Box<dyn ProviderAdaptor> + Send + Sync {
        move |_: &Channel| Box::new(MockForwardAdaptor::new(Ok(response.clone()), Ok(vec![])))
    }

    /// 成功闭环：转发返回 200 + usage，日志落一条（is_retry=false），配额累加 total。
    #[tokio::test]
    async fn success_forwards_and_records_log_and_accumulates_quota() {
        let (uc, keys, _channels, logs) =
            harness(single_ok_adaptor(ok_response(200, Some(usage(10, 5, 15)))));
        let key = save_key(&keys).await;
        let a = save_channel(&_channels, "a", &["gpt-4o"], 0).await;

        let result = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect("success");
        let ProxySuccess::NonStream(resp) = result else {
            panic!("expected non-stream");
        };
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.usage, Some(usage(10, 5, 15)));

        let logs = all_request_logs(&*logs).await;
        assert_eq!(logs.len(), 1);
        let log = &logs[0];
        assert_eq!(log.api_key_id, Some(key.id));
        assert_eq!(log.channel_id, Some(a.id), "日志归属实际处理请求的渠道");
        assert_eq!(log.model, "gpt-4o");
        assert_eq!(log.upstream_model.as_deref(), Some("gpt-4o"), "未映射直传");
        assert_eq!(log.status_code, 200);
        assert_eq!(log.prompt_tokens, Some(10));
        assert_eq!(log.completion_tokens, Some(5));
        assert_eq!(log.total_tokens, Some(15));
        assert!(!log.is_stream);
        assert!(!log.is_retry);
        assert_eq!(log.error_message, None);

        let saved = keys
            .find_by_id(key.id)
            .await
            .expect("find key")
            .expect("found");
        assert_eq!(saved.quota.used, 15, "配额累加 total tokens");
    }

    /// 模型映射：请求 client_model 映射为 upstream_model，body["model"] 一并改写。
    #[tokio::test]
    async fn applies_model_mapping_to_upstream_request() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let value = Arc::clone(&received);
        let adaptor = move |_: &Channel| -> Box<dyn ProviderAdaptor> {
            Box::new(MockForwardAdaptor::with_recorder(
                Arc::clone(&value),
                Ok(ok_response(200, None)),
                Ok(vec![]),
            ))
        };
        let (uc, keys, channels, _logs) = harness(adaptor);
        let key = save_key(&keys).await;
        let mut c = save_channel(&channels, "a", &["gpt-4o"], 0).await;
        c.model_mappings = vec![ModelMapping {
            client_model: "chat".to_string(),
            upstream_model: "gpt-4o".to_string(),
        }];
        channels.save(&c).await.expect("save mapped channel");

        uc.execute(request(Some(&key.key), "chat", false))
            .await
            .expect("success");

        let sent = received.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].model, "gpt-4o", "映射后的上游模型");
        assert_eq!(sent[0].body["model"], "gpt-4o", "body 一并改写");
    }

    /// 无 Bearer → 401，不产生日志。
    #[tokio::test]
    async fn missing_bearer_returns_unauthorized_without_log() {
        let (uc, _keys, channels, logs) = harness(single_ok_adaptor(ok_response(200, None)));
        save_channel(&channels, "a", &["gpt-4o"], 0).await;

        let err = uc
            .execute(request(None, "gpt-4o", false))
            .await
            .expect_err("no bearer");
        assert!(matches!(err, ProxyError::Unauthorized));
        assert!(
            all_request_logs(&*logs).await.is_empty(),
            "认证失败不写日志"
        );
    }

    /// 配额超限 → 429，不产生日志。
    #[tokio::test]
    async fn quota_exceeded_returns_quota_error_without_log() {
        let (uc, keys, channels, logs) = harness(single_ok_adaptor(ok_response(200, None)));
        let mut k = sample_api_key();
        k.quota.limit = Some(100);
        k.quota.used = 100;
        let saved = k.clone();
        keys.save(&k).await.expect("save key");
        save_channel(&channels, "a", &["gpt-4o"], 0).await;

        let err = uc
            .execute(request(Some(&saved.key), "gpt-4o", false))
            .await
            .expect_err("quota");
        assert!(matches!(err, ProxyError::QuotaExceeded));
        assert!(all_request_logs(&*logs).await.is_empty());
    }

    /// 空模型名 → InvalidRequest，不查库不写日志。
    #[tokio::test]
    async fn empty_model_is_invalid_request() {
        let (uc, keys, _channels, logs) = harness(single_ok_adaptor(ok_response(200, None)));
        save_key(&keys).await;

        let err = uc
            .execute(request(None, "  ", false))
            .await
            .expect_err("empty model");
        assert!(matches!(err, ProxyError::InvalidRequest(_)));
        assert!(all_request_logs(&*logs).await.is_empty());
    }

    /// 模型不被任何启用渠道支持 → NoCandidateChannel，不转发不写日志。
    #[tokio::test]
    async fn no_candidate_channel_errors_without_log() {
        let (uc, keys, channels, logs) = harness(single_ok_adaptor(ok_response(200, None)));
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;

        let err = uc
            .execute(request(Some(&key.key), "claude-3", false))
            .await
            .expect_err("no candidate");
        assert!(matches!(err, ProxyError::NoCandidateChannel(_)));
        assert!(all_request_logs(&*logs).await.is_empty());
    }

    /// 禁用渠道不参与候选：适配器不会对禁用渠道被调用。
    #[tokio::test]
    async fn disabled_channel_excluded_from_candidates() {
        let (uc, keys, channels, logs) = harness(single_ok_adaptor(ok_response(200, None)));
        let key = save_key(&keys).await;
        let mut c = save_channel(&channels, "off", &["gpt-4o"], 0).await;
        c.enabled = false;
        channels.save(&c).await.expect("save disabled channel");
        save_channel(&channels, "on", &["gpt-4o"], 1).await;

        let result = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect("success");
        assert!(matches!(result, ProxySuccess::NonStream(_)));
        assert_eq!(all_request_logs(&*logs).await.len(), 1);
    }

    /// 5xx 可重试：候选 a 失败记一条失败日志，候选 b 成功记一条成功日志，is_retry=true，配额只算成功。
    #[tokio::test]
    async fn retries_next_candidate_after_http_5xx() {
        let adaptor = move |c: &Channel| -> Box<dyn ProviderAdaptor> {
            match c.name.as_str() {
                "a" => Box::new(MockForwardAdaptor::new(
                    Ok(ProviderResponse {
                        status_code: 500,
                        body: vec![],
                        usage: None,
                    }),
                    Ok(vec![]),
                )),
                "b" => Box::new(MockForwardAdaptor::new(
                    Ok(ok_response(200, Some(usage(10, 5, 15)))),
                    Ok(vec![]),
                )),
                _ => unreachable!("unexpected channel"),
            }
        };
        let (uc, keys, channels, logs) = harness(adaptor);
        let key = save_key(&keys).await;
        let a = save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;

        let result = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect("retry success");
        assert!(matches!(result, ProxySuccess::NonStream(_)));

        let logs = all_request_logs(&*logs).await;
        assert_eq!(logs.len(), 2, "每次失败/成功各一条日志");
        let fail = logs
            .iter()
            .find(|l| l.status_code != 200)
            .expect("失败日志");
        assert_eq!(fail.status_code, 500);
        assert_eq!(fail.channel_id, Some(a.id), "失败日志归属候选 a");
        assert!(fail.error_message.as_deref().unwrap().contains("500"));
        assert!(!fail.is_retry);
        let ok = logs
            .iter()
            .find(|l| l.status_code == 200)
            .expect("成功日志");
        assert!(ok.is_retry, "第二次尝试标记为重试");
        assert_eq!(ok.total_tokens, Some(15));

        let saved = keys.find_by_id(key.id).await.expect("find").expect("found");
        assert_eq!(saved.quota.used, 15, "失败尝试不计配额，只累加成功 usage");
    }

    /// 传输错误可重试：候选 a 转发报 ProviderError（状态码缺失记 502），候选 b 成功。
    #[tokio::test]
    async fn retries_next_candidate_after_transport_error() {
        let adaptor = move |c: &Channel| -> Box<dyn ProviderAdaptor> {
            match c.name.as_str() {
                "a" => Box::new(MockForwardAdaptor::new(
                    Err(ProviderError::Request("connection refused".into())),
                    Ok(vec![]),
                )),
                "b" => Box::new(MockForwardAdaptor::new(
                    Ok(ok_response(200, Some(usage(1, 1, 2)))),
                    Ok(vec![]),
                )),
                _ => unreachable!("unexpected channel"),
            }
        };
        let (uc, keys, channels, logs) = harness(adaptor);
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;

        uc.execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect("retry success");

        let logs = all_request_logs(&*logs).await;
        assert_eq!(logs.len(), 2);
        let fail = logs
            .iter()
            .find(|l| l.status_code == 502)
            .expect("失败日志");
        assert!(
            fail.error_message
                .as_deref()
                .unwrap()
                .contains("connection refused")
        );
    }

    /// 全部候选失败 → NoChannelAvailable（attempts = 候选数），每条失败各写一条日志。
    #[tokio::test]
    async fn all_candidates_fail_returns_no_channel_available() {
        let adaptor = |_c: &Channel| -> Box<dyn ProviderAdaptor> {
            Box::new(MockForwardAdaptor::new(
                Err(ProviderError::Request("boom".into())),
                Ok(vec![]),
            ))
        };
        let (uc, keys, channels, logs) = harness(adaptor);
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;

        let err = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect_err("all failed");
        match err {
            ProxyError::NoChannelAvailable {
                attempts,
                last_error,
                ..
            } => {
                assert_eq!(attempts, 2, "不超过候选渠道数");
                assert!(last_error.contains("boom"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(all_request_logs(&*logs).await.len(), 2);
    }

    /// 4xx 客户端错误不重试：候选 a 返回 400 原样透传给客户端，只记一条日志。
    #[tokio::test]
    async fn does_not_retry_on_client_error_4xx() {
        let adaptor = |_c: &Channel| -> Box<dyn ProviderAdaptor> {
            Box::new(MockForwardAdaptor::new(
                Ok(ProviderResponse {
                    status_code: 400,
                    body: b"bad".to_vec(),
                    usage: None,
                }),
                Ok(vec![]),
            ))
        };
        let (uc, keys, channels, logs) = harness(adaptor);
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;

        let result = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect("pass through");
        let ProxySuccess::NonStream(resp) = result else {
            panic!("expected non-stream");
        };
        assert_eq!(resp.status_code, 400, "4xx 原样透传，不换候选");
        assert_eq!(all_request_logs(&*logs).await.len(), 1, "未发生重试");
    }

    // ---- 流式 ----

    /// 流式成功：逐帧透传，流结束后聚合 usage 记账并写一条成功日志（is_stream=true）。
    #[tokio::test]
    async fn stream_success_accumulates_usage_and_logs_on_completion() {
        let events = vec![
            Ok(StreamEvent {
                data: b"data: {\"delta\":\"hi\"}\n\n".to_vec(),
                usage: Some(usage(3, 2, 5)),
            }),
            Ok(StreamEvent {
                data: b"data: [DONE]\n\n".to_vec(),
                usage: None,
            }),
        ];
        let adaptor = move |_: &Channel| -> Box<dyn ProviderAdaptor> {
            Box::new(MockForwardAdaptor::new(
                Ok(ok_response(200, None)),
                Ok(events.clone()),
            ))
        };
        let (uc, keys, channels, logs) = harness(adaptor);
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;

        let result = uc
            .execute(request(Some(&key.key), "gpt-4o", true))
            .await
            .expect("stream");
        let ProxySuccess::Stream(stream) = result else {
            panic!("expected stream");
        };
        let collected: Vec<_> = stream.collect::<Vec<_>>().await;
        assert_eq!(collected.len(), 2);
        assert!(collected.iter().all(|r| r.is_ok()), "透传帧无错");
        assert_eq!(
            collected[0].as_ref().unwrap().data,
            b"data: {\"delta\":\"hi\"}\n\n"
        );

        let logs = all_request_logs(&*logs).await;
        assert_eq!(logs.len(), 1);
        let log = &logs[0];
        assert!(log.is_stream);
        assert_eq!(log.status_code, 200);
        assert_eq!(log.prompt_tokens, Some(3));
        assert_eq!(log.completion_tokens, Some(2));
        assert_eq!(log.total_tokens, Some(5));
        assert_eq!(log.error_message, None);

        let saved = keys.find_by_id(key.id).await.expect("find").expect("found");
        assert_eq!(saved.quota.used, 5, "流式结束后累加 usage total");
    }

    /// 流式打开失败可重试：候选 a 打开流失败记失败日志，候选 b 成功。
    #[tokio::test]
    async fn stream_open_failure_retries_next_candidate() {
        let events = vec![Ok(StreamEvent {
            data: b"data: ok\n\n".to_vec(),
            usage: Some(usage(1, 1, 2)),
        })];
        let adaptor = move |c: &Channel| -> Box<dyn ProviderAdaptor> {
            match c.name.as_str() {
                "a" => Box::new(MockForwardAdaptor::new(
                    Ok(ok_response(200, None)),
                    Err(ProviderError::Request("stream open failed".into())),
                )),
                "b" => Box::new(MockForwardAdaptor::new(
                    Ok(ok_response(200, None)),
                    Ok(events.clone()),
                )),
                _ => unreachable!("unexpected channel"),
            }
        };
        let (uc, keys, channels, logs) = harness(adaptor);
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;

        let result = uc
            .execute(request(Some(&key.key), "gpt-4o", true))
            .await
            .expect("retry");
        let ProxySuccess::Stream(stream) = result else {
            panic!("expected stream");
        };
        let collected: Vec<_> = stream.collect::<Vec<_>>().await;
        assert_eq!(collected.len(), 1);

        let logs = all_request_logs(&*logs).await;
        assert_eq!(logs.len(), 2);
        let fail = logs
            .iter()
            .find(|l| l.status_code == 502)
            .expect("失败日志");
        assert!(
            fail.error_message
                .as_deref()
                .unwrap()
                .contains("stream open failed")
        );
        assert!(!fail.is_retry);
        let ok = logs
            .iter()
            .find(|l| l.status_code == 200)
            .expect("成功日志");
        assert!(ok.is_retry);
    }

    /// 流中途出错：写一条失败日志（携带已聚合的增量 usage）+ 按增量 usage 记账后终止
    /// （已发出的帧不受影响；US19 每次请求累加密钥配额，含未收尾的流）。
    #[tokio::test]
    async fn stream_midway_error_writes_failure_log_and_bills_partial_usage() {
        let events = vec![
            Ok(StreamEvent {
                data: b"data: {\"delta\":\"partial\"}\n\n".to_vec(),
                usage: Some(usage(3, 2, 5)),
            }),
            Err(ProviderError::Request("connection reset".into())),
        ];
        let adaptor = move |_: &Channel| -> Box<dyn ProviderAdaptor> {
            Box::new(MockForwardAdaptor::new(
                Ok(ok_response(200, None)),
                Ok(events.clone()),
            ))
        };
        let (uc, keys, channels, logs) = harness(adaptor);
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;

        let result = uc
            .execute(request(Some(&key.key), "gpt-4o", true))
            .await
            .expect("stream");
        let ProxySuccess::Stream(stream) = result else {
            panic!("expected stream");
        };
        let collected: Vec<_> = stream.collect::<Vec<_>>().await;
        assert_eq!(collected.len(), 2, "一帧成功 + 一个错误终止");
        assert!(collected[0].is_ok());
        assert!(matches!(collected[1], Err(ProxyError::Provider(_))));

        let logs = all_request_logs(&*logs).await;
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].status_code, 502);
        assert_eq!(
            logs[0].total_tokens,
            Some(5),
            "失败日志携带已聚合的增量 usage"
        );
        assert!(
            logs[0]
                .error_message
                .as_deref()
                .unwrap()
                .contains("connection reset")
        );

        let saved = keys.find_by_id(key.id).await.expect("find").expect("found");
        assert_eq!(
            saved.quota.used, 5,
            "流中断时已下发的增量 usage 同样累加配额"
        );
    }

    /// 构造「全部候选固定失败（可重试状态码）」的共享 recorder 适配器：记录每次 forward 调用。
    fn failing_recorder(
        recorder: Arc<Mutex<Vec<ChatRequest>>>,
        status: u16,
    ) -> impl Fn(&Channel) -> Box<dyn ProviderAdaptor> + Send + Sync + 'static {
        move |_: &Channel| {
            Box::new(MockForwardAdaptor::with_recorder(
                Arc::clone(&recorder),
                Ok(ok_response(status, None)),
                Ok(vec![]),
            ))
        }
    }

    /// 重试关闭：即使有多个候选也只试首个（全部失败时 attempts=1，无额外转发）。
    #[tokio::test]
    async fn retry_disabled_tries_only_first_candidate() {
        let recorder = Arc::new(Mutex::new(Vec::new()));
        let (uc, keys, channels, _logs) = harness_with_retry(
            failing_recorder(Arc::clone(&recorder), 500),
            RetryPolicy {
                enabled: false,
                max_retries: None,
            },
        );
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;

        let err = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect_err("all failed");
        assert!(
            matches!(err, ProxyError::NoChannelAvailable { attempts: 1, .. }),
            "重试关闭应只试首个候选: {err:?}"
        );
        assert_eq!(recorder.lock().unwrap().len(), 1, "只发生一次 forward");
    }

    /// 重试开启且限次：max_retries=n → 总尝试 n+1（首个 + n 次重试），不越界。
    #[tokio::test]
    async fn retry_limit_caps_attempts() {
        let recorder = Arc::new(Mutex::new(Vec::new()));
        let (uc, keys, channels, _logs) = harness_with_retry(
            failing_recorder(Arc::clone(&recorder), 429),
            RetryPolicy {
                enabled: true,
                max_retries: Some(1),
            },
        );
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;
        save_channel(&channels, "c", &["gpt-4o"], 2).await;

        let err = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect_err("all failed");
        assert!(
            matches!(err, ProxyError::NoChannelAvailable { attempts: 2, .. }),
            "max_retries=1 应恰好尝试 2 个候选: {err:?}"
        );
        assert_eq!(recorder.lock().unwrap().len(), 2, "首个 + 1 次重试");
    }

    /// 重试限次高于候选数：实际尝试数被候选数封顶（不产生越界 / 空迭代）。
    #[tokio::test]
    async fn retry_limit_higher_than_candidates_is_bounded() {
        let recorder = Arc::new(Mutex::new(Vec::new()));
        let (uc, keys, channels, _logs) = harness_with_retry(
            failing_recorder(Arc::clone(&recorder), 503),
            RetryPolicy {
                enabled: true,
                max_retries: Some(10),
            },
        );
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;

        let err = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect_err("all failed");
        assert!(
            matches!(err, ProxyError::NoChannelAvailable { attempts: 2, .. }),
            "尝试数不应超过候选数: {err:?}"
        );
        assert_eq!(recorder.lock().unwrap().len(), 2);
    }

    /// 重试开启且不限（默认）：逐个尝试全部候选——ticket 07 默认行为不被设置落地破坏。
    #[tokio::test]
    async fn retry_unlimited_tries_all_candidates() {
        let recorder = Arc::new(Mutex::new(Vec::new()));
        let (uc, keys, channels, _logs) = harness_with_retry(
            failing_recorder(Arc::clone(&recorder), 500),
            RetryPolicy {
                enabled: true,
                max_retries: None,
            },
        );
        let key = save_key(&keys).await;
        save_channel(&channels, "a", &["gpt-4o"], 0).await;
        save_channel(&channels, "b", &["gpt-4o"], 1).await;
        save_channel(&channels, "c", &["gpt-4o"], 2).await;

        let err = uc
            .execute(request(Some(&key.key), "gpt-4o", false))
            .await
            .expect_err("all failed");
        assert!(
            matches!(err, ProxyError::NoChannelAvailable { attempts: 3, .. }),
            "不限重试应尝试全部候选: {err:?}"
        );
        assert_eq!(recorder.lock().unwrap().len(), 3);
    }
}
