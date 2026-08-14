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
    pub fn single(
        model_hash: &str,
        stage: ExecutionStage,
    ) -> Self {
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
        stages.iter().fold((0, 0), |(r, v), s| {
            (r + s.est_ram_mb, v + s.est_vram_mb)
        })
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
}