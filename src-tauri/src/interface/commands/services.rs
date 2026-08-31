//! Control plane: Service Module status commands.

use std::sync::{Arc, RwLock};

use crate::domain::settings::GatewaySettings;
use crate::interface::http::handlers::AppState;
use crate::interface::http::service_modules::{ServiceModuleStatus, ServiceRegistry};

#[tauri::command(rename_all = "camelCase")]
pub async fn get_service_statuses(
    state: tauri::State<'_, AppState>,
    settings: tauri::State<'_, Arc<RwLock<GatewaySettings>>>,
) -> Result<Vec<ServiceModuleStatus>, String> {
    let registry = ServiceRegistry::new();
    registry
        .validate()
        .expect("service module registry must be valid");
    let settings = settings.read().expect("settings lock poisoned").clone();
    Ok(registry
        .list_statuses(state.inner(), &settings.service_modules)
        .await)
}
