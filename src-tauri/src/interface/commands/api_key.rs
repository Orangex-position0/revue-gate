//! Control plane: API key management commands (list / create / update / delete / set_enabled).
//!
//! Commands are thin glue: parse input → call usecases → return. Key plaintext is returned
//! only once at create (one-time display); list / update / set_enabled uniformly mask it with the
//! `sk-revue-••••••••••••••••` placeholder, so no control-plane path ever echoes the plaintext
//! to the frontend (red line: docs/Spec-implementation.md, "API Key Security").

use tauri::State;
use uuid::Uuid;

use crate::domain::api_key::ApiKey;
use crate::infrastructure::sqlite::api_key::SqliteApiKeyRepository;
use crate::usecases::api_key::{
    ApiKeyInput, CreateApiKeyUsecase, DeleteApiKeyUsecase, ListApiKeysUsecase,
    SetApiKeyEnabledUsecase, UpdateApiKeyUsecase,
};

/// Masked key placeholder (same length as a real key to avoid UI layout shift).
const MASKED_KEY: &str = "sk-revue-••••••••••••••••";

/// List all API keys (sorted by name ascending; keys masked).
#[tauri::command]
pub async fn list_api_keys(repo: State<'_, SqliteApiKeyRepository>) -> Result<Vec<ApiKey>, String> {
    let keys = ListApiKeysUsecase
        .execute(&*repo)
        .await
        .map_err(|e| e.to_string())?;
    Ok(keys.into_iter().map(mask_key).collect())
}

/// Create an API key and return the full plaintext (one-time display; only this path returns plaintext).
#[tauri::command]
pub async fn create_api_key(
    repo: State<'_, SqliteApiKeyRepository>,
    input: ApiKeyInput,
) -> Result<ApiKey, String> {
    let key = CreateApiKeyUsecase
        .execute(&*repo, input)
        .await
        .map_err(|e| e.to_string())?;
    Ok(key)
}

/// Update an API key and return it (masked; the key itself cannot be updated).
#[tauri::command]
pub async fn update_api_key(
    repo: State<'_, SqliteApiKeyRepository>,
    id: String,
    input: ApiKeyInput,
) -> Result<ApiKey, String> {
    let id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    let key = UpdateApiKeyUsecase
        .execute(&*repo, id, input)
        .await
        .map_err(|e| e.to_string())?;
    Ok(mask_key(key))
}

/// Delete an API key.
#[tauri::command]
pub async fn delete_api_key(
    repo: State<'_, SqliteApiKeyRepository>,
    id: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    DeleteApiKeyUsecase
        .execute(&*repo, id)
        .await
        .map_err(|e| e.to_string())
}

/// Enable/disable an API key and return it (masked).
#[tauri::command]
pub async fn set_api_key_enabled(
    repo: State<'_, SqliteApiKeyRepository>,
    id: String,
    enabled: bool,
) -> Result<ApiKey, String> {
    let id = Uuid::parse_str(&id).map_err(|e| e.to_string())?;
    let key = SetApiKeyEnabledUsecase
        .execute(&*repo, id, enabled)
        .await
        .map_err(|e| e.to_string())?;
    Ok(mask_key(key))
}

/// Mask key plaintext: the key returned to the frontend keeps only the `sk-revue-` prefix + placeholder.
fn mask_key(mut api_key: ApiKey) -> ApiKey {
    api_key.key = MASKED_KEY.to_string();
    api_key
}

#[cfg(test)]
mod tests {
    use super::mask_key;
    use crate::test_support::sample_api_key;

    /// After masking, `key` is always the placeholder: the control plane never echoes key plaintext to the frontend.
    #[test]
    fn mask_key_clears_key_plaintext() {
        let key = sample_api_key();
        assert!(key.key.starts_with("sk-revue-"));
        let masked = mask_key(key);
        assert_eq!(masked.key, "sk-revue-••••••••••••••••");
    }
}
