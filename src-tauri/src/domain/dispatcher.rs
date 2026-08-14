//! Domain service: channel selection strategy (ChannelSelector) — filter candidate channels by model → sort by priority.
//!
//! Pure business logic, no DB or HTTP (see docs/Architecture-backend.md):
//! 1. Keep only enabled channels (disabled channels do not participate in dispatch);
//! 2. Model matching: the channel `models` contains the requested model, or `model_mappings` hits `client_model`,
//!    or `models` is empty (unrestricted channels accept any model);
//! 3. Stable sort ascending by priority (smaller values take precedence, same priority keeps input order).
//!
//! Orchestration (finding channels, retrying in order, applying mappings) lives in usecases/proxy.rs.

use super::channel::Channel;

/// Domain service for the channel selection strategy: filter candidate channels from all channels and sort them.
pub struct ChannelSelector;

impl ChannelSelector {
    /// Select candidate channels: enabled → model match → ascending priority.
    pub fn select(channels: &[Channel], model: &str) -> Vec<Channel> {
        let mut candidates: Vec<Channel> = channels
            .iter()
            .filter(|c| c.enabled && Self::supports(c, model))
            .cloned()
            .collect();
        candidates.sort_by_key(|c| c.priority);
        candidates
    }

    /// Whether the channel can serve the client model: when `models` is non-empty it must contain the requested model or have a `client_model`
    /// mapping hit; an empty `models` is treated as unrestricted and accepts any model.
    pub fn supports(channel: &Channel, model: &str) -> bool {
        if channel.models.is_empty() {
            return true;
        }
        channel.models.iter().any(|m| m == model)
            || channel
                .model_mappings
                .iter()
                .any(|m| m.client_model == model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channel::ModelMapping;
    use crate::test_support::sample_channel;

    fn channel(name: &str, models: &[&str], priority: i32, enabled: bool) -> Channel {
        let mut c = sample_channel();
        c.name = name.to_string();
        c.models = models.iter().map(|m| m.to_string()).collect();
        c.priority = priority;
        c.enabled = enabled;
        c
    }

    fn mapped_channel(client: &str, upstream: &str) -> Channel {
        let mut c = channel("mapped", &[], 0, true);
        c.model_mappings = vec![ModelMapping {
            client_model: client.to_string(),
            upstream_model: upstream.to_string(),
        }];
        c
    }

    /// Disabled channels are not candidates, even when the model matches.
    #[test]
    fn select_drops_disabled_channels() {
        let channels = vec![channel("off", &["gpt-4o"], 0, false)];
        assert!(ChannelSelector::select(&channels, "gpt-4o").is_empty());
    }

    /// `models` contains the requested model → selected.
    #[test]
    fn select_keeps_channel_when_model_in_models_list() {
        let channels = vec![channel("a", &["gpt-4o"], 1, true)];
        let selected = ChannelSelector::select(&channels, "gpt-4o");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "a");
    }

    /// `model_mappings.client_model` hits → selected (the mapping name is the name available to the client).
    #[test]
    fn select_keeps_channel_via_mapping_client_model() {
        let channels = vec![mapped_channel("chat", "gpt-4o")];
        let selected = ChannelSelector::select(&channels, "chat");
        assert_eq!(selected.len(), 1);
    }

    /// Empty `models` = unrestricted channel, accepts any model.
    #[test]
    fn select_keeps_unrestricted_channel_with_empty_models() {
        let channels = vec![channel("any", &[], 0, true)];
        assert_eq!(ChannelSelector::select(&channels, "anything-else").len(), 1);
    }

    /// Model not hit by any list / mapping → dropped.
    #[test]
    fn select_drops_channel_not_supporting_model() {
        let channels = vec![
            channel("a", &["gpt-4o"], 0, true),
            channel("b", &["claude-3"], 0, true),
        ];
        let selected = ChannelSelector::select(&channels, "claude-3");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "b");
    }

    /// Candidates sorted ascending by priority (smaller values take precedence); same priority keeps input order (stable sort).
    #[test]
    fn select_sorts_by_priority_ascending_and_is_stable() {
        let mut a = channel("a", &["gpt-4o"], 5, true);
        a.model_mappings = vec![ModelMapping {
            client_model: "chat".into(),
            upstream_model: "gpt-4o".into(),
        }];
        let b = channel("b", &["gpt-4o"], 0, true);
        let c1 = channel("c1", &["gpt-4o"], 3, true);
        let c2 = channel("c2", &["gpt-4o"], 3, true);

        let channels = vec![a, b, c1, c2];
        let names: Vec<String> = ChannelSelector::select(&channels, "gpt-4o")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, vec!["b", "c1", "c2", "a"], "优先级升序且同级稳定");
    }

    /// Returns empty candidates when no enabled / matching channels exist.
    #[test]
    fn select_empty_when_no_candidates() {
        let channels = vec![channel("a", &["claude-3"], 0, true)];
        assert!(ChannelSelector::select(&channels, "gpt-4o").is_empty());
    }
}
