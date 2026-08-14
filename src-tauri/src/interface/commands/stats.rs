//! 控制面：仪表盘统计命令（get_stats）。
//!
//! 命令是薄胶水：解析入参（前端本地时区偏移）→ 调统计用例 → 返回快照。
//! "今日" 与 7 天趋势按前端本地时区日界聚合，数据与请求日志一致（见 usecases/stats.rs）。

use chrono::Utc;
use tauri::State;

use crate::infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use crate::usecases::stats::{GetStatsUsecase, StatsQuery, StatsSnapshot};

/// 仪表盘统计：卡片指标（今日 / 累计请求数与 Token、平均延迟、渠道可用率）+ 7 天趋势。
/// `timezone_offset_minutes` 为前端本地时区偏移（JS `Date.getTimezoneOffset()`：西为正，东为负）。
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
