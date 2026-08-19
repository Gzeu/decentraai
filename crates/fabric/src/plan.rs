//! Execution plan model (M18).
//!
//! The scheduler's job evolved from "choose one worker" to "build the best
//! execution plan". The plan is the artifact of that: a typed, serializable
//! description of *what* runs, *where*, and *in what order*, plus the
//! fallback and the reservations it holds.
//!
//! The plan is deliberately engine-aware but engine-neutral. Whether a plan
//! has one stage or several is a pure function of the engines available and
//! the capabilities they advertise (see [`crate::engine`]). In the common case
//! — a single OpenAI-compatible engine — the planner emits a [`PlanKind::Single`],
//! which is exactly correct: `llama-server`, vLLM and SGLang each run one
//! model per process, and a monolithic GGUF cannot be split across two HTTP
//! backends without tensor-parallel support that the engine must provide.
//!
//! The [`PlanKind::Sequential`] and [`PlanKind::FanOut`] variants are supported
//! by the executor and become active when an engine advertises the relevant
//! capability (prefill/decode separation for sequential; independent parallel
//! sub-requests for fan-out). This keeps the abstraction honest: the planner
//! only emits what the engines can actually execute.

use crate::engine::EngineKind;
use serde::{Deserialize, Serialize};

/// One unit of work in a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionStage {
    pub stage_id: String,
    /// The worker (PeerId) that runs this stage.
    pub worker: String,
    /// The model hash this stage serves.
    pub model_hash: String,
    /// Which engine kind runs on that worker (informational for preference).
    pub engine: EngineKind,
    /// Memory budget this stage expects to reserve on the worker (MiB).
    pub est_ram_mb: u64,
    pub est_vram_mb: u64,
}

/// The shape of an execution plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlanKind {
    /// One worker runs the whole request end-to-end. The default, always
    /// producible, always executable.
    Single(ExecutionStage),
    /// Dependent stages run in order on different workers (e.g. prefill on a
    /// RAM-rich worker, decode on a latency-tuned worker). Only emitted when
    /// an engine advertises `prefill_decode_separation`.
    Sequential(Vec<ExecutionStage>),
    /// Independent copies of the same stage run concurrently on several
    /// workers (used for parallel/verifiable sub-requests, or speculative
    /// verification). Only emitted by callers that have such work.
    FanOut(Vec<ExecutionStage>),
}

impl PlanKind {
    /// All workers referenced by this plan.
    pub fn workers(&self) -> Vec<String> {
        match self {
            Self::Single(s) => vec![s.worker.clone()],
            Self::Sequential(ss) | Self::FanOut(ss) => {
                ss.iter().map(|s| s.worker.clone()).collect()
            }
        }
    }

    /// Number of execution stages.
    pub fn stage_count(&self) -> usize {
        match self {
            Self::Single(_) => 1,
            Self::Sequential(ss) | Self::FanOut(ss) => ss.len(),
        }
    }
}

/// A complete, attributable execution plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub plan_id: String,
    pub model_hash: String,
    pub kind: PlanKind,
    /// Where this plan's requests should fall through if a stage fails.
    pub fallback_orders: Vec<Vec<String>>,
}

impl ExecutionPlan {
    /// A trivial single-worker plan (the safest construction the planner can
    /// always produce).
    pub fn single(model_hash: &str, stage: ExecutionStage) -> Self {
        Self {
            plan_id: uuid::Uuid::new_v4().to_string(),
            model_hash: model_hash.to_string(),
            kind: PlanKind::Single(stage),
            fallback_orders: Vec::new(),
        }
    }

    pub fn workers(&self) -> Vec<String> {
        self.kind.workers()
    }

    pub fn stage_count(&self) -> usize {
        self.kind.stage_count()
    }

    /// Total RAM and VRAM the plan would reserve across all its stages.
    pub fn reservation_budget(&self) -> (u64, u64) {
        let stages: Vec<&ExecutionStage> = match &self.kind {
            PlanKind::Single(s) => vec![s],
            PlanKind::Sequential(ss) | PlanKind::FanOut(ss) => ss.iter().collect(),
        };
        stages
            .iter()
            .fold((0, 0), |(r, v), s| (r + s.est_ram_mb, v + s.est_vram_mb))
    }
}

/// Deterministic cost estimate of a plan (lower is better), used by the
/// planner to choose among otherwise-equal plans.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlanCost {
    /// Expected wall-clock in ms.
    pub estimated_ms: u32,
    /// Total MiB to move between nodes (prompt + model + KV).
    pub transfer_mib: u64,
    /// Number of cross-node hops (parallelizable stages count once).
    pub hops: u32,
}

impl PlanCost {
    pub fn total(&self) -> u32 {
        self.estimated_ms
    }
}

// ---------------------------------------------------------------------------
// Execution strategy (P1 — ExecutionStrategy foundation)
// ---------------------------------------------------------------------------

/// The *kind* of execution a logical request uses. `SingleWorker` and
/// `BatchFanOut` are the strategies the planner produces today; the rest are
/// gated experimental strategies from the research roadmap (see
/// `docs/research/EXECUTION-STRATEGY-ROADMAP.md`) and are **never** emitted by
/// the planner until an engine advertises the capability AND real measurements
/// prove net benefit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    /// One worker runs the entire request end-to-end (the default).
    SingleWorker,
    /// Multiple workers each run independent requests from a batch.
    BatchFanOut,
    /// Weak worker drafts, strong worker verifies (gated experimental).
    SpeculativeDraftVerify,
    /// One worker does prefill, another decode (gated experimental).
    DisaggregatedPrefillDecode,
    /// Route/migrate based on KV/cache state (gated experimental).
    CacheAwareRoute,
    /// Tensor/pipeline-parallel model execution across workers (gated
    /// experimental; llama.cpp RPC / vLLM TP/PP backends).
    CollaborativeModel,
}

impl StrategyKind {
    /// Whether the strategy is gated behind an experimental flag. The planner
    /// never selects experimental strategies without explicit opt-in + evidence.
    pub fn is_experimental(&self) -> bool {
        matches!(
            self,
            Self::SpeculativeDraftVerify
                | Self::DisaggregatedPrefillDecode
                | Self::CacheAwareRoute
                | Self::CollaborativeModel
        )
    }
}

/// Evidence provenance of an execution-strategy decision, following the
/// research roadmap rule: no fabricated measurements — missing data is
/// `UNKNOWN`, and experimental strategies are opt-in and clearly labelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceProvenance {
    /// Directly observed metrics (tokens/s, latency, RTT, throughput).
    Measured,
    /// Derived from conservative estimators (transfer cost, dry-run).
    Estimated,
    /// Logical conclusions from architecture and configuration.
    Inferred,
    /// Gated strategies under measurement.
    Experimental,
    /// Missing data — never fabricated.
    Unknown,
}

/// Why a strategy was selected over the alternatives (audit/observability).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyRationale {
    /// Human-readable explanation of the choice.
    pub reason: String,
    /// Strategies considered and rejected, with the rejection reason.
    pub rejected: Vec<RejectedStrategy>,
}

/// A strategy that was considered but not chosen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedStrategy {
    pub kind: StrategyKind,
    pub reason: String,
}

/// One concrete execution strategy for a request. The planner attaches this to
/// its plan so every decision carries *how* the request runs and *why*, with an
/// honest provenance flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionStrategy {
    pub kind: StrategyKind,
    pub rationale: StrategyRationale,
    pub provenance: EvidenceProvenance,
}

impl ExecutionStrategy {
    /// The default, always-producible strategy: one worker runs everything.
    /// The planner emits this unless it can justify something else.
    pub fn single_worker(reason: impl Into<String>) -> Self {
        Self {
            kind: StrategyKind::SingleWorker,
            rationale: StrategyRationale {
                reason: reason.into(),
                rejected: Vec::new(),
            },
            provenance: EvidenceProvenance::Inferred,
        }
    }

    /// Whether this strategy requires multi-worker collaboration.
    pub fn is_multi_worker(&self) -> bool {
        !matches!(self.kind, StrategyKind::SingleWorker)
    }
}

impl Default for ExecutionStrategy {
    /// Conservative default so persisted decisions without a strategy field
    /// (pre-P1) deserialize cleanly. Never claims anything beyond the baseline.
    fn default() -> Self {
        Self::single_worker("default (pre-P1 decision)")
    }
}

/// Whether a worker can run a request alone (`can_run`) and whether it can
/// safely participate in a multi-worker strategy (`can_collaborate`).
///
/// `can_run` reuses the existing eligibility projection (trusted + healthy +
/// serves the model). `can_collaborate` is deliberately conservative: it only
/// returns `true` for `BatchFanOut` today, because no engine DecentraAI runs
/// advertises speculative / disaggregated / collaborative-model capabilities —
/// claiming collaboration for a strategy the fabric cannot execute would be a
/// lie (see the P1 implementation note).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanRunReport {
    pub can_run: bool,
    pub can_collaborate: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(id: &str, w: &str) -> ExecutionStage {
        ExecutionStage {
            stage_id: id.to_string(),
            worker: w.to_string(),
            model_hash: "m1".to_string(),
            engine: EngineKind::LlamaServer,
            est_ram_mb: 512,
            est_vram_mb: 1024,
        }
    }

    #[test]
    fn single_plan_has_one_worker_and_budget() {
        let p = ExecutionPlan::single("m1", stage("s1", "w1"));
        assert_eq!(p.workers(), vec!["w1"]);
        assert_eq!(p.stage_count(), 1);
        let (r, v) = p.reservation_budget();
        assert_eq!(r, 512);
        assert_eq!(v, 1024);
    }

    #[test]
    fn sequential_plan_lists_ordered_workers() {
        let kind = PlanKind::Sequential(vec![stage("s1", "w1"), stage("s2", "w2")]);
        assert_eq!(kind.workers(), vec!["w1", "w2"]);
        assert_eq!(kind.stage_count(), 2);
    }

    #[test]
    fn plan_round_trips() {
        let p = ExecutionPlan::single("m1", stage("s1", "w1"));
        let j = serde_json::to_string(&p).unwrap();
        let back: ExecutionPlan = serde_json::from_str(&j).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn strategy_kinds_round_trip_and_experimental_flags() {
        for (kind, experimental) in [
            (StrategyKind::SingleWorker, false),
            (StrategyKind::BatchFanOut, false),
            (StrategyKind::SpeculativeDraftVerify, true),
            (StrategyKind::DisaggregatedPrefillDecode, true),
            (StrategyKind::CacheAwareRoute, true),
            (StrategyKind::CollaborativeModel, true),
        ] {
            assert_eq!(kind.is_experimental(), experimental, "{kind:?}");
            let json = serde_json::to_string(&kind).unwrap();
            let back: StrategyKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn execution_strategy_defaults_to_single_worker() {
        let s = ExecutionStrategy::single_worker("baseline");
        assert_eq!(s.kind, StrategyKind::SingleWorker);
        assert_eq!(s.provenance, EvidenceProvenance::Inferred);
        assert!(!s.is_multi_worker());
        assert_eq!(s.rationale.reason, "baseline");
        let json = serde_json::to_string(&s).unwrap();
        let back: ExecutionStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn provenance_serde_uses_screaming_snake_case() {
        assert_eq!(
            serde_json::to_string(&EvidenceProvenance::Measured).unwrap(),
            "\"MEASURED\""
        );
        assert_eq!(
            serde_json::to_string(&EvidenceProvenance::Unknown).unwrap(),
            "\"UNKNOWN\""
        );
    }

    #[test]
    fn can_run_report_round_trips() {
        let r = CanRunReport {
            can_run: true,
            can_collaborate: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: CanRunReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}
