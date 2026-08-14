//! 仪表盘统计用例：卡片指标（今日 / 累计请求数与 Token、平均延迟、渠道可用率）+ 7 天趋势聚合。
//!
//! 聚合逻辑全部在此（纯 Rust，不做 SQL 分组 / 时区函数）：经 `RequestLogRepository::stat_rows`
//! 拉取轻量投影行（不含请求体），按前端传入的本地时区日界在内存分组——sqlx 与 InMemory 仓储
//! 共用同一聚合代码路径，统计天然与请求日志一致（Spec §Testing seam A）。"今日" 与 7 天窗口
//! 的日界由 `timezone_offset_minutes`（JS `Date.getTimezoneOffset()` 语义，正 = 西）决定，
//! 时区转换只在本用例层，不落在 SQL 索引条件上（见 date-handling 规范）。

use chrono::{DateTime, Days, FixedOffset, NaiveDate, Utc};
use serde::Serialize;

use crate::domain::error::RepositoryError;
use crate::domain::request_log::{LogStatRow, RequestLogRepository};

/// 统计查询入参。
#[derive(Debug, Clone, Copy)]
pub struct StatsQuery {
    /// 前端本地时区相对 UTC 的分钟偏移（JS `Date.getTimezoneOffset()`：西为正，东为负）。
    pub timezone_offset_minutes: i32,
}

/// 统计用例层错误。
#[derive(Debug, thiserror::Error)]
pub enum StatsError {
    #[error("invalid timezone offset: {0}")]
    InvalidTimezoneOffset(i32),
    #[error("request log repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// 单日统计：7 天趋势折线的一个点（date 为本地时区日期）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyStat {
    /// 本地日期（YYYY-MM-DD）。
    pub date: NaiveDate,
    /// 当日请求数。
    pub requests: u64,
    /// 当日总 token（无 usage 的请求按 0 计）。
    pub tokens: u64,
}

/// 仪表盘统计快照：卡片指标 + 7 天趋势（与请求日志数据一致）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    /// 今日（本地时区）请求数。
    pub today_requests: u64,
    /// 今日（本地时区）总 token。
    pub today_tokens: u64,
    /// 累计请求数。
    pub total_requests: u64,
    /// 累计总 token。
    pub total_tokens: u64,
    /// 平均延迟（毫秒）：全部请求 duration_ms 的均值；无日志为 0。
    pub avg_latency_ms: f64,
    /// 渠道可用率（0..=1）：status_code < 400 的请求占比；无日志为 0。
    pub channel_availability: f64,
    /// 最近 7 天（含今日，旧→新）按本地日聚合。
    pub trend: Vec<DailyStat>,
}

/// 仪表盘统计用例：拉取全部轻量日志行，按本地时区日界在内存聚合。
pub struct GetStatsUsecase;
impl GetStatsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn RequestLogRepository,
        query: StatsQuery,
        now: DateTime<Utc>,
    ) -> Result<StatsSnapshot, StatsError> {
        // JS getTimezoneOffset 语义：正 = 西，负 = 东 → east_opt(-分钟*60)。
        // checked 防 i32::MIN 取负溢出（JS 实际范围约 ±840 分钟，仅防异常入参触发 debug panic）。
        let east_seconds = query
            .timezone_offset_minutes
            .checked_neg()
            .and_then(|m| m.checked_mul(60))
            .ok_or(StatsError::InvalidTimezoneOffset(
                query.timezone_offset_minutes,
            ))?;
        let tz = FixedOffset::east_opt(east_seconds).ok_or(StatsError::InvalidTimezoneOffset(
            query.timezone_offset_minutes,
        ))?;
        let today = now.with_timezone(&tz).date_naive();

        // 一次全量轻量扫描（无 request_body），今日 / 累计 / 平均 / 可用率 / 趋势全部在此推导。
        let rows = repo.stat_rows(None, None).await?;

        let total_requests = rows.len() as u64;
        let total_tokens = sum_tokens(&rows);
        let avg_latency_ms = if rows.is_empty() {
            0.0
        } else {
            rows.iter().map(|r| r.duration_ms as f64).sum::<f64>() / rows.len() as f64
        };
        let success = rows.iter().filter(|r| r.status_code < 400).count() as u64;
        let channel_availability = if total_requests == 0 {
            0.0
        } else {
            success as f64 / total_requests as f64
        };

        // 今日窗口 [今日 00:00, 明日 00:00)（本地日界转 UTC）。
        let today_start = day_start_utc(today, tz);
        let today_end = day_start_utc(today + Days::new(1), tz);
        let today_rows: Vec<&LogStatRow> = rows
            .iter()
            .filter(|r| r.created_at >= today_start && r.created_at < today_end)
            .collect();
        let today_requests = today_rows.len() as u64;
        let today_tokens = sum_tokens_of(&today_rows);

        // 7 天趋势：today-6 ..= today，逐日按本地日界分组（旧→新）。
        let mut trend = Vec::with_capacity(7);
        for i in (0..7).rev() {
            let day = today - Days::new(i);
            let start = day_start_utc(day, tz);
            let end = day_start_utc(day + Days::new(1), tz);
            let day_rows: Vec<&LogStatRow> = rows
                .iter()
                .filter(|r| r.created_at >= start && r.created_at < end)
                .collect();
            trend.push(DailyStat {
                date: day,
                requests: day_rows.len() as u64,
                tokens: sum_tokens_of(&day_rows),
            });
        }

        Ok(StatsSnapshot {
            today_requests,
            today_tokens,
            total_requests,
            total_tokens,
            avg_latency_ms,
            channel_availability,
            trend,
        })
    }
}

/// 本地某日零点转 UTC。FixedOffset 无 DST 跳变，`single` 恒成立
/// （偏移由前端在"现在"取到，窗口内若跨 DST 切换以现偏移近似——今日桶与
/// LogsPage 今日筛选口径一致；历史桶在跨 DST 时可能偏差，可接受近似）。
fn day_start_utc(day: NaiveDate, tz: FixedOffset) -> DateTime<Utc> {
    day.and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_local_timezone(tz)
        .single()
        .expect("fixed offset has no ambiguous local times")
        .with_timezone(&Utc)
}

/// token 求和：无 usage 的请求按 0 计。
fn sum_tokens(rows: &[LogStatRow]) -> u64 {
    rows.iter()
        .map(|r| r.total_tokens.unwrap_or(0) as u64)
        .sum()
}

/// 引用切片版 token 求和（避免克隆行）。
fn sum_tokens_of(rows: &[&LogStatRow]) -> u64 {
    rows.iter()
        .map(|r| r.total_tokens.unwrap_or(0) as u64)
        .sum()
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;
    use crate::domain::request_log::RequestLog;
    use crate::test_support::{InMemoryRequestLogRepository, sample_request_log};

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .expect("valid rfc3339")
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    /// 构造一条 created_at / tokens / status / duration 可控的日志。
    fn log_at(
        created_at: DateTime<Utc>,
        total_tokens: Option<u32>,
        status_code: u16,
        duration_ms: u64,
    ) -> RequestLog {
        let mut log = sample_request_log();
        log.created_at = created_at;
        log.total_tokens = total_tokens;
        log.status_code = status_code;
        log.duration_ms = duration_ms;
        log
    }

    /// 空仓储：全部指标归零，趋势为 7 个零桶（today-6 ..= today）。
    #[tokio::test]
    async fn empty_repo_returns_zeroed_snapshot() {
        let repo = InMemoryRequestLogRepository::new();
        let stats = GetStatsUsecase
            .execute(
                &repo,
                StatsQuery {
                    timezone_offset_minutes: 0,
                },
                utc("2026-08-14T12:00:00Z"),
            )
            .await
            .expect("stats");

        assert_eq!(stats.today_requests, 0);
        assert_eq!(stats.today_tokens, 0);
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.total_tokens, 0);
        assert_eq!(stats.avg_latency_ms, 0.0);
        assert_eq!(stats.channel_availability, 0.0);
        assert_eq!(stats.trend.len(), 7);
        assert!(stats.trend.iter().all(|d| d.requests == 0 && d.tokens == 0));
        assert_eq!(stats.trend.first().unwrap().date, date(2026, 8, 8));
        assert_eq!(stats.trend.last().unwrap().date, date(2026, 8, 14));
    }

    /// 混合跨天日志（UTC 时区，offset 0）：今日 / 累计 / 平均延迟 / 可用率 / 趋势逐项正确，
    /// 无 usage 按 0 计，窗口外日志只计累计不计趋势，失败请求计入分母压低可用率。
    #[tokio::test]
    async fn aggregates_today_total_and_trend() {
        let repo = InMemoryRequestLogRepository::new();
        // 今日（08-14）：三条成功 + 一条失败 + 一条无 usage。
        repo.save(&log_at(utc("2026-08-14T10:00:00Z"), Some(10), 200, 100))
            .await
            .expect("save");
        repo.save(&log_at(utc("2026-08-14T11:00:00Z"), Some(20), 200, 200))
            .await
            .expect("save");
        repo.save(&log_at(utc("2026-08-14T12:00:00Z"), Some(30), 500, 300))
            .await
            .expect("save");
        repo.save(&log_at(utc("2026-08-14T13:00:00Z"), None, 200, 0))
            .await
            .expect("save");
        // 昨日（08-13）。
        repo.save(&log_at(utc("2026-08-13T10:00:00Z"), Some(5), 200, 50))
            .await
            .expect("save");
        // 7 天窗口外（08-07）：只计入累计。
        repo.save(&log_at(utc("2026-08-07T23:59:59Z"), Some(1), 200, 10))
            .await
            .expect("save");

        let stats = GetStatsUsecase
            .execute(
                &repo,
                StatsQuery {
                    timezone_offset_minutes: 0,
                },
                utc("2026-08-14T12:00:00Z"),
            )
            .await
            .expect("stats");

        // 今日：4 条（含无 usage），token 10+20+30+0=60。
        assert_eq!(stats.today_requests, 4);
        assert_eq!(stats.today_tokens, 60);
        // 累计：6 条，token 10+20+30+0+5+1=66。
        assert_eq!(stats.total_requests, 6);
        assert_eq!(stats.total_tokens, 66);
        // 平均延迟：(100+200+300+0+50+10)/6 = 110。
        assert_eq!(stats.avg_latency_ms, 110.0);
        // 可用率：成功(<400) 5 / 6。
        assert!((stats.channel_availability - 5.0 / 6.0).abs() < 1e-9);

        // 趋势：今日桶 4 条 / 60，昨日桶 1 条 / 5，窗口内其余为 0。
        let trend = &stats.trend;
        assert_eq!(
            trend[6],
            DailyStat {
                date: date(2026, 8, 14),
                requests: 4,
                tokens: 60
            }
        );
        assert_eq!(
            trend[5],
            DailyStat {
                date: date(2026, 8, 13),
                requests: 1,
                tokens: 5
            }
        );
        assert_eq!(
            trend[0],
            DailyStat {
                date: date(2026, 8, 8),
                requests: 0,
                tokens: 0
            }
        );
    }

    /// 时区分桶：UTC+8 下 08-13T17:00Z = 本地 08-14 01:00，应计入今日与 08-14 桶。
    /// 验证"本地日界"而非 UTC 日界（数据跨 UTC 日界仍归属正确的本地日）。
    #[tokio::test]
    async fn buckets_days_by_timezone_offset() {
        let repo = InMemoryRequestLogRepository::new();
        // 17:00Z = 本地 08-14 01:00（+8）。
        repo.save(&log_at(utc("2026-08-13T17:00:00Z"), Some(7), 200, 10))
            .await
            .expect("save");
        // 15:59:59Z = 本地 08-13 23:59:59（+8）。
        repo.save(&log_at(utc("2026-08-13T15:59:59Z"), Some(3), 200, 10))
            .await
            .expect("save");

        let stats = GetStatsUsecase
            .execute(
                &repo,
                StatsQuery {
                    timezone_offset_minutes: -480,
                },
                utc("2026-08-14T12:00:00Z"),
            )
            .await
            .expect("stats");

        // now = 本地 20:00，今日 = 08-14。
        assert_eq!(stats.today_requests, 1);
        assert_eq!(stats.today_tokens, 7);
        assert_eq!(
            stats.trend[6],
            DailyStat {
                date: date(2026, 8, 14),
                requests: 1,
                tokens: 7
            }
        );
        assert_eq!(
            stats.trend[5],
            DailyStat {
                date: date(2026, 8, 13),
                requests: 1,
                tokens: 3
            }
        );
    }

    /// 统计与请求日志一致：直接对仓储全部日志按同一今日窗口过滤，与统计口径逐项吻合。
    #[tokio::test]
    async fn stats_match_direct_log_filtering() {
        let repo = InMemoryRequestLogRepository::new();
        repo.save(&log_at(utc("2026-08-14T00:00:00Z"), Some(1), 200, 10))
            .await
            .expect("save");
        repo.save(&log_at(utc("2026-08-13T23:59:59Z"), Some(2), 200, 20))
            .await
            .expect("save");
        repo.save(&log_at(utc("2026-08-14T23:59:59Z"), Some(4), 400, 40))
            .await
            .expect("save");

        let now = utc("2026-08-14T12:00:00Z");
        let stats = GetStatsUsecase
            .execute(
                &repo,
                StatsQuery {
                    timezone_offset_minutes: 0,
                },
                now,
            )
            .await
            .expect("stats");

        // 独立口径：从全部日志直接过滤今日窗口（UTC 时区下与统计口径一致）。
        let logs = repo.stat_rows(None, None).await.expect("stat rows");
        let today_start = now
            .with_timezone(&FixedOffset::east_opt(0).expect("utc"))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("midnight")
            .and_utc();
        let today_end = today_start + Days::new(1);
        let today_rows: Vec<&LogStatRow> = logs
            .iter()
            .filter(|r| r.created_at >= today_start && r.created_at < today_end)
            .collect();
        assert_eq!(stats.today_requests as usize, today_rows.len());
        assert_eq!(stats.today_tokens, sum_tokens_of(&today_rows));
    }

    /// 非法时区偏移：返回 InvalidTimezoneOffset，不 panic。
    #[tokio::test]
    async fn invalid_timezone_offset_errors() {
        let repo = InMemoryRequestLogRepository::new();
        let result = GetStatsUsecase
            .execute(
                &repo,
                StatsQuery {
                    timezone_offset_minutes: 25 * 60,
                },
                utc("2026-08-14T12:00:00Z"),
            )
            .await;
        assert!(matches!(result, Err(StatsError::InvalidTimezoneOffset(_))));
    }
}
