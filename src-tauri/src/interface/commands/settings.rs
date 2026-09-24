//! Control plane: scoped settings updates and the read-only detector catalog.
//!
//! The update lock serializes section writes. Validation and OS auto-start run before persistence;
//! persistence failure may leave the OS startup entry changed, while the next launch realigns it.
//! The shared settings snapshot is published only after the repository save succeeds.

use std::sync::{Arc, RwLock};

use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

use crate::domain::security_audit::{RuleCatalogEntry, rule_catalog};
use crate::domain::settings::{GatewaySettings, SettingsRepository};
use crate::usecases::settings::{GetSettingsUsecase, SaveSettingsUsecase, SettingsPatch, validate};

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

/// Serialize patch → validate → OS side effect (only desktop) → persist → publish.
/// The lock covers the read/modify/write so concurrent pages cannot lose another section's update.
#[tauri::command]
pub async fn save_settings_section(
    app: tauri::AppHandle,
    patch: SettingsPatch,
) -> Result<GatewaySettings, String> {
    let lock = app.state::<tokio::sync::Mutex<()>>();
    let _guard = lock.lock().await;
    let shared = app.state::<Arc<RwLock<GatewaySettings>>>();
    let mut settings = shared.read().expect("settings lock poisoned").clone();
    let changes_autostart = patch.changes_autostart();
    patch.apply_to(&mut settings);
    validate(&settings).map_err(|e| e.to_string())?;
    if changes_autostart {
        apply_autostart(&app, settings.autostart).map_err(|e| e.to_string())?;
    }
    let repo = app.state::<Arc<dyn SettingsRepository>>().inner().clone();
    let saved = SaveSettingsUsecase
        .execute(&*repo, settings)
        .await
        .map_err(|e| e.to_string())?;
    *shared.write().expect("settings lock poisoned") = saved.clone();
    Ok(saved)
}

/// Project only registered runtime detectors; no mutable rule repository is exposed.
#[tauri::command]
pub fn get_security_rules() -> Vec<RuleCatalogEntry> {
    rule_catalog()
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
