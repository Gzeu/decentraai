//! Experimental distributed-execution planning (two-worker, RPC-gated).
//!
//! This helper builds a `DistributedVerdict` for `{desktop}` / `{laptop}` /
//! `{desktop,laptop}` without changing the existing `ExecutionPlanner` score
//! formula or the production routing path. It is purely observability and
//! experimental planning:
//!
//! - single-worker plans are built via the existing `ExecutionPlanner`;
//! - a two-node RPC candidate is constructed via `DistributedExecutionCandidate`;
//! - no request is ever routed through RPC automatically.
//!
//! The coordinator remains authoritative: it decides when to run the
//! `rpc-experiment.sh` harness and when to feed measured latencies back into
//! the `PerformanceEstimate`.

use crate::planner::{ExecutionPlanner, RequestFacts, WorkerFacts};
use crate::distributed::{
    DistributedExecutionCandidate, DistributedVerdict, EvidenceKind, NetworkCost,
};

/// Builds the experimental distributed-execution verdict for a two-worker
/// fabric (Desktop + Laptop). This is intended to be called by the
/// coordinator in an EXPERIMENTAL path; production routing continues to use
/// the `ExecutionPlanner::plan` result.
pub fn experimental_two_worker_verdict(
    planner: &ExecutionPlanner,
    req: &RequestFacts,
    desktop: &WorkerFacts,
    laptop: &WorkerFacts,
    network: NetworkCost,
) -> DistributedVerdict {
    // Single-worker baselines via the existing planner.
    let workers = vec![desktop.clone(), laptop.clone()];
    let plan_single = planner.plan(req, &workers);
    let best_single_score = plan_single.rationale.chosen.clone();

    // CAN_RUN per worker.
    let can_run_desktop = desktop.serves_model
        && desktop.available_ram_mb >= req.est_ram_mb
        && desktop.available_vram_mb >= req.est_vram_mb;
    let can_run_laptop = laptop.serves_model
        && laptop.available_ram_mb >= req.est_ram_mb
        && laptop.available_vram_mb >= req.est_vram_mb;

    // CAN_COLLABORATE (experimental, RPC-gated).
    let can_collab_desktop = desktop.capabilities.supports_rpc_layer_split;
    let can_collab_laptop = laptop.capabilities.supports_rpc_layer_split;
    let can_collaborate_desktop_laptop = can_collab_desktop && can_collab_laptop;

    // When no collaborative backend exists, we stop at single-worker.
    if !can_collaborate_desktop_laptop {
        return DistributedVerdict {
            can_run_desktop,
            can_run_laptop,
            can_collaborate_desktop_laptop,
            best_single_worker_score: best_single_score,
            distributed_candidate: None,
            expected_benefit_percent: None,
            note: "no collaborative backend (RPC layer split) available; single-worker only".into(),
        };
    }

    // Construct the experimental two-node candidate. It has UNKNOWN
    // performance until the RPC harness produces real measurements.
    let mut candidate = DistributedExecutionCandidate::two_node_layer_split(
        req,
        desktop,
        laptop,
        network,
    );

    // Compare against the best single-worker score. Without measured two-node
    // performance, benefit remains UNKNOWN.
    let (expected_benefit_percent, note) = match (&best_single_score, candidate.performance.evidence) {
        (Some(_), EvidenceKind::Unknown) => (
            None,
            "distributed candidate is EXPERIMENTAL; no measured two-node benchmark yet".into(),
        ),
        (Some(single), EvidenceKind::Measured) => {
            // When the coordinator later feeds measured prefill/decode stats
            // into `candidate.performance`, it can also populate a derived
            // benefit. Here we only compare decode tokens/sec if present.
            if let Some(decode_tps) = candidate.performance.decode_tps {
                let single_tps = single.tps.max(0.001); // avoid division by zero
                let delta = ((decode_tps as f32 / single_tps) - 1.0) * 100.0;
                (Some(delta), "benefit derived from real two-node RPC benchmark".into())
            } else {
                (None, "candidate has measured provenance but missing decode_tps; benefit UNKNOWN".into())
            }
        }
        _ => (
            None,
            "distributed candidate has no calibrated performance; benefit UNKNOWN".into(),
        ),
    };

    DistributedVerdict {
        can_run_desktop,
        can_run_laptop,
        can_collaborate_desktop_laptop,
        best_single_worker_score: best_single_score,
        distributed_candidate: Some(candidate),
        expected_benefit_percent,
        note,
    }
}
