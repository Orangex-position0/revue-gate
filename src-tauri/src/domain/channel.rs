//! Channel aggregate root: entity + model mapping value object + ChannelRepository trait.
//!
//! Fields align with the requirements (`docs/Requirements.md` channel management): name/type/Base URL/API Key/
//! model list/priority/weight/model mappings/enable-disable state/last test. Repository traits live with the aggregate root, not split into subfolders.
//!
//! The serde derives are used for serialization across the Tauri Command boundary (frontend types in `src/types/index.ts`):
//! fields are output in camelCase and channel types as lowercase strings.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
    /// Weight field: v0.1 only persists and echoes it; weighted dispatch is a high-availability enhancement (Out of Scope).
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
