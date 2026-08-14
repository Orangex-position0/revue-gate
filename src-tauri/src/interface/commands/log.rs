//! Control plane: request log commands (list / detail / delete_before / clear).
//!
//! Commands are thin glue: parse input → call usecases → return. Logs record gateway-side
//! information only and never contain upstream keys (red line: docs/Spec-implementation.md
//! "Upstream Key Security"). Structured detail parsing (dialogue / params / tools) is done
//! in the usecases layer; this layer only passes through.

use chrono::{DateTime, Utc};
use tauri::State;
use uuid::Uuid;

use crate::domain::request_log::{LogPage, LogQuery};
use crate::infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use crate::usecases::log::{
    ClearLogsUsecase, DeleteLogsBeforeUsecase, GetLogDetailUsecase, ListLogsUsecase, LogDetail,
};

/// Paginated log query: multi-criteria filtering (keyword / key / channel / model / date range), ordered by creation time descending.
#[tauri::command(rename_all = "camelCase")]
pub async fn list_logs(
    repo: State<'_, SqliteRequestLogRepository>,
    query: LogQuery,
    page: u64,
    page_size: u64,
) -> Result<LogPage, String> {
    ListLogsUsecase
        .execute(&*repo, query, page, page_size)
        .await
        .map_err(|e| e.to_string())
}

/// Log detail: the request log plus dialogue / params / tool tags parsed from the request body.
#[tauri::command]
pub async fn get_log_detail(
    repo: State<'_, SqliteRequestLogRepository>,
    id: String,
) -> Result<LogDetail, String> {
    let id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    GetLogDetailUsecase
        .execute(&*repo, id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete logs created strictly before `before` (half-open upper bound), returning the deleted count.
#[tauri::command]
pub async fn delete_logs_before(
    repo: State<'_, SqliteRequestLogRepository>,
    before: DateTime<Utc>,
) -> Result<u64, String> {
    DeleteLogsBeforeUsecase
        .execute(&*repo, before)
        .await
        .map_err(|e| e.to_string())
}

/// Clear all request logs, returning the deleted count.
#[tauri::command]
pub async fn clear_logs(repo: State<'_, SqliteRequestLogRepository>) -> Result<u64, String> {
    ClearLogsUsecase
        .execute(&*repo)
        .await
        .map_err(|e| e.to_string())
}
