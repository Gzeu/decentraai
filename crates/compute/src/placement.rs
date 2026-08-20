//! Model placement and execution strategy framework (P14 Phase I–K).
//!
//! This module defines the data structures a network-aware planner uses to
//! decide where a workload runs. It is deliberately pure: no I/O, no async,
//! no global database. The actual planner lives in `decentraai_fabric`;
//! `ComputeManager` uses these types to build an explainable placement plan.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Execution strategies supported by the fabric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStrategy {
    #[default]
    Local,
    SingleWorker,
    Remote,
    BatchFanOut,
    Distributed,
    /// Future-ready strategies (not enabled automatically).
    Pipeline,
    TensorSharding,
    MultiGpu,
    PrefillDecodeSplit,
    RemoteGpuAggregation,
    Speculative,
}

impl ExecutionStrategy {
    pub fn label(&self) -> &'static str {
        match self {
            ExecutionStrategy::Local => "local",
            ExecutionStrategy::SingleWorker => "single_worker",
            ExecutionStrategy::Remote => "remote",
            ExecutionStrategy::BatchFanOut => "batch_fan_out",
            ExecutionStrategy::Distributed => "distributed",
            ExecutionStrategy::Pipeline => "pipeline",
            ExecutionStrategy::TensorSharding => "tensor_sharding",
            ExecutionStrategy::MultiGpu => "multi_gpu",
            ExecutionStrategy::PrefillDecodeSplit => "prefill_decode_split",
            ExecutionStrategy::RemoteGpuAggregation => "remote_gpu_aggregation",
            ExecutionStrategy::Speculative => "speculative",
        }
    }

    pub fn is_experimental(&self) -> bool {
        matches!(
            self,
            ExecutionStrategy::Pipeline
                | ExecutionStrategy::TensorSharding
                | ExecutionStrategy::MultiGpu
                | ExecutionStrategy::PrefillDecodeSplit
                | ExecutionStrategy::RemoteGpuAggregation
                | ExecutionStrategy::Speculative
        )
    }
}

/// Requirements of a model/workload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRequirements {
    pub model_id: String,
    pub min_gpu_count: u32,
    pub min_vram_mb: u64,
    pub min_ram_mb: u64,
    pub min_compute_capability: f64,
    pub context_tokens: u32,
    pub quantization: String,
    pub parallelism: u32,
}

/// Network quality constraints for a placement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkConstraints {
    pub max_rtt_ms: Option<u32>,
    pub max_jitter_ms: Option<u32>,
    pub max_packet_loss_percent: Option<f64>,
    pub min_bandwidth_mbps: Option<u32>,
}

/// Resource constraints for a placement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceConstraints {
    pub min_ram_mb: u64,
    pub min_vram_mb: u64,
    pub max_load_percent: u8,
    pub require_gpu: bool,
}

/// Trust constraints for a placement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustConstraints {
    pub min_reputation: f64,
    pub require_trusted: bool,
}

/// One candidate worker for a placement plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementCandidate {
    pub worker_id: String,
    pub score: f64,
    pub compute_fitness: f64,
    pub network_fitness: f64,
    pub trust_fitness: f64,
    pub health_fitness: f64,
    pub load_fitness: f64,
    pub model_available: bool,
    pub resource_headroom: f64,
    pub rtt_ms: Option<u32>,
    pub rejected_reason: Option<String>,
}

/// A rejected candidate with a safe, operational reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub worker_id: String,
    pub reason: String,
}

/// Output of the placement planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlacementPlan {
    pub model: ModelRequirements,
    pub strategy: ExecutionStrategy,
    pub candidates: Vec<PlacementCandidate>,
    pub rejected: Vec<RejectedCandidate>,
    pub selected_workers: Vec<String>,
    pub network_cost: f64,
    pub expected_resource_cost: f64,
    pub execution_mode: String,
}

/// A simple deterministic planner that scores candidates. The real fabric
/// planner uses richer signals; this pure helper produces an explainable plan
/// from synthetic inputs.
pub fn plan_placement(
    requirements: &ModelRequirements,
    candidates: Vec<PlacementCandidate>,
    strategy: ExecutionStrategy,
) -> PlacementPlan {
    let selected: Vec<String> = candidates
        .iter()
        .filter(|c| c.rejected_reason.is_none())
        .take(if strategy == ExecutionStrategy::SingleWorker { 1 } else { usize::MAX })
        .map(|c| c.worker_id.clone())
        .collect();
    let rejected: Vec<RejectedCandidate> = candidates
        .iter()
        .filter(|c| c.rejected_reason.is_some())
        .map(|c| RejectedCandidate {
            worker_id: c.worker_id.clone(),
            reason: c.rejected_reason.clone().unwrap_or_default(),
        })
        .collect();
    let network_cost: f64 = candidates.iter().filter_map(|c| c.rtt_ms.map(f64::from)).sum();
    let expected_resource_cost: f64 = requirements.min_vram_mb as f64
        + requirements.min_ram_mb as f64
        + (requirements.context_tokens as f64) / 1000.0;
    PlacementPlan {
        model: requirements.clone(),
        strategy,
        candidates,
        rejected,
        selected_workers: selected,
        network_cost,
        expected_resource_cost,
        execution_mode: strategy.label().to_string(),
    }
}

/// Future-safe market interfaces (P14 Phase R). Kept unused/optional.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComputeOffer {
    pub worker_id: String,
    pub capability: BTreeMap<String, f64>,
    pub price_hint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComputeDemand {
    pub requester: String,
    pub requirements: ModelRequirements,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComputePrice {
    pub policy_version: u32,
    pub unit: String,
    pub value: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComputeReservation {
    pub reservation_id: String,
    pub worker_id: String,
    pub requirements: ModelRequirements,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComputeSettlement {
    pub reservation_id: String,
    pub credits: u64,
    pub receipt_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, score: f64, rejected: Option<&str>) -> PlacementCandidate {
        PlacementCandidate {
            worker_id: id.to_string(),
            score,
            compute_fitness: score,
            network_fitness: score,
            trust_fitness: score,
            health_fitness: score,
            load_fitness: score,
            model_available: true,
            resource_headroom: score,
            rtt_ms: Some((100.0 / score) as u32),
            rejected_reason: rejected.map(|s| s.to_string()),
        }
    }

    #[test]
    fn single_worker_selects_one() {
        let candidates = vec![candidate("w1", 0.9, None), candidate("w2", 0.8, None)];
        let req = ModelRequirements {
            model_id: "llama.gguf".to_string(),
            ..Default::default()
        };
        let plan = plan_placement(&req, candidates, ExecutionStrategy::SingleWorker);
        assert_eq!(plan.selected_workers.len(), 1);
        assert_eq!(plan.selected_workers[0], "w1");
        assert_eq!(plan.strategy.label(), "single_worker");
    }

    #[test]
    fn rejected_candidates_kept_explainable() {
        let candidates = vec![
            candidate("w1", 0.9, None),
            candidate("w2", 0.8, Some("insufficient vram")),
        ];
        let req = ModelRequirements {
            model_id: "llama.gguf".to_string(),
            ..Default::default()
        };
        let plan = plan_placement(&req, candidates, ExecutionStrategy::SingleWorker);
        assert_eq!(plan.selected_workers.len(), 1);
        assert_eq!(plan.rejected.len(), 1);
        assert!(plan.rejected[0].reason.contains("vram"));
    }

    #[test]
    fn experimental_strategies_flagged() {
        assert!(ExecutionStrategy::MultiGpu.is_experimental());
        assert!(!ExecutionStrategy::Local.is_experimental());
    }
}
