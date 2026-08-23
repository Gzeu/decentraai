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
    /// This node's own peer id, when known, so the planner can label local
    /// execution honestly. `None` = unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_peer: Option<String>,
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
        .take(if strategy == ExecutionStrategy::SingleWorker {
            1
        } else {
            usize::MAX
        })
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
    let network_cost: f64 = candidates
        .iter()
        .filter_map(|c| c.rtt_ms.map(f64::from))
        .sum();
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

/// Weights for the placement engine's composite scoring.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlacementWeights {
    pub w_compute: f64,
    pub w_network: f64,
    pub w_trust: f64,
    pub w_health: f64,
    pub w_load: f64,
    pub w_model: f64,
    pub w_headroom: f64,
    /// FAIRNESS weight for the contribution-balance bias. The bias is
    /// saturated to ±0.15 BEFORE weighting: capacity/health/trust dominate.
    pub w_fairness: f64,
}

impl Default for PlacementWeights {
    fn default() -> Self {
        Self {
            w_compute: 0.25,
            w_network: 0.15,
            w_trust: 0.10,
            w_health: 0.10,
            w_load: 0.15,
            w_model: 0.15,
            w_headroom: 0.08,
            w_fairness: 0.02,
        }
    }
}

/// The pure, deterministic placement engine (Distributed Compute Fabric v2).
///
/// Turns a workload's requirements + the live fabric graph into an explainable
/// [`PlacementPlan`]. Scoring folds seven dimensions:
///
/// ```text
/// COMPUTE FITNESS + NETWORK FITNESS + TRUST + HEALTH +
/// CURRENT LOAD + MODEL AVAILABILITY + RESOURCE HEADROOM
/// ```
///
/// Hard gates (trust, health, remote opt-in, model availability) reject a
/// candidate with a safe reason BEFORE any score is computed; soft dimensions
/// only rank the remaining candidates. When no single node fits the workload
/// and `allow_distributed` is set, the engine falls back to the compute graph's
/// candidate groups (multi-worker / multi-GPU readiness) and records the group
/// as the selected workers with a `distributed` strategy.
#[derive(Debug, Clone)]
pub struct PlacementEngine {
    pub weights: PlacementWeights,
    pub allow_distributed: bool,
}

impl Default for PlacementEngine {
    fn default() -> Self {
        Self {
            weights: PlacementWeights::default(),
            allow_distributed: true,
        }
    }
}

impl PlacementEngine {
    /// Plans placement for `req` against the nodes in `graph`.
    pub fn plan(
        &self,
        req: &ModelRequirements,
        graph: &crate::fabric_graph::FabricGraph,
    ) -> PlacementPlan {
        let mut candidates = Vec::new();
        let mut rejected = Vec::new();

        for node in graph.compute.nodes().values() {
            // Hard gates first — never score a node that must be rejected.
            if let Some(reason) = self.reject_reason(node, req) {
                rejected.push(RejectedCandidate {
                    worker_id: node.peer_id.clone(),
                    reason,
                });
                continue;
            }
            let score = self.score(node, req, graph);
            candidates.push(PlacementCandidate {
                worker_id: node.peer_id.clone(),
                score,
                compute_fitness: self.compute_fitness(node, req),
                network_fitness: self.network_fitness(node, graph),
                trust_fitness: if node.trusted { 1.0 } else { 0.0 },
                health_fitness: if node.healthy { 1.0 } else { 0.0 },
                load_fitness: self.load_fitness(node),
                model_available: node.has_model(&req.model_id),
                resource_headroom: self.headroom(node, req),
                rtt_ms: node.link.as_ref().map(|l| l.rtt_us / 1000),
                rejected_reason: None,
            });
        }

        // Sort candidates best-first, deterministic on score ties.
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.worker_id.cmp(&b.worker_id))
        });

        // Single-worker strategy: take the best candidate. Every candidate that
        // passed the hard gates can run this workload by itself.
        if let Some(best) = candidates.first().cloned() {
            return PlacementPlan {
                model: req.clone(),
                strategy: if req.local_peer.as_deref() == Some(best.worker_id.as_str()) {
                    ExecutionStrategy::Local
                } else {
                    ExecutionStrategy::SingleWorker
                },
                candidates,
                rejected,
                selected_workers: vec![best.worker_id],
                network_cost: best.rtt_ms.unwrap_or(0) as f64,
                expected_resource_cost: expected_cost(req),
                execution_mode: "single_worker".to_string(),
            };
        }

        // No single node fits. If distributed placement is allowed, look for a
        // candidate group (increasing size, bounded) whose combined resources
        // satisfy the workload.
        if self.allow_distributed {
            let node_count = graph.compute.nodes().len();
            for size in 2..=node_count.min(4) {
                if let Some((group, _)) =
                    graph.compute.candidate_groups(req, size).into_iter().next()
                {
                    let network_cost = graph.group_score(&group, req);
                    return PlacementPlan {
                        model: req.clone(),
                        strategy: ExecutionStrategy::Distributed,
                        candidates,
                        rejected,
                        selected_workers: group,
                        network_cost,
                        expected_resource_cost: expected_cost(req),
                        execution_mode: "distributed".to_string(),
                    };
                }
            }
        }

        // Honest empty plan: no node and no group can run this workload.
        PlacementPlan {
            model: req.clone(),
            strategy: ExecutionStrategy::Distributed,
            candidates,
            rejected,
            selected_workers: Vec::new(),
            network_cost: 0.0,
            expected_resource_cost: expected_cost(req),
            execution_mode: "no_placement".to_string(),
        }
    }

    /// Hard-gate rejection reason, or `None` when the node is eligible.
    fn reject_reason(
        &self,
        node: &crate::fabric_graph::FabricNode,
        req: &ModelRequirements,
    ) -> Option<String> {
        if !node.trusted {
            return Some("untrusted worker".to_string());
        }
        if !node.healthy {
            return Some("worker unhealthy".to_string());
        }
        if !node.accepts_remote {
            return Some("does not accept remote inference".to_string());
        }
        if node.capability.gpu.is_none() && req.min_vram_mb > 0 {
            return Some("no gpu".to_string());
        }
        if node.gpu_count() < req.min_gpu_count {
            return Some(format!(
                "insufficient gpu count: {} < {}",
                node.gpu_count(),
                req.min_gpu_count
            ));
        }
        if node.total_vram_mb() < req.min_vram_mb {
            return Some(format!(
                "insufficient vram: {} < {} MiB",
                node.total_vram_mb(),
                req.min_vram_mb
            ));
        }
        if node.total_ram_mb() < req.min_ram_mb {
            return Some(format!(
                "insufficient ram: {} < {} MiB",
                node.total_ram_mb(),
                req.min_ram_mb
            ));
        }
        None
    }

    fn score(
        &self,
        node: &crate::fabric_graph::FabricNode,
        req: &ModelRequirements,
        graph: &crate::fabric_graph::FabricGraph,
    ) -> f64 {
        let w = &self.weights;
        w.w_compute * self.compute_fitness(node, req)
            + w.w_network * self.network_fitness(node, graph)
            + w.w_fairness * self.fairness_bias(node)
            + w.w_trust * if node.trusted { 1.0 } else { 0.0 }
            + w.w_health * if node.healthy { 1.0 } else { 0.0 }
            + w.w_load * self.load_fitness(node)
            + w.w_model
                * if node.has_model(&req.model_id) {
                    1.0
                } else {
                    0.0
                }
            + w.w_headroom * self.headroom(node, req)
    }

    fn compute_fitness(
        &self,
        node: &crate::fabric_graph::FabricNode,
        req: &ModelRequirements,
    ) -> f64 {
        let vram = node.total_vram_mb() as f64;
        let ram = node.total_ram_mb() as f64;
        let need_vram = req.min_vram_mb.max(1) as f64;
        let need_ram = req.min_ram_mb.max(1) as f64;
        // Saturating fit: 1.0 at exactly enough, grows slowly with headroom.
        ((vram / need_vram).min(3.0) + (ram / need_ram).min(3.0)) / 6.0
    }

    fn network_fitness(
        &self,
        node: &crate::fabric_graph::FabricNode,
        graph: &crate::fabric_graph::FabricGraph,
    ) -> f64 {
        let rtt_ms = node.link.as_ref().map(|l| l.rtt_us / 1000).unwrap_or(0);
        let reach = graph
            .links
            .get(&node.peer_id)
            .map(|l| l.reach_cost_ms(1))
            .unwrap_or(0);
        let total_ms = rtt_ms.max(reach) as f64;
        // 1.0 at loopback, decaying toward 0 as reach grows.
        1.0 / (1.0 + total_ms / 1000.0)
    }

    fn load_fitness(&self, node: &crate::fabric_graph::FabricNode) -> f64 {
        let load = node.availability.load_percent as f64 / 100.0;
        1.0 - load
    }

    /// Saturated fairness bias in −0.15..=+0.15 from the node's contribution
    /// balance. tanh keeps it smooth; ±200 credits reach the cap. A bias may
    /// reorder near-equal candidates but can NEVER outrank a hard gate
    /// failure or a decisively better capacity fit — those are decided
    /// before scoring.
    fn fairness_bias(&self, node: &crate::fabric_graph::FabricNode) -> f64 {
        const SATURATION: f64 = 200.0;
        0.15 * ((node.contribution_balance as f64 / SATURATION).clamp(-2.0, 2.0)).tanh()
    }

    fn headroom(&self, node: &crate::fabric_graph::FabricNode, req: &ModelRequirements) -> f64 {
        let free_vram = node
            .availability
            .available_vram_mb
            .unwrap_or(node.total_vram_mb());
        let free_ram = node.availability.available_ram_mb;
        let need_vram = req.min_vram_mb.max(1) as f64;
        let need_ram = req.min_ram_mb.max(1) as f64;
        ((free_vram as f64 / need_vram).min(2.0) + (free_ram as f64 / need_ram).min(2.0)) / 4.0
    }
}

fn expected_cost(req: &ModelRequirements) -> f64 {
    req.min_vram_mb as f64 + req.min_ram_mb as f64 + (req.context_tokens as f64) / 1000.0
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

    #[test]
    fn placement_engine_picks_single_capable_worker() {
        use crate::capability::{ComputeCapability, GpuSpec};
        use crate::fabric_graph::{FabricGraph, FabricNode};

        let mut graph = FabricGraph::new();
        let mut node = FabricNode {
            peer_id: "peer-a".to_string(),
            node_name: "A".to_string(),
            node_version: "1.0.0".to_string(),
            trusted: true,
            healthy: true,
            accepts_remote: true,
            capability: ComputeCapability {
                cpu_cores: 8,
                ram_mb: 64_000,
                gpu: Some(GpuSpec::simple("t", 24_000, "t")),
                engine: "llama_server".to_string(),
                served_models: vec![],
                can_provision: false,
                available_models: vec![],
            },
            availability: crate::availability::ComputeAvailability::ready(),
            link: None,
            contribution_balance: 0,
        };
        node.capability.served_models = vec![crate::capability::ServedModel {
            model_hash: "m1".to_string(),
            file_name: "m1.gguf".to_string(),
            size_mb: 100,
            est_ram_mb: 200,
            est_vram_mb: 200,
            context_tokens: 4096,
        }];
        graph.upsert(node);

        let req = ModelRequirements {
            model_id: "m1".to_string(),
            min_gpu_count: 1,
            min_vram_mb: 20_000,
            min_ram_mb: 30_000,
            context_tokens: 4096,
            ..Default::default()
        };
        let engine = PlacementEngine::default();
        let plan = engine.plan(&req, &graph);
        assert_eq!(plan.selected_workers, vec!["peer-a".to_string()]);
        assert!(plan.rejected.is_empty());
    }

    /// FAIRNESS BIAS pin (M15): two IDENTICAL nodes except contribution
    /// balance. The net giver (+200, bias cap) must win the tie — but the
    /// bias is bounded, so a decisively worse capacity fit still loses.
    #[test]
    fn placement_engine_fairness_bias_breaks_ties_only() {
        use crate::capability::{ComputeCapability, GpuSpec};
        use crate::fabric_graph::{FabricGraph, FabricNode};

        fn node(peer: &str, balance: i64) -> FabricNode {
            FabricNode {
                peer_id: peer.to_string(),
                node_name: peer.to_string(),
                node_version: "1.0.0".to_string(),
                trusted: true,
                healthy: true,
                accepts_remote: true,
                capability: ComputeCapability {
                    cpu_cores: 8,
                    ram_mb: 64_000,
                    gpu: Some(GpuSpec::simple("t", 24_000, "t")),
                    engine: "llama_server".to_string(),
                    served_models: vec![],
                    can_provision: false,
                    available_models: vec![],
                },
                availability: crate::availability::ComputeAvailability::ready(),
                link: None,
                contribution_balance: balance,
            }
        }

        let req = ModelRequirements {
            model_id: "m1".to_string(),
            min_gpu_count: 1,
            min_vram_mb: 20_000,
            min_ram_mb: 30_000,
            context_tokens: 4096,
            ..Default::default()
        };

        // Tie case: identical fit, giver wins.
        let mut graph = FabricGraph::new();
        graph.upsert(node("neutral", 0));
        graph.upsert(node("giver", 400));
        let plan = PlacementEngine::default().plan(&req, &graph);
        assert_eq!(
            plan.selected_workers,
            vec!["giver".to_string()],
            "contribution bias decides the otherwise-equal tie"
        );

        // Capacity dominates: a taker with better VRAM headroom beats a
        // giver whose node barely fits.
        let mut graph2 = FabricGraph::new();
        let mut tight_giver = node("tight-giver", 400);
        tight_giver.capability.ram_mb = 31_000;
        tight_giver.availability.available_ram_mb = 30_500;
        graph2.upsert(tight_giver);
        let mut roomy_taker = node("roomy-taker", -100);
        roomy_taker.capability.ram_mb = 64_000;
        roomy_taker.availability.available_ram_mb = 60_000;
        graph2.upsert(roomy_taker);
        let req_big = ModelRequirements {
            model_id: "m1".to_string(),
            min_gpu_count: 1,
            min_vram_mb: 20_000,
            min_ram_mb: 30_000,
            context_tokens: 4096,
            ..Default::default()
        };
        let plan2 = PlacementEngine::default().plan(&req_big, &graph2);
        assert_eq!(
            plan2.selected_workers,
            vec!["roomy-taker".to_string()],
            "capacity fit outranks the fairness bias"
        );
    }

    #[test]
    fn placement_engine_rejects_untrusted_with_reason() {
        use crate::capability::{ComputeCapability, GpuSpec};
        use crate::fabric_graph::{FabricGraph, FabricNode};

        let mut graph = FabricGraph::new();
        let node = FabricNode {
            peer_id: "peer-bad".to_string(),
            node_name: "B".to_string(),
            node_version: "1.0.0".to_string(),
            trusted: false,
            healthy: true,
            accepts_remote: true,
            contribution_balance: 0,
            capability: ComputeCapability {
                cpu_cores: 8,
                ram_mb: 64_000,
                gpu: Some(GpuSpec::simple("t", 24_000, "t")),
                engine: "llama_server".to_string(),
                served_models: vec![],
                can_provision: false,
                available_models: vec![],
            },
            availability: crate::availability::ComputeAvailability::ready(),
            link: None,
        };
        graph.upsert(node);

        let req = ModelRequirements {
            model_id: "m1".to_string(),
            min_gpu_count: 1,
            min_vram_mb: 20_000,
            min_ram_mb: 30_000,
            ..Default::default()
        };
        let engine = PlacementEngine::default();
        let plan = engine.plan(&req, &graph);
        assert!(plan.selected_workers.is_empty());
        assert_eq!(plan.rejected.len(), 1);
        assert!(plan.rejected[0].reason.contains("untrusted"));
    }

    #[test]
    fn placement_engine_falls_back_to_distributed_group() {
        use crate::capability::{ComputeCapability, GpuSpec};
        use crate::fabric_graph::{FabricGraph, FabricNode};

        let mut graph = FabricGraph::new();
        for (peer, vram) in [
            ("peer-a", 24_000u64),
            ("peer-b", 24_000u64),
            ("peer-c", 24_000u64),
        ] {
            graph.upsert(FabricNode {
                peer_id: peer.to_string(),
                node_name: peer.to_string(),
                node_version: "1.0.0".to_string(),
                trusted: true,
                healthy: true,
                accepts_remote: true,
                contribution_balance: 0,
                capability: ComputeCapability {
                    cpu_cores: 8,
                    ram_mb: 64_000,
                    gpu: Some(GpuSpec::simple("t", vram, "t")),
                    engine: "llama_server".to_string(),
                    served_models: vec![],
                    can_provision: false,
                    available_models: vec![],
                },
                availability: crate::availability::ComputeAvailability::ready(),
                link: None,
            });
        }

        // 70 GiB model: no single 24 GiB node fits; three combined do.
        let req = ModelRequirements {
            model_id: "big.gguf".to_string(),
            min_gpu_count: 2,
            min_vram_mb: 70_000,
            min_ram_mb: 60_000,
            ..Default::default()
        };
        let engine = PlacementEngine::default();
        let plan = engine.plan(&req, &graph);
        assert_eq!(
            plan.selected_workers.len(),
            3,
            "three workers must be selected"
        );
        assert_eq!(plan.execution_mode, "distributed");
    }

    #[test]
    fn multi_gpu_node_satisfies_min_gpu_count() {
        use crate::capability::{ComputeCapability, GpuSpec};
        use crate::fabric_graph::{FabricGraph, FabricNode};

        let mut graph = FabricGraph::new();
        let mut gpu = GpuSpec::simple("A6000", 48_000, "nvidia");
        gpu.count = 2;
        gpu.gpu_class = Some("datacenter".to_string());
        graph.upsert(FabricNode {
            peer_id: "peer-dual".to_string(),
            node_name: "dual".to_string(),
            node_version: "1.0.0".to_string(),
            trusted: true,
            healthy: true,
            accepts_remote: true,
            contribution_balance: 0,
            capability: ComputeCapability {
                cpu_cores: 16,
                ram_mb: 128_000,
                gpu: Some(gpu.clone()),
                engine: "llama_server".to_string(),
                served_models: vec![],
                can_provision: false,
                available_models: vec![],
            },
            availability: crate::availability::ComputeAvailability::ready(),
            link: None,
        });

        // A 70 GiB model needing 2 GPUs fits on the dual-GPU node.
        let req = ModelRequirements {
            model_id: "big.gguf".to_string(),
            min_gpu_count: 2,
            min_vram_mb: 70_000,
            min_ram_mb: 60_000,
            ..Default::default()
        };
        let engine = PlacementEngine::default();
        let plan = engine.plan(&req, &graph);
        assert!(
            plan.selected_workers == vec!["peer-dual".to_string()],
            "unexpected plan: selected={:?} rejected={:?}",
            plan.selected_workers,
            plan.rejected
        );
        assert_eq!(plan.execution_mode, "single_worker");
        assert!(plan.rejected.is_empty());
        // Total VRAM counts both GPUs.
        assert_eq!(gpu.total_vram_mb(), 96_000);
    }

    #[test]
    fn single_gpu_node_rejected_for_min_gpu_count_2() {
        use crate::capability::{ComputeCapability, GpuSpec};
        use crate::fabric_graph::{FabricGraph, FabricNode};

        let mut graph = FabricGraph::new();
        graph.upsert(FabricNode {
            peer_id: "peer-single".to_string(),
            node_name: "single".to_string(),
            node_version: "1.0.0".to_string(),
            trusted: true,
            healthy: true,
            accepts_remote: true,
            contribution_balance: 0,
            capability: ComputeCapability {
                cpu_cores: 8,
                ram_mb: 64_000,
                gpu: Some(GpuSpec::simple("RTX", 24_000, "x")),
                engine: "llama_server".to_string(),
                served_models: vec![],
                can_provision: false,
                available_models: vec![],
            },
            availability: crate::availability::ComputeAvailability::ready(),
            link: None,
        });

        let req = ModelRequirements {
            model_id: "big.gguf".to_string(),
            min_gpu_count: 2,
            min_vram_mb: 40_000,
            min_ram_mb: 30_000,
            ..Default::default()
        };
        let engine = PlacementEngine::default();
        let plan = engine.plan(&req, &graph);
        assert!(plan.selected_workers.is_empty());
        assert_eq!(plan.rejected.len(), 1);
        assert!(plan.rejected[0].reason.contains("gpu count"));
    }
}
