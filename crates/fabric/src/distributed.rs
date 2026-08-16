//! Experimental distributed-execution planning primitives (two-worker, llama.cpp RPC).
//!
//! This module introduces the minimal, planner-facing abstraction for
//! multi-worker distributed inference: a `DistributedExecutionCandidate` and
//! provenance markers that let the coordinator compare single-worker plans
//! against small collaborative plans without changing the existing
//! `ExecutionPlanner` scoring logic.
//!
//! Scope (M23 experimental):
//! - initial target is a **two-node** fabric (Desktop i7 + Laptop i5);
//! - candidates cover `{desktop}` / `{laptop}` / `{desktop,laptop}` only;
//! - llama.cpp RPC is strictly gated behind EXPERIMENTAL flags;
//! - no generic tensor-parallel engine is implemented here;
//! - no production traffic is automatically routed through RPC.
//!
//! The planner must be able to conclude "single-worker > multi-worker" and
//! "multi-worker > single-worker" based on **real measurements**. Estimates
//! derived from research or heuristics are clearly marked as `ESTIMATED`;
//! absence of data is `UNKNOWN`. Nothing here fabricates measured numbers.

use serde::{Deserialize, Serialize};

use crate::planner::{CandidateScore, RequestFacts, WorkerFacts};

/// Classification of how reliable a performance number or network cost is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceKind {
    /// Derived from DecentraAI's own real measurements (e.g. RPC harness JSON).
    Measured,
    /// Estimated from prior measurements and simple models (e.g. summing per-
    /// worker latencies, subtracting network cost). Useful but not a direct
    /// measurement.
    Estimated,
    /// Experimental only: provenance exists (e.g. research docs) but no local
    /// measurements and no calibrated estimator.
    Experimental,
    /// No data at all; the planner must treat the benefit as unknown.
    Unknown,
}

/// How a multi-worker distributed plan is classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DistributedClassification {
    /// Not considered; either no collaborative backend exists or the planner
    /// chose a single-worker plan.
    NotApplicable,
    /// Candidate exists but remains behind the experimental gate; benefit
    /// unknown without a real benchmark.
    Experimental,
    /// Candidate has calibrated measurements (e.g. Desktop vs Laptop vs
    /// Desktop+Laptop RPC harness runs) and can be considered as an option.
    Calibrated,
}

/// Network cost model for a candidate. This is deliberately minimal: the
/// planner can only reason about latency and bandwidth it actually measured
/// or a caller supplied. Absence of data is recorded as `Unknown`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkCost {
    /// Round-trip time in milliseconds between coordinator and the slowest
    /// participating worker. `None` when no measurement exists.
    pub rtt_ms: Option<u32>,
    /// Approximate throughput in megabits per second on the critical links.
    /// `None` when no measurement exists.
    pub bandwidth_mbps: Option<u32>,
    /// Evidence provenance for these fields.
    pub evidence: EvidenceKind,
}

impl NetworkCost {
    pub fn unknown() -> Self {
        Self {
            rtt_ms: None,
            bandwidth_mbps: None,
            evidence: EvidenceKind::Unknown,
        }
    }
}

/// Measured or estimated performance of a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceEstimate {
    /// Estimated or measured prompt processing throughput (tokens/sec).
    pub prefill_tps: Option<f32>,
    /// Estimated or measured decode throughput (tokens/sec).
    pub decode_tps: Option<f32>,
    /// Estimated or measured time-to-first-token in milliseconds.
    pub ttft_ms: Option<u32>,
    /// Estimated or measured total latency for a standard benchmark prompt.
    pub total_latency_ms: Option<u32>,
    /// Provenance for these numbers.
    pub evidence: EvidenceKind,
}

impl PerformanceEstimate {
    pub fn unknown() -> Self {
        Self {
            prefill_tps: None,
            decode_tps: None,
            ttft_ms: None,
            total_latency_ms: None,
            evidence: EvidenceKind::Unknown,
        }
    }
}

/// A concrete multi-worker plan candidate the planner can compare against
/// single-worker plans. It is intentionally small and planner-facing: it does
/// not implement routing or RPC itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistributedExecutionCandidate {
    /// Model hash this candidate is for.
    pub model_hash: String,
    /// Back-end kind used for collaboration (e.g. "llama.cpp_rpc_layer"). This
    /// is descriptive only; selection of the backend is up to the coordinator.
    pub backend: String,
    /// Ordered peer ids participating in this plan.
    pub workers: Vec<String>,
    /// Whether each worker can run the model alone (CAN_RUN) and whether it
    /// supports collaborative execution for this model (CAN_COLLABORATE).
    pub can_run: Vec<bool>,
    pub can_collaborate: Vec<bool>,
    /// Human-readable description of the partition strategy (e.g. "layer split
    /// desktop->laptop" or "no partition (single-node)"). This is purely
    /// observability; routing uses the engine configuration.
    pub partition_strategy: String,
    /// Whether host RAM/VRAM headroom is sufficient on each worker.
    pub memory_feasible: bool,
    /// Network cost model for this candidate.
    pub network: NetworkCost,
    /// Perf estimates (prefill/decode/latency) for this candidate.
    pub performance: PerformanceEstimate,
    /// Planner's classification of this candidate.
    pub classification: DistributedClassification,
    /// Confidence in the estimate (0.0..1.0); derived from how many runs fed
    /// the estimate and how stable they were. `0.0` for unknown/experimental.
    pub confidence: f32,
    /// Short provenance string (e.g. "RPC harness LOCAL-BLOCKED", "MEASURED
    /// laptop+desktop benchmark", "ESTIMATED from single-node stats").
    pub provenance: String,
}

impl DistributedExecutionCandidate {
    /// Constructs a minimal two-worker candidate `{desktop,laptop}` for the
    /// given request and worker facts. This does NOT claim any performance
    /// benefit; it merely records the configuration in a structured way.
    pub fn two_node_layer_split(
        req: &RequestFacts,
        desktop: &WorkerFacts,
        laptop: &WorkerFacts,
        network: NetworkCost,
    ) -> Self {
        let model = req.model_hash.clone();
        let workers = vec![desktop.peer_id.clone(), laptop.peer_id.clone()];
        let can_run = vec![
            desktop.serves_model && desktop.available_ram_mb >= req.est_ram_mb
                && desktop.available_vram_mb >= req.est_vram_mb,
            laptop.serves_model && laptop.available_ram_mb >= req.est_ram_mb
                && laptop.available_vram_mb >= req.est_vram_mb,
        ];
        let can_collaborate = vec![
            desktop.capabilities.supports_rpc_layer_split,
            laptop.capabilities.supports_rpc_layer_split,
        ];
        let memory_feasible = desktop.available_ram_mb + laptop.available_ram_mb >= req.est_ram_mb
            && desktop.available_vram_mb + laptop.available_vram_mb >= req.est_vram_mb;
        Self {
            model_hash: model,
            backend: "llama.cpp_rpc_layer".into(),
            workers,
            can_run,
            can_collaborate,
            partition_strategy: "experimental layer/pipeline split across desktop+laptop".into(),
            memory_feasible,
            network,
            performance: PerformanceEstimate::unknown(),
            classification: DistributedClassification::Experimental,
            confidence: 0.0,
            provenance: "EXPERIMENTAL: no two-node RPC benchmark yet".into(),
        }
    }
}

/// A minimal verdict the planner can surface about collaborative candidates
/// vs single-worker baselines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistributedVerdict {
    pub can_run_desktop: bool,
    pub can_run_laptop: bool,
    pub can_collaborate_desktop_laptop: bool,
    /// The best single-worker candidate's score, for comparison.
    pub best_single_worker_score: Option<CandidateScore>,
    /// Optional distributed candidate (two-node). `None` when no collaborative
    /// backend or when the experimental gate disallows collaboration.
    pub distributed_candidate: Option<DistributedExecutionCandidate>,
    /// Planner's conclusion about benefit: `None` when UNKNOWN.
    pub expected_benefit_percent: Option<f32>,
    /// Provenance note explaining why benefit is unknown/estimated/measured.
    pub note: String,
}
