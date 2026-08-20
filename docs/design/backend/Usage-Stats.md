# 用量统计读模型

> 统计页不是独立业务上下文，而是基于 `request_logs` 的读模型：同一批日志行一次读取、一次聚合，产出每日趋势、渠道排行和模型排行。

## 为什么需要

仪表盘已有 `get_stats`，用于展示今日 / 累计请求数、Token、平均延迟、渠道可用率和 7 天趋势。但独立用量页需要更细的观察维度：

1. 任意时间范围内按天查看请求数和 Token；
2. 查看各渠道的消费排行；
3. 查看各模型的消费排行。

这些数据全部来自 `request_logs`，没有多租户计费、账单结算或外部会计语义。因此这里不引入新的统计上下文 / 独立表，只把日志表投影成一个面向页面读取的 stats read model。

## 设计思路

### 所属边界

- 领域读模型：`src-tauri/src/domain/stats.rs`
- 用例编排：`src-tauri/src/usecases/stats.rs`
- 仓储投影：`RequestLogRepository::stat_rows(start_at, end_at)`
- SQLite 实现：`src-tauri/src/infrastructure/sqlite/request_log.rs`
- Tauri command：`src-tauri/src/interface/commands/stats.rs`

依赖方向保持 `interface → usecases → domain`，`infrastructure → domain`。`domain/stats.rs` 不依赖 SQL、Tauri、前端或仓储。

### 读模型类型

| 类型 | 说明 |
| --- | --- |
| `LogStatRow` | 从 `request_logs` 读取的轻量投影：`channel_id`、`model`、`status_code`、`total_tokens`、`duration_ms`、`created_at` |
| `DailyStat` | 单日本地日期桶：`date`、`requests`、`tokens` |
| `RankRow` | 排行行：`key`、`name`、`requests`、`tokens`、`avg_latency_ms`、`availability` |
| `UsageStats` | 用量页完整快照：`daily`、`by_channel`、`by_model` |
| `UsageStatsQuery` | 查询范围：`start_at`、`end_at`、`timezone_offset_minutes` |

`LogStatRow` 明确属于统计读模型，而不是完整日志实体。它不包含 `request_body`、错误详情、审计报告等大字段，避免统计扫描把日志详情 payload 拉回内存。

## 数据流

### 用量页

1. 前端调用 `usage_stats(startAt, endAt, timezoneOffsetMinutes)`。
2. `GetUsageStatsUsecase` 调用 `RequestLogRepository::stat_rows(Some(start_at), Some(end_at))`。
3. SQLite 使用半开区间 `[start_at, end_at)` 过滤 `created_at`，可命中时间索引。
4. 用例读取 `ChannelRepository::list()`，构建 `channel_id → channel.name` 显示名映射。
5. `domain::stats::aggregate_usage()` 单次遍历日志投影，同时更新：
   - `daily` 日期桶；
   - `by_channel` 累加器；
   - `by_model` 累加器。
6. 后端返回一个 `UsageStats` 快照，前端本地展示和排序。

### 仪表盘

`GetStatsUsecase` 保持原有扫描策略：继续调用 `stat_rows(None, None)` 读取全量轻量投影，以维持累计卡片和固定 7 天趋势的语义。区别只是聚合口径改为复用 `domain::stats::aggregate_dashboard()`，避免 dashboard 和 usage 对 Token、可用率、时区日期桶的定义漂移。

## 统计口径

| 指标 | 口径 |
| --- | --- |
| 请求数 | 日志行数 |
| Token | `total_tokens.unwrap_or(0)` |
| 平均延迟 | `duration_ms` 算术平均；空集合为 `0.0` |
| 可用率 | `status_code < 400` 的比例；空集合为 `0.0` |
| 日期桶 | 前端传入的 `timezoneOffsetMinutes` 转为 `FixedOffset`，按本地日期聚合 |
| 时间范围 | 半开区间 `[start_at, end_at)` |

用量页必须满足不变量：

```text
sum(daily.tokens) == sum(by_channel.tokens) == sum(by_model.tokens)
sum(daily.requests) == sum(by_channel.requests) == sum(by_model.requests)
```

因此：

- `channel_id IS NULL` 进入 `key = "unassigned"` / `name = "未分配渠道"`；
- 空模型名进入 `key = "unknown"` / `name = "unknown"`；
- 后端默认按 `tokens desc` 排行；其他列排序由前端本地完成。

## Command 契约

```rust
#[tauri::command(rename_all = "camelCase")]
pub async fn usage_stats(
    log_repo: State<'_, SqliteRequestLogRepository>,
    channel_repo: State<'_, SqliteChannelRepository>,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    timezone_offset_minutes: i32,
) -> Result<UsageStats, String>
```

前端传参：

```ts
invoke<UsageStats>("usage_stats", {
  startAt,
  endAt,
  timezoneOffsetMinutes,
});
```

`rename_all = "camelCase"` 是命令边界约定，必须和前端 `api.ts` 保持一致。

## 测试

- `domain::stats`：覆盖每日聚合、渠道 / 模型排行、空 Token 计 0、未分配渠道、未知模型和 totals 不变量。
- `usecases::stats`：覆盖 `GetUsageStatsUsecase` 只读取范围内日志，并正确映射渠道名。
- `infrastructure::sqlite::request_log`：覆盖 `stat_rows` 的 `[start_at, end_at)` 范围过滤，以及投影包含 `channel_id` / `model`。
- 旧 dashboard 测试保留，确保抽共享聚合后今日、累计、7 天趋势、时区桶和非法 offset 行为不变。

## 边界与演进

- 当前聚合在 Rust 内存完成，适合单用户本地 SQLite、十万级以内日志量。
- 当范围查询明显变慢时，再考虑把每日 / 排行聚合下推到 SQL `GROUP BY`，或对大范围历史做分页 / 采样。
- 当各视图需要独立刷新、独立缓存或异步加载时，再拆分 `usage_stats` 为多个 command。
- 当引入真实成本核算（Token × 价格）、多 key 账单或跨用户统计时，再考虑独立统计上下文或物化表。

## 相关文档

- [ADR 0002: Usage Stats as a Read Model over Request Logs](../../adr/0002-usage-stats-read-model.md)
- [数据库表结构](../../Database.md)
- [后端架构](../../Architecture-backend.md)
