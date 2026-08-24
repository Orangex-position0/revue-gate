# 安全审计引擎

> **当前实现文档（v0.2.0）**：安全审计模块已经落地为请求日志的风险扩展。它在请求转发到上游之前扫描 OpenAI-compatible 请求体，生成 `AuditReport`，并把 `risk_level`、`audit_action`、`audit_report` 保存到 `request_logs`。MVP 决策见 [ADR 0001](../../adr/0001-security-audit-mvp.md)，需求规格见 [Spec-security-audit-mvp.md](../../spec/Spec-security-audit-mvp.md)。

## 1. 概述

### 为什么需要安全审计

AI 对话经网关转发时，请求中可能携带风险内容：

- **敏感数据泄露**：请求里混入 API key、私钥、数据库连接串、JWT、Bearer token、云凭据。
- **环境信息外泄**：AI IDE 自动收集的上下文可能包含 `.env`、`~/.ssh`、云凭据目录等本地敏感路径。
- **风险动作触发**：失控 agent 或恶意 prompt 可能诱导工具执行下载运行、读取敏感文件后外传、metadata IP 探测或内网访问。
- **日志二次泄露**：如果完整 request body 长期保存，安全审计本身也必须避免把完整 secret 写进结构化证据。

安全审计引擎回答「请求里有什么风险」。它不是独立合规审计平台，第一版只围绕请求转发链路建立一个小闭环：扫描、聚合、必要时阻断、写入本地日志、供 UI 复核。

### 当前落点

| 层次           | 文件                                                    | 职责                                                                            |
| -------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------- |
| domain         | `src-tauri/src/domain/security_audit.rs`                | 审计设置、运行期策略、scope builder、detector 规则、finding、report、聚合与截断 |
| domain         | `src-tauri/src/domain/settings.rs`                      | `GatewaySettings.audit` 设置快照                                                |
| domain         | `src-tauri/src/domain/request_log.rs`                   | request log 的 `risk_level`、`audit_action`、`audit_report` 字段                |
| usecase        | `src-tauri/src/usecases/proxy.rs`                       | 在上游 attempt 前执行审计；根据 report 放行或阻断；retry 复用 report            |
| usecase        | `src-tauri/src/usecases/log.rs`                         | 日志详情解析 request body，同时保留 audit report                                |
| infrastructure | `src-tauri/src/infrastructure/sqlite/request_log.rs`    | `AuditReport` JSON 序列化/反序列化                                              |
| migration      | `src-tauri/migrations/002_request_log_audit_fields.sql` | `request_logs` 增加 `risk_level`、`audit_action`、`audit_report`                |
| interface/http | `src-tauri/src/interface/http/handlers.rs`              | 将安全阻断映射为 `403 security_policy_blocked`                                  |
| frontend       | `src/types/index.ts`                                    | 对齐后端 camelCase 的 audit 类型                                                |
| frontend       | `src/pages/settings/SettingsPage.tsx`                   | 安全审计设置 UI                                                                 |
| frontend       | `src/pages/logs/LogsPage.tsx`                           | 风险 badge 与日志详情报告展示                                                   |

### 核心组成

安全审计引擎是一条固定顺序的组件流水线。MVP 下四个组件中的三个（Scope Builder、Detector、Aggregator）是 `src-tauri/src/domain/security_audit.rs` 单文件内的**逻辑边界**，不是独立的 crate/module；只有 Action Executor 落在 usecase 层。倒置的依赖方向不变：全部组件只依赖 domain 类型，不接触 DB/HTTP。

| 组件                    | 职责                                                                                                      | 代码落点                                                                  | 输出                                 |
| ----------------------- | --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------ |
| **Audit Scope Builder** | 把 OpenAI-compatible 请求体扁平化为可扫描的字符串候选，维护 JSON Pointer 路径、scope kind、字节上限与截断 | `build_audit_scope`                                                       | `AuditScope`                         |
| **Detector**            | 声明规则元数据（id、category、risk、action、confidence、suggested action），并按规则分发到确定性匹配函数  | `detector_rules`、`find_rule_matches`、各 `find_*` 函数                   | `AuditFinding` 列表                  |
| **Aggregator**          | 汇总 finding，产出 request-level 风险等级、分数、动作与报告，含组合升级与截断                             | `AuditReport::for_scope`、`report_action`、`has_critical_block_candidate` | `AuditReport`                        |
| **Action Executor**     | 执行 `report.action`，决定放行或阻断；阻断路径不调用上游、不记账                                          | `usecases/proxy.rs`、`interface/http/handlers.rs`                         | 转发 / `403 security_policy_blocked` |

流水线顺序：`Audit Scope Builder → Detector → Aggregator → Action Executor`，对应「请求链路」一节数据面顺序中的审计段。`AuditReport` 里携带的 `scanned_bytes` / `candidate_bytes` / `scan_byte_limit` / `truncated` 是 Scope Builder 的字节账本原样透传，供 UI 解释扫描覆盖率。

## 2. 请求链路

数据面顺序：

```text
HTTP parse
-> Bearer 预检查
-> ProxyRequestUsecase
-> AuthenticateRequestUsecase
-> ChannelSelector::select_channels
-> 第一候选 channel 的模型映射
-> AuditPolicy::from(settings.audit)
-> build_audit_scope
-> AuditReport::for_scope
-> Enforce + blockCritical + critical block candidate 时短路
-> provider forward / retry
-> usage 记账
-> request_logs 写入
```

审计发生在认证成功、候选 channel 已选出、第一计划上游模型已确定之后，并且早于任何 provider attempt。这样阻断日志可以记录 `planned_channel_id` 和 `planned_upstream_model`，但通过 `upstream_forwarded = false` 明确说明请求未到达上游。

HTTP 错误语义保持区分：

| 场景                 | HTTP 状态 | error type                |
| -------------------- | --------- | ------------------------- |
| 缺失或无效 Bearer    | `401`     | `authentication_error`    |
| quota exceeded       | `429`     | `rate_limit_error`        |
| 非法 JSON / 非法请求 | `400`     | `invalid_request_error`   |
| 无候选 channel       | `404`     | `not_found_error`         |
| 安全策略阻断         | `403`     | `security_policy_blocked` |
| 上游候选全部失败     | `502`     | `api_error`               |

## 3. 关键设计

### 设置与策略

`AuditSettings` 是持久化设置，`AuditPolicy` 是 usecase 在一次请求开始时从设置快照解析出的运行期策略。当前字段一致：

```rust
pub struct AuditSettings {
    pub enabled: bool,
    pub mode: AuditMode,
    pub block_critical: bool,
    pub scan_system_messages: bool,
    pub scan_byte_limit: u32,
    pub store_payload: bool,
    pub evidence_level: AuditEvidenceLevel,
}
```

默认值：

| 字段                   | 默认值      | 说明                                                      |
| ---------------------- | ----------- | --------------------------------------------------------- |
| `enabled`              | `false`     | 默认不审计；风险字段写 `null`，避免把未审计请求误当 clean |
| `mode`                 | `observe`   | 观察模式永不阻断，包括 critical finding                   |
| `block_critical`       | `true`      | 只在 `mode = enforce` 时允许 critical 阻断                |
| `scan_system_messages` | `false`     | 默认跳过 system message                                   |
| `scan_byte_limit`      | `64 * 1024` | 按完整 scope item 截断扫描输入                            |
| `store_payload`        | `true`      | 沿用原始 request body 日志行为；用户可关闭                |
| `evidence_level`       | `summary`   | 当前保存脱敏摘要，不保存完整命中值                        |

当前 UI 已暴露这些设置。`evidence_level = detailed` 已进入类型和设置，但 MVP 仍不保存 full match；它是后续证据粒度扩展的模型预留。

### Audit Scope

`AuditScope` 是 detector 的唯一输入视图，不复制完整 JSON：

```rust
pub struct AuditScope {
    pub items: Vec<AuditScopeItem>,
    pub scanned_bytes: u32,
    pub candidate_bytes: u32,
    pub scan_byte_limit: u32,
    pub truncated: bool,
}

pub struct AuditScopeItem {
    pub path: String,
    pub kind: AuditScopeKind,
    pub text: String,
}
```

`path` 使用 JSON Pointer，例如 `/messages/0/content`、`/tools/1/function/parameters/properties/private_key`。`AuditScopeKind` 当前包括：

```rust
pub enum AuditScopeKind {
    MessageContent,
    SystemMessageContent,
    ToolCallArguments,
    ToolName,
    ToolDescription,
    ToolSchemaString,
    ToolSchemaKey,
    TopLevelParam,
}
```

扫描顺序固定，保证字节上限下的覆盖优先级稳定：

1. `user` / `tool` message content，按原始顺序。
2. message 内 tool call arguments，按原始顺序。
3. tools function name / description。
4. tools function parameters 的 schema key 和字符串叶子。
5. 非基础 top-level params。
6. system message content，仅当 `scan_system_messages = true`。

Scope builder 递归提取字符串叶子，不把对象或数组整体序列化后扫描。对象 key 默认不扫；但 tool schema key 单独作为 `ToolSchemaKey` 扫描，因为参数名可能表达敏感意图，但其风险语义不同于用户实际上传的 secret。

top-level params 默认跳过基础协议和采样字段：

```text
messages, tools, model, stream,
temperature, top_p, max_tokens, max_completion_tokens,
presence_penalty, frequency_penalty, stop, seed, n,
logit_bias, logprobs, top_logprobs, response_format,
tool_choice, parallel_tool_calls, modalities, audio
```

其他字段中的字符串叶子会作为 `TopLevelParam` 扫描，例如 `metadata`、`user`、`reasoning`、`web_search_options`、`extra_body`、`provider_options` 和未知扩展字段。

当加入下一项会超过 `scan_byte_limit` 时，scope 停止追加后续 item，不截取半个 item；report 保存 `scanned_bytes`、`candidate_bytes`、`scan_byte_limit` 和 `truncated`。

### Detector 规则

当前 detector 全部是项目内置确定性规则，不引入第三方 secret 扫描 crate，不调用 LLM 判别，不解析 DNS。规则元数据在 `detector_rules(policy)` 中定义，匹配由 `find_rule_matches` 分发。

| 规则 ID                                      | 类别                 | 风险等级   | 默认 finding 动作 |
| -------------------------------------------- | -------------------- | ---------- | ----------------- |
| `credential.private_key`                     | `credential`         | `critical` | `block` 或 `warn` |
| `credential.provider_api_key`                | `credential`         | `critical` | `block` 或 `warn` |
| `credential.aws_access_key_id`               | `credential`         | `critical` | `block` 或 `warn` |
| `credential.aws_secret_access_key`           | `credential`         | `critical` | `block` 或 `warn` |
| `credential.gcp_oauth_token`                 | `credential`         | `critical` | `block` 或 `warn` |
| `credential.azure_storage_connection_string` | `credential`         | `critical` | `block` 或 `warn` |
| `credential.database_url`                    | `credential`         | `critical` | `block` 或 `warn` |
| `credential.bearer_token`                    | `credential`         | `critical` | `block` 或 `warn` |
| `credential.jwt`                             | `credential`         | `critical` | `block` 或 `warn` |
| `credential.local_revue_key`                 | `credential`         | `high`     | `warn`            |
| `sensitive_path.local_secret`                | `sensitivePath`      | `high`     | `warn`            |
| `unicode.zero_width`                         | `unicodeObfuscation` | `medium`   | `warn`            |
| `unicode.bidi_control`                       | `unicodeObfuscation` | `medium`   | `warn`            |
| `prompt_injection.phrase`                    | `promptInjection`    | `medium`   | `warn`            |
| `pii.email`                                  | `pii`                | `low`      | `logOnly`         |
| `pii.phone`                                  | `pii`                | `low`      | `logOnly`         |
| `tool.downloadExecute`                       | `ToolRisk`           | `high`     | `warn`            |
| `tool.powershellDownloadExecute`             | `ToolRisk`           | `high`     | `warn`            |
| `tool.sensitiveFileExfiltration`             | `ToolRisk`           | `critical` | `block` 或 `warn` |
| `network.privateLiteral`                     | `NetworkRisk`        | `high`     | `warn`            |
| `network.metadataIp`                         | `NetworkRisk`        | `critical` | `block` 或 `warn` |
| `network.webhookOrTunnelHost`                | `NetworkRisk`        | `high`     | `warn`            |
| `network.ipProbeHost`                        | `NetworkRisk`        | `high`     | `warn`            |

表中的 `block` 或 `warn` 由 `block_critical` 决定：`block_critical = true` 时 critical 规则的 finding 动作为 `Block`，否则为 `Warn`。但 request-level 是否真的阻断还要经过 `AuditMode::Enforce` 判断。

#### 覆盖补丁（已实现 2026-08-20）

在 MVP 规则之上做的高价值、低误报、直接防泄露的纯表扩充，不改 schema：

- **provider 凭证前缀**：`find_provider_api_keys` 前缀表在既有 `sk-` / `xai-` / `anthropic-` / `AIza` 之外，新增 `ghp_`（GitHub PAT）与 `xoxb-`（Slack token），并入 `credential.provider_api_key`（继承 Critical/Block）。
- **公网 IP 探测域名**：新增 **`network.ipProbeHost`**（High/Warn），字面匹配 `ifconfig.me` / `ipinfo.io` / `ipify.org` / `icanhazip.com` / `api.ipify.org` / `checkip.amazonaws.com`。独立于 `webhookOrTunnelHost`，因为 IP 探测 ≠ 数据外传——探测解析自身外网地址（如 `curl ifconfig.me`）是 metadata 式外泄前兆。
- **敏感路径 needles**：`find_sensitive_paths` 在既有 `.env` / `~/.ssh` / 云凭据目录之外，新增 `.npmrc` / `.netrc` / `.git-credentials` / `.pypirc`，并入 `sensitive_path.local_secret`（沿用 High/Warn）。

排查后的其它候选（命名敏感字段 `secret_key` / `cookie` / `sessionid=` / `access_key`，以及 Git 信息读取 `git remote` / `git config` / `gh auth token`）是高误报源，缺乏白名单抑制时会拉低告警可信度，**明确暂缓**，与 Rule Registry / 自定义黑白名单绑定后再做。

### Finding 与证据

`AuditFinding` 是 detector 输出，也是 `AuditReport.findings` 中保存的结构：

```rust
pub struct AuditFinding {
    pub rule_id: String,
    pub category: String,
    pub risk_level: RiskLevel,
    pub action: AuditAction,
    pub confidence: AuditConfidence,
    pub scope_kind: AuditScopeKind,
    pub path: String,
    pub redacted_excerpt: String,
    pub match_hash: String,
    pub suggested_action: String,
}
```

`redacted_excerpt` 在命中位置前后各保留有限上下文，并用 `[redacted:<rule_id>]` 替换真实命中片段。它不保存完整 secret。

`match_hash` 当前使用 rule id 和归一化命中值计算稳定的 64-bit 十六进制 hash。它用于本地去重和测试稳定性，不是加密保护；低熵内容仍可能被猜测，所以日志展示不能把它当作安全承诺。

单条规则最多保留 20 条 finding；报告最多保留 50 条 finding。超出时仍保留 `total_findings` 和 `findings_truncated`。

### Audit Report 与聚合

`AuditReport` 是 request-level 风险投影：

```rust
pub struct AuditReport {
    pub mode: AuditMode,
    pub risk_level: RiskLevel,
    pub risk_score: u8,
    pub action: AuditAction,
    pub findings: Vec<AuditFinding>,
    pub total_findings: usize,
    pub findings_truncated: bool,
    pub scanned_bytes: u32,
    pub candidate_bytes: u32,
    pub scan_byte_limit: u32,
    pub truncated: bool,
    pub evidence_level: AuditEvidenceLevel,
    pub upstream_forwarded: bool,
    pub planned_channel_id: Option<Uuid>,
    pub planned_upstream_model: Option<String>,
}
```

聚合规则：

- 无 finding 时生成 `Clean / Allow` 报告。
- 风险等级取 finding 的最高 `RiskLevel`；存在 critical block candidate 时强制为 `Critical`。
- 固定评分映射：`clean = 0`、`info = 10`、`low = 25`、`medium = 50`、`high = 75`、`critical = 100`。
- request-level 动作：`Clean -> Allow`，`Info/Low -> LogOnly`，`Medium/High/Critical -> Warn`。
- 只有 `mode = Enforce`、`block_critical = true`、且存在 critical block candidate 时，request-level 动作为 `Block`。

组合升级在聚合阶段完成，detector 本身不做跨类别判断：

```text
ToolRisk + NetworkRisk + sensitivePath => Critical block candidate
ToolRisk + NetworkRisk + credential => Critical block candidate
```

另外，critical 且 finding action 为 `Block` 的规则天然是 block candidate，例如 provider/private/cloud credential、database URL、Bearer token、JWT、metadata IP 和敏感文件外传。

### 执行动作

MVP 执行四种 request-level 动作中的三种：

| 动作      | 当前行为                                                   |
| --------- | ---------------------------------------------------------- |
| `Allow`   | 继续转发，写 clean report                                  |
| `LogOnly` | 继续转发，写低风险报告                                     |
| `Warn`    | 继续转发，写告警报告                                       |
| `Block`   | 不调用上游，写本地日志，返回 `403 security_policy_blocked` |

`Redact` 和 `Confirm` 已保留在 `AuditAction` 类型中，但当前聚合器不会产出，也不会执行 payload 改写或桌面确认。

Block 路径保证：

- 不调用 provider adaptor。
- 不累计 provider usage。
- 写本地 request log。
- `status_code = 403`。
- `error_message = "blocked by security policy"`。
- `audit_report.upstream_forwarded = false`。
- 保留计划渠道和计划上游模型用于解释。

## 4. 数据与日志

### 与请求日志的关系

安全审计结果复用 `request_logs`，不另建 `audit_events`：

- `risk_level`：列表和筛选可直接读取的请求级风险等级。
- `audit_action`：请求级处置动作。
- `audit_report`：结构化 JSON，保存 finding、扫描字节、截断状态、planned upstream、upstream forwarded 等详情。

`AuditPolicy.enabled = false` 时，这三个字段写 `null`。只有启用审计且扫描完成但无 finding 时，才写 `Clean / Allow / empty findings`。

`store_payload = false` 时，`request_body = None`，但 `audit_report` 仍保存。日志详情在没有 request body 时显示空 conversation / params，同时仍展示风险报告；这保证了隐私和安全复核之间的基本平衡。

### 日志体脱敏存储（Payload Redaction）

> 已实现（2026-08-20）。语义见 CONTEXT.md「Payload Redaction」；MVP 决策见 [ADR 0001](../../adr/0001-security-audit-mvp.md)。

`store_payload = true` 时，落库前的 `request_body` 会做一次脱敏，防止完整 secret 以原始明文长期留在本地日志（「日志二次泄露」红线）。

**实现**：落库前对 body 递归脱敏，直接复用 detector 算法（`build_audit_scope` + `detect_findings`）拿命中 `(path, MatchSpan)`，把匹配片段逐字节替换为 `[redacted:<rule_id>]`，保留 JSON 结构与未命中内容。这比另写一套正则脱敏更省，且与 finding 的 `match_hash` 天然一致。代码落在 `domain/security_audit.rs` 的 `redact_body`，usecase 在 `proxy.rs` 计算一次存入 `AttemptContext`、retry 复用。

**关键契约**：

- **只脱敏落库副本，转发 body 永不变**（`Redact` action 仍延后，语义同 ADR 0001）。
- **脱敏与 `enabled` 解耦**：只要 `store_payload = true` 就脱敏，无论审计开关；`enabled = false` 时脱敏结果不计入 report，仅作落库防线。
- 重叠 / 相邻 span 合并后 last→first 逐字节改写，避免偏移漂移。
- **覆盖契约**：脱敏覆盖 = detector 扫描覆盖（detector 漏网、`scan_system_messages = false` 未扫系统消息、超 `scan_byte_limit` 未扫部分原样落库）。

### 与 retry 的关系

审计对象是 logical request，不是单次上游 attempt。`ProxyRequestUsecase` 在进入 retry loop 前生成一次 `AuditReport`：

- 放行后如果第一个候选失败，后续 retry attempt 复用同一份 report。
- 每条 attempt log 都带相同 `audit_report` 和同一 `trace_id`。
- 统计风险请求时应按 `trace_id` 去重，避免 retry 放大风险请求数。
- 落库脱敏（见「[日志体脱敏存储（Payload Redaction）](#日志体脱敏存储payload-redaction)」）在 `execute` 算一次、存入 `AttemptContext`、所有 retry attempt 复用同一份已脱敏 body；转发 payload 永不被改写。

## 5. 前端展示

设置页已经暴露：

- 启用请求审计。
- 观察 / 拦截模式。
- 拦截 critical 风险。
- 扫描 system messages。
- 扫描字节上限。
- 保存原始请求体。
- 证据级别。

日志列表显示风险 badge：`riskLevel` 或 `auditAction` 为空时显示“未审计”，否则显示 `<Risk> / <Action>`。

日志详情显示：

- 模式、风险分、最终动作。
- 是否已转发上游。
- 计划渠道、计划模型。
- 扫描截断、扫描字节、扫描上限。
- finding 数量、finding 截断状态。
- 证据级别。
- 每条 finding 的 rule id、category、risk level、action、scope kind、JSON Pointer path、confidence、suggested action、redacted excerpt。

当 `upstreamForwarded = false` 时，UI 明确提示请求被本地安全策略拦截，未到达上游 provider。

## 6. 质量与演进

### 测试覆盖

当前测试围绕公开 seam 验证行为：

- Usecase：覆盖 disabled、clean、log-only、warn、block；阻断时 provider 调用次数为 0，usage 不累计。
- HTTP：覆盖 `403 security_policy_blocked`，并区分 auth、quota、invalid request、no candidate、upstream failure。
- Log detail：覆盖 request body 存在和缺失时都能展示 audit report。
- Scope builder：覆盖 JSON Pointer、scope kind、扫描顺序、system message 开关、top-level param 规则、scan-byte 截断。
- Detector：每个 MVP detector 覆盖 hit、miss、boundary、false-positive 样本。
- Aggregator：覆盖动作优先级、observe/enforce、critical blocking、组合升级、score mapping、finding 截断。
- Retry：证明多次 attempt 复用同一份 risk report。
- Bounded-size：长 prompt 和大量 findings 保持有界。

### 安全红线

- **默认不阻断**：只有用户启用审计并切到 enforce，critical block candidate 才可能阻断。
- **阻断是本地策略拒绝**：阻断请求不转发、不消耗 provider quota、不记录 provider usage，HTTP 返回 `403 security_policy_blocked`。
- **证据默认脱敏**：日志只保存 `redacted_excerpt` 和 `match_hash`，不保存完整命中值。
- **审计信息只留本地**：风险字段进入本地 `request_logs`，不会发给上游 provider。
- **无 DNS 解析**：网络检测只看字面 IP 和 hostname 模式，避免额外网络 IO、缓存、超时和隐私问题。
- **响应审计暂缓**：MVP 不做响应侧同步扫描，也不做 streaming chunk 增量审计。

### 后续版本

下一版可以在当前 JSON report shape 之上拆出更独立的安全审计中心：

- Rule Registry：规则启停、版本、类别、默认等级、按 key/model/channel/scope 生效。
- Evidence Store：独立 retention、误报复核、hash 去重、JSONL 导出。
- Audit Log / `audit_events`：将多条 finding 拆成独立事件，支持跨请求筛选和报表。
- Redact 后转发：按 JSON Pointer 改写 request payload，再统一复用到所有 retry attempt。
- Confirm：桌面端对高风险请求进行显式确认。
- 响应审计：检查模型回显 secret、危险命令或泄露系统提示。
- 外部 guardrail provider：在用户接受网络调用、延迟和隐私成本后再接入。

### 改进计划（待办）

#### 明确暂缓

| 项                                                    | 暂缓原因                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| ----------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **请求级 body hash（可选）**                          | 存 `body_hash`（SHA-256）与 `body_len` 用以日志完整性核对。本地单用户网关上 ROI 有限——hash 只测完整性、不加密、不防篡改者（攻击者可同时改 body 与 hash），真实使用场景（日志被静默改 / 损坏 / 备份校验）目前基本不存在。**触发条件：出现真实完整性核对需求再加**。若将来补，**契约：必须 hash 脱敏后的落库 `request_body`（见「[日志体脱敏存储](#日志体脱敏存储payload-redaction)」），绝不 hash 原始请求体**——否则会把原始 secret 的指纹留在库中，正是脱敏要避免的二次泄露变体（SHA-256 对低熵 secret 可字典破解）。取值：`body_hash` = SHA-256(脱敏后落库 body)、`body_len` = 其长度。finding 级 `match_hash` 已覆盖去重。 |
| 规则入库 + 播种时机（WaLiAPI 25 条种子表）            | 归 "[后续版本](#后续版本) 的 Rule Registry / 安全审计中心；单用户本地工具手动调规则频率低                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| 自定义黑白名单                                        | **WaLiAPI 的该功能是死代码**（`apply_custom_rules` / `is_whitelisted` 零调用点，未接入运行时扫描），没有可照搬的现成实现；唯一 ROI 是"白名单抑制误报"，等误报真实出现再做                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| 全量扫描预算（字节/节点/深度/耗时，超限 fail-closed） | 已按 item 截断 + serde_json 递归上限兜底；单用户本地网关的威胁模型不是恶意 DoS                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| 响应侧 / streaming delta 扫描                         | 与"安全红线：响应审计暂缓"一致                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
