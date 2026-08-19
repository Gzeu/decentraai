//! DecentraAI execution fabric — the pure, engine-aware decision core for
//! distributed inference (M18–M23).
//!
//! DecentraAI is a decentralized AI infrastructure layer, not a marketplace,
//! cloud, or a wrapper around one inference server. This crate holds the
//! orchestration intelligence that runs every request end-to-end without the
//! user reasoning about topology. It is deliberately **pure** (no I/O, no
//! async) so:
//!
//! - every decision is deterministic and unit-testable with synthetic inputs,
//! - every type is serde-serializable so plans and fabric state can travel or
//!   persist,
//! - the production coordinator and a unit test exercise the exact same code.
//!
//! # The fabric model
//!
//! | Concern | Milestone | Module |
//! |---|---|---|
//! | Engine kind + capability ABI | M22 | [`engine`] |
//! | Execution plan (single / staged / fan-out) | M18 | [`plan`] |
//! | Network graph + transfer cost | M19 | [`network`] |
//! | KV-cache-aware routing | M20 | [`kv`] |
//! | Expert / distributed-MoE fabric | M21 | [`expert`] |
//! | Autonomous execution planner | M23 | [`planner`] |
//!
//! The planner ([`planner::ExecutionPlanner`]) consumes all of the above and
//! produces an [`plan::ExecutionPlan`]. Each module degrades gracefully when
//! an engine cannot express a capability (no expert routing → single worker,
//! no KV report → context-length-only routing), so the fabric never fabricates
//! behavior the runtime cannot actually provide.

pub mod advisory;
pub mod batch;
pub mod decision;
pub mod engine;
pub mod expert;
pub mod kv;
pub mod network;
pub mod plan;
pub mod planner;

pub use advisory::{
    FanOutAdvisory, ReplanDecision, fan_out_candidacy, rebalance_advisory, replan_decision,
};
pub use batch::{BatchAllocation, BatchAssignment, allocate_batch, set_kv_affinity};
pub use decision::{
    Adaptation, CandidateOutcome, ConstraintKind, ConstraintResult, ExecutionDecision,
    ExecutionEvent, ExecutionPhase, Observation, OrchestrationAction, WorkloadClass, adapt,
    classify, evaluate, observe, orchestrate, recovery_timeline,
};
pub use engine::{EngineCapabilities, EngineKind};
pub use expert::{ExpertDecision, ExpertLayout, ExpertRegistry, ExpertRouter, ExpertShard};
pub use kv::{ContextProfile, KVCacheState, KvPlanner, KvRoutingHint};
pub use network::{
    LinkMetrics, Locality, NetworkGraph, estimated_transfer_ms, prior_bandwidth_mbps,
    transfer_ms_per_mib,
};
pub use plan::{
    CanRunReport, EvidenceProvenance, ExecutionMode, ExecutionPlan, ExecutionStage,
    ExecutionStrategy, PerformanceProfile, PlanCost, PlanKind, RejectedStrategy, StrategyKind,
    StrategyRationale, TrustTier,
};
pub use planner::{
    CandidateScore, ExecutionPlanner, NormalizedMetrics, PlanResult, PlannerConfig,
    PlannerRationale, RequestFacts, ScoringWeights, WorkerFacts, base_score,
};
