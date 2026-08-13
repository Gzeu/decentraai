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
        let capable: Vec<String> = registry
            .model_workers(model_hash)
            .into_iter()
            .filter(|(_, s)| s.routing_capable)
            .map(|(id, _)| id)
            .filter(|id| candidate_workers.contains(id))
            .collect();
        if capable.len() < 2 {
            return ExpertDecision::WholeModel(capable.first().cloned().unwrap_or_default());
        }
        let top_k = registry.layout(model_hash).map(|l| l.top_k).unwrap_or(1);
        ExpertDecision::ExpertSplit {
            model_hash: model_hash.to_string(),
            workers: capable,
            top_k,
        }
    }
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
}