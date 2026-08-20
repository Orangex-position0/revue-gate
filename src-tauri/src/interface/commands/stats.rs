//! Control plane: dashboard stats command (get_stats).
//!
//! Commands are thin glue: parse input (frontend local timezone offset) → call the stats usecase → return a snapshot.
//! "Today" and the 7-day trend aggregate by the frontend local timezone day boundary, consistent with request logs (see usecases/stats.rs).

use chrono::{DateTime, Utc};
use tauri::State;

use crate::domain::stats::{UsageStats, UsageStatsQuery};
use crate::infrastructure::sqlite::channel::SqliteChannelRepository;
use crate::infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use crate::usecases::stats::{GetStatsUsecase, GetUsageStatsUsecase, StatsQuery, StatsSnapshot};

/// Dashboard stats: card metrics (today / cumulative requests and tokens, average latency, channel availability) + 7-day trend.
/// `timezone_offset_minutes` is the frontend local timezone offset (JS `Date.getTimezoneOffset()`: west positive, east negative).
#[tauri::command(rename_all = "camelCase")]
pub async fn get_stats(
    repo: State<'_, SqliteRequestLogRepository>,
    timezone_offset_minutes: i32,
) -> Result<StatsSnapshot, String> {
    GetStatsUsecase
        .execute(
            &*repo,
            StatsQuery {
                timezone_offset_minutes,
            },
            Utc::now(),
        )
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn usage_stats(
    log_repo: State<'_, SqliteRequestLogRepository>,
    channel_repo: State<'_, SqliteChannelRepository>,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    timezone_offset_minutes: i32,
) -> Result<UsageStats, String> {
    GetUsageStatsUsecase
        .execute(
            &*log_repo,
            &*channel_repo,
            UsageStatsQuery {
                start_at,
                end_at,
                timezone_offset_minutes,
            },
        )
        .await
        .map_err(|e| e.to_string())
}
