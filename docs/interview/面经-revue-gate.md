# revue-gate 项目面经

## 1. 项目简介（简历可用）

revue-gate 是一个本地运行的 LLM API Gateway 桌面应用，通过 Rust/Tauri + Axum + React 将多个上游模型渠道聚合为统一的 OpenAI-compatible 入口，并在本地完成认证、配额、渠道调度、协议适配、请求日志、安全审计和用量统计。

该项目适合用于后端工程、AI 工程基础设施、桌面端工程交叉方向的面试表达；个人职责边界、目标岗位和真实量化指标仍需补充，当前文档只基于仓库事实整理可讲内容。

## 2. 简历 bullet

- **请求治理闭环：** 围绕本地 LLM 网关“不能只转发，还要可治理”的问题，设计并实现从 Bearer 认证、配额检查、渠道选择、安全审计、模型映射、上游转发到 usage 记账和 request log 的统一请求链路；非流式和流式请求分别在响应完成与流结束时收敛日志和 token 用量，使每次模型调用都能被追踪、复核和按本地 key 治理，线上收益需结合演示日志与压测结果继续验证。
- **可扩展路由调度：** 针对多 provider、多模型、多渠道同时存在时路由不可解释的问题，将渠道筛选沉淀为独立的业务规则：按启用状态、模型列表或模型映射过滤候选渠道，再按优先级分组并在同组内使用权重随机生成失败切换队列；核心代理只消费候选队列，新增渠道配置不需要改动主转发逻辑。
- **协议边界隔离：** 针对 OpenAI-compatible、Claude、Gemini 等上游协议差异和 Anthropic Messages、OpenAI Responses 等下游协议差异容易混杂的问题，将“下游协议转换”和“上游 provider 适配”拆成两个边界：前者转换为 canonical chat，后者通过统一 adapter trait 处理 provider 请求、响应、usage 和 SSE，从而保持认证、配额、审计、路由和日志只在一条核心链路内实现。
- **安全与可观测治理：** 针对 AI 请求可能携带敏感凭证、危险路径或风险动作的问题，在请求转发前引入本地安全审计流水线，通过 scope builder、detector 和 report 聚合形成风险等级和动作；日志保存 trace id、渠道、模型、token、耗时、错误与审计报告，并对存储 payload 做脱敏，形成可定位、可复核但尽量避免二次泄露的观测闭环。
- **本地状态与统计读模型：** 针对单机单用户场景下配置、日志和统计需要私有可恢复的问题，使用 SQLite 保存渠道、密钥、请求日志和知识库状态，使用 Tauri Store 管理运行设置；统计页面从轻量日志投影聚合 dashboard、usage 趋势和排行，避免把 UI 统计逻辑散落到页面层，指标真实性仍需通过固定样本和端到端演示验证。
- **本地知识库增强：** 针对知识库如果独立维护模型凭据会产生第二套治理链路的问题，将 Knowledge Service 设计为同进程服务模块，负责文档解析、分块、FTS、embedding、hybrid 检索、citation 和 RAG 上下文构建，而模型生成继续复用 Gateway proxy 的认证、渠道、quota、日志和安全审计；当前能力包含已实现和待演进部分，面试表达时应区分代码已落地与规划预留。

## 3. 面试问题

### 主题一：项目定位与总体架构

#### 主问 1：你这个项目到底解决什么问题？为什么不是直接让客户端调用上游模型？

**第一人称口播版：** 我会先从使用场景讲起：这个项目解决的是本地环境里多个 AI 客户端、多个供应商和多套 API Key 难以统一治理的问题。直接让 ChatBox、CLI 或 SDK 分别调用上游，会导致上游 key 分散在各个客户端里，请求日志、token 消耗、失败原因和渠道健康度都不可集中观察。我在项目里把 revue-gate 定位成一个本地 LLM API Gateway：下游只连 `127.0.0.1:3000` 和本地生成的 key，上游 provider key 只存在本地渠道配置里。这样用户能用统一 OpenAI-compatible 接口完成调用，同时把认证、渠道选择、模型映射、配额、日志和审计集中在网关这一层。结果不是追求云端多租户，而是服务单机单用户、本地私有和可复核的治理场景。

**追问 1：为什么强调本地和单用户？**

**第一人称口播版：** 我会说这是项目边界上的主动取舍。这个项目不是 SaaS 网关，也没有账号体系、多租户隔离和云端计费，而是把用户已经拥有的供应商 key 放在本机，由本机网关统一代理。这样做的好处是实现复杂度和隐私边界都更可控，数据可以留在本地 SQLite，控制面可以用 Tauri 桌面应用承载，适合个人开发者或小团队管理自己的 AI 工具链。代价是它不解决多人权限、集中审计、横向扩容这些云服务问题，所以面试里我会明确说当前版本是单机单用户 MVP，后续如果做团队版，需要重新设计身份、权限、审计留存和部署拓扑。

**追问 2：它和普通反向代理有什么区别？**

**第一人称口播版：** 我会强调普通反向代理通常只做转发、路由或负载均衡，而这个项目的核心是请求治理。一次请求进入网关后，并不是简单转到某个 URL，而是先解析本地 key 做认证和 quota 检查，再根据模型、渠道启用状态、优先级、权重和模型映射生成候选队列，然后执行安全审计、provider 协议适配、失败重试、usage 记账和 request log 写入。它对外看起来像 OpenAI-compatible endpoint，但内部保留了“这个请求是谁发的、走了哪个渠道、为什么失败、消耗多少 token、是否有安全风险”的完整证据链，这才是它区别于普通代理的地方。

#### 主问 2：你如何划分 Data Plane 和 Control Plane？

**第一人称口播版：** 我会把它解释成两条不同性质的路径。Data Plane 是真实模型请求路径，由 Axum HTTP 服务提供 `/v1/chat/completions`、`/v1/models`、`/health` 等接口，负责认证、协议转换、渠道调度、上游转发、流式响应、记账和日志。Control Plane 是管理路径，由 React/Tauri 桌面界面通过 Tauri Commands 操作渠道、密钥、日志、统计、设置和服务生命周期。这样划分的结果是，AI 客户端不需要知道管理 UI，管理 UI 也不直接参与 `/v1/*` 数据转发。两条路径在 interface 层分开，但共同进入 usecases 和 domain 层，保证核心规则只实现一次。

**追问 1：为什么前端不直接请求 Axum 的管理接口？**

**第一人称口播版：** 我会说这是 Tauri 桌面应用场景下的自然选择。前端是本地 webview，不需要把控制面再暴露成一套 HTTP 管理 API；通过 Tauri Commands 可以直接走本地 IPC，权限面更小，也更符合桌面应用模式。Axum HTTP 只对外暴露数据面，让 ChatBox、OpenAI SDK 或 curl 这类客户端调用。管理操作则收束到 `src/lib/api.ts` 封装的 command adapter，再进入后端 `interface/commands/*`。这样做减少了对外暴露的管理接口，也让服务生命周期、托盘、自启动和本地 store 这些桌面能力更容易集成。

**追问 2：两条路径共用业务核心会不会互相影响？**

**第一人称口播版：** 我会回答它们共享的是业务模型和用例，而不是入口状态。比如渠道、密钥、日志、统计都是同一套 repository 和 domain 规则，Data Plane 和 Control Plane 都通过 usecase 访问这些能力。这样可以保证 UI 上禁用一个渠道后，数据面请求马上不会选择它；UI 创建本地 key 后，HTTP 认证也能使用它。风险是共享状态要注意一致性和锁边界，所以设置这类运行期配置会通过共享快照读取，数据库业务数据通过 repository 持久化。面试里我会强调这是“入口分离，核心复用”，不是两套系统。

#### 主问 3：后端为什么采用 Clean Architecture，而不是 handler 里直接写业务？

**第一人称口播版：** 我会说这个项目的业务规则并不只是 CRUD，而是有明显的组合链路：认证、quota、渠道选择、协议适配、retry、安全审计、日志和统计。如果这些逻辑直接写在 Axum handler 或 Tauri command 里，测试和演进都会很困难。所以我把入口层、用例层、领域层和基础设施层分开：handler 和 command 只做参数解析与错误映射；usecase 负责编排；domain 只放纯业务模型和规则；infrastructure 实现 SQLite、Tauri Store 和 provider HTTP。结果是 domain 不依赖 sqlx、reqwest、axum 或 tauri，核心链路可以用 in-memory repo 和 mock provider 测试。

**追问 1：这个分层有没有过度设计？**

**第一人称口播版：** 我会承认对于一个只有简单表单的桌面应用，完整 Clean Architecture 可能显得重。但 revue-gate 的复杂度在后端请求治理链路，不在 CRUD 表单。比如渠道选择必须可单测，provider 适配要可替换，proxy usecase 要能注入 mock 上游，统计聚合要能复用 SQLite 和 in-memory 仓储。如果都写在入口层，新增协议或 provider 时很容易牵一发动全身。当前分层是轻量版：没有复杂 bounded context，也没有 domain event，只保留聚合根、领域服务、repository trait 和 usecase 编排，所以我认为复杂度和收益是匹配的。

**追问 2：你怎么证明 domain 层是纯业务规则？**

**第一人称口播版：** 我会指向具体文件：`domain/dispatcher.rs` 只做候选渠道选择，不碰数据库和 HTTP；`domain/quota.rs` 只判断 `used >= limit` 是否超额；`domain/provider.rs` 只定义 adapter trait 和统一的请求响应类型，不直接发 reqwest 请求。真正的 SQLite 实现在 `infrastructure/sqlite/*`，上游 HTTP 实现在 `infrastructure/providers/*`，Axum 入口在 `interface/http/*`。这让 domain 单元测试可以不启动 Tauri、不建数据库、不访问网络。我的验证方式是看依赖方向和测试缝，而不是只看目录命名。

### 主题二：请求链路、认证与配额

#### 主问 4：一次 `/v1/chat/completions` 请求在系统里怎么走？

**第一人称口播版：** 我会按链路复述：客户端先带 Bearer token 请求本地 Axum endpoint，handler 负责解析 JSON、提取本地 API Key、生成或透传 trace id，并通过协议 registry 把请求转换为 canonical chat body。然后 `ProxyRequestUsecase` 做认证和 quota 检查，读取启用渠道并选择候选队列，基于设置生成安全审计策略，对 canonical body 扫描风险并决定是否阻断。通过后，代理会对每个候选渠道应用模型映射，调用对应 provider adapter 转发。非流式成功后累计 usage、写日志并返回；流式请求则包装上游 stream，在流结束时聚合 usage 并写日志。

**追问 1：为什么协议转换要发生在 proxy 前面？**

**第一人称口播版：** 我会说 proxy usecase 是核心治理边界，它希望看到统一形态的请求，而不是 OpenAI、Anthropic、Responses 各自的格式。把下游协议先转换成 canonical chat，可以让认证、配额、渠道选择、安全审计、日志和 provider adapter 都复用同一套逻辑。否则每接一个下游协议，就要复制一遍 proxy 链路。代价是日志目前主要保存 canonical body，不一定保留原始下游 body，所以调试某些协议转换细节会受限；这个风险在 ADR 里有记录，后续可以通过保存 conversion report 或原始 body 进一步增强可观测性。

**追问 2：错误状态是怎么映射的？**

**第一人称口播版：** 我会按语义区分：缺失或无效本地 key 返回 401，quota 超限返回 429，请求体非法是 400，没有候选渠道支持模型是 404，安全策略阻断是 403，所有候选上游都失败是 502。这个区分很重要，因为它帮助用户判断问题发生在本地认证、配额治理、路由配置、安全策略还是上游 provider。比如用户之前遇到的上游 403，说明本地 key 已经通过，错误来自 provider 或模型权限，而不是本地认证失败。这个错误语义在 `ProxyError` 和 HTTP handler 映射里可以对应到源码。

#### 主问 5：本地 API Key 和上游 API Key 如何隔离？

**第一人称口播版：** 我会强调这里有两个完全不同的概念：Local API Key 是 revue-gate 生成的 `sk-revue-*`，下游客户端用它访问本地网关，它是 quota、日志和审计的治理主体；上游 API Key 是 provider key，只配置在 Channel 里，由 provider adapter 发请求时使用。控制面创建本地 key 时只展示一次明文，列表里返回掩码；上游 key 在渠道返回给前端时也会被清空或掩码。上游错误体也不会原样回传，因为某些 provider 可能在 401 文本里回显提交的 key，项目会统一收敛成泛化错误体，避免下游看到真实上游凭据。

**追问 1：本地 key 泄露后怎么办？**

**第一人称口播版：** 我会从当前能力和待演进两个层面回答。当前版本可以在控制面禁用或删除某个本地 key，后续请求认证会失败；由于 request log 绑定 api_key_id，也能回看这个 key 产生过哪些请求和 token 消耗。它不是上游 provider key，泄露后影响范围受本地网关和 quota 限制。待演进方向是给 key 增加模型白名单、渠道白名单、过期时间和更细粒度权限，这样泄露后的 blast radius 更小。面试时我不会声称已经有完整多租户权限系统，因为仓库当前定位是单机单用户。

**追问 2：为什么创建 key 后只显示一次？**

**第一人称口播版：** 我会说这是控制面安全体验上的基本约束。Local API Key 是客户端访问网关的凭据，如果列表、编辑页或日志中一直显示明文，很容易在录屏、截图或调试时泄露。项目的 command 层在创建时返回明文，方便用户立即复制；list、update、enable/disable 这些路径会统一返回掩码。这样既满足可用性，也降低后续浏览管理界面时的泄露风险。这个设计不能替代更强的 secret storage，但对于本地 MVP 来说，是一个清晰且可验证的安全边界。

#### 主问 6：quota 是怎么判断和累计的？为什么你演示里先用 100 再换 1000？

**第一人称口播版：** 我会说 quota 的判断是请求开始前完成的：`QuotaPolicy` 规则很直接，当 limit 存在且 used 大于等于 limit 时，请求直接返回 429；没有 limit 就不限额。usage 的累计发生在上游请求成功后，非流式从响应 usage 里取 token，流式则在每个 SSE frame 中累计 usage，流结束后写回 key 的 used。演示里先创建 quota=100 的 `interview-demo`，跑一个非流式请求后通常会消耗 token，再继续使用同一个 key 可能触发 429；然后新建 quota=1000 的 `interview-demo-sse` 跑 SSE，是为了同时展示“配额治理生效”和“流式请求可正常完成”。

**追问 1：如果上游成功了，但本地累计 usage 失败怎么办？**

**第一人称口播版：** 我会回答这是项目里明确做了 best effort 处理的地方。上游已经处理成功后，如果因为本地数据库或仓储错误导致 quota 累计失败，不能再把 5xx 返回给客户端让它误以为请求失败并重试，否则可能造成重复调用或重复计费。所以 `record_success` 中累计 usage 和写日志失败会记录 warn，但不会阻断已经成功的响应。这个选择的边界是：本地记账可能在异常情况下缺失，需要通过日志告警和后续补偿优化；但它避免了把本地治理故障扩大成客户端重复请求和上游重复消费。

**追问 2：流式请求中途失败时 quota 怎么办？**

**第一人称口播版：** 我会说流式请求的边界更复杂，因为 HTTP status 一旦开始返回 SSE，就不能像非流式那样随时改成错误状态。项目的处理方式是：打开流之前的失败可以继续尝试下一个候选渠道；一旦流打开，stream wrapper 会边读事件边累计 usage 和响应文本。如果中途 provider stream 出错，系统会把已经累计到的 usage 做 best effort 记账，写一条失败日志，然后终止流。这样不能保证覆盖客户端断连等所有极端情况，但它把“已发生的上游消耗尽量记下来”作为 MVP 里的合理边界。

### 主题三：路由调度与 Provider 适配

#### 主问 7：渠道选择策略是怎样的？为什么要 priority 和 weight 都存在？

**第一人称口播版：** 我会先解释候选队列生成：系统先排除禁用渠道，再判断渠道是否支持请求模型，支持的方式包括模型列表直接包含、模型映射命中 client model，或者渠道模型列表为空代表不限制。之后按 priority 升序分组，小 priority 优先；同一 priority 内按 weight 做无放回随机排序，生成一次请求内固定的 failover 队列。priority 表示明确的层级偏好，比如主备；weight 表示同一层级内的流量比例或偏好。这样同时支持确定性的主备关系和轻量的加权分流。

**追问 1：为什么是无放回随机？**

**第一人称口播版：** 我会说无放回随机适合生成一次请求的失败切换队列。对于同一个 priority 组，第一次选择按 weight 决定谁先尝试；如果失败，不应该再选择已经失败过的同一渠道，而是从剩余渠道里继续按权重选择。这样既保留了权重倾向，也保证一次请求不会在同一个失败渠道上重复打转。实现上这个逻辑在 domain service 里完成，proxy 只消费有序候选列表。这样能让路由策略独立测试，也让 retry 行为更容易解释。

**追问 2：什么情况下不重试？**

**第一人称口播版：** 我会按错误性质解释：上游返回 429 或 5xx 被视为可重试，因为它们更像临时限流或服务端故障，网关可以尝试下一个候选渠道。普通 4xx 通常表示客户端请求、模型权限或上游配置问题，盲目重试其他渠道可能隐藏真实错误，所以不会按 retryable 处理。流式请求也有特殊边界：如果打开 stream 前失败，可以换候选；如果已经开始向客户端发 SSE，中途失败就不能换 HTTP status 或透明切到另一个 provider，只能记录失败并结束。这些取舍都是为了避免不透明和不可解释的重试。

#### 主问 8：模型映射解决什么问题？和模型列表有什么关系？

**第一人称口播版：** 我会说模型映射是为了把客户端看到的模型名和上游实际模型名解耦。比如下游客户端统一请求 `gpt-compatible`，不同上游渠道可能实际用 DeepSeek、OpenAI-compatible 或其他模型 ID。渠道选择时，如果 mapping 的 client model 命中，说明该渠道可以服务这个客户端模型；真正转发前，proxy 会把 body 里的 `model` 改写为 mapping 里的 upstream model。这样客户端配置可以保持稳定，上游渠道可以各自使用不同模型名。模型列表用于判断渠道支持哪些模型，模型映射则提供“客户端名到上游名”的转换。

**追问 1：如果模型列表为空为什么表示 unrestricted？**

**第一人称口播版：** 我会说这是为了兼容一些自定义 OpenAI-compatible 端点或用户尚未维护模型列表的场景。空列表意味着系统不主动限制模型匹配，任何请求模型都可以进入候选；真正是否支持由上游返回结果决定。好处是配置门槛低，尤其适合 custom endpoint；风险是如果用户拼错模型名，也可能路由到这个渠道并在上游才失败。因此面试里我会把它说成一个有边界的产品取舍：它提升兼容性，但严谨部署时最好填写明确模型列表或模型映射，以获得更可解释的路由行为。

**追问 2：映射发生在安全审计前还是后？**

**第一人称口播版：** 我会分两层讲。安全审计前，系统会先根据第一候选渠道计算计划中的 upstream model，并把 forwarding context 写进 audit report，这样如果被阻断，日志能说明原本计划走哪个渠道和哪个上游模型。真正每次 attempt 的 body 改写发生在 provider 转发前，因为不同候选渠道可能有不同 mapping。审计扫描的是 canonical request body 及上下文，不直接改写转发 payload；payload redaction 只影响本地存储日志。这个顺序兼顾了审计可解释性和多候选渠道的实际映射差异。

#### 主问 9：Provider Adapter 的边界是什么？为什么不把所有协议转换都放在一个地方？

**第一人称口播版：** 我会把 provider adapter 定义为“上游边界”。它负责 provider-specific 的 base URL、认证 header、请求格式、响应格式、usage 解析、模型发现和 SSE 事件转换。比如 OpenAI、DeepSeek、Custom 走 OpenAI-compatible passthrough；Claude 和 Gemini 需要把 canonical chat 转成各自协议，再把响应转回 OpenAI-compatible 形态。下游协议转换则是另一个边界，负责把客户端的 Anthropic Messages 或 OpenAI Responses 转成 canonical chat。两者不能混在一起，否则 client-facing 和 upstream-facing 的 headers、错误体、SSE 语法和可观测需求都会耦合。

**追问 1：新增一个 provider 要改哪些地方？**

**第一人称口播版：** 我会说首先要在领域枚举里表达新的 channel type，或者如果它是 OpenAI-compatible，可以复用现有 OpenAI-compatible adapter。真正新增 provider 时，需要实现 `ProviderAdaptor` trait，包括默认模型、默认 base URL、连通性测试、非流式 forward、流式 forward 和 usage 解析；再在 `adaptor_for` 里返回对应 adapter。控制面还要补渠道类型 label 和表单默认值。核心 proxy 不应该因为新增 provider 而增加大量分支，这就是 adapter 边界带来的主要收益。

**追问 2：为什么上游错误体要收敛？**

**第一人称口播版：** 我会说这是安全边界的一部分。有些 OpenAI-compatible provider 在 401 错误体里会回显提交的 API Key，比如提示 “incorrect key provided ...”。如果网关原样把上游错误体透传给下游客户端，就可能把只应该保存在本地渠道配置里的 provider key 暴露出去。所以项目里对上游非成功错误构造了泛化错误 JSON，只保留 status code，不保留原始文本。这样牺牲了一部分调试细节，但保护了上游凭据不跨越本地网关边界。需要排查时，可以通过本地日志和渠道测试定位。

### 主题四：安全审计、日志与统计

#### 主问 10：安全审计引擎做了什么？它和请求日志是什么关系？

**第一人称口播版：** 我会说安全审计回答的是“这个请求体里有没有风险内容”。它在认证成功、候选渠道确定之后、真正转发上游之前执行。流程是先把 canonical request body 按优先级展开成可扫描的 scope items，再用确定性 detector 查找 provider key、私钥、数据库连接串、JWT、敏感路径、风险命令、内网或 metadata IP 等风险，最后聚合成 AuditReport，包括 risk level、score、action 和 findings。请求日志会保存审计结果、风险等级、动作和脱敏后的 evidence，因此日志不仅是访问记录，也是安全复核入口。

**追问 1：为什么不调用 LLM 来判断风险？**

**第一人称口播版：** 我会说当前版本选择确定性规则，是为了本地、可解释、可测试和低成本。安全审计发生在请求转发前，如果再调用 LLM 判断风险，会引入额外延迟、费用、隐私边界和不确定性，而且可能需要把疑似敏感内容发给另一个模型，这和本地私有目标冲突。确定性 detector 虽然覆盖有限，但每条规则都有明确 metadata、风险等级和证据位置，适合 MVP 阶段先建立“扫描、聚合、阻断、写日志、UI 复核”的闭环。后续可以扩展规则库或加入可配置白名单，但不必一开始就依赖模型判别。

**追问 2：日志脱敏和转发 payload 是一回事吗？**

**第一人称口播版：** 我会明确区分：日志脱敏只影响本地存储的 request body，不改变真正转发给上游的 payload。原因是网关不应该悄悄改写用户发给模型的内容，否则会改变模型行为，难以解释。审计如果处于 observe 模式，即使发现风险也只是记录和提示；如果 enforce 且命中阻断条件，则完全不转发上游。只要 payload 被存储，项目会对匹配到的敏感片段做 redaction，避免日志二次泄露。这个边界能同时保证请求语义透明和本地证据安全。

#### 主问 11：request log 记录了哪些信息？它如何支撑排障？

**第一人称口播版：** 我会说 request log 是这个项目的可观测核心。它记录本地 API Key、渠道、客户端模型、实际上游模型、状态码、prompt/completion/total tokens、耗时、错误消息、是否流式、是否重试、trace id、请求体、响应 choices 摘要以及安全审计报告。排障时可以先看状态码判断是认证、quota、路由、上游还是安全策略问题；再看 channel 和 upstream model 确认路由；看 is_retry 判断是否发生失败切换；看 trace id 对齐结构化日志；看 audit report 判断是否有风险命中。这样用户不用猜“请求到底发生了什么”。

**追问 1：trace id 是怎么来的？**

**第一人称口播版：** 我会说 HTTP middleware 会读取客户端传入的 `x-request-id`，如果没有就生成一个 uuid v7 作为 trace id，并放进 request extensions，同时在响应 header 里回传。后续 handler、proxy usecase 和 request log 都使用同一个 trace id。这样客户端、网关日志和本地请求记录可以关联起来。对本地调试来说，它不是分布式 tracing 那么完整，但已经能解决“前端看到失败，后端日志是哪一条，数据库里对应哪次请求”这个问题。后续如果接入更完整 tracing，也可以沿用这个请求标识。

**追问 2：为什么失败也要写日志？**

**第一人称口播版：** 我会说失败日志比成功日志更重要。网关有重试和候选渠道机制，如果只记录最终成功，就无法知道前面哪些渠道失败过，也无法验证 failover 是否按预期工作。因此 proxy 在每次 retryable failure 时都会写一条失败 attempt 日志，包含 channel、status 或错误信息、is_retry 等字段。如果所有候选失败，用户可以从日志里看到每次尝试的原因。安全策略阻断也会写本地日志，因为虽然没有上游调用，但这是一次真实被治理的请求，应该能在控制面复核。

#### 主问 12：Dashboard 和 Usage 统计怎么做？为什么不直接用 SQL group by？

**第一人称口播版：** 我会说当前统计聚合的目标是和 request log 保持一致、容易测试，而不是追求大规模 OLAP。用例层会从 RequestLogRepository 拉轻量投影行，不包含 request body，然后在 Rust 里按前端传来的 timezone offset 聚合今日请求、今日 token、累计请求、累计 token、平均延迟、渠道可用率和 7 天趋势。Usage 页面也基于同一类投影生成按天、按渠道、按模型的排行。这样 SQLite 实现和 in-memory 测试仓储可以共用同一套聚合函数，减少 SQL 方言和时区函数造成的不一致。

**追问 1：这样日志多了会不会慢？**

**第一人称口播版：** 我会承认这是当前设计的可扩展性边界。MVP 是单机单用户，本地日志量预期不大，所以轻量扫描加内存聚合能换来实现简单和测试一致。如果后续日志量明显增长，需要引入按时间范围的索引优化、SQL 预聚合、日级 summary 表或增量统计缓存。面试里我不会说它已经适合大规模生产流量，而会说当前选择是匹配本地桌面 MVP 的复杂度，并且代码已经把统计聚合收敛在 usecase/domain 层，后续替换底层聚合策略时，前端和请求链路不需要大改。

**追问 2：渠道可用率怎么理解？**

**第一人称口播版：** 我会把它解释成基于请求日志的观测指标，而不是 provider 官方 SLA。当前聚合里 status code 小于 400 的请求计为成功，失败请求进入分母，用这个比例展示渠道或整体可用性。它能帮助用户看近期请求是否频繁失败，但它受样本量、用户请求内容、模型权限、quota 和网络状态影响。严格来说，如果要评估上游 provider 健康，还需要结合渠道连通性测试、错误分类和定期探测任务。当前 dashboard 的指标适合作为本地使用视角的可观测结果，而不是对外承诺的可用率。

### 主题五：多协议、SSE 与知识库

#### 主问 13：多协议转换为什么要引入 canonical chat？

**第一人称口播版：** 我会说 canonical chat 是为了让核心代理只理解一种内部请求形态。客户端可能用 OpenAI Chat Completions、Anthropic Messages 或 OpenAI Responses；如果每种协议都直接进入 proxy，认证、quota、审计、路由、日志和 provider 转发都会出现重复分支。项目引入 protocol registry，把不同下游协议先转换成基于 OpenAI Chat 的 canonical body，再进入统一 proxy；响应返回时再转换回客户端协议。这样新增客户端协议时，主要扩展 conversion layer，而不复制网关治理链路。

**追问 1：转换时不支持的字段怎么办？**

**第一人称口播版：** 我会说第一版采取受控兼容策略，不支持的平台特性或状态特性会返回 400，而不是静默丢弃。比如某些 multimodal blocks、server tools、stateful responses 字段，如果当前不能正确表达成 canonical chat，就明确拒绝。原因是 silently drop 会让客户端以为请求被完整处理，实际模型行为却变了，排障很困难。项目里还预留了 conversion report，用于记录字段映射和警告；当前主要在内存里使用，后续如果需要更强 observability，可以把转换报告写入 request log。

**追问 2：这和 provider adapter 有什么关系？**

**第一人称口播版：** 我会再次强调两者方向不同。Protocol conversion 是 client-facing，把不同下游 API 方言变成网关内部 canonical chat，或者把 canonical response 转回客户端期望格式。Provider adapter 是 upstream-facing，把 canonical chat 发给 OpenAI-compatible、Claude、Gemini 等上游服务，并解析它们的 usage 和 stream。两者中间隔着 proxy usecase，proxy 负责认证、quota、路由和日志。这个分层避免了“协议兼容”变成一个大而全模块，也方便未来同时扩展下游协议和上游 provider。

#### 主问 14：SSE 流式转发有什么难点？你们怎么处理 usage 和日志？

**第一人称口播版：** 我会说 SSE 的难点是响应一旦开始，HTTP status 和 headers 基本就提交了，后续失败不能像非流式那样换成普通错误响应。项目把失败分成两类：打开上游 stream 之前失败，可以按候选渠道继续重试；stream 成功打开之后，由 wrapper 逐帧转发 SSE，边观察 frame 中的 usage 和 choices delta，聚合 token 用量和响应文本。正常结束时写 200 成功日志并累计 quota；中途 provider error 时，把已累计 usage 尽量写回并记录失败日志。这样能在流式场景下保留基本治理闭环。

**追问 1：客户端断连怎么办？**

**第一人称口播版：** 我会如实说明这是当前 MVP 的已知边界。源码注释里也提到，如果客户端中途断开导致 stream 没有被完整 drain，当前 wrapper 可能无法完成 billing 和 logging。要完全解决这个问题，需要 cancellation-aware 的流包装、后台任务或更细粒度的 stream lifecycle 管理，确保即使客户端断开也能继续消费或至少记录终止状态。但这会增加复杂度，所以当前版本先处理正常结束和 provider 中途错误。面试时我会把它作为风险边界，而不是装作已经完美覆盖。

**追问 2：为什么要合成 response choices？**

**第一人称口播版：** 我会说非流式响应天然有完整 body，可以从 `choices` 中提取并脱敏后保存。流式响应则是一帧一帧返回 delta，结束时没有完整非流式 body。为了让日志详情仍能展示响应摘要，stream wrapper 会观察每个 SSE data line 中的 delta content，把文本拼起来，并结合 finish reason 合成一个类似 choices 的结构再写入日志。这个设计提高了日志可读性，但它也有边界：复杂工具调用、多模态或非标准 chunk 可能不完整，后续需要根据协议能力继续增强。

#### 主问 15：Knowledge Service 和网关主链路是什么关系？

**第一人称口播版：** 我会说 Knowledge Service 是 revue-gate 的增强模块，而不是另一个独立网关。它负责知识库 CRUD、文件上传、解析、分块、FTS5、embedding、vector search、hybrid retrieval、citation 和 RAG prompt 组装。真正生成答案时，它复用 Gateway proxy 调用模型，因此仍然受本地 API Key、渠道选择、quota、request log 和安全审计治理。这样设计避免了知识库模块自己维护一套 provider key、路由和日志，保持本地 AI 能力都从统一治理面经过。

**追问 1：文档 revision 为什么重要？**

**第一人称口播版：** 我会说知识库检索最怕半成品或旧版本污染结果。项目设计里只有 ready 状态且 revision 匹配的 chunks 才能被检索。上传或重处理文档时，解析和分块先在事务外完成，真正替换时在事务中推进 revision、删除旧 chunks 和 FTS 行、插入新 chunks、刷新统计。任一步失败都会回滚，已有 ready 文档继续保持旧 revision 可用。这个机制保证调用方要么看到完整旧版本，要么看到完整新版本，不会看到半处理状态。

**追问 2：Hybrid 检索怎么讲才不夸大？**

**第一人称口播版：** 我会谨慎表述为当前项目已实现 keyword、vector 和 hybrid 检索的基础闭环，但不是大规模向量数据库。Keyword 使用 SQLite FTS5，embedding 以本地 BLOB 保存，vector 检索当前是进程内 cosine similarity 线性扫描，hybrid 用 RRF 融合并可根据索引状态降级。它适合本地个人知识库 MVP，不适合直接宣称高并发或海量语料检索。后续如果要扩大规模，需要 HNSW、后台索引 worker、增量同步和更明确的资源控制。

## 4. 源码证据索引

| 主题 | 关键路径与内部符号 | 对应正文位置 |
| --- | --- | --- |
| 产品定位 | `README.zh-CN.md`、`docs/Requirements.md` | 项目简介、主问 1 |
| 后端分层 | `docs/Architecture-backend.md`、`src-tauri/src/domain.rs`、`src-tauri/src/usecases.rs`、`src-tauri/src/infrastructure.rs`、`src-tauri/src/interface.rs` | 主问 2、主问 3 |
| 前端控制面 | `docs/Architecture-frontend.md`、`src/lib/api.ts`、`src/pages/*` | 主问 2 |
| HTTP 数据面 | `src-tauri/src/interface/http/handlers.rs`、`src-tauri/src/interface/http/router.rs`、`TraceId`、`handle_protocol_request` | 主问 4、主问 11 |
| Proxy 主链路 | `src-tauri/src/usecases/proxy.rs`、`ProxyRequestUsecase`、`ProxyError`、`wrap_stream_bookkeeping` | 主问 4、主问 6、主问 14 |
| Local API Key | `src-tauri/src/domain/api_key.rs`、`src-tauri/src/usecases/api_key.rs`、`src-tauri/src/interface/commands/api_key.rs` | 主问 5、主问 6 |
| 配额策略 | `src-tauri/src/domain/quota.rs`、`QuotaPolicy::check`、`AccumulateUsageUsecase` | 主问 6 |
| 渠道调度 | `src-tauri/src/domain/dispatcher.rs`、`ChannelSelector::select_channels`、`supports` | 主问 7、主问 8 |
| 渠道模型和映射 | `src-tauri/src/domain/channel.rs`、`ModelMapping`、`apply_mapping` | 主问 8 |
| Provider Adapter | `src-tauri/src/domain/provider.rs`、`ProviderAdaptor`、`TokenUsage`、`src-tauri/src/infrastructure/providers.rs`、`adaptor_for` | 主问 9 |
| 上游错误收敛 | `src-tauri/src/infrastructure/providers.rs`、`upstream_error_body`、`upstream_error_event` | 主问 5、主问 9 |
| 安全审计 | `docs/design/backend/Security-Audit-Engine.md`、`src-tauri/src/domain/security_audit.rs`、`AuditReport`、`build_audit_scope`、`redact_body` | 主问 10 |
| 请求日志 | `src-tauri/src/domain/request_log.rs`、`src-tauri/src/usecases/log.rs`、`src-tauri/src/infrastructure/sqlite/request_log.rs` | 主问 11 |
| 统计聚合 | `src-tauri/src/usecases/stats.rs`、`src-tauri/src/domain/stats.rs`、`aggregate_dashboard`、`aggregate_usage` | 主问 12 |
| 多协议转换 | `docs/adr/0003-multi-protocol-conversion-engine.md`、`src-tauri/src/protocol/registry.rs`、`CodecRegistry`、`ProtocolKind` | 主问 13 |
| SSE 处理 | `src-tauri/src/interface/http/handlers.rs`、`transform_proxy_stream`、`src-tauri/src/protocol/sse.rs`、`wrap_stream_bookkeeping` | 主问 14 |
| Knowledge Service | `docs/design/backend/Knowledge.md`、`src-tauri/src/domain/knowledge.rs`、`src-tauri/src/usecases/knowledge.rs`、`src-tauri/src/usecases/knowledge/rag.rs` | 主问 15 |
| 数据库迁移 | `src-tauri/migrations/001_init.sql`、`002_request_log_audit_fields.sql`、`009_knowledge_sources.sql`、`012_knowledge_chunks_fts.sql` | 简历 bullet、主问 11、主问 15 |
| 演示脚本 | `docs/demo-guide.md` | 主问 6、演示准备 |

## 5. 待补事实清单

- 个人职责边界：是否独立完成、主要负责后端、前端、架构设计、调试演示，还是团队协作中的某一部分。
- 目标岗位：后端、AI Infra、全栈、桌面端或 Rust 工程。
- 真实指标：构建耗时、请求延迟、日志量、token 用量、测试覆盖、演示结果、PR 或提交记录。
- 上线结果：是否开源、是否被他人使用、是否有 demo 录屏或面试展示材料。
- 关键难点的个人决策记录：哪些设计是你主导，哪些是参考或协作完成。

