//! 网关设置用例：读取 / 保存编排。
//!
//! 无状态用例：仓储以 `&dyn SettingsRepository` 注入，seam A 测试可用内存 mock。
//! 保存前做信任边界校验（host 非空）；具体持久化介质（tauri-plugin-store）在
//! infrastructure，本层只依赖 domain trait。保存成功语义 = 仓储已持久化。

use crate::domain::error::RepositoryError;
use crate::domain::settings::{GatewaySettings, SettingsRepository};

/// 设置用例层错误。
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid host: {0}")]
    InvalidHost(String),
    #[error("settings repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// 读取设置：委托仓储（无持久化值时为默认设置）。
pub struct GetSettingsUsecase;
impl GetSettingsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn SettingsRepository,
    ) -> Result<GatewaySettings, SettingsError> {
        Ok(repo.load().await?)
    }
}

/// 保存设置：校验入参（信任边界）→ host 规范化 → 整体覆盖持久化。
/// 返回规范化后的设置，供调用方写入共享状态（持久化值与即时生效值同源）。
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

/// 入参校验（信任边界）：host 去首尾空白后必须非空。命令层在触发 OS 侧副作用
/// （开机自启）之前先校验，避免「自启已改、配置未存」的中间态。
pub(crate) fn validate(settings: &GatewaySettings) -> Result<(), SettingsError> {
    if settings.host.trim().is_empty() {
        return Err(SettingsError::InvalidHost(settings.host.clone()));
    }
    Ok(())
}

/// host 去首尾空白（与 api_key 名称规范化口径一致）。
fn normalize_host(mut settings: GatewaySettings) -> GatewaySettings {
    settings.host = settings.host.trim().to_string();
    settings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::settings::Theme;
    use crate::test_support::InMemorySettingsRepository;

    /// 空仓储：读取返回默认设置（host 127.0.0.1、port 3000、主题 system、重试默认）。
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
    }

    /// 保存后读取：逐字段回读一致（含重试策略等嵌套配置）。
    #[tokio::test]
    async fn save_then_load_round_trips() {
        let repo = InMemorySettingsRepository::new();
        let settings = GatewaySettings {
            host: "0.0.0.0".to_string(),
            port: 0, // 0 = 随机端口
            theme: Theme::Dark,
            minimize_to_tray: true,
            close_to_tray: true,
            autostart: true,
            retry: crate::domain::settings::RetryPolicy {
                enabled: true,
                max_retries: Some(3),
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

    /// 保存：host 为空白应拒绝且不落库（信任边界校验）。
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

    /// 保存：host 去首尾空白后落库并返回规范化值（与 api_key 名称规范化口径一致）。
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
