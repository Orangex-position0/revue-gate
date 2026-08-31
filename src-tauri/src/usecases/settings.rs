//! Gateway settings use cases: load / save orchestration.
//!
//! Stateless use cases: repositories are injected as `&dyn SettingsRepository`, seam A tests can use in-memory mocks.
//! A trust-boundary validation (non-empty host) runs before saving; the concrete persistence medium (tauri-plugin-store) lives in
//! infrastructure, this layer only depends on the domain trait. Save success semantics = the repository has persisted.

use crate::domain::error::RepositoryError;
use crate::domain::settings::{GatewaySettings, SettingsRepository};

/// Settings use case layer error.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid host: {0}")]
    InvalidHost(String),
    #[error("settings repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// Load settings: delegate to the repository (defaults when no persisted value).
pub struct GetSettingsUsecase;
impl GetSettingsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn SettingsRepository,
    ) -> Result<GatewaySettings, SettingsError> {
        Ok(repo.load().await?)
    }
}

/// Save settings: validate the input (trust boundary) → normalize host → persist a full overwrite.
/// Returns the normalized settings for the caller to write into shared state (the persisted value and the immediately-effective value share one source).
pub struct SaveSettingsUsecase;
impl SaveSettingsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn SettingsRepository,
        settings: GatewaySettings,
    ) -> Result<GatewaySettings, SettingsError> {
        validate(&settings)?;
        let settings = normalize_host(settings);
        repo.save(&settings).await?;
        Ok(settings)
    }
}

/// Input validation (trust boundary): host must be non-empty after trimming. The command layer validates before triggering OS-side side effects
/// (e.g. autostart), avoiding the intermediate state where "autostart was changed but the config was not saved".
pub(crate) fn validate(settings: &GatewaySettings) -> Result<(), SettingsError> {
    if settings.host.trim().is_empty() {
        return Err(SettingsError::InvalidHost(settings.host.clone()));
    }
    Ok(())
}

/// Trim the host (same normalization approach as api_key names).
fn normalize_host(mut settings: GatewaySettings) -> GatewaySettings {
    settings.host = settings.host.trim().to_string();
    settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::{ServiceModuleSettings, Theme};
    use crate::test_support::InMemorySettingsRepository;

    /// Empty repository: load returns default settings (host 127.0.0.1, port 3000, theme system, default retry).
    #[tokio::test]
    async fn get_returns_defaults_when_empty() {
        let repo = InMemorySettingsRepository::new();
        let settings = GetSettingsUsecase
            .execute(&repo)
            .await
            .expect("get settings");
        assert_eq!(settings, GatewaySettings::default());
        assert_eq!(settings.host, "127.0.0.1");
        assert_eq!(settings.port, 3000);
        assert_eq!(settings.theme, Theme::System);
        assert!(settings.retry.enabled);
        assert_eq!(settings.retry.max_retries, None);
        assert!(!settings.audit.enabled);
        assert!(settings.audit.store_payload);
    }

    /// Save then load: every field round-trips consistently (including nested config such as the retry policy).
    #[tokio::test]
    async fn save_then_load_round_trips() {
        let repo = InMemorySettingsRepository::new();
        let settings = GatewaySettings {
            host: "0.0.0.0".to_string(),
            port: 0, // 0 = random port
            theme: Theme::Dark,
            minimize_to_tray: true,
            close_to_tray: true,
            autostart: true,
            retry: crate::domain::settings::RetryPolicy {
                enabled: true,
                max_retries: Some(3),
            },
            audit: crate::domain::security_audit::AuditSettings {
                enabled: true,
                scan_byte_limit: 4096,
                store_payload: false,
                ..Default::default()
            },
            service_modules: ServiceModuleSettings {
                knowledge: false,
                mcp: true,
            },
        };

        SaveSettingsUsecase
            .execute(&repo, settings.clone())
            .await
            .expect("save");
        assert_eq!(
            GetSettingsUsecase.execute(&repo).await.expect("load"),
            settings,
            "保存后完整回读，嵌套 retry 策略不被吞掉"
        );
    }

    /// Save: a blank host is rejected and not persisted (trust-boundary validation).
    #[tokio::test]
    async fn save_rejects_blank_host() {
        let repo = InMemorySettingsRepository::new();
        let err = SaveSettingsUsecase
            .execute(
                &repo,
                GatewaySettings {
                    host: "   ".to_string(),
                    ..GatewaySettings::default()
                },
            )
            .await
            .expect_err("blank host should be rejected");
        assert!(matches!(err, SettingsError::InvalidHost(_)));
        assert_eq!(
            GetSettingsUsecase.execute(&repo).await.expect("load"),
            GatewaySettings::default(),
            "校验失败不落库"
        );
    }

    /// Save: the host is trimmed, persisted, and the normalized value is returned (same normalization approach as api_key names).
    #[tokio::test]
    async fn save_normalizes_host_whitespace() {
        let repo = InMemorySettingsRepository::new();
        let saved = SaveSettingsUsecase
            .execute(
                &repo,
                GatewaySettings {
                    host: " 127.0.0.1 ".to_string(),
                    ..GatewaySettings::default()
                },
            )
            .await
            .expect("save");
        assert_eq!(saved.host, "127.0.0.1", "返回值为规范化后的 host");
        let persisted = GetSettingsUsecase.execute(&repo).await.expect("load");
        assert_eq!(persisted.host, "127.0.0.1");
    }
}
