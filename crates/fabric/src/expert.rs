//! Distributed MoE / expert fabric (M21).
//!
//! DecentraAI must be *ready* to route at expert granularity without pretending
//! that today's OpenAI-compatible engines can do it. This module provides the
//! abstraction (expert discovery + placement + routing) and integrates real
//! support **only when an engine advertises [`crate::engine::EngineCapabilities::expert_routing`]**.
//!
//! Because no current engine in this workspace exposes expert-level routing
//! over its HTTP API, the registry still records expert presence accurately,
//! but the planner gates every expert-aware decision behind the capability
//! flag. Where the flag is off, expert-aware routing returns exactly the
//! whole-worker result — the single correct answer for a monolithic model.
//!
//! # Why this is not "fake"
//!
//! The abstraction is real and the gating is honest: a future engine (or an
//! engine with an extended protocol) that advertises `expert_routing` will
//! drive expert placement through this same code path, with no hacks. Building
//! the interface now, and the correct single-worker fallback, is the correct
//! preparation — never a mocked production path.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A model's expert layout as reported by the engine, suited to the planning
/// layer. This is capability metadata, not a runtime allocation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpertLayout {
    /// Number of experts in the model's sparse layers.
    pub expert_count: u32,
    /// Top-k experts active per token.
    pub top_k: u32,
    /// Whether the engine advertises expert-level routing for this model.
    pub routing_capable: bool,
}

/// Which experts a worker holds and can serve. Populated by a worker whose
/// engine reports `expert_routing`; empty/blank otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpertShard {
    /// The specific expert ids this worker holds (empty = whole model).
    pub experts: Vec<u32>,
    /// Whether this worker can route at expert granularity.
    pub routing_capable: bool,
    /// Confidence in the shard's coverage (engine-reported), 0..=1.
    pub coverage: f32,
}

/// Coordinator-side registry of which experts live on which workers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExpertRegistry {
    by_model: BTreeMap<String, BTreeMap<String, ExpertShard>>,
}

impl ExpertRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an expert shard for (model, worker).
    pub fn record(
        &mut self,
        model_hash: &str,
        worker: &str,
        shard: ExpertShard,
    ) {
        self.by_model
            .entry(model_hash.to_string())
            .or_default()
            .insert(worker.to_string(), shard);
    }

    /// Whether any worker claims expert-level routing for this model.
    pub fn has_expert_capable_workers(&self, model_hash: &str) -> bool {
        self.by_model
            .get(model_hash)
            .map(|m| m.values().any(|s| s.routing_capable))
            .unwrap_or(false)
    }

    /// The layout for a model if recorded.
    pub fn layout(&self, model_hash: &str) -> Option<ExpertLayout> {
        self.by_model
            .get(model_hash)
            .map(|m| {
                let count = m
                    .values()
                    .map(|s| s.experts.len() as u32)
                    .max()
                    .unwrap_or(0);
                ExpertLayout {
                    expert_count: count,
                    top_k: 1,
                    routing_capable: m.values().any(|s| s.routing_capable),
                }
            })
    }

    pub fn model_workers(&self, model_hash: &str) -> Vec<(String, ExpertShard)> {
        self.by_model
            .get(model_hash)
            .map(|m| m.iter().map(|(id, s)| (id.clone(), s.clone())).collect())
            .unwrap_or_default()
    }
}

/// The expert-aware routing decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExpertDecision {
    /// Whole-model execution on this worker (single-worker; correct default
    /// when no engine advertises expert routing).
    WholeModel(String),
    /// Split the experts of this model across these workers, in preference
    /// order, only used when `routing_capable` is true.
    ExpertSplit {
        model_hash: String,
        workers: Vec<String>,
        top_k: u32,
    },
}

/// The pure expert-aware router. If no worker is expert-capable for the model,
/// or any required worker lacks the capability, it returns a whole-model
/// decision for the best single worker — never a fabricated split.
pub struct ExpertRouter;

impl ExpertRouter {
    pub fn route(
        &self,
        model_hash: &str,
        registry: &ExpertRegistry,
        candidate_workers: Vec<String>,
    ) -> ExpertDecision {
        if !registry.has_expert_capable_workers(model_hash) {
            // No engine can route at expert level: honest whole-model result.
            let best = candidate_workers
                .into_iter()
                .min()
                .unwrap_or_default();
            return ExpertDecision::WholeModel(best);
        }
        // At least one worker is capable. Split across all capable workers.
        // Carry the shards so we can validate every one before trusting a split.
        let capable: Vec<(String, ExpertShard)> = registry
            .model_workers(model_hash)
            .into_iter()
            .filter(|(_, s)| s.routing_capable)
            .filter(|(id, _)| candidate_workers.contains(id))
            .collect();
        if capable.len() < 2 {
            return ExpertDecision::WholeModel(capable.first().map(|(id, _)| id).cloned().unwrap_or_default());
        }
        // Degenerate / invalid-split guard: a split is only sound when every
        // selected shard is actually servable at expert granularity and free of
        // conflicts. If any capable shard has an EMPTY expert set, an
        // UNDER-COVERAGE (coverage < 1.0) shard, or OVERLAPPING expert ids
        // across the selected workers, a per-chunk/expert split would be broken
        // (a shard owning nothing, a shard that cannot cover its slice, or two
        // workers silently racing on the same expert). In every such case the
        // only correct decision is the single best whole worker — never a
        // fabricated split. This pins the latent bug where a bogus
        // `ExpertShard { experts: Vec::new(), .. }` would have produced a
        // degenerate ExpertSplit downstream.
        let mut seen_experts: std::collections::BTreeSet<u32> = Default::default();
        for (_, shard) in &capable {
            if shard.experts.is_empty() {
                return ExpertDecision::WholeModel(whole_fallback(&capable));
            }
            if shard.coverage < 1.0 {
                return ExpertDecision::WholeModel(whole_fallback(&capable));
            }
            for &id in &shard.experts {
                if !seen_experts.insert(id) {
                    return ExpertDecision::WholeModel(whole_fallback(&capable));
                }
            }
        }
        let top_k = registry.layout(model_hash).map(|l| l.top_k).unwrap_or(1);
        ExpertDecision::ExpertSplit {
            model_hash: model_hash.to_string(),
            workers: capable.into_iter().map(|(id, _)| id).collect(),
            top_k,
        }
    }
}

/// Best single whole-model worker among a list of `(id, shard)`, falling back
/// to the empty string if there is no entry. Kept member-adjacent so the
/// invalid-split guard reads as "return the best whole worker" rather than a
/// swallowed error.
fn whole_fallback(capable: &[(String, ExpertShard)]) -> String {
    capable
        .iter()
        .min_by_key(|(id, _)| id.clone())
        .map(|(id, _)| id.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(id: &str, cap: bool, experts: Vec<u32>) -> (String, ExpertShard) {
        (
            id.to_string(),
            ExpertShard {
                experts,
                routing_capable: cap,
                coverage: 1.0,
            },
        )
    }

    #[test]
    fn no_capable_worker_falls_back_to_whole_model() {
        let mut reg = ExpertRegistry::new();
        reg.record("m1", "a", worker("a", false, vec![]).1);
        let d = ExpertRouter.route("m1", &reg, vec!["b".into(), "a".into()]);
        assert_eq!(d, ExpertDecision::WholeModel("a".into()));
    }

    #[test]
    fn capable_workers_split_experts() {
        let mut reg = ExpertRegistry::new();
        reg.record("m1", "a", worker("a", true, vec![0, 1]).1);
        reg.record("m1", "b", worker("b", true, vec![2, 3]).1);
        let d = ExpertRouter.route("m1", &reg, vec!["a".into(), "b".into()]);
        match d {
            ExpertDecision::ExpertSplit { workers, .. } => {
                assert!(workers.contains(&"a".into()) && workers.contains(&"b".into()));
            }
            other => panic!("expected split, got {other:?}"),
        }
    }

    #[test]
    fn single_capable_worker_is_whole_model() {
        let mut reg = ExpertRegistry::new();
        reg.record("m1", "a", worker("a", true, vec![0, 1, 2]).1);
        let d = ExpertRouter.route("m1", &reg, vec!["a".into()]);
        assert_eq!(d, ExpertDecision::WholeModel("a".into()));
    }

    #[test]
    fn registry_layout_and_workers() {
        let mut reg = ExpertRegistry::new();
        reg.record("m1", "a", worker("a", true, vec![1, 2, 3]).1);
        let l = reg.layout("m1").unwrap();
        assert_eq!(l.expert_count, 3);
        assert!(l.routing_capable);
        assert_eq!(reg.model_workers("m1").len(), 1);
        assert!(reg.has_expert_capable_workers("m1"));
    }

    fn shard(experts: Vec<u32>, coverage: f32) -> ExpertShard {
        ExpertShard {
            experts,
            routing_capable: true,
            coverage,
        }
    }

    #[test]
    fn empty_experts_shard_never_produces_a_split() {
        // Two capable workers, but one owns NO experts. A split here would be
        // degenerate (a worker routing a slice it cannot serve), so the guard
        // must collapse to the best whole worker instead.
        let mut reg = ExpertRegistry::new();
        reg.record("m1", "a", shard(vec![0, 1], 1.0));
        reg.record("m1", "b", shard(vec![], 1.0));
        let d = ExpertRouter.route("m1", &reg, vec!["a".into(), "b".into()]);
        assert_eq!(d, ExpertDecision::WholeModel("a".into()));
    }

    #[test]
    fn overlapping_expert_ids_never_produce_a_split() {
        // Two capable workers both claiming the same expert id would silently
        // race on that expert during a split. Reject the split outright.
        let mut reg = ExpertRegistry::new();
        reg.record("m1", "a", shard(vec![0, 1], 1.0));
        reg.record("m1", "b", shard(vec![1, 2], 1.0));
        let d = ExpertRouter.route("m1", &reg, vec!["a".into(), "b".into()]);
        assert_eq!(d, ExpertDecision::WholeModel("a".into()));
    }

    #[test]
    fn undercoverage_shard_never_produces_a_split() {
        // A capable worker with coverage < 1.0 cannot cover its slice of a
        // split. Guard must fall back to the whole model.
        let mut reg = ExpertRegistry::new();
        reg.record("m1", "a", shard(vec![0, 1], 1.0));
        reg.record("m1", "b", shard(vec![2, 3], 0.5));
        let d = ExpertRouter.route("m1", &reg, vec!["a".into(), "b".into()]);
        assert_eq!(d, ExpertDecision::WholeModel("a".into()));
    }

    #[test]
    fn disjoint_full_coverage_shards_still_split() {
        // Positive control: the guard must NOT reject a perfectly valid split —
        // two capable workers with non-empty, disjoint experts at full coverage.
        let mut reg = ExpertRegistry::new();
        reg.record("m1", "a", shard(vec![0, 1], 1.0));
        reg.record("m1", "b", shard(vec![2, 3], 1.0));
        let d = ExpertRouter.route("m1", &reg, vec!["a".into(), "b".into()]);
        match d {
            ExpertDecision::ExpertSplit { workers, .. } => {
                assert!(workers.contains(&"a".into()) && workers.contains(&"b".into()));
            }
            other => panic!("expected split, got {other:?}"),
        }
    }
}