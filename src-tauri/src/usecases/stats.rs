//! Dashboard stats use cases: card metrics (today / cumulative request counts and tokens, average latency, channel availability) + 7-day trend aggregation.
//!
//! All aggregation lives here (pure Rust, no SQL grouping / timezone functions): lightweight projection rows (without request bodies) are fetched via
//! `RequestLogRepository::stat_rows` and grouped in memory by the local-timezone day boundary passed from the frontend — sqlx and the InMemory repository
//! share the same aggregation code path, so stats naturally match the request logs (Spec §Testing seam A). The "today" and 7-day window
//! day boundaries are determined by `timezone_offset_minutes` (JS `Date.getTimezoneOffset()` semantics, positive = west),
//! and timezone conversion lives only in this use case layer, never in SQL index conditions (see the date-handling rules).

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::domain::channel::ChannelRepository;
use crate::domain::error::RepositoryError;
use crate::domain::request_log::RequestLogRepository;
use crate::domain::stats::{
    DailyStat, StatsAggregationError, UsageStats, UsageStatsQuery, aggregate_dashboard,
    aggregate_usage,
};

/// Stats query input.
#[derive(Debug, Clone, Copy)]
pub struct StatsQuery {
    /// The frontend local timezone offset from UTC in minutes (JS `Date.getTimezoneOffset()`: positive for west, negative for east).
    pub timezone_offset_minutes: i32,
}

/// Stats use case layer error.
#[derive(Debug, thiserror::Error)]
pub enum StatsError {
    #[error("invalid timezone offset: {0}")]
    InvalidTimezoneOffset(i32),
    #[error("invalid time range")]
    InvalidTimeRange,
    #[error("request log repository error: {0}")]
    Repository(#[from] RepositoryError),
}

impl From<StatsAggregationError> for StatsError {
    fn from(value: StatsAggregationError) -> Self {
        match value {
            StatsAggregationError::InvalidTimezoneOffset(offset) => {
                StatsError::InvalidTimezoneOffset(offset)
            }
            StatsAggregationError::InvalidTimeRange => StatsError::InvalidTimeRange,
        }
    }
}

/// Dashboard stats snapshot: card metrics + 7-day trend (consistent with request log data).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    /// Today's (local timezone) request count.
    pub today_requests: u64,
    /// Today's (local timezone) total tokens.
    pub today_tokens: u64,
    /// Cumulative request count.
    pub total_requests: u64,
    /// Cumulative total tokens.
    pub total_tokens: u64,
    /// Average latency (ms): mean of duration_ms across all requests; 0 when there are no logs.
    pub avg_latency_ms: f64,
    /// Channel availability (0..=1): share of requests with status_code < 400; 0 when there are no logs.
    pub channel_availability: f64,
    /// Last 7 days (including today, oldest → newest) aggregated by local day.
    pub trend: Vec<DailyStat>,
}

/// Dashboard stats use case: fetch all lightweight log rows and aggregate in memory by the local-timezone day boundary.
pub struct GetStatsUsecase;
impl GetStatsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn RequestLogRepository,
        query: StatsQuery,
        now: DateTime<Utc>,
    ) -> Result<StatsSnapshot, StatsError> {
        // One full lightweight scan (no request_body); today / cumulative / average / availability / trend are all derived here.
        let rows = repo.stat_rows(None, None).await?;
        let stats = aggregate_dashboard(&rows, query.timezone_offset_minutes, now)?;

        Ok(StatsSnapshot {
            today_requests: stats.today_requests,
            today_tokens: stats.today_tokens,
            total_requests: stats.total_requests,
            total_tokens: stats.total_tokens,
            avg_latency_ms: stats.avg_latency_ms,
            channel_availability: stats.channel_availability,
            trend: stats.trend,
        })
    }
}

pub struct GetUsageStatsUsecase;
impl GetUsageStatsUsecase {
    pub async fn execute(
        &self,
        log_repo: &dyn RequestLogRepository,
        channel_repo: &dyn ChannelRepository,
        query: UsageStatsQuery,
    ) -> Result<UsageStats, StatsError> {
        let rows = log_repo
            .stat_rows(Some(query.start_at), Some(query.end_at))
            .await?;
        let channels = channel_repo.list().await?;
        let channel_names: HashMap<Uuid, String> =
            channels.into_iter().map(|c| (c.id, c.name)).collect();
        Ok(aggregate_usage(&rows, query, &channel_names)?)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Days, FixedOffset, NaiveDate};

    use super::*;
    use crate::domain::request_log::RequestLog;
    use crate::domain::stats::{LogStatRow, day_start_utc};
    use crate::test_support::{
        InMemoryChannelRepository, InMemoryRequestLogRepository, sample_channel, sample_request_log,
    };

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .expect("valid rfc3339")
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    /// Build a log with controllable created_at / tokens / status / duration.
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

    /// Empty repository: all metrics are zero, trend is 7 zero buckets (today-6 ..= today).
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

    /// Mixed logs spanning days (UTC timezone, offset 0): today / cumulative / avg latency / availability / trend each correct,
    /// no-usage logs count as 0, logs outside the window count only toward cumulative not the trend, and failed requests count in the denominator lowering availability.
    #[tokio::test]
    async fn aggregates_today_total_and_trend() {
        let repo = InMemoryRequestLogRepository::new();
        // Today (08-14): three successes + one failure + one without usage.
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
        // Yesterday (08-13).
        repo.save(&log_at(utc("2026-08-13T10:00:00Z"), Some(5), 200, 50))
            .await
            .expect("save");
        // Outside the 7-day window (08-07): counts only toward cumulative.
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

        // Today: 4 rows (including the no-usage one), tokens 10+20+30+0=60.
        assert_eq!(stats.today_requests, 4);
        assert_eq!(stats.today_tokens, 60);
        // Cumulative: 6 rows, tokens 10+20+30+0+5+1=66.
        assert_eq!(stats.total_requests, 6);
        assert_eq!(stats.total_tokens, 66);
        // Average latency: (100+200+300+0+50+10)/6 = 110.
        assert_eq!(stats.avg_latency_ms, 110.0);
        // Availability: successes (<400) 5 / 6.
        assert!((stats.channel_availability - 5.0 / 6.0).abs() < 1e-9);

        // Trend: today bucket 4 rows / 60, yesterday bucket 1 row / 5, the rest within the window are 0.
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

    /// Timezone bucketing: under UTC+8, 08-13T17:00Z = local 08-14 01:00, which should count toward today and the 08-14 bucket.
    /// Verifies the "local day boundary" rather than the UTC day boundary (data crossing the UTC day boundary still lands in the correct local day).
    #[tokio::test]
    async fn buckets_days_by_timezone_offset() {
        let repo = InMemoryRequestLogRepository::new();
        // 17:00Z = local 08-14 01:00 (+8).
        repo.save(&log_at(utc("2026-08-13T17:00:00Z"), Some(7), 200, 10))
            .await
            .expect("save");
        // 15:59:59Z = local 08-13 23:59:59 (+8).
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

        // now = local 20:00, today = 08-14.
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

    /// Stats match the request logs: filtering all repository logs by the same today window yields the same values as the stats.
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

        // Independent check: filter all logs directly by the today window (consistent with the stats under UTC).
        let logs = repo.stat_rows(None, None).await.expect("stat rows");
        let today = now
            .with_timezone(&FixedOffset::east_opt(0).expect("utc"))
            .date_naive();
        let today_start = day_start_utc(today, FixedOffset::east_opt(0).expect("utc"));
        let today_end = today_start + Days::new(1);
        let today_rows: Vec<&LogStatRow> = logs
            .iter()
            .filter(|r| r.created_at >= today_start && r.created_at < today_end)
            .collect();
        assert_eq!(stats.today_requests as usize, today_rows.len());
        assert_eq!(
            stats.today_tokens,
            today_rows
                .iter()
                .map(|r| r.total_tokens.unwrap_or(0) as u64)
                .sum::<u64>()
        );
    }

    /// Invalid timezone offset: returns InvalidTimezoneOffset, no panic.
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

    #[tokio::test]
    async fn usage_stats_reads_one_range_and_returns_coherent_views() {
        let log_repo = InMemoryRequestLogRepository::new();
        let channel_repo = InMemoryChannelRepository::new();
        let channel = sample_channel();
        channel_repo.save(&channel).await.expect("save channel");

        let mut a = log_at(utc("2026-08-01T10:00:00Z"), Some(10), 200, 100);
        a.channel_id = Some(channel.id);
        a.model = "gpt-4o".to_string();
        log_repo.save(&a).await.expect("save a");

        let mut b = log_at(utc("2026-08-02T10:00:00Z"), Some(20), 500, 300);
        b.channel_id = Some(channel.id);
        b.model = "gpt-4o".to_string();
        log_repo.save(&b).await.expect("save b");

        let mut c = log_at(utc("2026-08-03T10:00:00Z"), Some(99), 200, 100);
        c.model = "outside".to_string();
        log_repo.save(&c).await.expect("save c");

        let stats = GetUsageStatsUsecase
            .execute(
                &log_repo,
                &channel_repo,
                UsageStatsQuery {
                    start_at: utc("2026-08-01T00:00:00Z"),
                    end_at: utc("2026-08-03T00:00:00Z"),
                    timezone_offset_minutes: 0,
                },
            )
            .await
            .expect("usage stats");

        assert_eq!(stats.daily.len(), 2);
        assert_eq!(stats.daily.iter().map(|d| d.tokens).sum::<u64>(), 30);
        assert_eq!(stats.by_channel[0].key, channel.id.to_string());
        assert_eq!(stats.by_channel[0].name, channel.name);
        assert_eq!(stats.by_channel[0].tokens, 30);
        assert!((stats.by_channel[0].availability - 0.5).abs() < 1e-9);
        assert_eq!(stats.by_model[0].key, "gpt-4o");
        assert_eq!(stats.by_model[0].tokens, 30);
    }
}
