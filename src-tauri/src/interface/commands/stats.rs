//! Control plane: dashboard stats command (get_stats).
//!
//! Commands are thin glue: parse input (frontend local timezone offset) → call the stats usecase → return a snapshot.
//! "Today" and the 7-day trend aggregate by the frontend local timezone day boundary, consistent with request logs (see usecases/stats.rs).

use chrono::Utc;
use tauri::State;

use crate::infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use crate::usecases::stats::{GetStatsUsecase, StatsQuery, StatsSnapshot};

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
