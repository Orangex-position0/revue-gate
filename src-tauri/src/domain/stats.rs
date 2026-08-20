//! Usage and dashboard statistics read model over request logs.

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Days, FixedOffset, NaiveDate, Utc};
use serde::Serialize;
use uuid::Uuid;

/// Stat projection row: lightweight columns for stats aggregation (no request_body).
#[derive(Debug, Clone, PartialEq)]
pub struct LogStatRow {
    /// Upstream channel used by the request; None means no channel was assigned.
    pub channel_id: Option<Uuid>,
    /// Client-requested model name.
    pub model: String,
    /// HTTP status code returned to the client (<400 counts as success).
    pub status_code: u16,
    /// Total tokens for this request; None is counted as 0.
    pub total_tokens: Option<u32>,
    /// Total request duration (milliseconds).
    pub duration_ms: u64,
    pub created_at: DateTime<Utc>,
}

/// Daily stat point (date is in the caller's local timezone).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyStat {
    pub date: NaiveDate,
    pub requests: u64,
    pub tokens: u64,
}

/// Leaderboard row for channel or model usage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankRow {
    pub key: String,
    pub name: String,
    pub requests: u64,
    pub tokens: u64,
    pub avg_latency_ms: f64,
    pub availability: f64,
}

/// Coarse usage stats response: one snapshot for all usage page views.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    pub daily: Vec<DailyStat>,
    pub by_channel: Vec<RankRow>,
    pub by_model: Vec<RankRow>,
}

#[derive(Debug, Clone, Copy)]
pub struct UsageStatsQuery {
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    /// JS `Date.getTimezoneOffset()` semantics: positive west, negative east.
    pub timezone_offset_minutes: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DashboardStats {
    pub total_requests: u64,
    pub total_tokens: u64,
    pub avg_latency_ms: f64,
    pub channel_availability: f64,
    pub today_requests: u64,
    pub today_tokens: u64,
    pub trend: Vec<DailyStat>,
}

#[derive(Debug, thiserror::Error)]
pub enum StatsAggregationError {
    #[error("invalid timezone offset: {0}")]
    InvalidTimezoneOffset(i32),
    #[error("usage stats end_at must be after start_at")]
    InvalidTimeRange,
}

pub fn fixed_offset(minutes: i32) -> Result<FixedOffset, StatsAggregationError> {
    let east_seconds = minutes
        .checked_neg()
        .and_then(|m| m.checked_mul(60))
        .ok_or(StatsAggregationError::InvalidTimezoneOffset(minutes))?;
    FixedOffset::east_opt(east_seconds).ok_or(StatsAggregationError::InvalidTimezoneOffset(minutes))
}

pub fn day_start_utc(day: NaiveDate, tz: FixedOffset) -> DateTime<Utc> {
    day.and_hms_opt(0, 0, 0)
        .expect("midnight is a valid time")
        .and_local_timezone(tz)
        .single()
        .expect("fixed offset has no ambiguous local times")
        .with_timezone(&Utc)
}

pub fn aggregate_dashboard(
    rows: &[LogStatRow],
    timezone_offset_minutes: i32,
    now: DateTime<Utc>,
) -> Result<DashboardStats, StatsAggregationError> {
    let tz = fixed_offset(timezone_offset_minutes)?;
    let today = now.with_timezone(&tz).date_naive();
    let total_requests = rows.len() as u64;
    let total_tokens = sum_tokens(rows.iter());
    let avg_latency_ms = avg_latency(rows);
    let channel_availability = availability(rows);

    let today_start = day_start_utc(today, tz);
    let today_end = day_start_utc(today + Days::new(1), tz);
    let today_rows: Vec<&LogStatRow> = rows
        .iter()
        .filter(|r| r.created_at >= today_start && r.created_at < today_end)
        .collect();
    let today_requests = today_rows.len() as u64;
    let today_tokens = sum_tokens(today_rows.iter().copied());

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
            tokens: sum_tokens(day_rows.iter().copied()),
        });
    }

    Ok(DashboardStats {
        total_requests,
        total_tokens,
        avg_latency_ms,
        channel_availability,
        today_requests,
        today_tokens,
        trend,
    })
}

pub fn aggregate_usage(
    rows: &[LogStatRow],
    query: UsageStatsQuery,
    channel_names: &HashMap<Uuid, String>,
) -> Result<UsageStats, StatsAggregationError> {
    if query.end_at <= query.start_at {
        return Err(StatsAggregationError::InvalidTimeRange);
    }
    let tz = fixed_offset(query.timezone_offset_minutes)?;
    let mut daily = daily_buckets(query.start_at, query.end_at, tz);
    let mut channel_groups: BTreeMap<String, (String, RankAccumulator)> = BTreeMap::new();
    let mut model_groups: BTreeMap<String, (String, RankAccumulator)> = BTreeMap::new();

    for row in rows {
        let day = row.created_at.with_timezone(&tz).date_naive();
        if let Some(bucket) = daily.get_mut(&day) {
            bucket.requests += 1;
            bucket.tokens += row.total_tokens.unwrap_or(0) as u64;
        }

        let (channel_key, channel_name) = match row.channel_id {
            Some(id) => (
                id.to_string(),
                channel_names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| id.to_string()),
            ),
            None => ("unassigned".to_string(), "未分配渠道".to_string()),
        };
        channel_groups
            .entry(channel_key)
            .or_insert_with(|| (channel_name, RankAccumulator::default()))
            .1
            .add(row);

        let model = row.model.trim();
        let (model_key, model_name) = if model.is_empty() {
            ("unknown".to_string(), "unknown".to_string())
        } else {
            (model.to_string(), model.to_string())
        };
        model_groups
            .entry(model_key)
            .or_insert_with(|| (model_name, RankAccumulator::default()))
            .1
            .add(row);
    }

    Ok(UsageStats {
        daily: daily.into_values().collect(),
        by_channel: rank_rows(channel_groups),
        by_model: rank_rows(model_groups),
    })
}

fn daily_buckets(
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    tz: FixedOffset,
) -> BTreeMap<NaiveDate, DailyStat> {
    let start_day = start_at.with_timezone(&tz).date_naive();
    let end_day = (end_at - chrono::TimeDelta::nanoseconds(1))
        .with_timezone(&tz)
        .date_naive();
    let mut buckets = BTreeMap::new();
    let mut day = start_day;
    while day <= end_day {
        buckets.insert(
            day,
            DailyStat {
                date: day,
                requests: 0,
                tokens: 0,
            },
        );
        day = day + Days::new(1);
    }
    buckets
}

#[derive(Debug, Default)]
struct RankAccumulator {
    requests: u64,
    tokens: u64,
    latency_ms_sum: u64,
    success: u64,
}

impl RankAccumulator {
    fn add(&mut self, row: &LogStatRow) {
        self.requests += 1;
        self.tokens += row.total_tokens.unwrap_or(0) as u64;
        self.latency_ms_sum += row.duration_ms;
        if row.status_code < 400 {
            self.success += 1;
        }
    }

    fn avg_latency_ms(&self) -> f64 {
        if self.requests == 0 {
            0.0
        } else {
            self.latency_ms_sum as f64 / self.requests as f64
        }
    }

    fn availability(&self) -> f64 {
        if self.requests == 0 {
            0.0
        } else {
            self.success as f64 / self.requests as f64
        }
    }
}

fn rank_rows(grouped: BTreeMap<String, (String, RankAccumulator)>) -> Vec<RankRow> {
    let mut ranks: Vec<RankRow> = grouped
        .into_iter()
        .map(|(key, (name, acc))| RankRow {
            key,
            name,
            requests: acc.requests,
            tokens: acc.tokens,
            avg_latency_ms: acc.avg_latency_ms(),
            availability: acc.availability(),
        })
        .collect();
    ranks.sort_by(|a, b| {
        b.tokens
            .cmp(&a.tokens)
            .then_with(|| b.requests.cmp(&a.requests))
            .then_with(|| a.name.cmp(&b.name))
    });
    ranks
}

fn sum_tokens<'a>(rows: impl IntoIterator<Item = &'a LogStatRow>) -> u64 {
    rows.into_iter()
        .map(|r| r.total_tokens.unwrap_or(0) as u64)
        .sum()
}

fn avg_latency(rows: &[LogStatRow]) -> f64 {
    if rows.is_empty() {
        0.0
    } else {
        rows.iter().map(|r| r.duration_ms as f64).sum::<f64>() / rows.len() as f64
    }
}

fn availability(rows: &[LogStatRow]) -> f64 {
    if rows.is_empty() {
        0.0
    } else {
        rows.iter().filter(|r| r.status_code < 400).count() as f64 / rows.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .expect("valid rfc3339")
    }

    fn row(
        channel_id: Option<Uuid>,
        model: &str,
        tokens: Option<u32>,
        status_code: u16,
        duration_ms: u64,
        created_at: &str,
    ) -> LogStatRow {
        LogStatRow {
            channel_id,
            model: model.to_string(),
            status_code,
            total_tokens: tokens,
            duration_ms,
            created_at: utc(created_at),
        }
    }

    #[test]
    fn usage_aggregates_daily_channel_and_model_in_one_snapshot() {
        let channel_a = Uuid::now_v7();
        let channel_b = Uuid::now_v7();
        let rows = vec![
            row(
                Some(channel_a),
                "gpt-4o",
                Some(10),
                200,
                100,
                "2026-08-01T02:00:00Z",
            ),
            row(
                Some(channel_a),
                "gpt-4o",
                None,
                500,
                300,
                "2026-08-01T03:00:00Z",
            ),
            row(
                Some(channel_b),
                "claude-3-5-sonnet",
                Some(30),
                200,
                200,
                "2026-08-02T02:00:00Z",
            ),
            row(None, "", Some(5), 200, 50, "2026-08-02T04:00:00Z"),
        ];
        let channel_names = HashMap::from([
            (channel_a, "OpenAI Prod".to_string()),
            (channel_b, "Claude Backup".to_string()),
        ]);

        let stats = aggregate_usage(
            &rows,
            UsageStatsQuery {
                start_at: utc("2026-08-01T00:00:00Z"),
                end_at: utc("2026-08-03T00:00:00Z"),
                timezone_offset_minutes: 0,
            },
            &channel_names,
        )
        .expect("usage stats");

        assert_eq!(
            stats.daily,
            vec![
                DailyStat {
                    date: NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
                    requests: 2,
                    tokens: 10,
                },
                DailyStat {
                    date: NaiveDate::from_ymd_opt(2026, 8, 2).unwrap(),
                    requests: 2,
                    tokens: 35,
                },
            ]
        );
        assert_eq!(stats.by_channel[0].key, channel_b.to_string());
        assert_eq!(stats.by_channel[0].name, "Claude Backup");
        assert_eq!(stats.by_channel[0].tokens, 30);
        assert_eq!(stats.by_channel[1].key, channel_a.to_string());
        assert_eq!(stats.by_channel[1].requests, 2);
        assert_eq!(stats.by_channel[1].tokens, 10);
        assert!((stats.by_channel[1].availability - 0.5).abs() < 1e-9);
        assert_eq!(stats.by_channel[2].key, "unassigned");
        assert_eq!(stats.by_model[0].key, "claude-3-5-sonnet");
        assert_eq!(stats.by_model[1].key, "gpt-4o");
        assert_eq!(stats.by_model[2].key, "unknown");

        let daily_tokens: u64 = stats.daily.iter().map(|r| r.tokens).sum();
        let channel_tokens: u64 = stats.by_channel.iter().map(|r| r.tokens).sum();
        let model_tokens: u64 = stats.by_model.iter().map(|r| r.tokens).sum();
        assert_eq!(daily_tokens, channel_tokens);
        assert_eq!(daily_tokens, model_tokens);
    }
}
