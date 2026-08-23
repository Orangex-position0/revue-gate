//! Channel management use cases: CRUD / enable-disable orchestration.
//!
//! Stateless use cases: repositories are injected as `&dyn ChannelRepository`, seam A tests can use in-memory mocks.
//! Update semantics: an empty `api_key` means "keep the original value" — the edit form does not prefill the upstream key, and leaving it blank does not overwrite
//! (red line: upstream keys are never stored as plaintext exposed downstream; the control plane masks them before returning, see interface/commands/channel.rs).
//! Scheduling rules (e.g. disabled channels are not scheduled) take effect in stage 07; this ticket only guarantees fields are persisted correctly.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::channel::{Channel, ChannelRepository, ChannelType, ModelMapping};
use crate::domain::error::RepositoryError;
use crate::domain::provider::{ProviderAdaptor, ProviderError};

/// Channel use case layer error.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("channel not found")]
    NotFound,
    #[error("invalid channel: {0}")]
    Validation(String),
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("channel repository error: {0}")]
    Repository(#[from] RepositoryError),
}

/// Input for creating / editing a channel (shared by create and update; on update an empty `api_key` = keep the original value).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInput {
    pub name: String,
    pub channel_type: ChannelType,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub models: Vec<String>,
    pub priority: i32,
    pub weight: i32,
    pub model_mappings: Vec<ModelMapping>,
    pub enabled: bool,
}

/// Create a channel: validate the input, build a new entity and save it, returning the persisted channel.
pub struct CreateChannelUsecase;
impl CreateChannelUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        input: ChannelInput,
    ) -> Result<Channel, ChannelError> {
        let input = normalize(input)?;
        let now = Utc::now();
        let channel = Channel {
            id: Uuid::now_v7(),
            name: input.name,
            channel_type: input.channel_type,
            base_url: input.base_url,
            api_key: input.api_key,
            models: input.models,
            priority: input.priority,
            weight: input.weight,
            model_mappings: input.model_mappings,
            enabled: input.enabled,
            last_test_at: None,
            last_test_ok: None,
            created_at: now,
            updated_at: now,
        };
        repo.save(&channel).await?;
        Ok(channel)
    }
}

/// Update a channel: load the original entity → merge the input (an empty api_key keeps the original value) → save.
pub struct UpdateChannelUsecase;
impl UpdateChannelUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        id: Uuid,
        input: ChannelInput,
    ) -> Result<Channel, ChannelError> {
        let input = normalize(input)?;
        let mut channel = repo.find_by_id(id).await?.ok_or(ChannelError::NotFound)?;
        channel.name = input.name;
        channel.channel_type = input.channel_type;
        channel.base_url = input.base_url;
        if let Some(api_key) = input.api_key {
            channel.api_key = Some(api_key);
        }
        channel.models = input.models;
        channel.priority = input.priority;
        channel.weight = input.weight;
        channel.model_mappings = input.model_mappings;
        channel.enabled = input.enabled;
        channel.updated_at = Utc::now();
        repo.save(&channel).await?;
        Ok(channel)
    }
}

/// Delete a channel; returns `NotFound` if missing.
pub struct DeleteChannelUsecase;
impl DeleteChannelUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        id: Uuid,
    ) -> Result<(), ChannelError> {
        repo.delete(id).await.map_err(map_repo_error)?;
        Ok(())
    }
}

/// List all channels.
pub struct ListChannelsUsecase;
impl ListChannelsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
    ) -> Result<Vec<Channel>, ChannelError> {
        Ok(repo.list().await?)
    }
}

/// Enable / disable a channel: set `enabled` and persist.
pub struct SetChannelEnabledUsecase;
impl SetChannelEnabledUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        id: Uuid,
        enabled: bool,
    ) -> Result<Channel, ChannelError> {
        let mut channel = repo.find_by_id(id).await?.ok_or(ChannelError::NotFound)?;
        channel.enabled = enabled;
        channel.updated_at = Utc::now();
        repo.save(&channel).await?;
        Ok(channel)
    }
}

/// Channel connectivity test result: echoed to the frontend + persisted to the channel's `last_test_*` fields.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub tested_at: DateTime<Utc>,
    pub error: Option<String>,
}

/// Channel connectivity test: calls the adaptor `test()`, persists the result to the channel's `last_test_*` and echoes it.
/// Adaptor configuration errors (missing api_key / base_url) are also recorded as failure results, not thrown as use case errors
/// (so the frontend can see the "not configured" failure reason, and the result can be persisted).
pub struct TestChannelUsecase;
impl TestChannelUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        id: Uuid,
        adaptor: &dyn ProviderAdaptor,
    ) -> Result<ChannelTestResult, ChannelError> {
        let mut channel = repo.find_by_id(id).await?.ok_or(ChannelError::NotFound)?;
        let tested_at = Utc::now();
        let result = match adaptor.test(&channel).await {
            Ok(tr) => ChannelTestResult {
                ok: tr.ok,
                latency_ms: tr.latency_ms,
                tested_at,
                error: tr.error,
            },
            Err(e) => ChannelTestResult {
                ok: false,
                latency_ms: 0,
                tested_at,
                error: Some(e.to_string()),
            },
        };
        channel.last_test_at = Some(tested_at);
        channel.last_test_ok = Some(result.ok);
        channel.updated_at = tested_at;
        repo.save(&channel).await?;
        Ok(result)
    }
}

pub struct FetchChannelModelsUsecase;
impl FetchChannelModelsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
        id: Option<Uuid>,
        input: ChannelInput,
        adaptor: &dyn ProviderAdaptor,
    ) -> Result<Vec<String>, ChannelError> {
        let existing = match id {
            Some(id) => Some(repo.find_by_id(id).await?.ok_or(ChannelError::NotFound)?),
            None => None,
        };
        let mut channel = build_channel_for_model_discovery(input)?;
        if channel.api_key.is_none() {
            channel.api_key = existing.as_ref().and_then(|c| c.api_key.clone());
        }
        if let Some(existing) = existing {
            channel.id = existing.id;
            channel.created_at = existing.created_at;
            channel.last_test_at = existing.last_test_at;
            channel.last_test_ok = existing.last_test_ok;
        }
        Ok(adaptor.fetch_models(&channel).await?)
    }
}

/// Normalize input: trim the name (a blank name errors); empty / whitespace-only Base URL / API Key normalize to None, and non-blank values are also trimmed.
fn normalize(input: ChannelInput) -> Result<ChannelInput, ChannelError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ChannelError::Validation("name must not be empty".into()));
    }
    if input.weight < 0 {
        return Err(ChannelError::Validation(
            "weight must not be negative".into(),
        ));
    }
    Ok(ChannelInput {
        name: name.to_string(),
        base_url: non_blank(input.base_url),
        api_key: non_blank(input.api_key),
        ..input
    })
}

/// Normalize to None if empty after trimming.
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn build_channel_for_model_discovery(input: ChannelInput) -> Result<Channel, ChannelError> {
    let input = normalize(input)?;
    let now = Utc::now();
    Ok(Channel {
        id: Uuid::now_v7(),
        name: input.name,
        channel_type: input.channel_type,
        base_url: input.base_url,
        api_key: input.api_key,
        models: input.models,
        priority: input.priority,
        weight: input.weight,
        model_mappings: input.model_mappings,
        enabled: input.enabled,
        last_test_at: None,
        last_test_ok: None,
        created_at: now,
        updated_at: now,
    })
}

/// Map a repository error to a use case error: `NotFound` is semantically the same as the use case's `NotFound`.
fn map_repo_error(e: RepositoryError) -> ChannelError {
    match e {
        RepositoryError::NotFound => ChannelError::NotFound,
        other => ChannelError::Repository(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{InMemoryChannelRepository, MockProviderAdaptor};

    /// Build a valid sample creation input.
    fn input(name: &str) -> ChannelInput {
        ChannelInput {
            name: name.to_string(),
            channel_type: ChannelType::OpenAi,
            base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-upstream".to_string()),
            models: vec!["gpt-4o".to_string()],
            priority: 0,
            weight: 1,
            model_mappings: vec![ModelMapping {
                client_model: "chat".to_string(),
                upstream_model: "gpt-4o".to_string(),
            }],
            enabled: true,
        }
    }

    /// Create: all fields round-trip, one row persisted in the repository.
    #[tokio::test]
    async fn create_persists_channel_and_roundtrips_fields() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");

        assert_eq!(created.name, "openai-prod");
        assert_eq!(created.channel_type, ChannelType::OpenAi);
        assert_eq!(created.api_key.as_deref(), Some("sk-upstream"));
        assert_eq!(created.models, vec!["gpt-4o"]);
        assert_eq!(created.priority, 0);
        assert_eq!(created.model_mappings.len(), 1);
        assert!(created.enabled);
        assert_eq!(created.last_test_at, None);

        let all = repo.list().await.expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], created);
    }

    /// Normalize: the name is trimmed; a whitespace-only Base URL normalizes to None; a non-blank Base URL is trimmed before persisting.
    #[tokio::test]
    async fn create_normalizes_whitespace() {
        let repo = InMemoryChannelRepository::new();

        let mut trimmed = input("  openai-prod  ");
        trimmed.base_url = Some("  https://api.openai.com/v1  ".to_string());
        let created = CreateChannelUsecase
            .execute(&repo, trimmed)
            .await
            .expect("create with padded input");
        assert_eq!(created.name, "openai-prod");
        assert_eq!(
            created.base_url.as_deref(),
            Some("https://api.openai.com/v1")
        );

        let mut blank_input = input("blank-url");
        blank_input.base_url = Some("   ".to_string());
        let created_blank = CreateChannelUsecase
            .execute(&repo, blank_input)
            .await
            .expect("create with blank base url");
        assert_eq!(created_blank.name, "blank-url");
        assert_eq!(created_blank.base_url, None);
    }

    /// Create: a blank name (including whitespace-only) is rejected and not persisted.
    #[tokio::test]
    async fn create_rejects_blank_name() {
        let repo = InMemoryChannelRepository::new();
        let err = CreateChannelUsecase
            .execute(&repo, input("   "))
            .await
            .expect_err("blank name should be rejected");
        assert!(matches!(err, ChannelError::Validation(_)));
        assert!(repo.list().await.expect("list").is_empty());
    }

    /// Create / update: a negative weight is rejected (dispatch semantics are undefined for negative weights).
    #[tokio::test]
    async fn create_and_update_reject_negative_weight() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");

        let mut neg = input("bad-weight");
        neg.weight = -1;
        assert!(matches!(
            CreateChannelUsecase.execute(&repo, neg).await,
            Err(ChannelError::Validation(_))
        ));

        let mut update = input("bad-weight");
        update.weight = -1;
        assert!(matches!(
            UpdateChannelUsecase
                .execute(&repo, created.id, update)
                .await,
            Err(ChannelError::Validation(_))
        ));
    }

    /// Update: fields are merged, leaving api_key blank keeps the original key, upsert produces no new row.
    #[tokio::test]
    async fn update_merges_fields_and_keeps_existing_key_when_blank() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");

        let mut update = input("openai-prod-v2");
        update.api_key = None; // edit form left blank → keep the original key
        update.priority = 5;
        update.enabled = false;

        let updated = UpdateChannelUsecase
            .execute(&repo, created.id, update)
            .await
            .expect("update");
        assert_eq!(updated.name, "openai-prod-v2");
        assert_eq!(updated.priority, 5);
        assert!(!updated.enabled);
        assert_eq!(updated.api_key.as_deref(), Some("sk-upstream"));
        assert_eq!(updated.created_at, created.created_at, "创建时间不变");

        let all = repo.list().await.expect("list");
        assert_eq!(all.len(), 1, "upsert 不新增行");
        assert_eq!(all[0], updated);
    }

    /// Update: explicitly passing a new api_key should overwrite the original key.
    #[tokio::test]
    async fn update_overwrites_key_when_provided() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");

        let mut update = input("openai-prod");
        update.api_key = Some("sk-upstream-new".to_string());
        let updated = UpdateChannelUsecase
            .execute(&repo, created.id, update)
            .await
            .expect("update");
        assert_eq!(updated.api_key.as_deref(), Some("sk-upstream-new"));
    }

    /// Update / delete / enable-disable: an unknown id returns NotFound.
    #[tokio::test]
    async fn mutate_unknown_id_errors_not_found() {
        let repo = InMemoryChannelRepository::new();
        let id = Uuid::now_v7();
        assert!(matches!(
            UpdateChannelUsecase.execute(&repo, id, input("x")).await,
            Err(ChannelError::NotFound)
        ));
        assert!(matches!(
            DeleteChannelUsecase.execute(&repo, id).await,
            Err(ChannelError::NotFound)
        ));
        assert!(matches!(
            SetChannelEnabledUsecase.execute(&repo, id, false).await,
            Err(ChannelError::NotFound)
        ));
    }

    /// Delete: the list is empty after deletion, a second delete reports NotFound.
    #[tokio::test]
    async fn delete_removes_channel_and_missing_errors() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create");

        DeleteChannelUsecase
            .execute(&repo, created.id)
            .await
            .expect("delete");
        assert!(repo.list().await.expect("list").is_empty());
        assert!(matches!(
            DeleteChannelUsecase.execute(&repo, created.id).await,
            Err(ChannelError::NotFound)
        ));
    }

    /// Enable-disable: false → true round-trip, state persisted correctly.
    #[tokio::test]
    async fn set_enabled_toggles_state() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create");

        let disabled = SetChannelEnabledUsecase
            .execute(&repo, created.id, false)
            .await
            .expect("disable");
        assert!(!disabled.enabled);

        let enabled = SetChannelEnabledUsecase
            .execute(&repo, created.id, true)
            .await
            .expect("enable");
        assert!(enabled.enabled);
    }

    /// List: returns all channels (ordering is left to the repository implementation; only content is verified here).
    #[tokio::test]
    async fn list_returns_all_channels() {
        let repo = InMemoryChannelRepository::new();
        CreateChannelUsecase
            .execute(&repo, input("a"))
            .await
            .expect("create a");
        CreateChannelUsecase
            .execute(&repo, input("b"))
            .await
            .expect("create b");

        let all = ListChannelsUsecase.execute(&repo).await.expect("list");
        assert_eq!(all.len(), 2);
    }

    /// Connectivity test: a success result is echoed and persisted as last_test_ok=true.
    #[tokio::test]
    async fn test_channel_persists_success_result() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");
        let adaptor = MockProviderAdaptor::new(crate::domain::provider::TestResult {
            ok: true,
            latency_ms: 42,
            error: None,
        });

        let result = TestChannelUsecase
            .execute(&repo, created.id, &adaptor)
            .await
            .expect("test channel");
        assert!(result.ok);
        assert_eq!(result.latency_ms, 42);
        assert_eq!(result.error, None);

        let saved = repo
            .find_by_id(created.id)
            .await
            .expect("find")
            .expect("exists");
        assert_eq!(saved.last_test_ok, Some(true));
        assert_eq!(saved.last_test_at, Some(result.tested_at));
    }

    /// Connectivity test: a failure result echoes the error reason and persists as last_test_ok=false.
    #[tokio::test]
    async fn test_channel_persists_failure_and_echoes_error() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");
        let adaptor = MockProviderAdaptor::new(crate::domain::provider::TestResult {
            ok: false,
            latency_ms: 12,
            error: Some("upstream responded 401".into()),
        });

        let result = TestChannelUsecase
            .execute(&repo, created.id, &adaptor)
            .await
            .expect("test channel");
        assert!(!result.ok);
        assert_eq!(result.error.as_deref(), Some("upstream responded 401"));

        let saved = repo
            .find_by_id(created.id)
            .await
            .expect("find")
            .expect("exists");
        assert_eq!(saved.last_test_ok, Some(false));
        assert_eq!(saved.last_test_at, Some(result.tested_at));
    }

    /// Connectivity test: adaptor configuration errors (missing api_key) are also recorded as failure results and echoed, not thrown as use case errors.
    #[tokio::test]
    async fn test_channel_records_config_error_as_failure() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");
        let adaptor = MockProviderAdaptor::not_configured("api key is required");

        let result = TestChannelUsecase
            .execute(&repo, created.id, &adaptor)
            .await
            .expect("config error mapped to failure result");
        assert!(!result.ok);
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("api key is required"),
            "配置错误原因应回显（含 ProviderError Display 前缀）"
        );

        let saved = repo
            .find_by_id(created.id)
            .await
            .expect("find")
            .expect("exists");
        assert_eq!(saved.last_test_ok, Some(false));
    }

    /// Connectivity test: an unknown id returns NotFound, with no persistence side effects.
    #[tokio::test]
    async fn test_channel_unknown_id_errors_not_found() {
        let repo = InMemoryChannelRepository::new();
        let adaptor = MockProviderAdaptor::new(crate::domain::provider::TestResult {
            ok: true,
            latency_ms: 1,
            error: None,
        });
        assert!(matches!(
            TestChannelUsecase
                .execute(&repo, Uuid::now_v7(), &adaptor)
                .await,
            Err(ChannelError::NotFound)
        ));
    }

    #[tokio::test]
    async fn fetch_channel_models_returns_provider_reported_models() {
        let repo = InMemoryChannelRepository::new();
        let adaptor = MockProviderAdaptor::with_models(vec!["gpt-4o".into(), "gpt-4o-mini".into()]);

        let models = FetchChannelModelsUsecase
            .execute(&repo, None, input("draft"), &adaptor)
            .await
            .expect("fetch");

        assert_eq!(models, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[tokio::test]
    async fn fetch_channel_models_keeps_existing_key_when_edit_input_is_blank() {
        let repo = InMemoryChannelRepository::new();
        let created = CreateChannelUsecase
            .execute(&repo, input("openai-prod"))
            .await
            .expect("create");
        let adaptor = MockProviderAdaptor::with_models(vec!["gpt-4o".into()]);
        let mut update = input("openai-prod");
        update.api_key = None;

        let models = FetchChannelModelsUsecase
            .execute(&repo, Some(created.id), update, &adaptor)
            .await
            .expect("fetch");

        assert_eq!(models, vec!["gpt-4o"]);
    }

    #[tokio::test]
    async fn fetch_channel_models_reports_unsupported_provider() {
        let repo = InMemoryChannelRepository::new();
        let adaptor = MockProviderAdaptor::new(crate::domain::provider::TestResult {
            ok: true,
            latency_ms: 0,
            error: None,
        });

        let err = FetchChannelModelsUsecase
            .execute(&repo, None, input("draft"), &adaptor)
            .await
            .expect_err("unsupported");

        assert!(matches!(
            err,
            ChannelError::Provider(ProviderError::Unsupported(_))
        ));
    }
}
