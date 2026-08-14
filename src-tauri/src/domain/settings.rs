//! 网关设置领域模型：端口 / host、主题、托盘行为、开机自启、失败重试策略。
//!
//! 纯业务类型与规则，零技术依赖：持久化由 tauri-plugin-store 实现在 infrastructure，
//! 用例编排在 usecases/settings.rs（seam A 可用内存 mock 测试）。重试策略默认
//! `{ enabled: true, max_retries: None }` 与 ticket 07 的「逐个渠道尝试」默认行为一致，
//! 本模块只做配置落库与生效（见 ticket 12）。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::domain::error::RepositoryError;

/// 界面主题三态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    #[default]
    System,
    Dark,
}

/// 失败重试策略：`enabled=false` 只试首个候选；`enabled=true` 且 `max_retries=None`
/// 表示不限制（沿用 ticket 07 逐个尝试）；`max_retries=Some(n)` 表示首个之后最多再试 n 次。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryPolicy {
    pub enabled: bool,
    /// 首次之后的额外重试次数；None = 不限制。
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

/// 网关设置快照：设置页可编辑的全部配置项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GatewaySettings {
    /// 服务监听 host；默认 127.0.0.1。
    pub host: String,
    /// 服务监听端口；0 = 随机可用端口。
    pub port: u16,
    /// 界面主题。
    pub theme: Theme,
    /// 最小化时隐藏到托盘。
    pub minimize_to_tray: bool,
    /// 关闭窗口时隐藏到托盘（而非退出）。
    pub close_to_tray: bool,
    /// 开机自启。
    pub autostart: bool,
    /// 失败重试策略。
    pub retry: RetryPolicy,
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
        }
    }
}

/// 设置仓储：读写网关设置快照。持久化格式 / 存储介质由实现决定。
#[async_trait]
pub trait SettingsRepository: Send + Sync {
    /// 读取设置；无持久化值时返回默认设置（`GatewaySettings::default()`）。
    async fn load(&self) -> Result<GatewaySettings, RepositoryError>;
    /// 整体覆盖保存设置快照。
    async fn save(&self, settings: &GatewaySettings) -> Result<(), RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // 序列化往返不变量：任意设置 `to_value` 后 `from_value` 必须还原原值
    // （强制档 PBT，见 rules/property-based-testing.md §2.1）。
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
        ) {
            let settings = GatewaySettings {
                host,
                port,
                theme,
                minimize_to_tray: minimize,
                close_to_tray: close,
                autostart,
                retry: RetryPolicy { enabled: retry_enabled, max_retries },
            };
            let value = serde_json::to_value(&settings).expect("serialize");
            let back: GatewaySettings = serde_json::from_value(value).expect("deserialize");
            prop_assert_eq!(back, settings);
        }
    }
}
