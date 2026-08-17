//! Domain service: channel selection strategy (ChannelSelector) — filter candidate channels by model → group by priority → weighted random within group.
//!
//! Pure business logic, no DB or HTTP (see docs/Architecture-backend.md):
//! 1. Keep only enabled channels (disabled channels do not participate in dispatch);
//! 2. Model matching: the channel `models` contains the requested model, or `model_mappings` hits `client_model`,
//!    or `models` is empty (unrestricted channels accept any model);
//! 3. Stable sort ascending by priority (smaller values take precedence) to group channels of equal priority;
//! 4. Within each priority group, order channels by weighted random without replacement (weight 0 channels fall to the
//!    tail of their group as fallback only; negative weights are clamped to 0 — the command layer rejects negatives).
//!
//! The result is one ordered failover queue that the proxy consumes in order (retrying only on retryable failures);
//! scheduling is stateless — each request reshuffles once and retries just walk the same queue.
//!
//! Orchestration (finding channels, retrying in order, applying mappings) lives in usecases/proxy.rs.

use rand::Rng;

use super::channel::Channel;

/// Domain service for the channel selection strategy: filter candidate channels and build an ordered failover queue.
pub struct ChannelSelector;

impl ChannelSelector {
    /// Select candidate channels and order them into a failover queue: enabled → model match → group by priority
    /// (ascending) → within each group, weighted random without replacement. The RNG is injected so tests can use a
    /// seeded generator; production passes `rand::rng()`.
    pub fn select_channels<R: Rng>(channels: &[Channel], model: &str, rng: &mut R) -> Vec<Channel> {
        // 1. Filter candidates: keep enabled channels that support the requested model.
        let mut candidates: Vec<Channel> = channels
            .iter()
            .filter(|c| c.enabled && Self::supports(c, model))
            .cloned()
            .collect();
        // 2. Stable sort ascending by priority so equal-priority channels group together (input order preserved).
        candidates.sort_by_key(|c| c.priority);

        // 3. For each priority group, draw channels by weighted random without replacement and append to the queue.
        let mut ordered = Vec::with_capacity(candidates.len());
        let mut start = 0;
        while start < candidates.len() {
            let priority = candidates[start].priority;
            let mut end = start;
            while end < candidates.len() && candidates[end].priority == priority {
                end += 1;
            }

            // Weighted random without replacement: each pick is proportional to the remaining weights.
            // Weight 0 contributes nothing to the total, so it is only reached when every positive weight is gone.
            let mut group = candidates[start..end].to_vec();
            while !group.is_empty() {
                let total: i64 = group.iter().map(|c| i64::from(c.weight.max(0))).sum();
                let idx = if total > 0 {
                    let mut point = rng.random_range(0..total);
                    let mut selected = 0;
                    for (i, c) in group.iter().enumerate() {
                        point -= i64::from(c.weight.max(0));
                        if point < 0 {
                            selected = i;
                            break;
                        }
                    }
                    selected
                } else {
                    // All remaining weights are 0 → fall back to input order.
                    0
                };
                ordered.push(group.remove(idx));
            }

            start = end;
        }
        ordered
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
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(0)
    }

    fn channel(name: &str, models: &[&str], priority: i32, enabled: bool) -> Channel {
        let mut c = sample_channel();
        c.name = name.to_string();
        c.models = models.iter().map(|m| m.to_string()).collect();
        c.priority = priority;
        c.enabled = enabled;
        c
    }

    /// Channel with a fixed model set and an explicit weight (for weight-sensitive tests).
    fn weighted(name: &str, priority: i32, weight: i32) -> Channel {
        let mut c = channel(name, &["gpt-4o"], priority, true);
        c.weight = weight;
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
        assert!(ChannelSelector::select_channels(&channels, "gpt-4o", &mut rng()).is_empty());
    }

    /// `models` contains the requested model → selected.
    #[test]
    fn select_keeps_channel_when_model_in_models_list() {
        let channels = vec![channel("a", &["gpt-4o"], 1, true)];
        let selected = ChannelSelector::select_channels(&channels, "gpt-4o", &mut rng());
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "a");
    }

    /// `model_mappings.client_model` hits → selected (the mapping name is the name available to the client).
    #[test]
    fn select_keeps_channel_via_mapping_client_model() {
        let channels = vec![mapped_channel("chat", "gpt-4o")];
        let selected = ChannelSelector::select_channels(&channels, "chat", &mut rng());
        assert_eq!(selected.len(), 1);
    }

    /// Empty `models` = unrestricted channel, accepts any model.
    #[test]
    fn select_keeps_unrestricted_channel_with_empty_models() {
        let channels = vec![channel("any", &[], 0, true)];
        assert_eq!(
            ChannelSelector::select_channels(&channels, "anything-else", &mut rng()).len(),
            1
        );
    }

    /// Model not hit by any list / mapping → dropped.
    #[test]
    fn select_drops_channel_not_supporting_model() {
        let channels = vec![
            channel("a", &["gpt-4o"], 0, true),
            channel("b", &["claude-3"], 0, true),
        ];
        let selected = ChannelSelector::select_channels(&channels, "claude-3", &mut rng());
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "b");
    }

    /// Priority groups are ordered ascending (smaller first); within a same-priority group the order is shuffled
    /// but the set is preserved (the middle two channels here are equal priority and appear together).
    #[test]
    fn select_orders_groups_by_priority_and_shuffles_within_group() {
        let a = channel("a", &["gpt-4o"], 5, true);
        let b = channel("b", &["gpt-4o"], 0, true);
        let c1 = channel("c1", &["gpt-4o"], 3, true);
        let c2 = channel("c2", &["gpt-4o"], 3, true);

        let channels = vec![a, b, c1, c2];
        let names: Vec<String> = ChannelSelector::select_channels(&channels, "gpt-4o", &mut rng())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names[0].as_str(), "b", "优先级最小的一组在最前");
        assert_eq!(names[3].as_str(), "a", "优先级最大的一组在最后");
        let mut middle = names[1..3].to_vec();
        middle.sort();
        assert_eq!(middle, ["c1", "c2"], "同优先级渠道集合不变，顺序随机");
    }

    /// Weight 0 contributes nothing, so it always falls to the tail of its priority group (fallback only).
    #[test]
    fn select_puts_zero_weight_last_within_group() {
        let heavy = weighted("heavy", 0, 5);
        let zero = weighted("zero", 0, 0);
        let channels = vec![zero, heavy];
        let names: Vec<String> = ChannelSelector::select_channels(&channels, "gpt-4o", &mut rng())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["heavy", "zero"], "0 权重渠道恒排组内最后");
    }

    /// A single-channel group is returned as-is (no shuffle needed).
    #[test]
    fn select_single_channel_group_returns_it() {
        let only = weighted("only", 0, 1);
        let selected = ChannelSelector::select_channels(&[only], "gpt-4o", &mut rng());
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "only");
    }

    /// All-zero-weight group falls back to input order (total weight 0 → pick the first each round).
    #[test]
    fn select_all_zero_weight_preserves_input_order() {
        let a = weighted("a", 0, 0);
        let b = weighted("b", 0, 0);
        let names: Vec<String> = ChannelSelector::select_channels(&[a, b], "gpt-4o", &mut rng())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["a", "b"], "全 0 权重退化为输入顺序");
    }

    /// Negative weight is clamped to 0 as a defense against dirty data (the command layer rejects it).
    #[test]
    fn select_clamps_negative_weight_as_zero() {
        let neg = weighted("neg", 0, -5);
        let pos = weighted("pos", 0, 1);
        let channels = vec![neg, pos];
        let names: Vec<String> = ChannelSelector::select_channels(&channels, "gpt-4o", &mut rng())
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["pos", "neg"], "负权重被钳制为 0，排最后");
    }

    /// Weighted selection actually biases toward higher weight: over many fresh seeds, a 9:1 split favors the heavy
    /// channel for the first slot the vast majority of the time (threshold is far below the true ~90% rate).
    #[test]
    fn weighted_selection_prefers_higher_weight() {
        let trials = 2000u64;
        let mut heavy_first = 0u64;
        for i in 0..trials {
            let heavy = weighted("heavy", 0, 9);
            let light = weighted("light", 0, 1);
            let selected = ChannelSelector::select_channels(
                &[heavy, light],
                "gpt-4o",
                &mut SmallRng::seed_from_u64(i),
            );
            if selected[0].name == "heavy" {
                heavy_first += 1;
            }
        }
        assert!(
            heavy_first > trials * 7 / 10,
            "heavy first {heavy_first}/{trials}"
        );
    }

    /// Returns empty candidates when no enabled / matching channels exist.
    #[test]
    fn select_empty_when_no_candidates() {
        let channels = vec![channel("a", &["claude-3"], 0, true)];
        assert!(ChannelSelector::select_channels(&channels, "gpt-4o", &mut rng()).is_empty());
    }

    /// Property test (mandatory-tier PBT, algorithm): for arbitrary priority/weight combinations the queue must be
    /// a permutation of the candidates, priority-ordered across groups, and weight-0 channels never precede a
    /// positive-weight channel within their group.
    mod property {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn select_preserves_set_and_priority_order(
                specs in prop::collection::vec((0i32..4i32, 0i32..11i32), 0..20)
            ) {
                let channels: Vec<Channel> = specs
                    .iter()
                    .enumerate()
                    .map(|(i, (p, w))| {
                        let mut c = sample_channel();
                        c.name = format!("c{i}");
                        c.models = vec!["m".to_string()];
                        c.priority = *p;
                        c.weight = *w;
                        c
                    })
                    .collect();

                // Derive a per-input seed so different inputs exercise different RNG paths.
                let mut seed = 0u64;
                for (p, w) in &specs {
                    seed = seed
                        .wrapping_mul(31)
                        .wrapping_add(((*p as u64) << 32) ^ (*w as u64));
                }
                let selected = ChannelSelector::select_channels(
                    &channels,
                    "m",
                    &mut SmallRng::seed_from_u64(seed),
                );

                // Permutation: same multiset of names.
                let mut want: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
                want.sort();
                let mut got: Vec<&str> = selected.iter().map(|c| c.name.as_str()).collect();
                got.sort();
                prop_assert_eq!(got, want);

                // Priority groups ordered non-decreasing across the queue.
                for w in selected.windows(2) {
                    prop_assert!(w[0].priority <= w[1].priority);
                }

                // Within a group, a weight-0 channel never precedes a positive-weight channel.
                for w in selected.windows(2) {
                    if w[0].priority == w[1].priority {
                        prop_assert!(!(w[0].weight == 0 && w[1].weight > 0));
                    }
                }
            }
        }
    }
}
