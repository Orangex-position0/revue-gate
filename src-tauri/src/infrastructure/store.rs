//! Infrastructure settings repository: a JSON file persistence implementation based on tauri-plugin-store.
//!
//! Implements the domain `SettingsRepository` trait (dependency inversion: infrastructure → domain).
//! Settings are stored as a whole under the `gateway` key in `settings.json` (AppData directory); `load` falls back
//! to defaults when the value is missing or fails to parse (`#[serde(default)]` keeps old JSON forward-compatible);
//! `save` explicitly calls `save()` to flush to disk after `set` triggers async auto-save, so settings are
//! persisted by the time the command returns.

use std::sync::Arc;

use async_trait::async_trait;
use tauri::Wry;
use tauri_plugin_store::Store;

use crate::domain::error::RepositoryError;
use crate::domain::settings::{GatewaySettings, SettingsRepository};

/// Top-level key of the settings snapshot in the store file.
const SETTINGS_KEY: &str = "gateway";

/// Settings repository based on tauri-plugin-store.
pub struct StoreSettingsRepository {
    store: Arc<Store<Wry>>,
}

impl StoreSettingsRepository {
    /// Constructor: holds a Store that is already built (and auto-loaded from disk).
    pub fn new(store: Arc<Store<Wry>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl SettingsRepository for StoreSettingsRepository {
    async fn load(&self) -> Result<GatewaySettings, RepositoryError> {
        match self.store.get(SETTINGS_KEY) {
            Some(value) => serde_json::from_value(value).map_err(repo_err),
            None => Ok(GatewaySettings::default()),
        }
    }

    async fn save(&self, settings: &GatewaySettings) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(settings).map_err(repo_err)?;
        self.store.set(SETTINGS_KEY, value);
        self.store.save().map_err(repo_err)
    }
}

/// Normalizes store plugin / serde errors into a repository error (message stringified; internal types not
/// leaked to domain).
fn repo_err(e: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Database(e.to_string())
}
