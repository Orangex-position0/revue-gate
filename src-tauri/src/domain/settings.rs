//! Gateway settings domain model: port / host, theme, tray behavior, autostart, failure retry policy.
//!
//! Pure business types and rules, zero technical dependencies: persistence is implemented by tauri-plugin-store in infrastructure,
//! use-case orchestration is in usecases/settings.rs (seam A can be tested with an in-memory mock). The retry policy default
//! `{ enabled: true, max_retries: None }` matches ticket 07's "try channels one by one" default behavior;
//! this module only persists and applies the configuration (see ticket 12).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::error::RepositoryError;
use crate::domain::security_audit::AuditSettings;

/// Three-state UI theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    #[default]
    System,
    Dark,
}

/// Failure retry policy: `enabled=false` only tries the first candidate; `enabled=true` with `max_retries=None`
/// means unlimited (keeping ticket 07's try-each-one behavior); `max_retries=Some(n)` means at most n more tries after the first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryPolicy {
    pub enabled: bool,
    /// Extra retries after the first; None = unlimited.
    pub max_retries: Option<u32>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: None,
        }
    }
}

/// Service Module enablement settings. Routes are mounted from this snapshot when the HTTP server starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServiceModuleSettings {
    pub knowledge: bool,
    pub mcp: bool,
}

impl Default for ServiceModuleSettings {
    fn default() -> Self {
        Self {
            knowledge: true,
            mcp: true,
        }
    }
}

impl ServiceModuleSettings {
    pub fn is_enabled(&self, module_id: &str) -> bool {
        match module_id {
            "knowledge" => self.knowledge,
            "mcp" => self.mcp,
            _ => false,
        }
    }
}

/// Gateway settings snapshot: every config item editable on the settings page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GatewaySettings {
    /// Service listen host; default 127.0.0.1.
    pub host: String,
    /// Service listen port; 0 = random available port.
    pub port: u16,
    /// UI theme.
    pub theme: Theme,
    /// Hide to tray when minimized.
    pub minimize_to_tray: bool,
    /// Hide to tray (instead of exiting) when the window closes.
    pub close_to_tray: bool,
    /// Autostart at login.
    pub autostart: bool,
    /// Failure retry policy.
    pub retry: RetryPolicy,
    /// Request-time security audit policy controls.
    pub audit: AuditSettings,
    /// Service Module route enablement; takes effect the next time the HTTP server starts.
    pub service_modules: ServiceModuleSettings,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
            theme: Theme::System,
            minimize_to_tray: false,
            close_to_tray: false,
            autostart: false,
            retry: RetryPolicy::default(),
            audit: AuditSettings::default(),
            service_modules: ServiceModuleSettings::default(),
        }
    }
}

/// Settings repository: reads and writes the gateway settings snapshot. Persistence format / storage medium is implementation-defined.
#[async_trait]
pub trait SettingsRepository: Send + Sync {
    /// Load settings; returns defaults (`GatewaySettings::default()`) when nothing is persisted.
    async fn load(&self) -> Result<GatewaySettings, RepositoryError>;
    /// Overwrite-save the full settings snapshot.
    async fn save(&self, settings: &GatewaySettings) -> Result<(), RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // Serialization round-trip invariant: any settings must be restored by `from_value` after `to_value`
    // (mandatory-tier PBT, see rules/property-based-testing.md §2.1).
    proptest! {
        #[test]
        fn settings_json_roundtrip(
            host in ".*",
            port in 0u16..=65535u16,
            theme in prop::sample::select(vec![Theme::Light, Theme::System, Theme::Dark]),
            minimize in prop::bool::ANY,
            close in prop::bool::ANY,
            autostart in prop::bool::ANY,
            retry_enabled in prop::bool::ANY,
            max_retries in prop::option::of(0u32..=100),
            audit_enabled in prop::bool::ANY,
            block_critical in prop::bool::ANY,
            scan_system in prop::bool::ANY,
            scan_byte_limit in 0u32..=1_000_000u32,
            store_payload in prop::bool::ANY,
            knowledge in prop::bool::ANY,
            mcp in prop::bool::ANY,
        ) {
            let settings = GatewaySettings {
                host,
                port,
                theme,
                minimize_to_tray: minimize,
                close_to_tray: close,
                autostart,
                retry: RetryPolicy { enabled: retry_enabled, max_retries },
                audit: AuditSettings {
                    enabled: audit_enabled,
                    block_critical,
                    scan_system_messages: scan_system,
                    scan_byte_limit,
                    store_payload,
                    ..AuditSettings::default()
                },
                service_modules: ServiceModuleSettings { knowledge, mcp },
            };
            let value = serde_json::to_value(&settings).expect("serialize");
            let back: GatewaySettings = serde_json::from_value(value).expect("deserialize");
            prop_assert_eq!(back, settings);
        }
    }

    #[test]
    fn old_settings_json_defaults_service_modules_to_enabled() {
        let settings: GatewaySettings = serde_json::from_value(serde_json::json!({
            "host": "127.0.0.1",
            "port": 3456,
            "theme": "system",
            "minimizeToTray": false,
            "closeToTray": false,
            "autostart": false,
            "retry": {
                "enabled": true,
                "max_retries": null
            },
            "audit": AuditSettings::default()
        }))
        .expect("deserialize old settings");

        assert!(settings.service_modules.knowledge);
        assert!(settings.service_modules.mcp);
    }
}
