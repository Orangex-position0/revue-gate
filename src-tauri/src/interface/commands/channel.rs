//! Control plane: channel management commands (list / create / update / delete / set_enabled / test).
//!
//! Commands are thin glue: parse input → call usecases → return. Before returning, the upstream key is
//! uniformly masked (`mask_api_key`), so no control-plane path echoes the upstream key to the frontend
//! (red line: docs/Spec-implementation.md, "Upstream Key Security").

use tauri::State;
use uuid::Uuid;

use crate::domain::channel::{Channel, ChannelRepository};
use crate::infrastructure::providers::adaptor_for;
use crate::infrastructure::sqlite::channel::SqliteChannelRepository;
use crate::usecases::channel::{
    ChannelInput, ChannelTestResult, CreateChannelUsecase, DeleteChannelUsecase,
    ListChannelsUsecase, SetChannelEnabledUsecase, TestChannelUsecase, UpdateChannelUsecase,
};

/// List all channels (sorted by priority ascending; upstream keys masked).
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

/// Create a channel and return it (upstream key masked).
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

/// Update a channel and return it (upstream key masked; empty `input.api_key` = keep the original value).
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

/// Delete a channel.
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

/// Enable/disable a channel and return it (upstream key masked).
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

/// Channel connectivity test: resolve the adapter for the channel by id, call the upstream model list
/// endpoint, and persist the result. The adapter is resolved by the currently stored channel type;
/// the usecase re-fetches the latest channel internally and echoes the test result.
#[tauri::command]
pub async fn test_channel(
    repo: State<'_, SqliteChannelRepository>,
    id: String,
) -> Result<ChannelTestResult, String> {
    let id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    let channel = repo
        .find_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "channel not found".to_string())?;
    let adaptor = adaptor_for(channel.channel_type);
    TestChannelUsecase
        .execute(&*repo, id, &*adaptor)
        .await
        .map_err(|e| e.to_string())
}

/// Mask the upstream key: channels returned to the frontend never carry `api_key`.
fn mask_api_key(mut channel: Channel) -> Channel {
    channel.api_key = None;
    channel
}

#[cfg(test)]
mod tests {
    use super::mask_api_key;
    use crate::test_support::sample_channel;

    /// After masking, `api_key` is always None: the control plane never echoes the upstream key to the frontend.
    #[test]
    fn mask_api_key_clears_upstream_key() {
        let channel = sample_channel();
        assert!(channel.api_key.is_some());
        let masked = mask_api_key(channel);
        assert_eq!(masked.api_key, None);
    }
}
