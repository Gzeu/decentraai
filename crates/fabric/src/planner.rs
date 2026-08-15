//! Autonomous execution planner (M23).
//!
//! The planner turns a request + the current fabric state into an
//! [`ExecutionPlan`], making *all* the orchestration decisions the user should
//! never need to think about:
//!
//! - which workers serve the model (and are trusted/healthy),
//! - whether to execute locally or on a remote worker,
//! - whether a single worker suffices or a multi-stage plan is justified,
//! - how network cost (M19) and KV state (M20) weigh in,
//! - expert-split candidacy where the engine advertises it (M21),
//! - the deterministic ranking and the fallback order.
//!
//! It is pure and I/O-free: the coordinator passes it the live registry,
//! the link graph, and the request; it returns the best plan. Tests drive it
//! with synthetic fabric; production feeds it real measurements.

use crate::engine::{EngineCapabilities, EngineKind};
use crate::expert::{ExpertRegistry, ExpertRouter};
use crate::kv::{ContextProfile, KVCacheState, KvPlanner};
use crate::network::{LinkMetrics, NetworkGraph};
use crate::plan::{ExecutionPlan, ExecutionStage, PlanKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A candidate worker the planner can place stages on.
#[derive(Debug, Clone)]
pub struct WorkerFacts {
    pub peer_id: String,
    pub trusted: bool,
    pub healthy: bool,
    pub engine: EngineKind,
    /// Derived nominal execution throughput (tokens/sec) for scoring.
    pub tokens_per_second: u32,
    /// Nominal single-token latency (ms).
    pub latency_ms: u32,
    /// Whether `tokens_per_second`/`latency_ms` reflect REAL measured
    /// completions (at least one verified completion fed the EWMA) or are
    /// estimated/unknown (never measured, e.g. a nominal default or 0).
    /// Purely additive provenance — it never affects scoring. The coordinator
    /// sets it honestly from the advertisement's advertised perf.
    pub perf_measured: bool,
    pub queue_depth: u32,
    pub load_percent: u8,
    pub available_ram_mb: u64,
    pub available_vram_mb: u64,
    pub serves_model: bool,
    /// Models this worker has on disk (registry), not currently loaded.
    /// Lets the coordinator discover what a worker COULD serve (it swaps its
    /// engine on request), distinct from `serves_model` which only tells what
    /// is loaded right now.
    pub available_models: Vec<decentraai_compute::ServedModel>,
    /// Engine-reported capabilities (probed; defaults conservative).
    pub capabilities: EngineCapabilities,
    /// KV state for the model on this worker.
    pub kv: KVCacheState,
}

/// The request facts the planner plans for.
#[derive(Debug, Clone)]
pub struct RequestFacts {
    pub model_hash: String,
    pub est_ram_mb: u64,
    pub est_vram_mb: u64,
    /// Full context budget this worker must accommodate (model ctx size).
    pub context: ContextProfile,
    /// Prompt + any model that must move to the worker (MiB), for network cost.
    pub transfer_mib: u64,
    /// Whether local execution exists (i.e. this node can self-run).
    pub local_peer: Option<String>,
    /// Request priority 0..=255 (higher = more urgent). Drives priority-aware
    /// scoring: urgent work favors the lowest-latency, least-queued worker.
    pub priority: u8,
    /// Optional required capability (snake_case name, e.g. `"ocr"`), the link
    /// between "agent intent → capabilities → model → fabric plan". The planner
    /// is engine-neutral and holds NO capability data, so it does not verify
    /// this here — when present it records an honest UNKNOWN verdict in the
    /// rationale and a reasoning note. `None` means no capability requirement.
    /// Additive/optional so existing constructions default to no requirement.
    pub required_capability: Option<String>,
    /// Persisted capability claims for the requested model, as `(capability
    /// snake_case, provenance "verified"|"inferred")`. Supplied by a coordinator
    /// that has real data (e.g. the local registry projection). When present
    /// AND `required_capability` is set, the planner resolves a real
    /// evidence-backed verdict instead of UNKNOWN. Empty = no data (honest
    /// UNKNOWN). Additive; defaults empty.
    pub capability_claims: Vec<(String, String)>,
}

/// Configurable objective weights for the planner's [`ExecutionPlanner::score`].
///
/// The `Default` weights reproduce the previous hard-coded constants, so this
/// exposes tuning ("prioritize throughput") without changing current behavior.
/// Weights are not required to sum to 1; they only rank candidates within one
/// request, so any non-negative combination is a legal objective.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlannerConfig {
    pub w_tps: f64,
    pub w_latency: f64,
    pub w_load: f64,
    pub w_queue: f64,
    pub w_headroom: f64,
    pub w_net: f64,
    pub w_kv: f64,
    /// Weight for *cache locality*: steering a continuation back to the worker
    /// that already holds the session's KV prefix, avoiding a cold prefill.
    pub w_locality: f64,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            w_tps: 0.25,
            w_latency: 0.15,
            w_load: 0.15,
            w_queue: 0.10,
            w_headroom: 0.15,
            w_net: 0.10,
            w_kv: 0.10,
            w_locality: 0.15,
        }
    }
}

/// The per-candidate component scores that made up a planner score. Kept pure
/// and serde-serializable so a coordinator can persist / display *why* the
/// chosen worker won without re-running the score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateScore {
    pub peer_id: String,
    pub total: f32,
    pub tps: f32,
    pub latency: f32,
    pub load: f32,
    pub queue: f32,
    pub headroom: f32,
    pub net: f32,
    pub kv: f32,
    pub locality: f32,
    /// Perf provenance of the chosen worker: `true` = its
    /// `tokens_per_second`/`latency_ms` reflect real measured completions,
    /// `false` = estimated/unknown. Mirrors `WorkerFacts::perf_measured`.
    /// Additive — it is NOT part of the score formula and never affects `total`.
    pub perf_measured: bool,
}

/// Honest verdict for a capability requirement carried on a request. The fabric
/// has no capability data (engine-neutral), so it can only record that a
/// requirement was requested and that it is UNVERIFIED — never that it is
/// satisfied. A coordinator that holds real `ModelCapabilities` may overwrite
/// this with an evidence-backed verdict; until then `satisfied` stays `false`
/// with `evidence = "UNKNOWN"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRequirementView {
    /// Snake_case capability name, e.g. `"ocr"`.
    pub capability: String,
    /// Short human label for the capability. Derived from the name here (the
    /// fabric has no capability taxonomy, so it cannot prettify beyond that).
    pub label: String,
    /// Whether the requirement is satisfied. Only ever `true` with real
    /// evidence; the fabric leaves it `false`.
    pub satisfied: bool,
    /// Evidence provenance: `"VERIFIED"`, `"INFERRED"`, `"UNKNOWN"`, or
    /// `"MISSING"`. `"UNKNOWN"` is the honest state when no data exists.
    pub evidence: String,
}

/// Observability of a planning decision: the chosen worker's component scores
/// and the margin over the runner-up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannerRationale {
    /// The worker selected, if any were eligible.
    pub chosen_worker: Option<String>,
    /// Component scores of the chosen worker.
    pub chosen: Option<CandidateScore>,
    /// `chosen.total - runner_up.total`; `None` when fewer than 2 candidates.
    pub runner_up_delta: Option<f32>,
    /// All eligible candidates ranked (score desc, PeerId asc).
    pub ranked: Vec<CandidateScore>,
    /// Capability-requirement verdict for this request, when one was requested.
    /// `None` when the request carried no capability requirement. Honest: never
    /// claims satisfied without real evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_requirement: Option<CapabilityRequirementView>,
}

/// The planner's materialized decision plus the plan it chose.
#[derive(Debug, Clone)]
pub struct PlanResult {
    pub plan: ExecutionPlan,
    /// Why this plan over alternatives (for audit/observability).
    pub reasoning: String,
    pub estimated_ms: u32,
    /// Per-candidate score breakdown behind the decision.
    pub rationale: PlannerRationale,
}

impl ExecutionPlan {
    /// Score a plan's estimated cost from a worker's nominal perf.
    fn cost_estimate(stages: &[&ExecutionStage], workers: &BTreeMap<String, WorkerFacts>) -> u32 {
        let mut total = 0u32;
        for s in stages {
            let w = workers.get(&s.worker);
            let latency = w.map(|f| f.latency_ms).unwrap_or(100);
            let tps = w.map(|f| f.tokens_per_second).unwrap_or(50);
            // Rough expected ms: latency + per-token generation. Prompt-only
            // factor is ignored here (KV-aware separation is a future split).
            total += latency + 100_000 / tps.max(1);
        }
        total
    }
}

/// The autonomous execution planner (M23).
pub struct ExecutionPlanner {
    pub network: NetworkGraph,
    pub experts: ExpertRegistry,
    pub allow_multi_stage: bool,
    /// Objective weights driving [`ExecutionPlanner::score`].
    pub config: PlannerConfig,
}

impl Default for ExecutionPlanner {
    fn default() -> Self {
        Self {
            network: NetworkGraph::new(),
            experts: ExpertRegistry::new(),
            allow_multi_stage: true,
            config: PlannerConfig::default(),
        }
    }
}

impl ExecutionPlanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds the best execution plan for a request over the given fabric.
    pub fn plan(
        &self,
        req: &RequestFacts,
        workers: &[WorkerFacts],
    ) -> PlanResult {
        // Deterministic candidate order: eligibility first, then score.
        let by_id: BTreeMap<String, WorkerFacts> = workers
            .iter()
            .map(|f| (f.peer_id.clone(), f.clone()))
            .collect();

        let eligible: Vec<&WorkerFacts> = workers
            .iter()
            .filter(|f| f.trusted && f.healthy && f.serves_model)
            .collect();

        // KV-aware hint narrows the field for continuation / long context.
        let kv_hint = KvPlanner.route(
            &req.context,
            &eligible
                .iter()
                .map(|f| (f.peer_id.clone(), true, f.kv))
                .collect::<Vec<_>>(),
            eligible
                .iter()
                .any(|f| f.capabilities.prefill_decode_separation),
        );

        // Score eligible workers: perf + load + network reach cost (+ KV
        // locality for continuations whose prefix is resident on a worker).
        let mut ranked: Vec<(CandidateScore, &WorkerFacts)> = eligible
            .iter()
            .map(|f| {
                let cs = self.candidate_score(
                    f,
                    req,
                    kv_hint.prefer_kv_headroom,
                    kv_hint.cache_locality_worker.as_deref(),
                    &self.config,
                );
                (cs, *f)
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.0.total
                .partial_cmp(&a.0.total)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.peer_id.cmp(&b.0.peer_id)) // PeerId asc tie-break
        });

        let Some((_, best)) = ranked.first() else {
            let rationale = PlannerRationale {
                chosen_worker: None,
                chosen: None,
                runner_up_delta: None,
                ranked: Vec::new(),
                capability_requirement: capability_view(req, &req.capability_claims),
            };
            return PlanResult {
                plan: ExecutionPlan {
                    plan_id: uuid::Uuid::new_v4().to_string(),
                    model_hash: req.model_hash.clone(),
                    kind: PlanKind::Single(ExecutionStage {
                        stage_id: "none".into(),
                        worker: String::new(),
                        model_hash: req.model_hash.clone(),
                        engine: EngineKind::RemoteOpenAI,
                        est_ram_mb: req.est_ram_mb,
                        est_vram_mb: req.est_vram_mb,
                    }),
                    fallback_orders: Vec::new(),
                },
                reasoning: append_capability_note(
                    "no eligible worker serves this model",
                    req,
                ),
                estimated_ms: 0,
                rationale,
            };
        };

        let fallback_orders = self.fallback_orders(&ranked);
        let stage = self.build_stage(best, req);
        let plan = ExecutionPlan {
            plan_id: uuid::Uuid::new_v4().to_string(),
            model_hash: req.model_hash.clone(),
            kind: PlanKind::Single(stage.0.clone()),
            fallback_orders,
        };
        let est = ExecutionPlan::cost_estimate(&[&stage.0], &by_id);
        let rationale = PlannerRationale {
            chosen_worker: Some(best.peer_id.clone()),
            chosen: Some(ranked[0].0.clone()),
            runner_up_delta: if ranked.len() >= 2 {
                Some(ranked[0].0.total - ranked[1].0.total)
            } else {
                None
            },
            ranked: ranked.iter().map(|(cs, _)| cs.clone()).collect(),
            capability_requirement: capability_view(req, &req.capability_claims),
        };
        PlanResult {
            reasoning: append_capability_note(&stage.1, req),
            estimated_ms: est,
            plan,
            rationale,
        }
    }

    fn build_stage(
        &self,
        f: &WorkerFacts,
        req: &RequestFacts,
    ) -> (ExecutionStage, String) {
        let stage = ExecutionStage {
            stage_id: format!("s1-{id}", id = f.peer_id),
            worker: f.peer_id.clone(),
            model_hash: req.model_hash.clone(),
            engine: f.engine,
            est_ram_mb: req.est_ram_mb,
            est_vram_mb: req.est_vram_mb,
        };
        let mut reasons = format!(
            "single-stage on {} ({} tps, {}ms latency, queue {}): load {:.0}%",
            f.peer_id, f.tokens_per_second, f.latency_ms, f.queue_depth, f.load_percent,
        );
        if f.capabilities.expert_routing {
            // Expert-aware decision.
            if let crate::expert::ExpertDecision::ExpertSplit { workers, .. } =
                ExpertRouter.route(&req.model_hash, &self.experts, vec![f.peer_id.clone()])
            {
                reasons.push_str(&format!("; expert split across {workers:?}"));
            }
        }
        (stage, reasons)
    }

    /// Computes a worker's score and its per-component breakdown with the
    /// given objective weights. Pure and deterministic — the single place the
    /// score formula lives (used by both the ranker and the rationale).
    fn candidate_score(
        &self,
        f: &WorkerFacts,
        req: &RequestFacts,
        prefer_kv_headroom: bool,
        cache_locality_worker: Option<&str>,
        cfg: &PlannerConfig,
    ) -> CandidateScore {
        let tps_score = (f.tokens_per_second as f32 / 200.0).clamp(0.0, 1.0);
        let latency_score = 1.0 - (f.latency_ms as f32 / 1000.0).clamp(0.0, 1.0);
        let load_score = 1.0 - (f.load_percent as f32 / 100.0);
        let queue_score = (1.0 - f.queue_depth as f32 / 10.0).clamp(0.0, 1.0);
        // Priority-aware: urgent requests (priority > 0) amplify the value of a
        // fast, unqueued worker, biasing selection toward the best available
        // compute. At priority 0 the factor is exactly 1.0, so default behavior
        // is unchanged.
        let priority_on = (f32::from(req.priority) / 255.0).clamp(0.0, 1.0);
        let priority_boost = 1.0 + 0.5 * priority_on;
        let headroom = if req.est_ram_mb > 0 {
            (f.available_ram_mb as f64 / req.est_ram_mb as f64).min(1.0) as f32
        } else {
            1.0
        };
        // Network: prefer workers that are cheaper to reach.
        let link = self.network.get(&f.peer_id);
        let net_score = self.network_score(&link);
        // KV: boost workers with headroom when the request is KV-hungry.
        let kv_score = if prefer_kv_headroom {
            match f.kv.headroom_tokens() {
                Some(h) if h >= req.context.total_slots() => 0.2,
                _ => 0.0,
            }
        } else {
            0.0
        };
        // KV locality (M20): for a continuation whose prefix is resident on a
        // specific worker, steer back to that worker — a cold prefill on
        // another worker would re-ingest the whole prefix. Only the prefix
        // host scores here; every other worker gets 0.
        let locality_score = match cache_locality_worker {
            Some(host) if host == f.peer_id => 1.0,
            _ => 0.0,
        };

        let total = (cfg.w_tps * tps_score as f64
            + cfg.w_latency * (latency_score * priority_boost) as f64
            + cfg.w_load * load_score as f64
            + cfg.w_queue * (queue_score * priority_boost) as f64
            + cfg.w_headroom * headroom as f64
            + cfg.w_net * net_score as f64
            + cfg.w_kv * kv_score as f64
            + cfg.w_locality * locality_score as f64) as f32;

        CandidateScore {
            peer_id: f.peer_id.clone(),
            total,
            tps: tps_score,
            latency: latency_score,
            load: load_score,
            queue: queue_score,
            headroom,
            net: net_score,
            kv: kv_score,
            locality: locality_score,
            perf_measured: f.perf_measured,
        }
    }

    fn network_score(&self, link: &LinkMetrics) -> f32 {
        let rtt_ms = link.rtt_us / 1000;
        let rtt_score = (1.0 - (rtt_ms as f32 / 200.0)).clamp(0.0, 1.0);
        rtt_score * 0.7 + (if link.bandwidth_mbps >= 100 { 0.3 } else { 0.1 })
    }

    /// Builds deterministic fallback worker orders (ranked, minus already used).
    fn fallback_orders(&self, ranked: &[(CandidateScore, &WorkerFacts)]) -> Vec<Vec<String>> {
        let mut orders = Vec::new();
        if ranked.len() > 1 {
            let mut rest: Vec<String> = ranked.iter().map(|(cs, _)| cs.peer_id.clone()).collect();
            for _ in 0..(ranked.len().saturating_sub(1)) {
                orders.push(rest.clone());
                if !rest.is_empty() {
                    rest.remove(0);
                }
            }
        }
        orders
    }
}

/// Resolves a capability requirement against a caller-supplied set of real
/// capability claims. Pure and I/O-free so a coordinator that holds the
/// taxonomy (e.g. a projection of the hub taxonomy persisted in the local
/// registry) can produce an honest, evidence-backed verdict instead of the
/// fabric's always-UNKNOWN fallback.
///
/// `claims` is a generic, serde-free slice of `(capability_name, provenance)`
/// where `capability_name` is the snake_case capability and `provenance` is
/// either `"verified"` or `"inferred"`. The fabric deliberately does NOT depend
/// on the registry or hub crates — it only consumes the resolved shapes.
///
/// Honesty semantics (the requirement is interpreted as needing VERIFIED
/// evidence):
/// - a matching claim with provenance `"verified"` → satisfied, `"VERIFIED"`;
/// - else a matching claim with provenance `"inferred"` → NOT satisfied,
///   `"INFERRED"` (an inferred claim never satisfies a verified requirement);
/// - else → NOT satisfied, `"MISSING"`.
///
/// `label` is derived purely from the capability name (`'_'` → `' '`); it never
/// invents a label not derivable from the name. Capability matching is
/// case-insensitive on the snake_case names.
pub fn resolve_capability_requirement(
    required_capability: &str,
    claims: &[(&str, &str)],
) -> CapabilityRequirementView {
    let eq = |name: &str| name.eq_ignore_ascii_case(required_capability);
    let matching = claims.iter().filter(|(cap, _)| eq(cap)).collect::<Vec<_>>();

    let (satisfied, evidence) = if matching
        .iter()
        .any(|(_, prov)| prov.eq_ignore_ascii_case("verified"))
    {
        (true, "VERIFIED")
    } else if matching
        .iter()
        .any(|(_, prov)| prov.eq_ignore_ascii_case("inferred"))
    {
        (false, "INFERRED")
    } else {
        (false, "MISSING")
    };

    CapabilityRequirementView {
        capability: required_capability.to_string(),
        label: required_capability.replace('_', " "),
        satisfied,
        evidence: evidence.to_string(),
    }
}

/// Builds the honest capability-requirement verdict for a request, or `None`
/// when no capability was required.
///
/// - When the request carries a requirement AND `claims` supplies real,
///   non-empty claims → resolve an evidence-backed verdict via
///   [`resolve_capability_requirement`].
/// - When the request carries a requirement but no claims are supplied → the
///   honest UNKNOWN fallback: NOT satisfied with `evidence = "UNKNOWN"`, since
///   the fabric holds no capability data and never claims a requirement met
///   without real evidence. A coordinator with real claims may resolve it.
fn capability_view(
    req: &RequestFacts,
    claims: &[(String, String)],
) -> Option<CapabilityRequirementView> {
    req.required_capability.as_ref().map(|cap| {
        if claims.is_empty() {
            return CapabilityRequirementView {
                capability: cap.clone(),
                label: cap.replace('_', " "),
                satisfied: false,
                evidence: "UNKNOWN".to_string(),
            };
        }
        let refs: Vec<(&str, &str)> = claims
            .iter()
            .map(|(c, p)| (c.as_str(), p.as_str()))
            .collect();
        resolve_capability_requirement(cap, &refs)
    })
}

/// Appends an honest note to the reasoning when a capability requirement was
/// requested but the fabric cannot verify it. Unchanged otherwise.
fn append_capability_note(reasoning: &str, req: &RequestFacts) -> String {
    match &req.required_capability {
        Some(cap) => format!(
            "{reasoning}; capability requirement '{cap}' was requested but the fabric does not verify capabilities here (evidence UNKNOWN)"
        ),
        None => reasoning.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker_facts(id: &str, tps: u32, latency: u32, load: u8) -> WorkerFacts {
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

    fn req() -> RequestFacts {
        RequestFacts {
            model_hash: "m1".into(),
            est_ram_mb: 512,
            est_vram_mb: 0,
            context: ContextProfile {
                prompt_tokens: 100,
                max_output_tokens: 64,
                is_continuation: false,
                prefix_resident_on: None,
            },
            transfer_mib: 0,
            local_peer: None,
            priority: 0,
            required_capability: None,
            capability_claims: Vec::new(),
        }
    }

    #[test]
    fn picks_fastest_eligible_worker() {
        let ws = vec![
            worker_facts("slow", 40, 400, 90),
            worker_facts("fast", 180, 50, 10),
        ];
        let p = ExecutionPlanner::default().plan(&req(), &ws);
        assert_eq!(p.plan.workers(), vec!["fast"]);
        assert_eq!(p.plan.stage_count(), 1);
    }

    #[test]
    fn priority_boosts_the_latency_sensitive_score() {
        // A pure, deterministic check of the priority-aware objective: for the
        // SAME worker, a high-priority request must value its low-latency /
        // low-queue position strictly more than an impartial one does, so an
        // urgent request is steered toward the fastest available compute.
        let fast = worker_facts("fast", 180, 50, 10);
        let planner = ExecutionPlanner::default();
        let cfg = PlannerConfig::default();
        let mut low = req();
        low.priority = 0;
        let mut high = req();
        high.priority = 255;

        let lo = planner.candidate_score(&fast, &low, false, None, &cfg);
        let hi = planner.candidate_score(&fast, &high, false, None, &cfg);
        // At priority 0 the boost factor is exactly 1.0; at 255 it is 1.5, so
        // the latency*queue contribution (and thus the total) strictly grows.
        assert!(hi.total > lo.total, "high-priority must value the fast worker more");
        assert!((lo.latency - hi.latency).abs() < f32::EPSILON, "latency term unchanged");
    }

    #[test]
    fn continuation_is_steered_to_prefix_host_by_locality_score() {
        // M20 continuation affinity: when a session's KV prefix is resident on
        // a specific worker, the planner must prefer it (avoiding a cold
        // prefill), EVEN when that worker is slower than an alternative. This
        // pins the locality term in the score — before it, the prefix host
        // only won via the PeerId tiebreak.
        let fast = worker_facts("fast", 180, 50, 10);
        let mut host = worker_facts("host", 150, 80, 20);
        host.kv = KVCacheState::Partial { used: 5, capacity: 4096 };

        let mut continuation = req();
        continuation.context.is_continuation = true;
        continuation.context.prefix_resident_on = Some("host".into());

        let p = ExecutionPlanner::default().plan(&continuation, &[fast, host]);
        assert_eq!(
            p.plan.workers(),
            vec!["host"],
            "continuation must go back to the prefix host"
        );
        // The locality term is the reason: only the prefix host gets it.
        let chosen = p.rationale.chosen.as_ref().expect("chosen score present");
        assert_eq!(chosen.locality, 1.0, "prefix host gets the locality term");
        let runner_up_locality = p
            .rationale
            .ranked
            .iter()
            .find(|c| c.peer_id == "fast")
            .map(|c| c.locality)
            .unwrap();
        assert_eq!(runner_up_locality, 0.0, "non-host gets no locality term");
    }

    #[test]
    fn no_eligible_worker_yields_empty_plan() {
        let mut slow = worker_facts("slow", 40, 400, 90);
        slow.serves_model = false;
        let p = ExecutionPlanner::default().plan(&req(), &[slow]);
        assert!(p.plan.workers().is_empty() || p.plan.workers() == vec![""]);
        assert!(p.reasoning.contains("no eligible"));
    }

    #[test]
    fn untrusted_workers_are_skipped() {
        let mut trusted = worker_facts("fast", 180, 50, 10);
        let mut untrusted = worker_facts("other", 200, 20, 5);
        trusted.trusted = true;
        untrusted.trusted = false;
        let p = ExecutionPlanner::default().plan(&req(), &[trusted, untrusted]);
        assert_eq!(p.plan.workers(), vec!["fast"]);
    }

    #[test]
    fn fallback_orders_exclude_top_worker() {
        let ws = vec![
            worker_facts("a", 180, 50, 10),
            worker_facts("b", 150, 60, 20),
            worker_facts("c", 120, 80, 30),
        ];
        let p = ExecutionPlanner::default().plan(&req(), &ws);
        assert!(!p.plan.fallback_orders.is_empty());
        for order in &p.plan.fallback_orders {
            assert!(order.contains(&"a".to_string()) || order.contains(&"b".to_string()) || order.contains(&"c".to_string()));
        }
    }

    #[test]
    fn network_cost_steers_low_rtt_worker_when_perf_is_equal() {
        let mut planner = ExecutionPlanner::default();
        planner.network.set(
            "far",
            LinkMetrics::prior(crate::network::Locality::Remote, Some(80_000)),
        );
        planner.network.set(
            "near",
            LinkMetrics::prior(crate::network::Locality::Lan, Some(2_000)),
        );
        // Identical nominal performance: link cost is the only differentiator.
        let ws = vec![
            worker_facts("far", 150, 40, 10),
            worker_facts("near", 150, 40, 10),
        ];
        let p = planner.plan(&req(), &ws);
        assert_eq!(p.plan.workers(), vec!["near"]);
    }

    #[test]
    fn throughput_beats_remote_rtt_for_large_generation() {
        let mut planner = ExecutionPlanner::default();
        planner.network.set(
            "fast",
            LinkMetrics::prior(crate::network::Locality::Remote, Some(80_000)),
        );
        planner.network.set(
            "slow",
            LinkMetrics::prior(crate::network::Locality::Lan, Some(2_000)),
        );
        // A much faster worker still wins a throughput-bound request.
        let ws = vec![
            worker_facts("fast", 500, 10, 5),
            worker_facts("slow", 60, 120, 60),
        ];
        let p = planner.plan(&req(), &ws);
        assert_eq!(p.plan.workers(), vec!["fast"]);
    }

    #[test]
    fn default_config_weights_reproduce_previous_score() {
        // The Default weights must equal the historical hard-coded constants;
        // regression-guards the refactor that moved them into PlannerConfig.
        let cfg = PlannerConfig::default();
        assert_eq!(cfg.w_tps, 0.25);
        assert_eq!(cfg.w_latency, 0.15);
        assert_eq!(cfg.w_load, 0.15);
        assert_eq!(cfg.w_queue, 0.10);
        assert_eq!(cfg.w_headroom, 0.15);
        assert_eq!(cfg.w_net, 0.10);
        assert_eq!(cfg.w_kv, 0.10);
    }

    #[test]
    fn default_objective_reproduces_original_ranking() {
        // With default weights the planner must select the same worker it did
        // before the configurable-weights refactor.
        let ws = vec![
            worker_facts("slow", 40, 400, 90),
            worker_facts("fast", 180, 50, 10),
        ];
        let p = ExecutionPlanner::default().plan(&req(), &ws);
        assert_eq!(p.plan.workers(), vec!["fast"]);
    }

    #[test]
    fn weight_inversion_reverses_ranking() {
        // Heavily weight headroom: a lower-throughput worker with far more RAM
        // must win over the faster, memory-poor worker.
        let config = PlannerConfig {
            w_headroom: 1.0,
            ..PlannerConfig::default()
        };
        let slow_roomy = {
            let mut w = worker_facts("roomy", 40, 400, 90);
            w.available_ram_mb = 64 * 1024;
            w
        };
        let fast_tight = {
            let mut w = worker_facts("tight", 180, 50, 10);
            w.available_ram_mb = 256;
            w
        };
        let p = ExecutionPlanner {
            config,
            ..ExecutionPlanner::default()
        }
        .plan(&req(), &[fast_tight, slow_roomy]);
        assert_eq!(p.plan.workers(), vec!["roomy"]);
    }

    #[test]
    fn rationale_records_chosen_and_runner_up_delta() {
        let ws = vec![
            worker_facts("a", 180, 50, 10),
            worker_facts("b", 150, 60, 20),
        ];
        let p = ExecutionPlanner::default().plan(&req(), &ws);
        assert_eq!(p.rationale.chosen_worker.as_deref(), Some("a"));
        let chosen = p.rationale.chosen.as_ref().expect("chosen scores present");
        // With default weights, total must equal the weighted component sum.
        let tps: f32 = (180.0_f32 / 200.0_f32).clamp(0.0_f32, 1.0_f32);
        let latency: f32 = 1.0_f32 - (50.0_f32 / 1000.0_f32).clamp(0.0_f32, 1.0_f32);
        let load: f32 = 1.0_f32 - (10.0_f32 / 100.0_f32);
        let queue: f32 = 1.0_f32;
        let headroom: f32 = (4096.0_f32 / 512.0_f32).min(1.0_f32);
        let net = p.rationale.chosen.as_ref().unwrap().net; // network default
        let kv = 0.0;
        let expect = 0.25 * tps + 0.15 * latency + 0.15 * load + 0.10 * queue + 0.15 * headroom + 0.10 * net + 0.10 * kv;
        assert!((chosen.total - expect).abs() < 1e-4);
        assert!(p.rationale.runner_up_delta.unwrap() >= 0.0);
        assert_eq!(p.rationale.ranked.len(), 2);
        // ranked[0] is the chosen worker.
        assert_eq!(p.rationale.ranked[0].peer_id, "a");
    }

    #[test]
    fn perf_provenance_is_recorded_without_changing_score() {
        // The perf-provenance marker must flow from WorkerFacts into the
        // CandidateScore/rationale for the CHOSEN worker, and must be purely
        // additive: measured vs estimated candidates get identical `total`.
        let planner = ExecutionPlanner::default();
        let cfg = PlannerConfig::default();
        // Identical nominal perf -> identical total, but the marker differs.
        let mut m_a = worker_facts("a", 150, 60, 20);
        m_a.perf_measured = true;
        let mut e_a = worker_facts("a", 150, 60, 20);
        e_a.perf_measured = false;
        let cs_measured = planner.candidate_score(&m_a, &req(), false, None, &cfg);
        let cs_estimated = planner.candidate_score(&e_a, &req(), false, None, &cfg);
        assert!(cs_measured.perf_measured);
        assert!(!cs_estimated.perf_measured);
        assert_eq!(cs_measured.total, cs_estimated.total, "marker must not change the score");

        // Through the plan, the chosen worker's rationale records the marker.
        // The measured worker has strictly better perf, so it wins and is
        // marked measured.
        let mut measured = worker_facts("measured", 200, 30, 5);
        measured.perf_measured = true;
        let mut estimated = worker_facts("estimated", 150, 60, 20);
        estimated.perf_measured = false;
        let p = planner.plan(&req(), &[estimated.clone(), measured]);
        let chosen = p.rationale.chosen.as_ref().expect("chosen score present");
        assert!(chosen.perf_measured, "measured worker must win and be marked measured");
        assert!(p.rationale.ranked[0].perf_measured);

        // And the estimated-only case records the ESTIMATED marker.
        let p = planner.plan(&req(), &[estimated]);
        let chosen = p.rationale.chosen.as_ref().expect("chosen score present");
        assert!(!chosen.perf_measured, "unmeasured worker is marked estimated");
    }

    #[test]
    fn rationale_empty_when_no_eligible_worker() {
        let mut w = worker_facts("slow", 40, 400, 90);
        w.serves_model = false;
        let p = ExecutionPlanner::default().plan(&req(), &[w]);
        assert!(p.rationale.chosen_worker.is_none());
        assert!(p.rationale.chosen.is_none());
        assert!(p.rationale.ranked.is_empty());
    }

    #[test]
    fn planner_config_round_trips_serde() {
        let c = PlannerConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let back: PlannerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        // Rationale is serde-serializable too.
        let p = ExecutionPlanner::default().plan(
            &req(),
            &[worker_facts("a", 180, 50, 10), worker_facts("b", 150, 60, 20)],
        );
        let rj = serde_json::to_string(&p.rationale).unwrap();
        let rback: PlannerRationale = serde_json::from_str(&rj).unwrap();
        assert_eq!(rback, p.rationale);
    }

    #[test]
    fn no_required_capability_records_none_verdict() {
        // A request WITHOUT a capability requirement must not fabricate a
        // verdict: the rationale field stays `None`.
        let p = ExecutionPlanner::default().plan(
            &req(),
            &[worker_facts("a", 180, 50, 10), worker_facts("b", 150, 60, 20)],
        );
        assert!(p.rationale.capability_requirement.is_none());
    }

    #[test]
    fn required_capability_records_honest_unknown_verdict() {
        // A request WITH a capability requirement records an honest UNKNOWN
        // verdict: the fabric cannot verify capabilities (engine-neutral), so
        // `satisfied` is false and the reasoning names the requirement. It
        // never claims the capability is met.
        let mut with_ocr = req();
        with_ocr.required_capability = Some("ocr".to_string());
        let p = ExecutionPlanner::default().plan(
            &with_ocr,
            &[worker_facts("a", 180, 50, 10), worker_facts("b", 150, 60, 20)],
        );

        let view = p
            .rationale
            .capability_requirement
            .expect("a requirement was requested, so a verdict must be recorded");
        assert_eq!(view.capability, "ocr");
        assert_eq!(view.label, "ocr");
        assert!(!view.satisfied, "fabric cannot verify capabilities: never satisfied");
        assert_eq!(view.evidence, "UNKNOWN");
        assert!(
            p.reasoning.contains("capability requirement 'ocr'"),
            "reasoning must note the requested capability"
        );
        assert!(
            p.reasoning.contains("does not verify capabilities"),
            "reasoning must state the fabric does not verify it"
        );
    }

    #[test]
    fn capability_requirement_with_real_claims_resolves_evidence_backed_verdict() {
        // A request with a requirement AND real persisted claims (supplied by a
        // coordinator) resolves an evidence-backed verdict instead of UNKNOWN.
        let ws = || vec![worker_facts("a", 180, 50, 10)];
        let run = |claims: Vec<(String, String)>| {
            let mut r = req();
            r.required_capability = Some("ocr".to_string());
            r.capability_claims = claims;
            ExecutionPlanner::default().plan(&r, &ws()).rationale
                .capability_requirement
                .expect("requirement requested -> verdict present")
        };

        // Verified claim -> satisfied at VERIFIED.
        let v = run(vec![("ocr".to_string(), "verified".to_string())]);
        assert!(v.satisfied);
        assert_eq!(v.evidence, "VERIFIED");

        // Only inferred claim -> NOT satisfied, INFERRED (never a false pass).
        let v = run(vec![("ocr".to_string(), "inferred".to_string())]);
        assert!(!v.satisfied);
        assert_eq!(v.evidence, "INFERRED");

        // No matching capability -> MISSING.
        let v = run(vec![("coding".to_string(), "verified".to_string())]);
        assert!(!v.satisfied);
        assert_eq!(v.evidence, "MISSING");

        // Empty claims (no data) -> honest UNKNOWN (unchanged fallback).
        let v = run(vec![]);
        assert!(!v.satisfied);
        assert_eq!(v.evidence, "UNKNOWN");
    }

    #[test]
    fn capability_requirement_view_round_trips_serde() {
        // PlannerRationale round-trip above, so a coordinator can persist /
        // display the honest verdict.
        let mut with_ocr = req();
        with_ocr.required_capability = Some("ocr".to_string());
        let p = ExecutionPlanner::default().plan(
            &with_ocr,
            &[worker_facts("a", 180, 50, 10), worker_facts("b", 150, 60, 20)],
        );
        let view = p.rationale.capability_requirement.as_ref().unwrap();
        let json = serde_json::to_string(view).unwrap();
        let back: CapabilityRequirementView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, *view);
    }

    #[test]
    fn resolve_requirement_with_no_claims_stays_unknown() {
        // A requirement with empty claims keeps the honest UNKNOWN fallback.
        let mut with_ocr = req();
        with_ocr.required_capability = Some("ocr".to_string());
        let view = capability_view(&with_ocr, &[]).unwrap();
        assert!(!view.satisfied);
        assert_eq!(view.evidence, "UNKNOWN");
    }

    #[test]
    fn resolve_requirement_with_matching_verified_claim_satisfies() {
        // A matching claim with provenance "verified" → satisfied, VERIFIED.
        let view = resolve_capability_requirement("ocr", &[("ocr", "verified")]);
        assert!(view.satisfied);
        assert_eq!(view.evidence, "VERIFIED");
    }

    #[test]
    fn resolve_requirement_with_only_inferred_claim_is_not_satisfied() {
        // An inferred claim never satisfies a verified requirement.
        let view = resolve_capability_requirement("ocr", &[("ocr", "inferred")]);
        assert!(!view.satisfied);
        assert_eq!(view.evidence, "INFERRED");
    }

    #[test]
    fn resolve_requirement_with_no_matching_capability_is_missing() {
        // No matching capability at all → MISSING.
        let view = resolve_capability_requirement("ocr", &[("asr", "verified"), ("tts", "inferred")]);
        assert!(!view.satisfied);
        assert_eq!(view.evidence, "MISSING");
    }

    #[test]
    fn resolve_requirement_matches_capability_case_insensitively() {
        // "OCR" must match a claim named "ocr".
        let view = resolve_capability_requirement("OCR", &[("ocr", "verified")]);
        assert!(view.satisfied);
        assert_eq!(view.evidence, "VERIFIED");
    }

    #[test]
    fn resolve_requirement_derives_label_from_name() {
        // label derives purely from the name: '_' → ' '.
        let view = resolve_capability_requirement("image_ocr", &[("image_ocr", "inferred")]);
        assert_eq!(view.label, "image ocr");
        assert_eq!(view.capability, "image_ocr");
    }

    #[test]
    fn resolve_requirement_is_a_pure_free_function() {
        // Call the free function directly with a synthetic claims slice; it
        // needs no planner, no I/O, and only depends on its inputs.
        let claims: &[(&str, &str)] = &[("ocr", "inferred"), ("asr", "verified")];
        let view = resolve_capability_requirement("ocr", claims);
        assert!(!view.satisfied);
        assert_eq!(view.evidence, "INFERRED");
        let verified = resolve_capability_requirement("asr", claims);
        assert!(verified.satisfied);
        assert_eq!(verified.evidence, "VERIFIED");
    }
}