//! 控制面：设置命令（get/save）。
//!
//! 命令只做薄胶水：`get_settings` 经用例读取仓储；`save_settings` 按「校验 →
//! 应用 OS 侧开机自启 → 持久化 → 更新共享设置」顺序执行。校验 / 自启失败即整体
//! 失败、零副作用；持久化失败时 OS 侧自启虽已改动，但下次启动会按已存配置重对齐。
//! 共享设置（`Arc<RwLock<GatewaySettings>>`）是 host/port/retry 的即时生效源：
//! 保存后 start_server / 托盘启动与代理重试立刻读到新值。

use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::domain::settings::{GatewaySettings, SettingsRepository};
use crate::usecases::settings::{GetSettingsUsecase, SaveSettingsUsecase, validate};

/// 查询当前设置快照（未持久化值时返回默认设置）。
#[tauri::command]
pub async fn get_settings(
    repo: tauri::State<'_, Arc<dyn SettingsRepository>>,
) -> Result<GatewaySettings, String> {
    GetSettingsUsecase
        .execute(repo.inner().as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// 保存设置：校验 → 应用开机自启（OS 侧）→ 持久化 → 更新共享设置（即时生效）。
#[tauri::command]
pub async fn save_settings(app: tauri::AppHandle, settings: GatewaySettings) -> Result<(), String> {
    validate(&settings).map_err(|e| e.to_string())?;
    apply_autostart(&app, settings.autostart).map_err(|e| e.to_string())?;
    let repo = app.state::<Arc<dyn SettingsRepository>>().inner().clone();
    let saved = SaveSettingsUsecase
        .execute(&*repo, settings.clone())
        .await
        .map_err(|e| e.to_string())?;
    // 写回用例返回的规范化值（host 已去空白），避免共享状态与持久化值分叉。
    *app.state::<Arc<RwLock<GatewaySettings>>>()
        .write()
        .expect("settings lock poisoned") = saved;
    Ok(())
}

/// 应用开机自启：仅在期望值与 OS 当前状态不一致时执行（避免重复写注册表/启动项）。
pub(crate) fn apply_autostart(
    app: &AppHandle,
    enabled: bool,
) -> Result<(), tauri_plugin_autostart::Error> {
    let manager = app.autolaunch();
    if manager.is_enabled()? == enabled {
        return Ok(());
    }
    if enabled {
        manager.enable()
    } else {
        manager.disable()
    }
}
