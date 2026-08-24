# ADR 0002: Usage Stats as a Read Model over Request Logs

- **Status**: accepted
- **Date**: 2026-08-20
- **Authors**: Orangex-position0 / Claude

## Context

The dashboard already exposes `StatsSnapshot` (today/cumulative request and token counts, average latency, channel availability, 7-day trend) via `GetStatsUsecase`, which does one full scan of `request_logs` through `RequestLogRepository::stat_rows(None, None)` and aggregates in memory by local-time day.

We now want a standalone **usage statistics page** with three views: a by-day request/token bar chart over an arbitrary time range, a per-channel consumption leaderboard, and a per-model leaderboard. The `request_logs` table already stores `channel_id` (nullable), `model`, `prompt/completion/total_tokens`, `duration_ms`, `status_code`, `created_at` — but the lightweight projection `LogStatRow` used by stats only projects `status_code`, `total_tokens`, `duration_ms`, `created_at`. It carries no `channel_id` or `model`, so per-channel / per-model breakdown is impossible today.

revue-gate is a single-user, local, SQLite gateway. Log volume is in the tens-to-hundreds of thousands of rows at most; there is no multi-tenant cost metering, no billing, and no external accounting surface.

## Decision

We will treat **usage statistics as a read model / projection over request logs**, not as a separate bounded context. Specifically:

- Add a `domain/stats.rs` module hosting the stats read model: `LogStatRow` (extended), `DailyStat`, `RankRow`, `UsageStats`, `UsageStatsQuery`, plus a shared in-memory aggregation core (local-day boundaries, timezone conversion, token-count-as-zero, availability, average latency, and grouping by day / channel / model).
- Extend `LogStatRow` with `channel_id: Option<String>` and `model: String`; update `stat_rows` SQL to select them and to honor a `[start_at, end_at)` range filter (so usage queries hit the `created_at` index instead of a full scan).
- Add a new `GetUsageStatsUsecase` reading `RequestLogRepository::stat_rows(start, end)` and producing `UsageStats { daily, by_channel, by_model }` from one in-memory pass.
- Expose a single coarse-grained Tauri command `usage_stats` returning all three views at once, sharing the same range and timezone.
- Keep `GetStatsUsecase` (dashboard) behavior unchanged: it keeps scanning the full range; it only switches to the shared aggregation core so both paths agree on the same counting semantics.
- Leaderboard rows are returned in default `tokens` descending order; the frontend re-sorts locally for other columns.
- `by_channel` emits an "unassigned" row for `channel_id IS NULL`; `by_model` emits an "unknown" row for any unmapped model. Invariant: `daily` total = sum of `by_channel` = sum of `by_model` (all over the queried range).
- `RankRow` carries `key` (channel_id or model) plus a display `name` (channel name joined from `channels`, or the model string).

## Consequences

- Positive: One query pass and one `invoke` produce all three coherent views (bar chart, channel leaderboard, model leaderboard) with no cross-request drift or extra scans; fits the project's "local IPC refetch is zero-cost, keep out of the store" convention.
- Positive: Extracting the shared aggregation core into `domain/stats.rs` makes dashboard and usage share one counting authority (timezone boundaries, token-as-zero, availability), so the two pages never drift on口径 and a future metric fix lands in one place.
- Positive: Modeling stats as a read model (not a separate context) avoids a second data source, a second repository, and a duplicated aggregation path for what is purely a projection of `request_logs`.
- Negative: One command returns all three views even when only one is visible; acceptable for local IPC, becomes a cost only if a single dimension grows large and needs isolated caching or SQL grouping pushdown.
- Negative: `request_logs` aggregation is still in-memory in Rust rather than pushed down to SQL `GROUP BY`; fine at this scale through the index-filtered range, revisit only for very large histories.
- Neutral: Leaderboard ordering is a presentation concern resolved by local re-sort; the backend pins only the default (tokens desc).
- Neutral: Dashboard and usage share the aggregation core but not the scan-range strategy (dashboard stays full-range; usage is range-filtered), preserving the dashboard's existing cumulative semantics.

## Alternatives

- Create a separate `Statistic` bounded context with its own repository and data source. Rejected because usage is a pure projection of `request_logs`; an independent context adds a second read path, anti-corruption surface, and consistency risk (同源两读、口径漂移) for a single local process.
- Model `LogStatRow` without extending it and compute per-channel/per-model breakdown only in usecases. Rejected because the projection must carry `channel_id` and `model` to aggregate by them at all; pushing them higher without placing them in the domain row leaks the raw shape and duplicates logic.
- Expose fine-grained commands (`usage_trend`, `usage_ranking`). Rejected because the three views share range, timezone, and one scan; separate commands multiply scans, orchestration, and tests for no local-IPC benefit, and allow the views to drift apart in snapshot timing.
- Have the backend accept a `sort_by`/`order` parameter. Rejected because the leaderboard is a small in-memory list (dozens of rows); local re-sort gives instant response with no extra round-trip and no extra backend ordering logic.
- Drop unassigned/unknown requests from leaderboards. Rejected because the invariant "leaderboard sums equal the chart total" is the core value of usage accounting; dropping `channel_id IS NULL` requests would make the page show fewer requests than the bar chart and look like a bug.
- Reuse `GetStatsUsecase` and add range params to it. Rejected because dashboard semantics (today/cumulative cards + fixed 7-day) and usage semantics (arbitrary range + ranking) are different concerns; forcing both through one usecase couples their evolution and risks changing the dashboard's behavior.

## Reversal Criteria

Revisit this decision when:

- Log volume grows large enough that in-memory aggregation over an index-filtered range is measurably slow; then push `GROUP BY` down to SQL or add pagination/sampling.
- A single view needs its own refresh cadence or caching independent of the others; then split the command per view.
- Users need real cost/billing metering (token × price) or cross-key accounting; that introduces a second aggregation axis and a separate concern worth its own context.
