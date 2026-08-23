//! Channel aggregate root: entity + model mapping value object + ChannelRepository trait.
//!
//! Fields align with the requirements (`docs/Requirements.md` channel management): name/type/Base URL/API Key/
//! model list/priority/weight/model mappings/enable-disable state/last test. Repository traits live with the aggregate root, not split into subfolders.
//!
//! The serde derives are used for serialization across the Tauri Command boundary (frontend types in `src/types/index.ts`):
//! fields are output in camelCase and channel types as lowercase strings.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

use crate::domain::error::RepositoryError;

/// Built-in channel types. OpenAI / DeepSeek / Custom use OpenAI-compatible passthrough,
/// Claude / Gemini use protocol conversion adapters (implemented in infrastructure/providers, out of scope this stage).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    OpenAi,
    DeepSeek,
    Custom,
    Claude,
    Gemini,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown channel type: {0}")]
pub struct ParseChannelTypeError(String);

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ChannelType::OpenAi => "openai",
            ChannelType::DeepSeek => "deepseek",
            ChannelType::Custom => "custom",
            ChannelType::Claude => "claude",
            ChannelType::Gemini => "gemini",
        })
    }
}

impl FromStr for ChannelType {
    type Err = ParseChannelTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" => Ok(ChannelType::OpenAi),
            "deepseek" => Ok(ChannelType::DeepSeek),
            "custom" => Ok(ChannelType::Custom),
            "claude" => Ok(ChannelType::Claude),
            "gemini" => Ok(ChannelType::Gemini),
            _ => Err(ParseChannelTypeError(s.to_string())),
        }
    }
}

/// Model mapping value object: unified client model name ↔ actual upstream model name; passthrough when unmapped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMapping {
    /// Unified model name used by the client (downstream).
    pub client_model: String,
    /// Actual model name on the upstream channel.
    pub upstream_model: String,
}

/// Channel entity: a channel configuration that can be dispatched to upstream.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: Uuid,
    /// Admin-readable name, e.g. `openai-prod`.
    pub name: String,
    pub channel_type: ChannelType,
    /// Upstream Base URL; when None, ProviderAdaptor provides a default.
    pub base_url: Option<String>,
    /// Upstream API Key. Red line: must never be exposed to downstream or written in plaintext to request logs;
    /// masked before control-plane commands return (see interface/commands/channel.rs).
    pub api_key: Option<String>,
    /// List of upstream models this channel supports.
    pub models: Vec<String>,
    /// Dispatch priority (smaller values take precedence; candidates sorted ascending by priority).
    pub priority: i32,
    /// Dispatch weight: proportional share when routing within a priority group (0 = fallback only, tried last).
    pub weight: i32,
    /// Model mapping table.
    pub model_mappings: Vec<ModelMapping>,
    /// Enable/disable state; disabled channels do not participate in dispatch (behavior takes effect in stage 07).
    pub enabled: bool,
    /// Timestamp of the last connectivity test (connectivity tests arrive with the stage 06 ProviderAdaptor).
    pub last_test_at: Option<DateTime<Utc>>,
    /// Whether the last connectivity test succeeded.
    pub last_test_ok: Option<bool>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Channel repository trait: interface defined in the domain layer, sqlx implementation in infrastructure, usecases depend only on the trait.
#[async_trait::async_trait]
pub trait ChannelRepository: Send + Sync {
    /// Look up a channel by id.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Channel>, RepositoryError>;
    /// List all channels (ordering is implementation-defined, ascending by priority by default).
    async fn list(&self) -> Result<Vec<Channel>, RepositoryError>;
    /// Create or overwrite-save a channel.
    async fn save(&self, channel: &Channel) -> Result<(), RepositoryError>;
    /// Delete a channel by id; returns `RepositoryError::NotFound` when not found.
    async fn delete(&self, id: Uuid) -> Result<(), RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_type_display_uses_persisted_domain_strings() {
        assert_eq!(ChannelType::OpenAi.to_string(), "openai");
        assert_eq!(ChannelType::DeepSeek.to_string(), "deepseek");
        assert_eq!(ChannelType::Custom.to_string(), "custom");
        assert_eq!(ChannelType::Claude.to_string(), "claude");
        assert_eq!(ChannelType::Gemini.to_string(), "gemini");
    }

    #[test]
    fn channel_type_from_str_accepts_known_values_and_rejects_unknown() {
        assert_eq!("openai".parse::<ChannelType>(), Ok(ChannelType::OpenAi));
        assert_eq!("deepseek".parse::<ChannelType>(), Ok(ChannelType::DeepSeek));
        assert_eq!("custom".parse::<ChannelType>(), Ok(ChannelType::Custom));
        assert_eq!("claude".parse::<ChannelType>(), Ok(ChannelType::Claude));
        assert_eq!("gemini".parse::<ChannelType>(), Ok(ChannelType::Gemini));
        assert!("mystery".parse::<ChannelType>().is_err());
    }
}
