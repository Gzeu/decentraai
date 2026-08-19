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
    /// A sequence of stages, each potentially on a different worker/engine
    /// with its own model (e.g. OCR → summarize → generate). Each stage is
    /// itself an `ExecutionPlan` (usually SingleWorker or DataParallelReplica)
    /// and the strategy is the composition (gated experimental).
    MultiModelPipeline,
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
                | Self::MultiModelPipeline
        )
    }

    /// The M11 execution mode this strategy maps to (Model-Fabric Execution
    /// Spec §1.3). The planner must always produce strategies that can be
    /// expressed as valid `ExecutionPlan`s under these modes.
    pub fn execution_mode(&self) -> ExecutionMode {
        match self {
            Self::SingleWorker | Self::CacheAwareRoute => ExecutionMode::SingleWorker,
            Self::BatchFanOut => ExecutionMode::DataParallelReplica,
            Self::SpeculativeDraftVerify => ExecutionMode::Speculative,
            Self::DisaggregatedPrefillDecode => ExecutionMode::PrefillDecodeDisaggregated,
            Self::CollaborativeModel => ExecutionMode::TensorPipelineParallel,
            Self::MultiModelPipeline => ExecutionMode::MultiModelPipeline,
        }
    }

    /// Minimum engine capabilities required for this strategy (Model-Fabric
    /// Execution Spec §2.1). Only the flags that are *required* are set; the
    /// caller must AND with any worker-specific requirements (model fit, VRAM,
    /// network) before selecting a strategy. `continuous_batching` is
    /// preferred for BatchFanOut but not required, so it stays unset here.
    pub fn required_capabilities(&self) -> crate::engine::EngineCapabilities {
        use crate::engine::EngineCapabilities;
        match self {
            // Any engine can serve a whole request alone.
            Self::SingleWorker => EngineCapabilities::conservative(),
            Self::BatchFanOut => EngineCapabilities::conservative(),
            Self::SpeculativeDraftVerify => EngineCapabilities {
                speculative_decoding: true,
                ..EngineCapabilities::conservative()
            },
            Self::DisaggregatedPrefillDecode => EngineCapabilities {
                kv_offload: true,
                ..EngineCapabilities::conservative()
            },
            Self::CacheAwareRoute => EngineCapabilities {
                prefix_cache: true,
                ..EngineCapabilities::conservative()
            },
            Self::CollaborativeModel => EngineCapabilities {
                tensor_parallel: true,
                pipeline_parallel: true,
                ..EngineCapabilities::conservative()
            },
            // Composition of per-stage plans: every stage is checked
            // individually, so the composite imposes no extra requirement.
            Self::MultiModelPipeline => EngineCapabilities::conservative(),
        }
    }

    /// Whether an engine's advertised capabilities satisfy the minimum for
    /// this strategy (§2.1). This is a *necessary* condition, not sufficient:
    /// model fit, trust tier, and measured evidence are checked separately.
    pub fn meets_capabilities(&self, caps: &crate::engine::EngineCapabilities) -> bool {
        let required = self.required_capabilities();
        required.streaming <= caps.streaming
            && required.kv_report <= caps.kv_report
            && required.prefill_decode_separation <= caps.prefill_decode_separation
            && required.expert_routing <= caps.expert_routing
            && required.tensor_parallel <= caps.tensor_parallel
            && required.continuous_batching <= caps.continuous_batching
            && required.speculative_decoding <= caps.speculative_decoding
            && required.kv_offload <= caps.kv_offload
            && required.prefix_cache <= caps.prefix_cache
            && required.pipeline_parallel <= caps.pipeline_parallel
    }

    /// Whether the strategy is permitted within a trust tier (spec §4.2).
    /// The planner must filter candidate strategies by tier before scoring;
    /// KV/cache migration across tiers is disallowed.
    pub fn allowed_in(&self, tier: TrustTier) -> bool {
        match tier {
            TrustTier::Public => {
                matches!(self, Self::SingleWorker | Self::BatchFanOut)
            }
            TrustTier::TrustedRemote => {
                matches!(
                    self,
                    Self::SingleWorker | Self::BatchFanOut | Self::CacheAwareRoute
                )
            }
            TrustTier::TrustedCluster => {
                matches!(
                    self,
                    Self::SingleWorker
                        | Self::BatchFanOut
                        | Self::SpeculativeDraftVerify
                        | Self::DisaggregatedPrefillDecode
                        | Self::CacheAwareRoute
                        | Self::CollaborativeModel
                )
            }
        }
    }
}

/// The M11 execution mode at the fabric level (Model-Fabric Execution Spec §1.2).
/// A strategy kind maps to exactly one mode; the mode describes *how* the
/// request physically runs across the fabric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Entire request runs on a single worker.
    SingleWorker,
    /// Multiple replicas of the same model; each request runs on one replica.
    DataParallelReplica,
    /// Tensor + pipeline parallelism across multiple GPUs/workers.
    TensorPipelineParallel,
    /// Draft + verify models.
    Speculative,
    /// Prefill and decode split across engines.
    PrefillDecodeDisaggregated,
    /// A sequence of stages, each with its own execution mode (usually
    /// SingleWorker or DataParallelReplica per stage).
    MultiModelPipeline,
}

impl ExecutionMode {
    /// Whether the mode is permitted within a trust tier (spec §4.1).
    pub fn allowed_in(&self, tier: TrustTier) -> bool {
        match tier {
            TrustTier::Public => {
                matches!(self, Self::SingleWorker | Self::DataParallelReplica)
            }
            TrustTier::TrustedRemote => {
                matches!(
                    self,
                    Self::SingleWorker
                        | Self::DataParallelReplica
                        | Self::PrefillDecodeDisaggregated
                )
            }
            TrustTier::TrustedCluster => {
                matches!(
                    self,
                    Self::SingleWorker
                        | Self::DataParallelReplica
                        | Self::Speculative
                        | Self::PrefillDecodeDisaggregated
                        | Self::TensorPipelineParallel
                )
            }
        }
    }
}

/// Trust tiers for fabric collaboration (Model-Fabric Execution Spec §4).
///
/// The tier is derived from WorkerFacts, policy and configuration; the planner
/// filters candidate strategies and execution modes by tier before scoring.
/// KV/cache migration across tiers is disallowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    /// Public/heterogeneous peers: only complete replica execution.
    #[default]
    Public,
    /// Trusted same-region peers (known operators): replica execution and
    /// limited prefill/decode split.
    TrustedRemote,
    /// Trusted low-latency clusters: tensor/pipeline parallelism after
    /// benchmark verification.
    TrustedCluster,
}

impl TrustTier {
    /// Conservative rank for ordering: higher = more permissive.
    pub fn rank(self) -> u8 {
        match self {
            Self::Public => 0,
            Self::TrustedRemote => 1,
            Self::TrustedCluster => 2,
        }
    }
}

/// Per (worker, model, engine) measured performance profile
/// (Model-Fabric Execution Spec §2.2). Every field is optional: a missing
/// field MUST be treated as UNKNOWN and can never justify an experimental
/// strategy. The planner compares net benefit of an alternative strategy
/// against a SingleWorker baseline built from these numbers, and detects N+1
/// regressions where adding workers decreases performance.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PerformanceProfile {
    /// Time to first token (seconds).
    pub ttft_ms: Option<f64>,
    /// Inter-token latency (ms/token).
    pub inter_token_ms: Option<f64>,
    /// Tokens per second (decode throughput).
    pub tokens_per_sec: Option<f64>,
    /// Queue wait time (ms).
    pub queue_wait_ms: Option<f64>,
    /// Prompt processing time (ms).
    pub prompt_processing_ms: Option<f64>,
    /// Decode time per token (ms).
    pub decode_ms: Option<f64>,
    /// p50 latency (ms).
    pub p50_ms: Option<f64>,
    /// p95 latency (ms).
    pub p95_ms: Option<f64>,
    /// p99 latency (ms).
    pub p99_ms: Option<f64>,
    /// Error / timeout rate (0.0..=1.0).
    pub error_rate: Option<f64>,
    /// Prefix-cache hit rate (0.0..=1.0).
    pub prefix_cache_hit_rate: Option<f64>,
    /// GPU utilization (0.0..=1.0).
    pub gpu_utilization: Option<f64>,
    /// Memory pressure (0.0..=1.0).
    pub memory_pressure: Option<f64>,
    /// Optional energy/cost estimate per request (arbitrary unit).
    pub energy_cost: Option<f64>,
}

impl PerformanceProfile {
    /// Number of metrics actually measured. Used to decide whether evidence is
    /// strong enough to compare strategies; missing fields are UNKNOWN and can
    /// never justify an experimental strategy (§2.2).
    pub fn measured_count(&self) -> usize {
        [
            self.ttft_ms,
            self.inter_token_ms,
            self.tokens_per_sec,
            self.queue_wait_ms,
            self.prompt_processing_ms,
            self.decode_ms,
            self.p50_ms,
            self.p95_ms,
            self.p99_ms,
            self.error_rate,
            self.prefix_cache_hit_rate,
            self.gpu_utilization,
            self.memory_pressure,
            self.energy_cost,
        ]
        .iter()
        .filter(|m| m.is_some())
        .count()
    }

    /// Whether enough evidence exists to reason about strategy trade-offs.
    /// Conservative threshold: at least throughput+latency+error-rate, the
    /// three pillars the planner's net-benefit comparison needs.
    pub fn has_core_evidence(&self) -> bool {
        self.tokens_per_sec.is_some()
            && (self.ttft_ms.is_some() || self.p50_ms.is_some() || self.inter_token_ms.is_some())
            && self.error_rate.is_some()
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

/// Evidence a strategy must show before it may be promoted from EXPERIMENTAL
/// to BETA/PRODUCTION (Model-Fabric Execution Spec §6). Every criterion is a
/// hard gate: a strategy that cannot prove all of them stays experimental,
/// regardless of how attractive its expected speedup looks on paper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PromotionEvidence {
    /// Required capabilities are verified on the target engines and not stale
    /// (measurements are younger than their freshness window).
    pub capabilities_verified: bool,
    /// PerformanceProfile shows consistent net benefit vs the SingleWorker
    /// baseline across repeated runs (not one lucky measurement).
    pub net_benefit_proven: bool,
    /// Network and trust tiers are enforced (planner filters by tier before
    /// scoring; KV/cache migration across tiers is disallowed).
    pub tiers_enforced: bool,
    /// Threat model updated and security implications reviewed by a human.
    pub threat_model_reviewed: bool,
    /// Rollback/fallback paths to SingleWorker/DataParallelReplica are tested.
    pub rollback_tested: bool,
}

impl PromotionEvidence {
    /// Whether every hard gate passes. A single missing gate keeps the
    /// strategy EXPERIMENTAL — promotion is deliberately conservative.
    pub fn promotable(&self) -> bool {
        self.capabilities_verified
            && self.net_benefit_proven
            && self.tiers_enforced
            && self.threat_model_reviewed
            && self.rollback_tested
    }

    /// Human-readable breakdown of which gates are open (empty = promotable).
    pub fn unmet(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.capabilities_verified {
            out.push("capabilities not verified");
        }
        if !self.net_benefit_proven {
            out.push("net benefit not proven");
        }
        if !self.tiers_enforced {
            out.push("trust tiers not enforced");
        }
        if !self.threat_model_reviewed {
            out.push("threat model not reviewed");
        }
        if !self.rollback_tested {
            out.push("rollback not tested");
        }
        out
    }
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

    // ---- Model-Fabric Execution Spec §1.3: StrategyKind ↔ ExecutionMode ----

    #[test]
    fn strategy_to_execution_mode_mapping() {
        assert_eq!(
            StrategyKind::SingleWorker.execution_mode(),
            ExecutionMode::SingleWorker
        );
        assert_eq!(
            StrategyKind::BatchFanOut.execution_mode(),
            ExecutionMode::DataParallelReplica
        );
        assert_eq!(
            StrategyKind::SpeculativeDraftVerify.execution_mode(),
            ExecutionMode::Speculative
        );
        assert_eq!(
            StrategyKind::DisaggregatedPrefillDecode.execution_mode(),
            ExecutionMode::PrefillDecodeDisaggregated
        );
        assert_eq!(
            StrategyKind::CollaborativeModel.execution_mode(),
            ExecutionMode::TensorPipelineParallel
        );
        assert_eq!(
            StrategyKind::MultiModelPipeline.execution_mode(),
            ExecutionMode::MultiModelPipeline
        );
        // CacheAwareRoute is orthogonal to mode: it stays SingleWorker (or
        // DataParallelReplica) with cache-aware routing.
        assert_eq!(
            StrategyKind::CacheAwareRoute.execution_mode(),
            ExecutionMode::SingleWorker
        );
    }

    // ---- Model-Fabric Execution Spec §4: trust tiers ----

    #[test]
    fn public_tier_allows_only_replica_strategies() {
        for (kind, allowed) in [
            (StrategyKind::SingleWorker, true),
            (StrategyKind::BatchFanOut, true),
            (StrategyKind::SpeculativeDraftVerify, false),
            (StrategyKind::DisaggregatedPrefillDecode, false),
            (StrategyKind::CacheAwareRoute, false),
            (StrategyKind::CollaborativeModel, false),
            (StrategyKind::MultiModelPipeline, false),
        ] {
            assert_eq!(kind.allowed_in(TrustTier::Public), allowed, "{kind:?}");
        }
        assert!(ExecutionMode::SingleWorker.allowed_in(TrustTier::Public));
        assert!(ExecutionMode::DataParallelReplica.allowed_in(TrustTier::Public));
        assert!(!ExecutionMode::TensorPipelineParallel.allowed_in(TrustTier::Public));
        assert!(!ExecutionMode::Speculative.allowed_in(TrustTier::Public));
    }

    #[test]
    fn trusted_remote_adds_limited_cache_and_pd() {
        assert!(StrategyKind::CacheAwareRoute.allowed_in(TrustTier::TrustedRemote));
        assert!(ExecutionMode::PrefillDecodeDisaggregated.allowed_in(TrustTier::TrustedRemote));
        assert!(!StrategyKind::CollaborativeModel.allowed_in(TrustTier::TrustedRemote));
        assert!(!ExecutionMode::TensorPipelineParallel.allowed_in(TrustTier::TrustedRemote));
    }

    #[test]
    fn trusted_cluster_allows_advanced_strategies() {
        for kind in [
            StrategyKind::SingleWorker,
            StrategyKind::BatchFanOut,
            StrategyKind::SpeculativeDraftVerify,
            StrategyKind::DisaggregatedPrefillDecode,
            StrategyKind::CacheAwareRoute,
            StrategyKind::CollaborativeModel,
        ] {
            assert!(kind.allowed_in(TrustTier::TrustedCluster), "{kind:?}");
        }
        // MultiModelPipeline is a composition of per-stage plans; each stage
        // must be tier-checked individually (the composite itself is a
        // planning-time construct).
        assert!(ExecutionMode::TensorPipelineParallel.allowed_in(TrustTier::TrustedCluster));
        assert!(ExecutionMode::Speculative.allowed_in(TrustTier::TrustedCluster));
    }

    #[test]
    fn trust_tier_defaults_public_and_ranks() {
        assert_eq!(TrustTier::default(), TrustTier::Public);
        assert_eq!(TrustTier::Public.rank(), 0);
        assert_eq!(TrustTier::TrustedRemote.rank(), 1);
        assert_eq!(TrustTier::TrustedCluster.rank(), 2);
        // Serde round-trip with snake_case wire names.
        let json = serde_json::to_string(&TrustTier::TrustedRemote).unwrap();
        assert_eq!(json, "\"trusted_remote\"");
        let back: TrustTier = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TrustTier::TrustedRemote);
    }

    // ---- Model-Fabric Execution Spec §2.1: capability requirements ----

    #[test]
    fn single_worker_requires_nothing_extra() {
        use crate::engine::EngineCapabilities;
        // Even a conservative engine serves a whole request alone.
        assert!(StrategyKind::SingleWorker.meets_capabilities(&EngineCapabilities::conservative()));
        assert!(StrategyKind::BatchFanOut.meets_capabilities(&EngineCapabilities::conservative()));
        // But the experimental strategies require their advertised flags.
        assert!(
            !StrategyKind::SpeculativeDraftVerify
                .meets_capabilities(&EngineCapabilities::conservative())
        );
        assert!(
            !StrategyKind::CollaborativeModel
                .meets_capabilities(&EngineCapabilities::conservative())
        );
    }

    #[test]
    fn capability_requirements_map_to_spec_flags() {
        use crate::engine::EngineCapabilities;
        assert!(
            StrategyKind::SpeculativeDraftVerify.meets_capabilities(&EngineCapabilities {
                speculative_decoding: true,
                ..EngineCapabilities::conservative()
            })
        );
        assert!(
            StrategyKind::DisaggregatedPrefillDecode.meets_capabilities(&EngineCapabilities {
                kv_offload: true,
                ..EngineCapabilities::conservative()
            })
        );
        assert!(
            StrategyKind::CacheAwareRoute.meets_capabilities(&EngineCapabilities {
                prefix_cache: true,
                ..EngineCapabilities::conservative()
            })
        );
        assert!(
            StrategyKind::CollaborativeModel.meets_capabilities(&EngineCapabilities {
                tensor_parallel: true,
                pipeline_parallel: true,
                ..EngineCapabilities::conservative()
            })
        );
        // CollaborativeModel needs BOTH tensor and pipeline parallel.
        assert!(
            !StrategyKind::CollaborativeModel.meets_capabilities(&EngineCapabilities {
                tensor_parallel: true,
                ..EngineCapabilities::conservative()
            })
        );
    }

    #[test]
    fn vllm_advertised_capabilities_enable_advanced_strategies() {
        use crate::engine::EngineKind;
        let vllm = EngineKind::Vllm.advertised_capabilities();
        assert!(StrategyKind::SingleWorker.meets_capabilities(&vllm));
        assert!(StrategyKind::BatchFanOut.meets_capabilities(&vllm));
        assert!(StrategyKind::SpeculativeDraftVerify.meets_capabilities(&vllm));
        assert!(StrategyKind::DisaggregatedPrefillDecode.meets_capabilities(&vllm));
        assert!(StrategyKind::CacheAwareRoute.meets_capabilities(&vllm));
        assert!(StrategyKind::CollaborativeModel.meets_capabilities(&vllm));
        // llama-server cannot do collaborative/speculative — honest.
        let llama = EngineKind::LlamaServer.advertised_capabilities();
        assert!(StrategyKind::SingleWorker.meets_capabilities(&llama));
        assert!(!StrategyKind::CollaborativeModel.meets_capabilities(&llama));
        assert!(!StrategyKind::SpeculativeDraftVerify.meets_capabilities(&llama));
    }

    // ---- Model-Fabric Execution Spec §2.2: PerformanceProfile ----

    #[test]
    fn performance_profile_counts_measured_metrics() {
        let empty = PerformanceProfile::default();
        assert_eq!(empty.measured_count(), 0);
        assert!(!empty.has_core_evidence());

        let core = PerformanceProfile {
            ttft_ms: Some(120.0),
            tokens_per_sec: Some(18.0),
            error_rate: Some(0.01),
            ..Default::default()
        };
        assert_eq!(core.measured_count(), 3);
        assert!(core.has_core_evidence());

        // Missing error rate or throughput = UNKNOWN => no evidence.
        let partial = PerformanceProfile {
            ttft_ms: Some(120.0),
            tokens_per_sec: Some(18.0),
            ..Default::default()
        };
        assert!(!partial.has_core_evidence());
    }

    #[test]
    fn performance_profile_serde_defaults_missing_fields() {
        // Old/wire payloads with no metrics must deserialize to all-None.
        let json = r#"{"ttft_ms": 10.0}"#;
        let p: PerformanceProfile = serde_json::from_str(json).unwrap();
        assert_eq!(p.ttft_ms, Some(10.0));
        assert_eq!(p.inter_token_ms, None);
        assert_eq!(p.measured_count(), 1);
    }

    // ---- Model-Fabric Execution Spec §6: promotion gates ----

    #[test]
    fn promotion_requires_all_gates() {
        let empty = PromotionEvidence::default();
        assert!(!empty.promotable());
        assert_eq!(empty.unmet().len(), 5);

        let full = PromotionEvidence {
            capabilities_verified: true,
            net_benefit_proven: true,
            tiers_enforced: true,
            threat_model_reviewed: true,
            rollback_tested: true,
        };
        assert!(full.promotable());
        assert!(full.unmet().is_empty());
    }

    #[test]
    fn one_open_gate_blocks_promotion() {
        let almost = PromotionEvidence {
            capabilities_verified: true,
            net_benefit_proven: true,
            tiers_enforced: true,
            threat_model_reviewed: true,
            rollback_tested: false,
        };
        assert!(!almost.promotable());
        assert_eq!(almost.unmet(), vec!["rollback not tested"]);
    }
}
