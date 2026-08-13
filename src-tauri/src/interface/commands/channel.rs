//! 控制面：渠道管理命令（list / create / update / delete / set_enabled）。
//!
//! 命令是薄胶水：解析入参 → 调 usecases → 返回。返回前统一遮蔽上游密钥
//! （`mask_api_key`），保证控制面任何路径都不把上游密钥回显给前端
//! （红线见 docs/Spec-implementation.md「上游密钥安全」）。

use tauri::State;
use uuid::Uuid;

use crate::domain::channel::Channel;
use crate::infrastructure::sqlite::channel::SqliteChannelRepository;
use crate::usecases::channel::{
    ChannelInput, CreateChannelUsecase, DeleteChannelUsecase, ListChannelsUsecase,
    SetChannelEnabledUsecase, UpdateChannelUsecase,
};

/// 列出全部渠道（按优先级升序；上游密钥遮蔽）。
#[tauri::command]
pub async fn list_channels(
    repo: State<'_, SqliteChannelRepository>,
) -> Result<Vec<Channel>, String> {
    let channels = ListChannelsUsecase
        .execute(&*repo)
        .await
        .map_err(|e| e.to_string())?;
    Ok(channels.into_iter().map(mask_api_key).collect())
}

/// 创建渠道并返回（上游密钥遮蔽）。
#[tauri::command]
pub async fn create_channel(
    repo: State<'_, SqliteChannelRepository>,
    input: ChannelInput,
) -> Result<Channel, String> {
    let channel = CreateChannelUsecase
        .execute(&*repo, input)
        .await
        .map_err(|e| e.to_string())?;
    Ok(mask_api_key(channel))
}

/// 更新渠道并返回（上游密钥遮蔽；`input.api_key` 为空 = 保持原值）。
#[tauri::command]
pub async fn update_channel(
    repo: State<'_, SqliteChannelRepository>,
    id: String,
    input: ChannelInput,
) -> Result<Channel, String> {
    let id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    let channel = UpdateChannelUsecase
        .execute(&*repo, id, input)
        .await
        .map_err(|e| e.to_string())?;
    Ok(mask_api_key(channel))
}

/// 删除渠道。
#[tauri::command]
pub async fn delete_channel(
    repo: State<'_, SqliteChannelRepository>,
    id: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    DeleteChannelUsecase
        .execute(&*repo, id)
        .await
        .map_err(|e| e.to_string())
}

/// 启停渠道并返回（上游密钥遮蔽）。
#[tauri::command]
pub async fn set_channel_enabled(
    repo: State<'_, SqliteChannelRepository>,
    id: String,
    enabled: bool,
) -> Result<Channel, String> {
    let id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    let channel = SetChannelEnabledUsecase
        .execute(&*repo, id, enabled)
        .await
        .map_err(|e| e.to_string())?;
    Ok(mask_api_key(channel))
}

/// 遮蔽上游密钥：返回给前端的渠道不携带 `api_key`。
fn mask_api_key(mut channel: Channel) -> Channel {
    channel.api_key = None;
    channel
}

#[cfg(test)]
mod tests {
    use super::mask_api_key;
    use crate::test_support::sample_channel;

    /// 遮蔽后 `api_key` 恒为 None：控制面不向上游密钥回显给前端。
    #[test]
    fn mask_api_key_clears_upstream_key() {
        let channel = sample_channel();
        assert!(channel.api_key.is_some());
        let masked = mask_api_key(channel);
        assert_eq!(masked.api_key, None);
    }
}
