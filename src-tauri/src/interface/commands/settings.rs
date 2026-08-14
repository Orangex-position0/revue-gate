//! Control plane: settings commands (get/save).
//!
//! Commands are thin glue: `get_settings` reads the repository via a usecase; `save_settings` runs
//! "validate → apply OS auto-start → persist → update shared settings" in order. Validation or
//! auto-start failure fails the whole call with zero side effects; if persistence fails, the OS
//! auto-start is already changed but the next startup realigns to the stored config. Shared
//! settings (`Arc<RwLock<GatewaySettings>>`) are the immediate source of host/port/retry:
//! after save, start_server, tray start, and proxy retry read the new values at once.

use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::domain::settings::{GatewaySettings, SettingsRepository};
use crate::usecases::settings::{GetSettingsUsecase, SaveSettingsUsecase, validate};

/// Query the current settings snapshot (returns defaults when nothing is persisted).
#[tauri::command]
pub async fn get_settings(
    repo: tauri::State<'_, Arc<dyn SettingsRepository>>,
) -> Result<GatewaySettings, String> {
    GetSettingsUsecase
        .execute(repo.inner().as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Save settings: validate → apply OS auto-start → persist → update shared settings (effective immediately).
#[tauri::command]
pub async fn save_settings(app: tauri::AppHandle, settings: GatewaySettings) -> Result<(), String> {
    validate(&settings).map_err(|e| e.to_string())?;
    apply_autostart(&app, settings.autostart).map_err(|e| e.to_string())?;
    let repo = app.state::<Arc<dyn SettingsRepository>>().inner().clone();
    let saved = SaveSettingsUsecase
        .execute(&*repo, settings.clone())
        .await
        .map_err(|e| e.to_string())?;
    // Write back the normalized value returned by the usecase (host trimmed) so shared state and persisted value never diverge.
    *app.state::<Arc<RwLock<GatewaySettings>>>()
        .write()
        .expect("settings lock poisoned") = saved;
    Ok(())
}

/// Apply OS auto-start: only runs when the desired value differs from the OS state (avoids rewriting the registry/startup items).
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
