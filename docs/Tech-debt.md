# 待办 / 待优化清单

> 记录代码审查与日常开发中发现的待解决 / 待优化点。每一条含：现状、位置、问题、建议、优先级。
> 修复后请从本文档移除对应条目，避免残留已过期项。

## 潜在优化

### 4. 统计聚合：每请求全表扫描 + O(7n) 内存过滤

- **位置**：`usecases/stats.rs:87` `repo.stat_rows(None, None)` 拉全量轻量行，随后 `:106`、`:119` 对全量做 7 次 `filter`（今天 1 次 + 7 天趋势各 1 次）
- **问题**：单用户本地 SQLite、日志量数千~数万条时无问题；若长期累计几十万条以上，每次拉仪表盘都是 O(7n)，性能与内存开销随日志量线性增长。
- **建议**：届时再把聚合下推到 SQL（GROUP BY 时间截断），或只拉 `[today-6 天, now]` 窗口内的行；当前不建议动。
- **优先级**：中（触发条件：日志量达到明显拖慢仪表盘时）

### 10. 渠道调度缺 cooldown（坏渠道每次请求浪费一次尝试）

- **位置**：`src-tauri/src/domain/dispatcher.rs`（当前为无状态「优先级分组 + 组内权重随机」，无失败记忆）
- **问题**：坏渠道在队首被打中时，每次请求都会浪费一次尝试后才落到健康渠道；单机单用户可接受（现有 retry 已兜住），但若某渠道持续失败，代价随请求量线性放大。
- **建议**：等有失败 metrics 后（见 `docs/design/backend/Loadbalancing-reference.md` 可演进路线第 5 条）做轻量 channel cooldown：记录每渠道连续失败次数与 `cooldown_until`，429/5xx/timeout 达阈值后短时移出候选池，成功清零。当前不引入 `circuit` 模块；在没有 metrics 和 half-open 探测语义前，不把该能力命名为 circuit breaker。
- **优先级**：低（触发条件：观察到某渠道持续失败、每次请求都在空转时）

## 架构记录

### 11. 暂不整体重构架构

- **位置**：`src-tauri/src/{domain,usecases,infrastructure,interface}`
- **问题**：作为网关项目，曾考虑是否应放弃领域分层并参考主流 AI gateway / gateway 架构整体重构。
- **结论**：不做整体架构重构。网关项目仍有清晰领域语言，如 Provider、Upstream Channel、Model、Routing、Retry、Fallback、Request Log、Token Usage、Local API Key、Auth、Health、Channel Cooldown、Quota。后续新增能力时保持 gateway 领域语言清晰，但避免把轻量技术机制过度领域化。
- **优先级**：低

## 非缺陷记录（仅备忘）

### 6. handlers.rs 测试直接 sqlx::query 打 pool

- **位置**：`interface/http/handlers.rs:428,463,553,663`（均在 `#[cfg(test)]` 内）
- **说明**：测试断言直接用 `sqlx::query` 验证落库结果，绕过了仓储层。属测试便利，无安全问题（均为无参数查询）；如希望测试只经仓储断言，可改，非必须。
