//! 基础设施层设置仓储：基于 tauri-plugin-store 的 JSON 文件持久化实现。
//!
//! 实现 domain 的 `SettingsRepository` trait（依赖倒置：infrastructure → domain）。
//! 设置整体以 `gateway` 键存于 `settings.json`（AppData 目录）；`load` 无值或解析失败时
//! 回退默认设置（`#[serde(default)]` 保证旧版 JSON 缺字段时向前兼容）。`save` 在
//! `set` 触发异步自动保存之后显式 `save()` 落盘，保证命令返回时设置已持久化。

use std::sync::Arc;

use async_trait::async_trait;
use tauri::Wry;
use tauri_plugin_store::Store;

use crate::domain::error::RepositoryError;
use crate::domain::settings::{GatewaySettings, SettingsRepository};

/// 设置快照在 store 文件中的顶层键。
const SETTINGS_KEY: &str = "gateway";

/// 基于 tauri-plugin-store 的设置仓储。
pub struct StoreSettingsRepository {
    store: Arc<Store<Wry>>,
}

impl StoreSettingsRepository {
    /// 构造：持有一个已构建（并自动加载磁盘内容）的 Store。
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

/// 把 store 插件 / serde 错误归一为仓储错误（错误信息字符串化，不泄漏内部类型到 domain）。
fn repo_err(e: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Database(e.to_string())
}
