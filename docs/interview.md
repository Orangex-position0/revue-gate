# 项目经历（简历）

> 格式约定：每个项目一个 `###` 小节，依次为「技术栈标签 → 项目描述 → 项目亮点」。技术名词保留英文，其余用中文。项目亮点写「设计决策 / 两难取舍 + 为什么这么选 + 带来什么」，避免套设计模式名词或落到代码级细节，给面试官留追问空间。

### revue-gate — 本地 LLM API 网关桌面应用

`Rust` `Tauri` `Axum` `SQLite` `sqlx` `React` `TypeScript` `DDD`

- **项目描述**：
  独立设计并实现的本地 LLM API 网关桌面应用，将 OpenAI / Claude / Gemini / DeepSeek / 自定义 OpenAI-compatible 等多供应商聚合到一个统一 OpenAI 兼容端点，向下游 AI 客户端（ChatBox / NextChat / OpenAI SDK 等）透明提供认证、渠道调度、模型映射、转发、配额记账与请求日志，上游密钥永不暴露给下游。
- **项目亮点**：
  - **协议直通与转换的取舍**：5 类供应商中只有 Claude / Gemini 需要真协议转换、其余走 OpenAI-compatible 直通——直通避免逐字段搬运的语义丢失，转换保证对下游「始终像 OpenAI」，协议差异收敛在适配器层、新增供应商核心零改动；
  - **流式转发的记账时机**：SSE 响应头发出后状态码不可逆、usage 常只在最后一帧，中途断开时「上游已计费、下游没收到完整内容」——把记账延迟到流结束统一落账、中途断开按已下发增量记账并记失败日志，保证上下游账目一致；
  - **失败重试的副作用控制**：只对上游限流 / 服务端错误换渠道重试、客户端错误原样透传；配额只在成功那次累加、失败只记日志，记账失败不回抛错误，避免重试导致重复扣费；
  - **转发面与管理面分离**：对外 HTTP 只管 OpenAI 兼容转发，管理能力（写库、启停服务）走进程内命令、不暴露到对外端口——少一个可被攻击的管理端口，也不让管理逻辑污染转发协议面；
  - **本地单机定位的取舍**：同类产品多是多租户服务端网关，本项刻意不做账号体系 / 计费 / 集群，换取「装完即用、数据不出本机、上游密钥只留在本机」；
  - **核心编排的可测试性**：让网关调度闭环脱离真实网络与数据库即可单测，用内存 mock 覆盖每条编排分支，SSE 这类网络时序行为再单独走真实 HTTP 验证。

# 项目可能的面试题

## 1. 前端页面中要聚合查询多个信息，后端如何编写？如何优化这部分设计？

场景：类似电商页面的商品查询，前端需要同时显示多个查询信息

难点：

- 多个信息源若由前端串行请求，会产生瀑布式延迟（总耗时 ≈ 各请求之和），首屏慢。
- 接口粒度难平衡：太细 → 往返多、开销大、还容易出现 N+1；太粗 → 一个巨型接口耦合高、难复用。
- 一次聚合多块数据时，某一块失败不应拖垮整个页面，需要失败隔离与降级。

解决方式：

- 后端提供**聚合接口**（BFF / 聚合端点）：一次请求返回页面所需的多块数据，把多次往返压成一次。
- 聚合接口内部**并发查询**多个数据源（async 并行 future），总耗时 ≈ 最慢的那个，而非各查询之和。
- 每个子查询**独立容错**：失败用默认值/降级，不影响其他块；慢查询可拆出去做懒加载。
- 项目落地：revue-gate 的 Dashboard 由 `GetStatsUsecase` 一次扫描轻量投影行（`stat_rows`，不含请求体），在内存里同时算出今日/累计请求数、token、平均延迟、可用率、7 天趋势，前端一次 `invoke` 拿到整个 `StatsSnapshot`。

---

# 面试问答（设计向）

> 以下问题按主题分组，`> **参考回答**` 给出可讲的要点。标注「诚实短板」的是追问时建议主动承认的已知边界——比硬撑更有说服力。

## 一、架构分层

### Q1：为什么用 Clean Architecture + DDD 分层？一个本地单机网关，是不是过度设计？

> **参考回答**：
>
> - 分层不是为「将来会变」预留，而是为**可测试性**和**依赖方向**服务。核心闭环（认证 → 选渠道 → 映射 → 转发 → 记账 → 日志）是有明确输入输出的编排逻辑，把它和 HTTP、SQLite、Reqwest 解耦，才能用内存 mock 仓储测到每条分支（Seam A）。
> - 收益：`domain` 层零技术依赖（不出现 `sqlx`/`reqwest`/`axum`/`tauri`），`ChannelSelector`、`QuotaPolicy` 这类纯判断成了无副作用函数，单测不起任何服务。
> - 成本是诚实的：三个聚合根（Channel / ApiKey / RequestLog）就做了三个仓储 trait，样板比「直接调 sqlx」多。但换来 `ProxyRequestUsecase` 不碰真实网络，全注入 `Arc<dyn Trait>`。
> - 结论：对「多供应商协议差异 + 编排复杂」这类问题，分层划算；若只是直通转发器，我不会上这套。

### Q2：依赖倒置具体怎么实现？domain 怎么定义接口又零依赖？

> **参考回答**：
>
> - 仓储 trait（如 `ChannelRepository`）定义在 `domain/channel.rs`，只依赖领域实体和 `RepositoryError`；sqlx 实现在 `infrastructure/sqlite/`。用例层只依赖 trait——依赖方向 `interface → usecases → domain ← infrastructure` 全部指向内层。
> - 供应商适配器同理：`ProviderAdaptor` trait 定义在 `domain/provider.rs`，方法签名只返回领域类型（`ProviderResponse`、`Usage`），**不暴露 reqwest/axum 类型**；`forward_stream` 返回 OpenAI 兼容的 SSE 字节流，数据面只透传。
> - 生产在 `lib.rs` 装配时用 `adaptor_for(channel_type)` 闭包解析具体适配器，测试用 `MockForwardAdaptor` 替换——「按接口注入」的落点。

## 二、供应商协议隔离

### Q3：`ProviderAdaptor` trait 怎么设计？新增一个供应商要改哪几处？

> **参考回答**：
>
> - trait 只有五个方法：`channel_type` / `default_models` / `default_base_url` / `test`（连通性探测）/ `forward`（非流式）/ `forward_stream`（流式）。
> - 关键设计点：**业务错误不进 `ProviderError`**。`ProviderError` 只表达「未配置 / 传输失败 / 响应解析失败」；上游 4xx/5xx 由 `forward` 以 `status_code + body` 原样返回，**重试决策留给用例层**（`is_retryable`）。适配器只关心「怎么讲对方的话」，不关心「要不要换渠道」。
> - 协议差异收敛在两处：`Usage` 归一化（OpenAI `usage` / Claude `input|output_tokens` / Gemini `usageMetadata` → 统一 `prompt/completion/total_tokens`），以及响应体的 OpenAI 兼容转换。
> - 新增供应商只改 `infrastructure/providers/`：新建一个实现 + 在 `adaptor_for` 加一个 `ChannelType` 分支，domain / usecases / interface 零改动。OpenAI / DeepSeek / Custom 共用 OpenAI-compatible 直通，只有 Claude / Gemini 要真转换。

### Q4：为什么 OpenAI / DeepSeek / Custom 直通，Claude / Gemini 要协议转换？

> **参考回答**：
>
> - 前者本来就兼容 OpenAI 请求/响应格式，网关「改一下 `body["model"]` 就透传」，转换成本为零，也避免逐字段搬运引入语义丢失。
> - Claude / Gemini 的请求体结构（messages 的 role/content、system 提示位置）和响应体（usage 字段名、content 结构）与 OpenAI 不同，必须双向映射才能对下游保持「看起来就是 OpenAI」的承诺。
> - 直通还带来一个好处：上游返回的非 JSON 错误（HTML、纯文本）也能原样转发，不丢信息。

## 三、渠道调度

### Q5：`ChannelSelector` 怎么选渠道？优先级和权重有什么区别？

> **参考回答**：
>
> - 四步：① 只保留 `enabled`；② 模型匹配——`models` 含请求模型，或 `model_mappings` 命中 `client_model`，或 `models` 为空（视为不限模型）；③ 按 `priority` **升序分组**（值小优先，组间硬排序）；④ 组内按 `weight` **加权随机不放回**（权重大优先被选中，0 权重排组内最后作兜底）。
> - `priority` 语义是「主备」：高优先级组先试，失败才落到下一组。
> - `weight` 语义是「分摊」：同优先级多渠道时按权重比例随机分流。结果是一个有序 failover 队列，proxy 逐个尝试（每请求洗一次牌，重试只是消费同一队列，无状态）。负权重在命令层被拒绝，调度层仍 `max(0)` 兜底防脏数据。

### Q6：模型映射（model mapping）解决什么问题？在哪里生效？

> **参考回答**：
>
> - 下游统一用一个名字（如 `chat`），各上游实际模型名不同（`gpt-4o` / `claude-3-5`），映射表 `client_model → upstream_model` 让下游感知不到上游差异。
> - 生效于 `apply_mapping`（`usecases/proxy.rs`）：命中就改写 `body["model"]` 为 `upstream_model`，`ChatRequest.model` 保持一致；未命中原样直传。
> - 两个边界：`ChannelSelector::supports` 也认 `model_mappings.client_model`（否则映射后的名字选不中渠道）；`/v1/models` 返回「客户端可见模型」的合并去重，和调度一致。

## 四、配额记账

### Q7：`QuotaPolicy` 为什么判定 `used >= limit` 超限，而不是 `used > limit`？

> **参考回答**：
>
> - 配额语义是「可用额度」，`used == limit` 表示额度刚好用完，再放一个请求必然超限，所以**边界值算超限**，对应 HTTP 429 让下游立刻感知。
> - `limit = None` 表示不限额，用 `Option` 而非 `u64::MAX`，语义清晰且避开大数边界。
> - 纯函数（`QuotaPolicy::check(&Quota) -> QuotaCheck`），单测覆盖 `None` / `< limit` / `== limit` / `> limit` 四条。

### Q8：并发下配额会不会超卖？check 和 accumulate 之间有竞态吗？

> **参考回答**：
>
> - **诚实短板**：有。当前是「认证时 `find_by_key` 判 `used < limit`，成功后 `AccumulateUsageUsecase` 再 `find_by_id → used + tokens → save`」，是**读-改-写 + check-then-act**，无原子性。并发请求可能同时通过检查、同时累加，导致 `used` 略超 `limit`（甚至丢更新）。
> - 为什么接受：单用户本地网关，并发量级极低，配额是软限额而非计费准确性要求。
> - 升级路径：单条原子 SQL `UPDATE api_keys SET quota_used = quota_used + ? WHERE id = ? AND (quota_limit IS NULL OR quota_used + ? <= quota_limit)`，用受影响行数判断超限，check 与 accumulate 合为一步。

## 五、SSE 流式转发

### Q9：SSE 流式怎么透传？`[DONE]` 收尾为什么重要？

> **参考回答**：
>
> - 用例层 `forward_stream` 返回 `BoxStream<Result<StreamEvent, ProviderError>>`，数据面 `chat_completions` 用 `Body::from_stream` 逐帧写进 `text/event-stream`，**不改写增量内容**，实时下发。
> - 记账/日志在流结束才做：包装流逐帧累加 usage，流正常结束（`None`）或中途出错（`Err`）时落账。流式 usage 常在最后一帧才带全量，逐帧 `accumulate` 等价于取最后一帧。
> - `[DONE]` 是 OpenAI SSE 终止哨兵，下游 SDK（ChatBox / NextChat / OpenAI SDK）靠它判定本轮结束。OpenAI 直通时上游返回、原样透传；Claude/Gemini 转换适配器自己合成 `data: [DONE]\n\n`，否则下游会一直等。

### Q10：流中途上游断开，客户端看到什么？网关怎么记账？

> **参考回答**：
>
> - 响应头已发出，状态无法改成 5xx，客户端看到「已下发帧原样透传后截断、**没有 `[DONE]`**」——测试明确断言 `!text.contains("[DONE]")`。
> - 网关侧：流中断写一条 **502 失败日志**（携带已聚合增量 usage），并把**已下发增量 usage 也计入配额**，避免上游已算钱、下游没记账。
> - **诚实短板**（代码有 `ponytail:` 注释）：若**下游客户端**中途断开且流没被完全消费，记账/日志不触发。v0.1 接受，升级需 cancellation-aware 包装器在 drop 时兜底落账。

## 六、失败重试

### Q11：什么错误会重试？重试会不会重复扣配额？

> **参考回答**：
>
> - `is_retryable`：**429（上游限流）和 5xx（服务端错误）**换下一个候选；**4xx 是客户端/模型错误**，重试无意义，原样透传。传输错误（连接拒绝等）也算可重试。
> - 每次尝试写**一条独立日志**，`is_retry` 标记第几次；重试次数受设置中心 retry 策略控制：关闭→只试首个，开启+限次→`max_retries + 1`，开启+不限→试完全部候选，`min(次数, 候选数)` 封顶。
> - 配额**只在成功那次累加**（测试断言「失败尝试不计配额」）。记账/日志 best-effort：上游已处理完，记账失败绝不能回抛 5xx 导致重复扣费，只 `tracing::warn!`。

## 七、双面架构

### Q12：为什么数据面（Axum）和控制面（Tauri Commands）分开？前端为什么不直连数据面 HTTP？

> **参考回答**：
>
> - 职责不同：数据面对外暴露 OpenAI 兼容接口，是「被下游 AI 客户端调用」的网关；控制面是「给本机 React 管理界面用」的增删改查。分开后数据面的 `/v1/*` 协议面不被管理逻辑污染，控制面鉴权/语义独立。
> - 前端不直连数据面，因为**控制面需要的能力（写 SQLite、读设置、启停服务）不该通过对外 HTTP 暴露**。走 Tauri `invoke` 命令，渲染进程直接和 Rust 后端通信，少一层监听端口，也少了「谁都能连管理端口」的攻击面。
> - 两条面最终在 `interface` 层汇合到同一套 `usecases → domain`，业务核心只有一份。

## 八、可观测性

### Q13：`x-request-id` 怎么贯穿全链路？

> **参考回答**：
>
> - 最外层 `trace_id_middleware`：读请求头 `x-request-id`，有则复用、无则 `Uuid::now_v7()` 生成，塞进 request extensions，并在响应头原样回传——客户端能凭它对账。
> - `TraceLayer` 的 `MakeSpan`（`TraceIdSpan`）把 trace_id 带进 tracing span，`OnResponse` 在响应后输出结构化日志（status + latency + trace_id）。
> - 请求日志表也存 trace_id，所以「一条请求」能在结构化日志、request_logs 表、下游回传头三处对齐。测试专门验证结构化日志输出包含 trace_id。

## 九、测试策略

### Q14：Seam A / Seam B 双缝测试是什么？为什么 SSE 必须走真实 HTTP？

> **参考回答**：
>
> - **Seam A**（主）：`test_support.rs` 内存 mock 仓储 + `MockForwardAdaptor`，覆盖用例编排每条分支（认证失败不写日志、5xx 重试、4xx 不重试、流中断记账……），快速无 IO。
> - **Seam B**（补充）：`tower::ServiceExt::oneshot` + 真实 sqlx 内存库 + 真实路由，验证路由存在、Bearer 401、配额 429、`/v1/models` 合并去重、SSE 含 `[DONE]`。补 Seam A 测不到的集成点：中间件、状态码映射、真实 reqwest 转发。
> - SSE **必须**真实 HTTP：流式的「逐帧透传、`[DONE]` 收尾、中途断开截断」本质是网络时序行为，mock stream 测不出「响应头已发出后状态无法变更」这类约束。seam B 里真起了 mock 上游，甚至用 `sleep` 造「先发几帧再 RST」的中断场景。

## 十、同类产品对比

### Q15：同类产品（one-api / new-api / LiteLLM / WaLiAPI）怎么做的？你做了什么不同/改进？

> **参考回答**（先定位差异，再讲选择理由）：
>
> - **one-api / new-api**：面向多租户的**服务端**网关，跑服务器上，带账号体系、计费、兑换码、多用户隔离，解决「一群人共享一批 key」。
> - **LiteLLM**：功能最全的**统一网关库/服务**，覆盖几十家供应商 + 大量企业特性，但重、要部署、面向生产集群。
> - **revue-gate 差异**：定位**本地桌面应用**——单用户、本地 SQLite、无账号体系、无遥测。刻意**不做**多租户/计费/集群，换「装完即用、数据不出本机、上游密钥只留在本机」。
> - 架构上的明确选择：
>     1. `ProviderAdaptor` trait 把协议转换限制在一个目录，核心代理只依赖 trait，新增供应商不动核心；
>     2. 双面架构：对外 HTTP 只管转发，管理走 Tauri 命令，管理面不额外开端口；
>     3. 双缝测试把编排分支和真实 HTTP 行为分开验证，尤其是 SSE 中断这类网络时序场景。
> - 与参考项目 WaLiAPI：功能对齐参考，架构独立设计（没有照搬它的分层）。
