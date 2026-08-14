//! 控制面：请求日志命令（list / detail / delete_before / clear）。
//!
//! 命令是薄胶水：解析入参 → 调 usecases → 返回。日志仅记录网关侧信息，不含任何
//! 上游密钥（红线见 docs/Spec-implementation.md「上游密钥安全」）；详情的结构化
//! 解析（对话 / 参数 / 工具）在 usecases 层完成，本层只透传。

use chrono::{DateTime, Utc};
use tauri::State;
use uuid::Uuid;

use crate::domain::request_log::{LogPage, LogQuery};
use crate::infrastructure::sqlite::request_log::SqliteRequestLogRepository;
use crate::usecases::log::{
    ClearLogsUsecase, DeleteLogsBeforeUsecase, GetLogDetailUsecase, ListLogsUsecase, LogDetail,
};

/// 分页查询日志：多条件筛选（keyword / 密钥 / 渠道 / 模型 / 日期范围），按创建时间倒序。
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

/// 日志详情：请求日志 + 从请求体解析出的对话 / 参数 / 工具标签。
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

/// 删除创建时间严格早于 `before` 的日志（左闭右开上界），返回删除条数。
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

/// 清空全部请求日志，返回删除条数。
#[tauri::command]
pub async fn clear_logs(repo: State<'_, SqliteRequestLogRepository>) -> Result<u64, String> {
    ClearLogsUsecase
        .execute(&*repo)
        .await
        .map_err(|e| e.to_string())
}
