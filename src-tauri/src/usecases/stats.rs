//! Dashboard stats use cases: card metrics (today / cumulative request counts and tokens, average latency, channel availability) + 7-day trend aggregation.
//!
//! All aggregation lives here (pure Rust, no SQL grouping / timezone functions): lightweight projection rows (without request bodies) are fetched via
//! `RequestLogRepository::stat_rows` and grouped in memory by the local-timezone day boundary passed from the frontend — sqlx and the InMemory repository
//! share the same aggregation code path, so stats naturally match the request logs (Spec §Testing seam A). The "today" and 7-day window
//! day boundaries are determined by `timezone_offset_minutes` (JS `Date.getTimezoneOffset()` semantics, positive = west),
//! and timezone conversion lives only in this use case layer, never in SQL index conditions (see the date-handling rules).

use chrono::{DateTime, Days, FixedOffset, NaiveDate, Utc};
use serde::Serialize;

use crate::domain::error::RepositoryError;
use crate::domain::request_log::{LogStatRow, RequestLogRepository};

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
    #[error("request log repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// Daily stat: one point of the 7-day trend line (date is the local-timezone date).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyStat {
    /// Local date (YYYY-MM-DD).
    pub date: NaiveDate,
    /// Requests on that day.
    pub requests: u64,
    /// Total tokens that day (requests without usage count as 0).
    pub tokens: u64,
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
        // JS getTimezoneOffset semantics: positive = west, negative = east → east_opt(-minutes*60).
        // checked guards against i32::MIN negation overflow (the JS range is about ±840 minutes; this only prevents a debug panic from abnormal input).
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

        // One full lightweight scan (no request_body); today / cumulative / average / availability / trend are all derived here.
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

        // Today's window [today 00:00, tomorrow 00:00) (local day boundary converted to UTC).
        let today_start = day_start_utc(today, tz);
        let today_end = day_start_utc(today + Days::new(1), tz);
        let today_rows: Vec<&LogStatRow> = rows
            .iter()
            .filter(|r| r.created_at >= today_start && r.created_at < today_end)
            .collect();
        let today_requests = today_rows.len() as u64;
        let today_tokens = sum_tokens_of(&today_rows);

        // 7-day trend: today-6 ..= today, grouped by local day boundary (oldest → newest).
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

/// Convert local midnight of a day to UTC. FixedOffset has no DST jumps, so `single` always succeeds
/// (the offset is captured by the frontend at "now"; if the window crosses a DST switch, the current offset is used as an approximation — today's bucket
/// matches the LogsPage today filter; historical buckets may deviate across DST, an acceptable approximation).
fn day_start_utc(day: NaiveDate, tz: FixedOffset) -> DateTime<Utc> {
    day.and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_local_timezone(tz)
        .single()
        .expect("fixed offset has no ambiguous local times")
        .with_timezone(&Utc)
}

/// Sum tokens: requests without usage count as 0.
fn sum_tokens(rows: &[LogStatRow]) -> u64 {
    rows.iter()
        .map(|r| r.total_tokens.unwrap_or(0) as u64)
        .sum()
}

/// Token sum over a slice of references (avoids cloning rows).
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
}
