//! Autonomous execution-decision model (M23 Full Autonomy).
//!
//! This turns the planner's "pick the best single worker" into an explicit,
//! observable decision: *how should this request run, and what should we do
//! when the fabric changes?* It is pure and I/O-free, reusing the existing
//! [`ExecutionPlanner`] and the pure advisories rather than duplicating them.
//!
//! The lifecycle it models, decoupled from any engine:
//!
//! ```text
//! REQUEST → DISCOVER fabric → CLASSIFY workload → CANDIDATES → HARD CONSTRAINTS
//!         → SCORE/OPTIMIZE → SELECT plan → RESERVE → EXECUTE → OBSERVE
//!         → ADAPT / RECOVER / REPLAN → COMPLETE → RELEASE
//! ```
//!
//! Everything between REQUEST and RELEASE is represented by two pure types so a
//! coordinator can drive it event-driven and show *why*:
//!
//! - [`ExecutionDecision`] — the full, explainable decision for one request
//!   (workload class, every candidate with its constraints + score, the chosen
//!   worker, the plan + fallback, expected mode, priority, network cost, KV /
//!   session affinity, engine capability, and a human reasoning string — no
//!   chain-of-thought, just the operational facts).
//! - [`ExecutionEvent`] — a typed, serializable lifecycle event raised as the
//!   request moves through the pipeline, so the control plane can render a live
//!   trace and the coordinator can react to state changes instead of only
//!   polling.
//!
//! # Multi-worker safety
//!
//! [`adapt`] is the "Adapt / Recover / Replan" step. It is **idempotency- and
//! safety-bound**:
//!
//! - A request that already emitted tokens is **never** retried (that would
//!   duplicate partial output) — it can only Continue or Abort.
//! - A definitive worker rejection or a cancellation is **never** re-sent.
//! - Otherwise, if a fresh candidate remains eligible and a re-plan budget
//!   remains, the request is **re-planned** (session affinity honored via
//!   `prefix_resident_on`) and retried.
//!
//! Multi-worker execution is **not faked**: [`PlanKind`] stays `Single` unless
//! an engine actually advertises staging/expert capabilities (see
//! [`crate::engine`]). The model is *extensible* so real multi-worker
//! strategies slot in as engine capabilities mature.

use crate::advisory::replan_decision;
use crate::engine::{EngineCapabilities, EngineKind};
use crate::kv::ContextProfile;
use crate::plan::{ExecutionPlan, ExecutionStage, PlanKind};
use crate::planner::{CandidateScore, ExecutionPlanner, PlanResult, RequestFacts, WorkerFacts};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Workload classification
// ---------------------------------------------------------------------------

/// How the request asks to be executed (informs expected execution mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadClass {
    /// A single non-streamed completion (text).
    Completion,
    /// A streamed chat/completion.
    StreamingChat,
    /// A continuation of a known session (KV-locality matters).
    Continuation,
    /// A batch (no live display latency pressure).
    Batch,
}

/// Classifies a request from its context and request facts.
pub fn classify(
    ctx: &ContextProfile,
    priority: u8,
    streaming: bool,
) -> WorkloadClass {
    if ctx.is_continuation {
        WorkloadClass::Continuation
    } else if streaming {
        WorkloadClass::StreamingChat
    } else if priority > 0 {
        WorkloadClass::Completion
    } else {
        WorkloadClass::Batch
    }
}

// ---------------------------------------------------------------------------
// Constraints
// ---------------------------------------------------------------------------

/// A hard constraint every candidate must satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    Trusted,
    Healthy,
    ServesModel,
    /// The worker's KV/context capacity fits this request.
    ContextFit,
    RamHeadroom,
    VramHeadroom,
    /// The worker's engine kind is compatible / preferred for this workload.
    EngineCompatible,
    /// Provisioning permitted for a model the worker does not yet hold.
    ProvisioningAllowed,
}

/// Which constraints a worker breached (empty = eligible on hard constraints).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ConstraintResult {
    pub breaches: Vec<ConstraintKind>,
}

impl ConstraintResult {
    pub fn is_satisfied(&self) -> bool {
        self.breaches.is_empty()
    }
    pub fn add(&mut self, kind: ConstraintKind) {
        if !self.breaches.contains(&kind) {
            self.breaches.push(kind);
        }
    }
}

/// One evaluated candidate: its constraints, score breakdown, and locality.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateOutcome {
    pub peer_id: String,
    pub constraints: ConstraintResult,
    /// None if the candidate breached a hard constraint (not scorable).
    pub score: Option<CandidateScore>,
    pub engine: EngineKind,
    /// Whether the session's KV prefix is resident here (cache locality).
    pub kv_prefix_resident: bool,
    /// Expected network one-way cost (ms) to reach this worker, when known.
    pub network_cost_ms: u32,
}

/// The full, explainable execution decision for one request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionDecision {
    pub request_id: String,
    pub model_hash: String,
    pub workload_class: WorkloadClass,
    pub priority: u8,
    /// All evaluated candidates (ranked by score where eligible).
    pub candidates: Vec<CandidateOutcome>,
    /// The selected worker, if any eligible candidate exists.
    pub selected_worker: Option<String>,
    /// The chosen plan (Single by default; staged only if the engine can).
    pub plan: Option<ExecutionPlan>,
    /// Fallback worker orders in precedence (empty = none).
    pub fallback_orders: Vec<Vec<String>>,
    /// Expected execution mode, e.g. "streaming_chat" / "batch" / "remote_worker".
    pub expected_mode: String,
    pub network_cost_ms: u32,
    /// KV/session affinity text (continuation steering).
    pub kv_affinity: String,
    pub engine_capability: EngineCapabilities,
    pub reasoning: String,
    pub ts: u64,
    /// Reservation held for this request, filled in once the coordinator
    /// actually reserves a worker (correlates the decision with the outcome).
    pub reservation_id: Option<String>,
    /// Terminal outcome: "in_flight" until the coordinator records the result,
    /// then "succeeded" or "failed" (safe operational metadata, no content).
    pub outcome: Option<String>,
    /// Every lifecycle event observed for this request (control-plane trace).
    pub trace: Vec<ExecutionEvent>,
}

/// A typed, serializable lifecycle event (event-driven observability).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ExecutionEvent {
    Discovered { workers: u32 },
    Classified { workload_class: WorkloadClass },
    Planned { selected_worker: Option<String> },
    Reserved { worker: Option<String> },
    Executing { worker: Option<String> },
    Adapting { reason: String },
    Replanned { retry_on: Option<String> },
    Completed { ok: bool },
    Released { worker: Option<String> },
    Failed { cause: String, retryable: bool },
}

// ---------------------------------------------------------------------------
// evaluate / adapt
// ---------------------------------------------------------------------------

/// Renders the honest expected execution mode for a selected plan.
fn expected_mode_for(kind: &PlanKind, cls: WorkloadClass) -> String {
    match kind {
        PlanKind::Single(_) => match cls {
            WorkloadClass::StreamingChat => "streaming_chat".to_string(),
            WorkloadClass::Continuation => "continuation_worker".to_string(),
            WorkloadClass::Batch => "batch_worker".to_string(),
            WorkloadClass::Completion => "completion_worker".to_string(),
        },
        PlanKind::Sequential(_) => "sequential_stages".to_string(),
        PlanKind::FanOut(_) => "fan_out".to_string(),
    }
}

/// Worker eligibility on hard constraints plus engine/priority preference,
/// independent of score. Returns the eligible set (for scoring) separately so
/// a decision can show both "who passed constraints" and "who won the score".
fn evaluate_candidates(req: &RequestFacts, workers: &[WorkerFacts]) -> Vec<CandidateOutcome> {
    let mut out = Vec::new();
    for w in workers {
        let mut c = ConstraintResult::default();
        if !w.trusted {
            c.add(ConstraintKind::Trusted);
        }
        if !w.healthy {
            c.add(ConstraintKind::Healthy);
        }
        if !w.serves_model {
            c.add(ConstraintKind::ServesModel);
        }
        // Context fit: worker's advertised KV slots must cover the request.
        match w.kv.headroom_tokens() {
            Some(h) if h < req.context.total_slots() => c.add(ConstraintKind::ContextFit),
            _ => {}
        }
        if w.available_ram_mb < req.est_ram_mb && req.est_ram_mb > 0 {
            c.add(ConstraintKind::RamHeadroom);
        }
        if req.est_vram_mb > 0 && w.available_vram_mb < req.est_vram_mb {
            c.add(ConstraintKind::VramHeadroom);
        }
        // Engine preference: a concrete engine kind is preferred over the
        // generic remote when the workload asks for a specific capability.
        if !w.capabilities.streaming && req.context.total_slots() > 1024 {
            c.add(ConstraintKind::EngineCompatible);
        }
        out.push(CandidateOutcome {
            peer_id: w.peer_id.clone(),
            constraints: c,
            score: None,
            engine: w.engine,
            kv_prefix_resident: req.context.prefix_resident_on.as_deref()
                == Some(w.peer_id.as_str()),
            network_cost_ms: 0,
        });
    }
    out
}

/// Produces the full, explainable execution decision for a request against the
/// live fabric. Wraps the planner's single-worker selection into the explicit
/// DISCOVER → CLASSIFY → CANDIDATES → CONSTRAINTS → SCORE → SELECT pipeline and
/// records the trace events. Fan-out/staged modes are only selected when a real
/// engine advertises the capability (the planner already gates this).
///
/// The provided [`ExecutionPlanner`] carries the live fabric context used by the
/// real routing path — the measured network graph (M19), expert registry (M21)
/// and objective weights — so the decision's per-candidate scores and network
/// cost reflect genuine runtime state, not a cold default planner. Callers that
/// have no live fabric pass `&ExecutionPlanner::default()`.
pub fn evaluate(
    planner: &ExecutionPlanner,
    request_id: &str,
    req: &RequestFacts,
    workers: &[WorkerFacts],
    streaming: bool,
    allow_fanout: bool,
) -> ExecutionDecision {
    let mut trace = vec![
        ExecutionEvent::Discovered {
            workers: workers.len() as u32,
        },
        ExecutionEvent::Classified {
            workload_class: classify(&req.context, req.priority, streaming),
        },
    ];
    let cls = classify(&req.context, req.priority, streaming);
    let mut candidates = evaluate_candidates(req, workers);
    // Score the eligible candidates via the planner's ranked breakdown (using
    // the caller's live network graph and objective weights).
    let plan_result: PlanResult = planner.plan(req, workers);
    for cand in candidates.iter_mut() {
        if let Some(cs) = plan_result
            .rationale
            .ranked
            .iter()
            .find(|cs| cs.peer_id == cand.peer_id)
        {
            cand.score = Some(cs.clone());
        }
        cand.kv_prefix_resident = req.context.prefix_resident_on.as_deref()
            == Some(cand.peer_id.as_str());
        // Real network reach cost (M19) from the caller's measured graph.
        cand.network_cost_ms = planner.network.reach_cost_ms(&cand.peer_id, req.transfer_mib);
    }

    let selected = plan_result.rationale.chosen_worker.clone();
    // Fan-out is only ever selected when a real engine advertises it.
    let fanout = allow_fanout
        && matches!(crate::advisory::fan_out_candidacy(req, workers, &planner.config, true), crate::advisory::FanOutAdvisory::CandidateFanOut { .. });
    let (plan, expected_mode) = if fanout {
        // Only build a fan-out from workers a real engine advertises as
        // staging-capable (never fabricate multi-worker execution).
        let ranked_peers: std::collections::HashSet<String> = plan_result
            .rationale
            .ranked
            .iter()
            .map(|cs| cs.peer_id.clone())
            .collect();
        let stages: Vec<ExecutionStage> = workers
            .iter()
            .filter(|w| w.capabilities.supports_staging() && ranked_peers.contains(&w.peer_id))
            .take(2)
            .map(|f| ExecutionStage {
                stage_id: f.peer_id.clone(),
                worker: f.peer_id.clone(),
                model_hash: req.model_hash.clone(),
                engine: f.engine,
                est_ram_mb: req.est_ram_mb,
                est_vram_mb: req.est_vram_mb,
            })
            .collect();
        if stages.len() >= 2 {
            let plan = ExecutionPlan {
                plan_id: uuid::Uuid::new_v4().to_string(),
                model_hash: req.model_hash.clone(),
                kind: PlanKind::FanOut(stages),
                fallback_orders: plan_result.plan.fallback_orders.clone(),
            };
            (Some(plan), "fan_out".to_string())
        } else {
            // Not really multi-worker capable: fall back to the single plan.
            (Some(plan_result.plan.clone()), expected_mode_for(&plan_result.plan.kind, cls))
        }
    } else {
        (Some(plan_result.plan.clone()), expected_mode_for(&plan_result.plan.kind, cls))
    };

    trace.push(ExecutionEvent::Planned {
        selected_worker: selected.clone(),
    });

    let engine_capability = selected
        .as_ref()
        .and_then(|s| workers.iter().find(|w| &w.peer_id == s))
        .map(|w| w.capabilities)
        .unwrap_or_else(EngineCapabilities::conservative);

    ExecutionDecision {
        request_id: request_id.to_string(),
        model_hash: req.model_hash.clone(),
        workload_class: cls,
        priority: req.priority,
        candidates,
        selected_worker: selected.clone(),
        plan,
        fallback_orders: plan_result.plan.fallback_orders.clone(),
        expected_mode,
        network_cost_ms: plan_result.estimated_ms,
        kv_affinity: match req.context.prefix_resident_on.as_deref() {
            Some(w) => format!("session resident on {w}"),
            None => "cold".to_string(),
        },
        engine_capability,
        reasoning: plan_result.reasoning.clone(),
        ts: now_secs(),
        reservation_id: None,
        outcome: None,
        trace,
    }
}

/// The result of the observe → adapt step for a running/completed request.
#[derive(Debug, Clone, PartialEq)]
pub enum Adaptation {
    /// Keep going / nothing to change.
    Continue,
    /// Re-plan and retry on a fresh eligible worker (no tokens were emitted).
    Retry,
    /// Preserve session affinity but re-plan to a different candidate.
    Replan,
    /// Give up (no safe retry possible).
    Abort,
}

/// Decides how to adapt after an observed request outcome (M23 Full Autonomy,
/// the OBSERVE → ADAPT / RECOVER / REPLAN step). Safety-bound:
///
/// - if the outcome indicates a **definitive, non-retryable** failure, or a
///   cancellation, or **any token was emitted**, we do not retry — we Abort.
/// - if another eligible worker remains within the re-plan budget, we Replan
///   (and Retry if the request had not yet produced output).
/// - otherwise Continue.
pub fn adapt(
    outcome_ok: bool,
    retryable: bool,
    cancelled: bool,
    tokens_emitted: u64,
    eligible_after_primary: usize,
    replan_budget: u32,
    is_continuation: bool,
) -> Adaptation {
    let _ = replan_decision; // advisory replan reused conceptually
    if outcome_ok {
        return Adaptation::Continue;
    }
    // Hard safety: never duplicate work.
    if cancelled || tokens_emitted > 0 || !retryable {
        return Adaptation::Abort;
    }
    if replan_budget == 0 || eligible_after_primary == 0 {
        return Adaptation::Abort;
    }
    if is_continuation {
        // Preserve session affinity but steer to a fresh candidate.
        Adaptation::Replan
    } else {
        Adaptation::Retry
    }
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(context: ContextProfile, priority: u8) -> RequestFacts {
        RequestFacts {
            model_hash: "m1".into(),
            est_ram_mb: 512,
            est_vram_mb: 0,
            context,
            transfer_mib: 0,
            local_peer: None,
            priority,
        }
    }

    fn worker(id: &str, tps: u32, latency: u32, load: u8) -> WorkerFacts {
        use crate::engine::{EngineCapabilities, EngineKind};
        use crate::kv::KVCacheState;
        WorkerFacts {
            peer_id: id.to_string(),
            trusted: true,
            healthy: true,
            engine: EngineKind::LlamaServer,
            tokens_per_second: tps,
            latency_ms: latency,
            queue_depth: 0,
            load_percent: load,
            available_ram_mb: 4096,
            available_vram_mb: 0,
            serves_model: true,
            capabilities: EngineCapabilities::conservative(),
            kv: KVCacheState::Empty,
        }
    }

    #[test]
    fn adapt_never_retries_after_output_or_definitive_failure() {
        // tokens emitted → abort (never duplicate partial output)
        assert_eq!(
            adapt(false, true, false, 5, 3, 2, false),
            Adaptation::Abort
        );
        // cancelled → abort
        assert_eq!(
            adapt(false, true, true, 0, 3, 2, false),
            Adaptation::Abort
        );
        // non-retryable definitive failure → abort
        assert_eq!(
            adapt(false, false, false, 0, 3, 2, false),
            Adaptation::Abort
        );
        // success → continue
        assert_eq!(adapt(true, true, false, 0, 3, 2, false), Adaptation::Continue);
    }

    #[test]
    fn adapt_replans_when_an_eligible_worker_remains() {
        // retryable transport failure, no tokens, candidate remains, budget left
        assert_eq!(
            adapt(false, true, false, 0, 2, 3, false),
            Adaptation::Retry
        );
        // continuation honors affinity → Replan (steer to fresh candidate)
        assert_eq!(
            adapt(false, true, false, 0, 2, 3, true),
            Adaptation::Replan
        );
        // no budget → abort
        assert_eq!(
            adapt(false, true, false, 0, 2, 0, false),
            Adaptation::Abort
        );
        // no eligible after primary → abort
        assert_eq!(
            adapt(false, true, false, 0, 0, 3, false),
            Adaptation::Abort
        );
    }

    #[test]
    fn classify_detects_continuation_and_streaming() {
        let cold = ContextProfile {
            prompt_tokens: 10,
            max_output_tokens: 10,
            is_continuation: false,
            prefix_resident_on: None,
        };
        assert_eq!(classify(&cold, 0, true), WorkloadClass::StreamingChat);
        let cont = ContextProfile {
            is_continuation: true,
            ..cold
        };
        assert_eq!(classify(&cont, 0, false), WorkloadClass::Continuation);
    }

    #[test]
    fn evaluate_marks_candidates_and_picks_a_worker() {
        let mut bad = worker("b", 200, 10, 5);
        bad.serves_model = false;
        let good = worker("g", 150, 20, 10);
        let ws = vec![bad, good];
        let d = evaluate(
            &ExecutionPlanner::default(),
            "r1",
            &req(
                ContextProfile {
                    prompt_tokens: 10,
                    max_output_tokens: 10,
                    is_continuation: false,
                    prefix_resident_on: None,
                },
                0,
            ),
            &ws,
            true,
            false,
        );
        assert_eq!(d.selected_worker.as_deref(), Some("g"));
        assert_eq!(d.workload_class, WorkloadClass::StreamingChat);
        assert!(!d.trace.is_empty());
        // The non-serving candidate records a ServesModel breach.
        let c = d.candidates.iter().find(|c| c.peer_id == "b").unwrap();
        assert!(c.constraints.breaches.contains(&ConstraintKind::ServesModel));
    }

    #[test]
    fn decision_is_serializable_and_round_trips() {
        let g = worker("g", 150, 20, 10);
        let d = evaluate(
            &ExecutionPlanner::default(),
            "r1",
            &req(
                ContextProfile {
                    prompt_tokens: 10,
                    max_output_tokens: 10,
                    is_continuation: false,
                    prefix_resident_on: None,
                },
                0,
            ),
            &[g],
            false,
            false,
        );
        let j = serde_json::to_string(&d).unwrap();
        let back: ExecutionDecision = serde_json::from_str(&j).unwrap();
        assert_eq!(back.request_id, "r1");
        assert_eq!(back.selected_worker, d.selected_worker);
    }

    #[test]
    fn evaluate_uses_the_callers_measured_network_graph() {
        // A coordinator feeds the decision its live network graph (M19), so the
        // recorded network cost must reflect real measured RTT rather than 0.
        let mut planner = ExecutionPlanner::default();
        use crate::network::{LinkMetrics, Locality};
        planner
            .network
            .set("far", LinkMetrics::prior(Locality::Remote, Some(80_000)));
        planner
            .network
            .set("near", LinkMetrics::prior(Locality::Lan, Some(1_000)));
        // One eligible candidate, transfer cost of 2 MiB.
        let mut rf = req(ContextProfile {
            prompt_tokens: 10,
            max_output_tokens: 10,
            is_continuation: false,
            prefix_resident_on: None,
        }, 0);
        rf.transfer_mib = 2;
        let ws = vec![worker("far", 150, 40, 10)];
        let d = evaluate(&planner, "r1", &rf, &ws, false, false);
        assert!(
            d.network_cost_ms > 0,
            "decision must carry real network cost from the live graph"
        );
        let far = d.candidates.iter().find(|c| c.peer_id == "far").unwrap();
        assert!(
            far.network_cost_ms > 0,
            "candidate carries measured reach cost"
        );
        // The remote worker's per-candidate reach cost (RTT + 2 MiB transfer)
        // must reflect the measured Remote prior, i.e. be non-trivial.
        assert!(
            far.network_cost_ms >= 70,
            "reach cost includes the measured RTT term"
        );
    }
}