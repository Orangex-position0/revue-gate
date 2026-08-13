//! 控制面：密钥管理命令（list / create / update / delete / set_enabled）。
//!
//! 命令是薄胶水：解析入参 → 调 usecases → 返回。密钥明文只在 create 时返回一次
//! （一次性展示），list / update / set_enabled 统一遮蔽为 `sk-revue-••••••••••••••••`
//! 占位符，保证控制面任何路径都不把密钥明文回显给前端（红线见
//! docs/Spec-implementation.md「密钥安全」）。

use tauri::State;
use uuid::Uuid;

use crate::domain::api_key::ApiKey;
use crate::infrastructure::sqlite::api_key::SqliteApiKeyRepository;
use crate::usecases::api_key::{
    ApiKeyInput, CreateApiKeyUsecase, DeleteApiKeyUsecase, ListApiKeysUsecase,
    SetApiKeyEnabledUsecase, UpdateApiKeyUsecase,
};

/// 遮蔽后的密钥占位符（长度与真实密钥一致，避免 UI 抖动）。
const MASKED_KEY: &str = "sk-revue-••••••••••••••••";

/// 列出全部密钥（按名称升序；密钥遮蔽）。
#[tauri::command]
pub async fn list_api_keys(repo: State<'_, SqliteApiKeyRepository>) -> Result<Vec<ApiKey>, String> {
    let keys = ListApiKeysUsecase
        .execute(&*repo)
        .await
        .map_err(|e| e.to_string())?;
    Ok(keys.into_iter().map(mask_key).collect())
}

/// 创建密钥并返回完整明文（一次性展示，仅此路径下发明文）。
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

/// 更新密钥并返回（密钥遮蔽；密钥本身不可更新）。
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

/// 删除密钥。
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

/// 启停密钥并返回（密钥遮蔽）。
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

/// 遮蔽密钥明文：返回给前端的密钥只保留 `sk-revue-` 前缀 + 占位符。
fn mask_key(mut api_key: ApiKey) -> ApiKey {
    api_key.key = MASKED_KEY.to_string();
    api_key
}

#[cfg(test)]
mod tests {
    use super::mask_key;
    use crate::test_support::sample_api_key;

    /// 遮蔽后 `key` 恒为占位符：控制面不向密钥明文回显给前端。
    #[test]
    fn mask_key_clears_key_plaintext() {
        let key = sample_api_key();
        assert!(key.key.starts_with("sk-revue-"));
        let masked = mask_key(key);
        assert_eq!(masked.key, "sk-revue-••••••••••••••••");
    }
}
