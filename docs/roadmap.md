# revue-gate Roadmap

版本规划：MVP 只做核心网关，扩展功能后置。

## v0.1.0 (MVP) — 核心网关

- 对外接口：`POST /v1/chat/completions`、`GET /v1/models`、`GET /health`，含 SSE 流式转发
- 渠道管理：CRUD / 启停 / 测试 / 模型映射 / 优先级权重 / 5 类内置渠道
- 密钥管理：`sk-revue-*` 本地密钥 / Bearer 认证 / 配额上限
- 请求转发：非流式 + 流式代理 / 失败重试 / 渠道调度
- 请求日志：全量记录 / 分页 / 多条件筛选 / 日志详情
- 仪表盘：请求数 / Token / 延迟 / 渠道可用率
- 设置中心：端口 host / 主题 / 托盘 / 开机自启 / 重试策略

## v0.2.0 — 安全审计中心

- 请求扫描：自动检测敏感信息泄露（API Key、私钥、JWT、Cookie、Bearer Token）、敏感文件路径（`~/.ssh`、`.env`、云凭据）、Unicode 隐写字符、可疑工具调用（curl 外联、管道上传）、网络风险（公网 IP 探测、webhook/隧道域名）、追踪像素与风控指纹
- 风险等级：clean / info / low / medium / high / critical，综合评分 0-100
- 处理策略（策略模式）：只审计 / 警告 / 脱敏 / 阻断，阻断返回 `451`；默认只审计不影响请求
- 规则管理：内置风险规则 + 自定义黑白名单（域名 / 工具 / 路径 / 关键词）
- 安全配置面板：独立开关控制 Unicode 检测、工具/命令风险、外联/追踪风险、严重风险强制阻断
- 日志展示：请求列表安全等级 Badge、详情页风险摘要 / 评分 / 处理动作 / 脱敏证据
- 仪表盘完整时间粒度：自定义时间范围、周 / 月对比趋势（对齐 WaLiAPI 全部粒度）

## v0.3.0 — 扩展能力

- 更多对外接口：`/v1/completions`、`/v1/responses`、`/v1/embeddings`、`/v1/images/*`、`/v1/audio/*`、`/v1/messages`（Anthropic 协议）
- RAG / 知识库（多来源导入、索引、搜索、问答）
- MCP 服务（工具网关 / Agent 网关）
- 导入导出（渠道 JSON 备份）
- 本地 AI 工具自动配置（Claude Code / Codex / Gemini CLI / Claude Desktop 等）
- 密钥授权增强：允许模型 / 渠道白名单、过期时间
- 日志自动保留策略：配置保留期（如 N 天后自动清理）
- 高可用增强：健康探测、熔断、负载均衡加权、按天/成本统计
