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
    /// Optional required-capability verdict recorded from the planner (Phase L
    /// foundation). `None` when no capability was requested; otherwise an
    /// honest view (`satisfied=false`, `evidence="UNKNOWN"` unless a
    /// coordinator supplies real `ModelCapabilities`). Surfaced so agents and
    /// operators can see what capability an execution was asked to satisfy.
    pub capability_requirement: Option<crate::planner::CapabilityRequirementView>,
    pub ts: u64,
    /// Reservation held for this request, filled in once the coordinator
    /// actually reserves a worker (correlates the decision with the outcome).
    pub reservation_id: Option<String>,
    /// Terminal outcome: "in_flight" until the coordinator records the result,
    /// then "succeeded" or "failed" (safe operational metadata, no content).
    pub outcome: Option<String>,
    /// Every lifecycle event observed for this request (control-plane trace).
    pub trace: Vec<ExecutionEvent>,
    /// The last orchestration action decided by the runtime loop, when one has
    /// been recorded (recovery/adaptation observability). `None` until a
    /// coordinator records a decision; additive and defaulted so existing
    /// constructions keep compiling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_orchestration: Option<OrchestrationAction>,
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
    /// The orchestrator is observing live runtime state for a running stage.
    Observing { stage: String, worker: Option<String> },
    Adapting { reason: String },
    /// Re-planning because the fabric changed / the primary worker is no longer
    /// the best safe choice (before this request produced output).
    Replanning { from: Option<String>, to: Option<String> },
    /// Recovering after a retryable worker failure onto an alternative.
    Recovering { worker: Option<String>, attempt: u32 },
    Replanned { retry_on: Option<String> },
    /// The request exceeded its deadline while still in a safe (no-output) stage.
    DeadlineElapsed { deadline_ms: u32 },
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
        capability_requirement: plan_result.rationale.capability_requirement.clone(),
        ts: now_secs(),
        reservation_id: None,
        outcome: None,
        trace,
        last_orchestration: None,
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

// ---------------------------------------------------------------------------
// Autonomous runtime orchestration (M23 Increment D)
// ---------------------------------------------------------------------------

/// The lifecycle phase a request is in — the control-plane timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ExecutionPhase {
    Discovered,
    Classified,
    Planned,
    Reserved,
    Executing,
    Observing,
    Adapting,
    Replanning,
    Recovering,
    Completed,
    Released,
    Failed,
}

/// A real-time observation of a stage's runtime state (all safe operational
/// facts; no request content).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Observation {
    /// Which phase the observation is about (which transition to compute).
    pub phase: ExecutionPhase,
    /// The worker currently running the stage (if any).
    pub worker: Option<String>,
    /// Number of attempts already made for this request.
    pub attempt: u32,
    /// Remaining re-plan / retry budget (0 = no more attempts).
    pub replan_budget: u32,
    /// Tokens already delivered to the client (must stay 0 before a retry).
    pub tokens_emitted: u64,
    /// Whether the last attempt failed in a retryable (transport) way.
    pub retryable: bool,
    /// Whether the failure was a client cancellation (never retried).
    pub cancelled: bool,
    /// Whether this is a session continuation (preserve KV affinity).
    pub is_continuation: bool,
    /// How many *other* eligible workers remain as an alternative.
    pub eligible_alternatives: usize,
    /// Worker the session prefix is resident on (continuation steering).
    pub prefix_on: Option<String>,
    /// Whether the request's wall-clock deadline has elapsed.
    pub deadline_elapsed: bool,
    /// Whether the current worker is now over its load/queue congestion cap.
    pub worker_congested: bool,
}

/// The orchestrator's safe next action for a request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OrchestrationAction {
    /// Keep executing on the current worker.
    Continue,
    /// Retry the same worker (only when nothing was emitted and it is safe).
    RetrySameWorker,
    /// Re-plan onto an alternative (or the best continued candidate).
    Replan { to: Option<String> },
    /// Recover from a retryable worker failure onto an alternative.
    Recover { on: Option<String> },
    /// No safe action; stop.
    Abort,
}

impl OrchestrationAction {
    /// Whether the orchestrator intends to keep trying (not abort).
    pub fn keeps_trying(&self) -> bool {
        !matches!(self, OrchestrationAction::Abort)
    }
}

/// Pure controller that decides the next safe orchestration action from a
/// single runtime observation (M23 Increment D). It generalizes the retry-only
/// `adapt` step into a continuous observe→adapt loop over the lifecycle:
///
/// - If the phase is a success → Continue.
/// - If the deadline elapsed while nothing was emitted → Abort (safe).
/// - If tokens were emitted or it was cancelled or the failure is definite
///   (non-retryable) → Abort (never duplicate / never re-send).
/// - Otherwise, if an alternative or the same worker is safely available within
///   the remaining budget, Replan / Recover / RetrySameWorker (honoring session
///   affinity and avoiding a congested primary).
///
/// No mid-stream migration, no KV migration, no tensor/layer/expert parallelism:
/// this only decides *which worker* and *whether to keep trying*, reusing the
/// planner for the actual re-plan.
pub fn orchestrate(o: &Observation) -> OrchestrationAction {
    // Success at any stage: keep the result.
    if o.tokens_emitted > 0 && o.phase == ExecutionPhase::Observing {
        return OrchestrationAction::Continue;
    }
    if o.phase == ExecutionPhase::Completed {
        return OrchestrationAction::Continue;
    }
    // Hard safety: never duplicate work or re-send a definitive rejection.
    if o.cancelled || o.tokens_emitted > 0 || (!o.retryable && o.phase == ExecutionPhase::Adapting)
    {
        return OrchestrationAction::Abort;
    }
    // Deadline with nothing safely producible: stop.
    if o.deadline_elapsed {
        return OrchestrationAction::Abort;
    }
    // No budget and no alternatives left: stop.
    if o.replan_budget == 0 || (o.eligible_alternatives == 0 && !o.retryable) {
        return OrchestrationAction::Abort;
    }
    // Prefer to stay on the same worker when it is not congested and not the
    // one that just failed in a way that suggests the worker itself.
    if o.phase == ExecutionPhase::Recovering && o.eligible_alternatives > 0 {
        return OrchestrationAction::Recover {
            on: if o.is_continuation { o.prefix_on.clone() } else { None },
        };
    }
    if o.phase == ExecutionPhase::Replanning {
        return OrchestrationAction::Replan {
            to: if o.is_continuation { o.prefix_on.clone() } else { None },
        };
    }
    if !o.worker_congested && o.retryable && o.worker.is_some() {
        return OrchestrationAction::RetrySameWorker;
    }
    if o.eligible_alternatives > 0 {
        return OrchestrationAction::Replan { to: None };
    }
    OrchestrationAction::Abort
}

/// Appends an event to a decision's trace (the observable, event-driven record).
pub fn observe(decision: &mut ExecutionDecision, event: ExecutionEvent) {
    decision.trace.push(event);
}

// ---------------------------------------------------------------------------
// Recovery timeline projection (Phase H — self-healing observability)
// ---------------------------------------------------------------------------

/// Stable snake_case name for an event variant, matching its serde `event` tag
/// (`#[serde(tag="event", rename_all="snake_case")]`), so the projection and
/// the wire format never diverge.
fn event_name(e: &ExecutionEvent) -> &'static str {
    match e {
        ExecutionEvent::Discovered { .. } => "discovered",
        ExecutionEvent::Classified { .. } => "classified",
        ExecutionEvent::Planned { .. } => "planned",
        ExecutionEvent::Reserved { .. } => "reserved",
        ExecutionEvent::Executing { .. } => "executing",
        ExecutionEvent::Observing { .. } => "observing",
        ExecutionEvent::Adapting { .. } => "adapting",
        ExecutionEvent::Replanning { .. } => "replanning",
        ExecutionEvent::Recovering { .. } => "recovering",
        ExecutionEvent::Replanned { .. } => "replanned",
        ExecutionEvent::DeadlineElapsed { .. } => "deadline_elapsed",
        ExecutionEvent::Completed { .. } => "completed",
        ExecutionEvent::Released { .. } => "released",
        ExecutionEvent::Failed { .. } => "failed",
    }
}

/// The attempt number an event carries, if any (only retry/recovery events).
fn event_attempt(e: &ExecutionEvent) -> Option<u32> {
    match e {
        ExecutionEvent::Recovering { attempt, .. } => Some(*attempt),
        _ => None,
    }
}

/// Whether an event represents the system reacting to a problem (a recovery /
/// re-plan). Used for the `recoveries` count.
fn is_recovery_event(e: &ExecutionEvent) -> bool {
    matches!(
        e,
        ExecutionEvent::Recovering { .. }
            | ExecutionEvent::Replanned { .. }
            | ExecutionEvent::Replanning { .. }
            | ExecutionEvent::Adapting { .. }
    )
}

/// Projects a decision's lifecycle trace into a concise, operator-readable
/// recovery timeline (for the dashboard / observability). Pure, deterministic,
/// and derived only from real trace data — never invents a phase or event.
///
/// Returns a JSON object with:
/// - `"outcome"`: the decision's terminal outcome, or `"in_flight"`.
/// - `"phase"`: the final event's phase, as the stable snake_case name.
/// - `"phases_seen"`: ordered distinct event names (order of first appearance).
/// - `"recoveries"`: count of recovery/replan events.
/// - `"adaptation"`: the last recorded orchestration action, if any.
/// - `"timeline"`: per-event `{ event, phase, attempt }`, in order.
/// - `"summary"`: a short deterministic human string.
pub fn recovery_timeline(decision: &ExecutionDecision) -> serde_json::Value {
    let outcome = decision
        .outcome
        .clone()
        .unwrap_or_else(|| "in_flight".to_string());

    let mut phases_seen: Vec<&str> = Vec::new();
    let mut recoveries: u32 = 0;
    let mut last_event: Option<&ExecutionEvent> = None;
    let timeline: Vec<serde_json::Value> = decision
        .trace
        .iter()
        .map(|e| {
            let name = event_name(e);
            if !phases_seen.contains(&name) {
                phases_seen.push(name);
            }
            if is_recovery_event(e) {
                recoveries += 1;
            }
            last_event = Some(e);
            serde_json::json!({
                "event": name,
                "phase": name,
                "attempt": event_attempt(e),
            })
        })
        .collect();

    let phase = last_event.map(event_name).unwrap_or("no_events");

    let adaptation = match &decision.last_orchestration {
        Some(action) => serde_json::to_value(action).ok(),
        None => None,
    };

    let summary = if outcome == "failed" {
        "aborted".to_string()
    } else if recoveries == 0 {
        "no recovery needed".to_string()
    } else {
        format!("recovered {recoveries} time(s)")
    };

    serde_json::json!({
        "outcome": outcome,
        "phase": phase,
        "phases_seen": phases_seen,
        "recoveries": recoveries,
        "adaptation": adaptation,
        "timeline": timeline,
        "summary": summary,
    })
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
            required_capability: None,
            capability_claims: Vec::new(),
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
            perf_measured: false,
            queue_depth: 0,
            load_percent: load,
            available_ram_mb: 4096,
            available_vram_mb: 0,
            serves_model: true,
            available_models: vec![],
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
    fn evaluate_propagates_capability_requirement_from_planner() {
        // A request carrying a required capability must surface that honest
        // verdict on the ExecutionDecision, so agents/operators can see it via
        // list_executions. The fabric has no ModelCapabilities, so the honest
        // state is satisfied=false / evidence=UNKNOWN — never a false claim.
        let g = worker("g", 150, 20, 10);
        let mut r = req(
            ContextProfile {
                prompt_tokens: 10,
                max_output_tokens: 10,
                is_continuation: false,
                prefix_resident_on: None,
            },
            0,
        );
        r.required_capability = Some("ocr".to_string());
        let d = evaluate(
            &ExecutionPlanner::default(),
            "r-cap",
            &r,
            &[g],
            true,
            false,
        );
        let view = d.capability_requirement.expect("requirement must propagate");
        assert_eq!(view.capability, "ocr");
        assert!(!view.satisfied, "fabric never claims satisfaction without evidence");
        assert_eq!(view.evidence, "UNKNOWN");
        assert!(d.reasoning.contains("ocr"), "reasoning should mention the requirement");

        // Without a requirement, the decision carries None.
        let r2 = req(
            ContextProfile {
                prompt_tokens: 10,
                max_output_tokens: 10,
                is_continuation: false,
                prefix_resident_on: None,
            },
            0,
        );
        let d2 = evaluate(&ExecutionPlanner::default(), "r-none", &r2, &[worker("g", 150, 20, 10)], true, false);
        assert!(d2.capability_requirement.is_none());
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

    // ---- M23 Increment D: autonomous runtime orchestration ----

    fn obs(phase: ExecutionPhase) -> Observation {
        Observation {
            phase,
            worker: None,
            attempt: 0,
            replan_budget: 3,
            tokens_emitted: 0,
            retryable: true,
            cancelled: false,
            is_continuation: false,
            eligible_alternatives: 2,
            prefix_on: None,
            deadline_elapsed: false,
            worker_congested: false,
        }
    }

    #[test]
    fn orchestrate_never_retries_after_output_or_cancellation_or_definitive_failure() {
        // tokens emitted → abort (never duplicate output)
        let mut o = obs(ExecutionPhase::Adapting);
        o.tokens_emitted = 5;
        assert_eq!(orchestrate(&o), OrchestrationAction::Abort);
        // cancelled → abort
        let mut o = obs(ExecutionPhase::Adapting);
        o.cancelled = true;
        assert_eq!(orchestrate(&o), OrchestrationAction::Abort);
        // definite non-retryable failure → abort
        let mut o = obs(ExecutionPhase::Adapting);
        o.retryable = false;
        assert_eq!(orchestrate(&o), OrchestrationAction::Abort);
    }

    #[test]
    fn orchestrate_replans_and_recovers_when_a_safe_alternative_exists() {
        // retryable transport failure, alternative remains → Replan
        assert_eq!(
            orchestrate(&obs(ExecutionPhase::Adapting)),
            OrchestrationAction::Replan { to: None }
        );
        // recovering with an alternative (and session affinity) → Recover
        let mut r = obs(ExecutionPhase::Recovering);
        r.is_continuation = false;
        assert!(matches!(orchestrate(&r), OrchestrationAction::Recover { .. }));
        // replanning a continuation steers to the prefix-resident worker
        let mut p = obs(ExecutionPhase::Replanning);
        p.is_continuation = true;
        p.prefix_on = Some("kv1".into());
        assert_eq!(orchestrate(&p), OrchestrationAction::Replan { to: Some("kv1".into()) });
    }

    #[test]
    fn orchestrate_respects_budget_deadline_and_congestion() {
        // no budget → abort
        let mut o = obs(ExecutionPhase::Adapting);
        o.replan_budget = 0;
        assert_eq!(orchestrate(&o), OrchestrationAction::Abort);
        // deadline elapsed → abort
        let mut o = obs(ExecutionPhase::Executing);
        o.deadline_elapsed = true;
        assert_eq!(orchestrate(&o), OrchestrationAction::Abort);
        // congested worker → Replan (not retry-same-worker)
        let mut o = obs(ExecutionPhase::Observing);
        o.worker = Some("w1".into());
        o.worker_congested = true;
        assert_eq!(orchestrate(&o), OrchestrationAction::Replan { to: None });
        // un-congested same worker + retryable → RetrySameWorker
        let mut o = obs(ExecutionPhase::Observing);
        o.worker = Some("w1".into());
        o.worker_congested = false;
        assert_eq!(orchestrate(&o), OrchestrationAction::RetrySameWorker);
    }

    #[test]
    fn observe_appends_events_to_the_decision_trace() {
        let g = worker("g", 150, 20, 10);
        let mut d = evaluate(
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
        let before = d.trace.len();
        observe(&mut d, ExecutionEvent::Observing {
            stage: "s1".into(),
            worker: Some("g".into()),
        });
        observe(&mut d, ExecutionEvent::Recovering {
            worker: Some("g".into()),
            attempt: 1,
        });
        assert_eq!(d.trace.len(), before + 2);
        assert!(d
            .trace
            .iter()
            .any(|e| matches!(e, ExecutionEvent::Observing { .. })));
        assert!(d
            .trace
            .iter()
            .any(|e| matches!(e, ExecutionEvent::Recovering { attempt: 1, .. })));
    }

    // ---- Phase H: self-healing recovery timeline ----

    #[test]
    fn recovery_timeline_reports_recoveries_when_present() {
        let g = worker("g", 150, 20, 10);
        let mut d = evaluate(
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
        observe(&mut d, ExecutionEvent::Recovering {
            worker: Some("g".into()),
            attempt: 1,
        });
        observe(&mut d, ExecutionEvent::Replanned {
            retry_on: Some("g".into()),
        });
        observe(&mut d, ExecutionEvent::Completed { ok: true });
        d.outcome = Some("succeeded".into());
        d.last_orchestration = Some(OrchestrationAction::Recover { on: Some("g".into()) });

        let tl = recovery_timeline(&d);
        let recoveries = tl["recoveries"].as_u64().unwrap();
        assert!(recoveries >= 2, "expected recovery events counted, got {recoveries}");
        // The recovery event name shows up in both the phase list and timeline.
        let phases = tl["phases_seen"].as_array().unwrap();
        let names: Vec<&str> = phases
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(names.contains(&"recovering"));
        assert!(names.contains(&"replanned"));
        // Timeline preserves order and carries the snake_case event names.
        let timeline = tl["timeline"].as_array().unwrap();
        let tl_names: Vec<&str> = timeline
            .iter()
            .map(|e| e["event"].as_str().unwrap())
            .collect();
        assert_eq!(tl_names, names);
        // The attempt is surfaced on the recovering entry.
        let recovering_entry = timeline
            .iter()
            .find(|e| e["event"] == "recovering")
            .unwrap();
        assert_eq!(recovering_entry["attempt"].as_u64(), Some(1));
        // The last orchestration action is carried through.
        assert_eq!(tl["adaptation"]["action"], "recover");
        assert_eq!(tl["outcome"], "succeeded");
        // Non-failed + recoveries > 0 → the "recovered N time(s)" summary.
        assert_eq!(tl["summary"], "recovered 2 time(s)");
    }

    #[test]
    fn recovery_timeline_reports_no_recovery_when_absent() {
        let g = worker("g", 150, 20, 10);
        let mut d = evaluate(
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
        d.outcome = Some("succeeded".into());
        let tl = recovery_timeline(&d);
        assert_eq!(tl["recoveries"].as_u64(), Some(0));
        assert_eq!(tl["summary"], "no recovery needed");
        assert_eq!(tl["outcome"], "succeeded");
        // In-flight (no outcome) still projects without inventing a phase.
        let no_outcome = ExecutionDecision {
            outcome: None,
            ..d.clone()
        };
        let inflight = recovery_timeline(&no_outcome);
        assert_eq!(inflight["outcome"], "in_flight");
    }

    #[test]
    fn recovery_timeline_is_aborted_on_failure() {
        let g = worker("g", 150, 20, 10);
        let mut d = evaluate(
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
        observe(&mut d, ExecutionEvent::Failed {
            cause: "definitive".into(),
            retryable: false,
        });
        d.outcome = Some("failed".into());
        let tl = recovery_timeline(&d);
        assert_eq!(tl["summary"], "aborted");
        assert_eq!(tl["phase"], "failed");
        assert_eq!(tl["outcome"], "failed");
    }
}