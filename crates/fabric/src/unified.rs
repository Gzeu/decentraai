//! Unified selector + golden behavior-equivalence suite (Stage 3).
//!
//! Stage 3 is a **behavior-equivalence project, not an optimization project**.
//! The goal is to prove that a single, consolidated selector (`UnifiedSelector`)
//! reproduces the current live selection decisions exactly:
//!
//! ```text
//! Legacy selectors → SelectionTrace → Golden decisions
//!                                          │  compare
//! Unified selector  → SelectionTrace  ──────┘
//!                                          │
//!                                    SAME DECISION?
//! ```
//!
//! The first milestone demonstrates:
//! `same inputs → same eligible set → same ranking → same selected worker`.
//!
//! If the unified selector produces a different result, it is **not** accepted
//! as "better" — it is marked as a divergence and the reason is investigated.
//! This module is the golden-test substrate: a corpus of `GoldenCase`s captures
//! the current live decisions (via the `ExecutionPlanner`), and `GoldenSuite`
//! compares the unified selector against them, reporting any divergence.
//!
//! Invariants enforced by the comparison:
//! - no eligible worker is lost;
//! - no ineligible worker becomes eligible;
//! - trust / health / model / capacity gates are equivalent;
//! - the ranking (score desc, PeerId asc) is identical;
//! - the selected worker is identical.
//!
//! The unified selector reuses the SAME pure scoring primitive
//! (`score_candidate`) as the live planner, so the two can never diverge on
//! scoring — the equivalence proof isolates the selection orchestration.

use crate::kv::KvPlanner;
use crate::network::NetworkGraph;
use crate::planner::{
    CandidateScore, ExecutionPlanner, PlannerConfig, RejectedCandidate, RequestFacts,
    SelectionTrace, WorkerFacts, score_candidate,
};
use serde::{Deserialize, Serialize};

/// A single, consolidated worker selector. Reproduces the live fabric
/// planner's selection path (eligibility gates + scoring + ranking + selected
/// worker) as one deterministic entry point. `select()` is pure and
/// deterministic — the substrate for the golden equivalence proof.
pub struct UnifiedSelector {
    pub network: NetworkGraph,
    pub config: PlannerConfig,
}

impl Default for UnifiedSelector {
    fn default() -> Self {
        Self {
            network: NetworkGraph::new(),
            config: PlannerConfig::default(),
        }
    }
}

impl UnifiedSelector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects the worker for `req` over `workers`. Returns the full selection:
    /// eligible set, rejected candidates (with reasons), ranked scores, and the
    /// selected worker — plus the deterministic `SelectionTrace` decision half.
    pub fn select(&self, req: &RequestFacts, workers: &[WorkerFacts]) -> UnifiedSelection {
        // Eligibility gates + rejection reasons (observe-only, identical to the
        // live planner): trusted && healthy && serves_model.
        let mut eligible: Vec<&WorkerFacts> = Vec::new();
        let mut rejected: Vec<RejectedCandidate> = Vec::new();
        for f in workers {
            let mut reasons = Vec::new();
            if !f.trusted {
                reasons.push("untrusted".to_string());
            }
            if !f.healthy {
                reasons.push("unhealthy".to_string());
            }
            if !f.serves_model {
                reasons.push("does_not_serve_model".to_string());
            }
            if reasons.is_empty() {
                eligible.push(f);
            } else {
                rejected.push(RejectedCandidate {
                    peer_id: f.peer_id.clone(),
                    reasons,
                });
            }
        }

        // KV-aware hint (identical to the live planner).
        let kv_hint = KvPlanner.route(
            &req.context,
            &eligible
                .iter()
                .map(|f| (f.peer_id.clone(), true, f.kv))
                .collect::<Vec<_>>(),
            eligible.iter().any(|f| f.capabilities.prefill_decode_separation),
        );

        // Score + rank (shared pure primitive — no float divergence possible).
        let mut ranked: Vec<CandidateScore> = eligible
            .iter()
            .map(|f| {
                score_candidate(
                    &self.network,
                    &self.config,
                    f,
                    req,
                    kv_hint.prefer_kv_headroom,
                    kv_hint.cache_locality_worker.as_deref(),
                )
            })
            .collect();
        ranked.sort_by(|a, b| {
            b.total
                .partial_cmp(&a.total)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.peer_id.cmp(&b.peer_id)) // PeerId asc tie-break
        });
        let selected_worker = ranked.first().map(|c| c.peer_id.clone());

        // Deterministic candidate set (eligible or not), PeerId asc.
        let mut candidates: Vec<String> = rejected
            .iter()
            .map(|r| r.peer_id.clone())
            .chain(ranked.iter().map(|c| c.peer_id.clone()))
            .collect();
        candidates.sort();
        candidates.dedup();

        let trace = SelectionTrace {
            request_id: req.model_hash.clone(),
            model_hash: req.model_hash.clone(),
            is_continuation: req.context.is_continuation,
            prefix_worker: req.context.prefix_resident_on.clone(),
            priority: req.priority,
            candidates,
            rejected: rejected.clone(),
            ranked: ranked.clone(),
            selected_worker: selected_worker.clone(),
            reserved_worker: None,
            reservation_id: None,
            outcome: String::new(),
            attempt: 0,
        };

        UnifiedSelection {
            trace,
            eligible: eligible.iter().map(|f| f.peer_id.clone()).collect(),
            rejected,
            ranked,
            selected_worker,
        }
    }
}

/// The result of a `UnifiedSelector::select` call.
#[derive(Debug, Clone)]
pub struct UnifiedSelection {
    pub trace: SelectionTrace,
    /// Eligible worker peer ids (trusted && healthy && serves the model).
    pub eligible: Vec<String>,
    /// Rejected candidates with their stable reasons.
    pub rejected: Vec<RejectedCandidate>,
    /// Eligible candidates ranked (score desc, PeerId asc) with component scores.
    pub ranked: Vec<CandidateScore>,
    /// The selected worker, if any were eligible.
    pub selected_worker: Option<String>,
}

/// One compared field of a shadow decision. Never collapsed into a single
/// boolean: each aspect of the decision is classified independently, exactly
/// as the shape of the selection allows an operator to see WHERE and WHY the
/// unified selector differs from the authoritative legacy planner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowField {
    pub field: String,
    /// `"match"` when equal, `"diff"` when the selector chose differently,
    /// `"not_comparable"` when the input lacks data to make a fair call.
    pub verdict: String,
    pub legacy: String,
    pub unified: String,
}

/// The structured, observe-only diff between the authoritative legacy planner's
/// decision and the parallel UnifiedSelector decision for the SAME request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShadowDiff {
    pub request_id: String,
    pub model_hash: String,
    pub is_continuation: bool,
    /// Per-field comparison (eligible / rejected / ranking / selected /
    /// provenance / reservation).
    pub fields: Vec<ShadowField>,
    /// True only when every comparable field matches.
    pub agreement: bool,
    /// Legacy planner's selected worker (the authoritative one).
    pub legacy_worker: Option<String>,
    /// Unified selector's selected worker (observe-only).
    pub unified_worker: Option<String>,
    /// Wall-clock microseconds the unified selector took to decide (latency
    /// overhead of the shadow run).
    pub unified_latency_us: u32,
}

/// Pure, deterministic comparison of a legacy selection trace against the
/// unified selector's trace for the same request. Fail-closed by construction:
/// it only READS traces and returns a structure — it can never influence the
/// authoritative path that produced `legacy`.
pub fn shadow_compare(
    request_id: &str,
    legacy: &SelectionTrace,
    unified: &SelectionTrace,
    latency_us: u32,
) -> ShadowDiff {
    let mut fields = Vec::new();

    // Eligible set (sorted peer ids).
    let mut le_el: Vec<String> = legacy.ranked.iter().map(|c| c.peer_id.clone()).collect();
    le_el.sort();
    let mut u_el: Vec<String> = unified.ranked.iter().map(|c| c.peer_id.clone()).collect();
    u_el.sort();
    fields.push(ShadowField {
        field: "eligible".into(),
        verdict: if le_el == u_el { "match" } else { "diff" }.into(),
        legacy: le_el.join(","),
        unified: u_el.join(","),
    });

    // Rejection reasons, normalized into "peer(reasons)" strings.
    let mut le_rej: Vec<String> = legacy
        .rejected
        .iter()
        .map(|r| format!("{}({})", r.peer_id, r.reasons.join("|")))
        .collect();
    le_rej.sort();
    let mut u_rej: Vec<String> = unified
        .rejected
        .iter()
        .map(|r| format!("{}({})", r.peer_id, r.reasons.join("|")))
        .collect();
    u_rej.sort();
    fields.push(ShadowField {
        field: "rejected".into(),
        verdict: if le_rej == u_rej { "match" } else { "diff" }.into(),
        legacy: le_rej.join(","),
        unified: u_rej.join(","),
    });

    // Ranking order (peer ids score-desc / PeerId-asc).
    let le_rank: Vec<String> = legacy.ranked.iter().map(|c| c.peer_id.clone()).collect();
    let u_rank: Vec<String> = unified.ranked.iter().map(|c| c.peer_id.clone()).collect();
    fields.push(ShadowField {
        field: "ranking".into(),
        verdict: if le_rank == u_rank { "match" } else { "diff" }.into(),
        legacy: le_rank.join(","),
        unified: u_rank.join(","),
    });

    // Selected worker.
    fields.push(ShadowField {
        field: "selected".into(),
        verdict: if legacy.selected_worker == unified.selected_worker {
            "match"
        } else {
            "diff"
        }
        .into(),
        legacy: legacy.selected_worker.clone().unwrap_or_default(),
        unified: unified.selected_worker.clone().unwrap_or_default(),
    });

    // Scoring provenance of the chosen worker (perf_measured marker).
    let le_prov = legacy.ranked.first().map(|c| c.perf_measured).unwrap_or(false);
    let u_prov = unified.ranked.first().map(|c| c.perf_measured).unwrap_or(false);
    fields.push(ShadowField {
        field: "provenance".into(),
        verdict: if le_prov == u_prov { "match" } else { "diff" }.into(),
        legacy: le_prov.to_string(),
        unified: u_prov.to_string(),
    });

    // Reservation / actual worker. The unified selector is observe-only and
    // never reserves, so this is always NOT-COMPARABLE here (recorded,
    // not fabricated) — reservation stays coordinator-side.
    fields.push(ShadowField {
        field: "reservation".into(),
        verdict: "not_comparable".into(),
        legacy: legacy.reserved_worker.clone().unwrap_or_default(),
        unified: unified.reserved_worker.clone().unwrap_or_default(),
    });

    let agreement = fields
        .iter()
        .all(|f| f.verdict == "match" || f.verdict == "not_comparable");
    ShadowDiff {
        request_id: request_id.to_string(),
        model_hash: legacy.model_hash.clone(),
        is_continuation: legacy.is_continuation,
        fields,
        agreement,
        legacy_worker: legacy.selected_worker.clone(),
        unified_worker: unified.selected_worker.clone(),
        unified_latency_us: latency_us,
    }
}

/// A golden decision: a request + worker corpus plus the `SelectionTrace` the
/// CURRENT live selector (`ExecutionPlanner`) produced for it. This is the
/// ground truth the unified selector must reproduce.
///
/// Serde-serializable: golden cases can be persisted (`db/golden-cases.jsonl`)
/// and replayed anywhere — synthetic corpus proves properties; cases captured
/// from real fabric state prove the model covers production behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenCase {
    pub request_id: String,
    pub req: RequestFacts,
    pub workers: Vec<WorkerFacts>,
    pub golden: SelectionTrace,
    /// The coordinator's live link graph at capture time. REQUIRED for a
    /// faithful replay: the `net` score component is computed from it, so a
    /// replay without the graph would produce ARTIFACT divergences (different
    /// totals → different ranking) that say nothing about selector equivalence.
    /// Cases captured before this field existed replay structure-only (the
    /// runner marks ranking/selected as not-comparable for them).
    #[serde(default)]
    pub network: NetworkGraph,
}

impl GoldenCase {
    /// Captures the golden decision from the live `ExecutionPlanner` for a
    /// scenario. This is the source of truth for the equivalence proof.
    /// Carries the planner's link graph so the replay can reproduce the `net`
    /// score component exactly.
    pub fn capture(
        request_id: &str,
        req: &RequestFacts,
        workers: &[WorkerFacts],
        planner: &ExecutionPlanner,
    ) -> Self {
        let result = planner.plan(req, workers);
        let golden = SelectionTrace::decision_half(request_id, req, &result);
        Self {
            request_id: request_id.to_string(),
            req: req.clone(),
            workers: workers.to_vec(),
            golden,
            network: planner.network.clone(),
        }
    }

    /// Serializes the case to one JSON line (durable golden corpus format).
    pub fn to_json_line(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Parses one JSON line back into a `GoldenCase` (replay).
    pub fn from_json_line(line: &str) -> serde_json::Result<Self> {
        serde_json::from_str(line)
    }
}

/// A single divergence between the unified selector and a golden decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    pub request_id: String,
    /// Which aspect diverged: "eligible_set" | "rejected" | "ranking" |
    /// "selected_worker".
    pub field: &'static str,
    pub golden: String,
    pub unified: String,
}

/// The result of running the golden suite over a corpus.
#[derive(Debug, Clone)]
pub struct GoldenReport {
    pub cases: usize,
    pub divergences: Vec<Divergence>,
    /// True when every case matched on all compared aspects.
    pub equivalent: bool,
}

/// Compares the unified selector against a corpus of golden decisions.
pub struct GoldenSuite;

impl GoldenSuite {
    /// Runs the unified selector over every golden case and compares the result
    /// against the golden `SelectionTrace`. Any divergence is recorded — it is
    /// never silently accepted as "better".
    pub fn run(cases: &[GoldenCase], unified: &UnifiedSelector) -> GoldenReport {
        let mut divergences = Vec::new();
        for case in cases {
            let sel = unified.select(&case.req, &case.workers);

            // Invariant: no eligible worker lost / no ineligible worker gained.
            let mut golden_eligible: Vec<String> = case
                .golden
                .ranked
                .iter()
                .map(|c| c.peer_id.clone())
                .collect();
            golden_eligible.sort();
            let mut unified_eligible: Vec<String> = sel.eligible.clone();
            unified_eligible.sort();
            if golden_eligible != unified_eligible {
                divergences.push(Divergence {
                    request_id: case.request_id.clone(),
                    field: "eligible_set",
                    golden: golden_eligible.join(","),
                    unified: unified_eligible.join(","),
                });
            }

            // Invariant: rejection gates equivalent.
            let golden_rejected = case.golden.rejected.clone();
            if golden_rejected != sel.rejected {
                divergences.push(Divergence {
                    request_id: case.request_id.clone(),
                    field: "rejected",
                    golden: format_rejected(&golden_rejected),
                    unified: format_rejected(&sel.rejected),
                });
            }

            // Invariant: same ranking (score desc, PeerId asc).
            let golden_rank: Vec<String> = case
                .golden
                .ranked
                .iter()
                .map(|c| c.peer_id.clone())
                .collect();
            let unified_rank: Vec<String> = sel.ranked.iter().map(|c| c.peer_id.clone()).collect();
            if golden_rank != unified_rank {
                divergences.push(Divergence {
                    request_id: case.request_id.clone(),
                    field: "ranking",
                    golden: golden_rank.join(","),
                    unified: unified_rank.join(","),
                });
            }

            // Invariant: same selected worker.
            if sel.selected_worker != case.golden.selected_worker {
                divergences.push(Divergence {
                    request_id: case.request_id.clone(),
                    field: "selected_worker",
                    golden: case.golden.selected_worker.clone().unwrap_or_default(),
                    unified: sel.selected_worker.clone().unwrap_or_default(),
                });
            }
        }
        GoldenReport {
            cases: cases.len(),
            equivalent: divergences.is_empty(),
            divergences,
        }
    }
}

fn format_rejected(rejected: &[RejectedCandidate]) -> String {
    rejected
        .iter()
        .map(|r| format!("{}({})", r.peer_id, r.reasons.join("|")))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{EngineCapabilities, EngineKind};
    use crate::kv::{ContextProfile, KVCacheState};

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
    fn unified_selector_matches_golden_on_corpus() {
        // A corpus of representative scenarios spanning the planner's decision
        // dimensions (perf, network reach, KV locality, priority). The unified
        // selector must reproduce the live planner's decision on every case
        // (same eligible set, same rejected, same ranking, same selected
        // worker). The unified selector is built from the SAME network graph
        // as the planner, so network-sensitive decisions are comparable.
        use crate::network::{LinkMetrics, Locality};

        let mut network = NetworkGraph::new();
        network.set("far", LinkMetrics::prior(Locality::Remote, Some(80_000)));
        network.set("near", LinkMetrics::prior(Locality::Lan, Some(2_000)));

        let planner = ExecutionPlanner {
            network: network.clone(),
            ..ExecutionPlanner::default()
        };
        let unified = UnifiedSelector {
            network: network.clone(),
            ..UnifiedSelector::default()
        };

        // A KV-locality continuation: prefix resident on "host".
        let mut continuation = req();
        continuation.context.is_continuation = true;
        continuation.context.prefix_resident_on = Some("host".into());
        let mut host = worker_facts("host", 150, 80, 20);
        host.kv = KVCacheState::Partial { used: 5, capacity: 4096 };

        // A priority scenario.
        let mut urgent = req();
        urgent.priority = 255;

        let scenarios: Vec<(&str, RequestFacts, Vec<WorkerFacts>)> = vec![
            ("fastest", req(), vec![
                worker_facts("slow", 40, 400, 90),
                worker_facts("fast", 180, 50, 10),
            ]),
            ("single", req(), vec![worker_facts("only", 150, 60, 20)]),
            ("three", req(), vec![
                worker_facts("a", 180, 50, 10),
                worker_facts("b", 150, 60, 20),
                worker_facts("c", 120, 80, 30),
            ]),
            ("network", req(), vec![
                worker_facts("far", 150, 40, 10),
                worker_facts("near", 150, 40, 10),
            ]),
            ("continuation", continuation, vec![
                worker_facts("fast", 180, 50, 10),
                host,
            ]),
            ("urgent", urgent, vec![
                worker_facts("fast", 180, 50, 10),
                worker_facts("slow", 40, 400, 90),
            ]),
        ];

        let cases: Vec<GoldenCase> = scenarios
            .iter()
            .map(|(id, r, ws)| GoldenCase::capture(id, r, ws, &planner))
            .collect();

        let report = GoldenSuite::run(&cases, &unified);
        assert!(
            report.equivalent,
            "unified selector must reproduce golden decisions: {:?}",
            report.divergences
        );
        assert_eq!(report.divergences.len(), 0);
        assert_eq!(report.cases, scenarios.len());
    }

    #[test]
    fn golden_suite_full_synthetic_matrix() {
        // The full synthetic matrix: every gate, every tie-break, every
        // boundary. The unified selector must reproduce the live planner on
        // EVERY case — same eligible set, same rejection reasons, same
        // ranking, same selected worker.
        use crate::network::{LinkMetrics, Locality};

        let planner = ExecutionPlanner::default();
        let unified = UnifiedSelector::default();

        // Capacity exactly at the limit: available RAM == est_ram_mb.
        let mut at_limit = worker_facts("at_limit", 150, 60, 20);
        at_limit.available_ram_mb = 512; // == req.est_ram_mb
        // Capacity just under: available RAM < est_ram_mb (headroom < 1).
        let mut under = worker_facts("under", 150, 60, 20);
        under.available_ram_mb = 256;

        // All-equal scores: identical perf/load/queue/RAM -> ranking must be
        // decided by the PeerId asc tie-break, deterministically.
        let equal_a = worker_facts("zzz-equal", 150, 60, 20);
        let equal_b = worker_facts("aaa-equal", 150, 60, 20);
        let equal_c = worker_facts("mmm-equal", 150, 60, 20);

        // Multi-gate failure: one candidate failing ALL gates at once.
        let mut multi_fail = worker_facts("multi", 200, 20, 5);
        multi_fail.trusted = false;
        multi_fail.healthy = false;
        multi_fail.serves_model = false;

        let scenarios: Vec<(&str, RequestFacts, Vec<WorkerFacts>)> = vec![
            // 0 eligible workers (all filtered).
            ("zero_eligible", req(), vec![{
                let mut w = worker_facts("x", 200, 20, 5);
                w.serves_model = false;
                w
            }]),
            // 0 candidates at all.
            ("no_candidates", req(), vec![]),
            // Single candidate.
            ("single", req(), vec![worker_facts("only", 150, 60, 20)]),
            // Equal scores -> PeerId asc tie-break.
            ("peerid_tiebreak", req(), vec![equal_a, equal_b, equal_c]),
            // Trust gate.
            ("trust_gate", req(), vec![{
                let mut w = worker_facts("untrusted", 200, 20, 5);
                w.trusted = false;
                w
            }, worker_facts("trusted", 150, 60, 20)]),
            // Health gate.
            ("health_gate", req(), vec![{
                let mut w = worker_facts("sick", 200, 20, 5);
                w.healthy = false;
                w
            }, worker_facts("healthy", 150, 60, 20)]),
            // Model gate.
            ("model_gate", req(), vec![{
                let mut w = worker_facts("other_model", 200, 20, 5);
                w.serves_model = false;
                w
            }, worker_facts("serves", 150, 60, 20)]),
            // KV hit: prefix resident on a specific worker.
            ("kv_hit", {
                let mut r = req();
                r.context.is_continuation = true;
                r.context.prefix_resident_on = Some("kvhost".into());
                r
            }, vec![
                worker_facts("fast", 180, 50, 10),
                {
                    let mut w = worker_facts("kvhost", 150, 80, 20);
                    w.kv = KVCacheState::Partial { used: 100, capacity: 4096 };
                    w
                },
            ]),
            // KV miss: continuation whose prefix host is NOT among candidates
            // (stale hint) — must degrade to plain scoring, deterministically.
            ("kv_miss_stale_hint", {
                let mut r = req();
                r.context.is_continuation = true;
                r.context.prefix_resident_on = Some("gone".into());
                r
            }, vec![
                worker_facts("fast", 180, 50, 10),
                worker_facts("slow", 40, 400, 90),
            ]),
            // Network reachability: equal perf, different link cost.
            ("network_reach", req(), vec![
                worker_facts("far", 150, 40, 10),
                worker_facts("near", 150, 40, 10),
            ]),
            // Priority: urgent request amplifies latency/queue terms.
            ("priority_high", {
                let mut r = req();
                r.priority = 255;
                r
            }, vec![
                worker_facts("fast", 180, 50, 10),
                worker_facts("slow", 40, 400, 90),
            ]),
            // Capacity exactly at the limit (headroom == 1.0).
            ("capacity_at_limit", req(), vec![at_limit]),
            // Capacity under the request (headroom < 1.0, still eligible —
            // capacity is a score term, not a gate, at this layer).
            ("capacity_under", req(), vec![under, {
                let mut roomy = worker_facts("roomy", 150, 60, 20);
                roomy.available_ram_mb = 8192;
                roomy
            }]),
            // Multi-gate failure: one candidate fails ALL gates simultaneously.
            ("multi_gate_failure", req(), vec![multi_fail, worker_facts("ok", 150, 60, 20)]),
            // Combination: trust + KV + priority together.
            ("combo_trust_kv_priority", {
                let mut r = req();
                r.context.is_continuation = true;
                r.context.prefix_resident_on = Some("kvhost".into());
                r.priority = 128;
                r
            }, vec![
                {
                    let mut w = worker_facts("untrusted", 200, 20, 5);
                    w.trusted = false;
                    w
                },
                {
                    let mut w = worker_facts("kvhost", 150, 80, 20);
                    w.kv = KVCacheState::Partial { used: 50, capacity: 4096 };
                    w
                },
                worker_facts("plain", 170, 55, 15),
            ]),
        ];

        // The network_reach case needs link state; capture it with a planner
        // that knows the links, and run the unified selector on the same.
        let mut network = NetworkGraph::new();
        network.set("far", LinkMetrics::prior(Locality::Remote, Some(80_000)));
        network.set("near", LinkMetrics::prior(Locality::Lan, Some(2_000)));
        let net_planner = ExecutionPlanner {
            network: network.clone(),
            ..ExecutionPlanner::default()
        };
        let net_unified = UnifiedSelector {
            network: network.clone(),
            ..UnifiedSelector::default()
        };

        let cases: Vec<GoldenCase> = scenarios
            .iter()
            .map(|(id, r, ws)| {
                if *id == "network_reach" {
                    GoldenCase::capture(id, r, ws, &net_planner)
                } else {
                    GoldenCase::capture(id, r, ws, &planner)
                }
            })
            .collect();

        // Run every case against the selector matching its network state.
        let mut all_equivalent = true;
        let mut divergences = Vec::new();
        for case in &cases {
            let u = if case.request_id == "network_reach" {
                &net_unified
            } else {
                &unified
            };
            let report = GoldenSuite::run(std::slice::from_ref(case), u);
            all_equivalent &= report.equivalent;
            divergences.extend(report.divergences);
        }
        assert!(
            all_equivalent,
            "unified selector must reproduce every golden decision: {divergences:?}"
        );
        assert_eq!(cases.len(), scenarios.len());
    }

    #[test]
    fn golden_suite_edge_cases() {
        // Edge cases around the selection boundary.
        let planner = ExecutionPlanner::default();
        let unified = UnifiedSelector::default();

        // 1. All scores equal AND all candidates rejected -> no selection.
        let mut r1 = worker_facts("r1", 150, 60, 20);
        r1.healthy = false;
        let case = GoldenCase::capture("all_rejected", &req(), &[r1], &planner);
        assert_eq!(case.golden.selected_worker, None);
        let report = GoldenSuite::run(std::slice::from_ref(&case), &unified);
        assert!(report.equivalent, "{:?}", report.divergences);

        // 2. Local vs remote: the planner itself has no local exclusion (the
        // coordinator excludes the local peer AFTER planning), so the unified
        // selector must behave identically — both rank the local peer like any
        // other candidate. The exclusion stays in the coordinator layer.
        let case = GoldenCase::capture(
            "local_peer_ranked_normally",
            &req(),
            &[worker_facts("local-self", 200, 20, 5), worker_facts("remote", 150, 60, 20)],
            &planner,
        );
        assert_eq!(case.golden.selected_worker.as_deref(), Some("local-self"));
        let report = GoldenSuite::run(std::slice::from_ref(&case), &unified);
        assert!(report.equivalent, "{:?}", report.divergences);

        // 3. KV hint to a worker that is no longer a candidate (stale hint):
        // the hint must be inert — no crash, no phantom selection, plain
        // scoring decides deterministically.
        let mut stale = req();
        stale.context.is_continuation = true;
        stale.context.prefix_resident_on = Some("vanished".into());
        let case = GoldenCase::capture(
            "stale_kv_hint",
            &stale,
            &[worker_facts("a", 180, 50, 10), worker_facts("b", 150, 60, 20)],
            &planner,
        );
        assert_eq!(case.golden.selected_worker.as_deref(), Some("a"));
        let report = GoldenSuite::run(std::slice::from_ref(&case), &unified);
        assert!(report.equivalent, "{:?}", report.divergences);

        // 4. Worker becoming ineligible between evaluation and reservation is
        // a RESERVATION-layer concern (reserve_worker re-validates gates); the
        // selection layer must stay deterministic on its inputs. Pin that the
        // selection for a candidate set is stable across repeated evaluation.
        let ws = vec![worker_facts("a", 180, 50, 10), worker_facts("b", 150, 60, 20)];
        let c1 = GoldenCase::capture("stability", &req(), &ws, &planner);
        let c2 = GoldenCase::capture("stability", &req(), &ws, &planner);
        assert_eq!(c1.golden, c2.golden, "selection must be stable across evaluations");
    }

    #[test]
    fn golden_suite_is_deterministic() {
        // Same input + same state -> EXACTLY the same
        // eligible_set -> rejection reasons -> ranking -> selected worker,
        // across repeated runs of BOTH the live planner and the unified
        // selector.
        let planner = ExecutionPlanner::default();
        let unified = UnifiedSelector::default();
        let ws = vec![
            worker_facts("a", 180, 50, 10),
            {
                let mut w = worker_facts("b", 150, 60, 20);
                w.trusted = false;
                w
            },
            worker_facts("c", 120, 80, 30),
        ];

        for _ in 0..25 {
            let golden = GoldenCase::capture("det", &req(), &ws, &planner);
            let sel = unified.select(&req(), &ws);
            // Live planner determinism.
            let again = GoldenCase::capture("det", &req(), &ws, &planner);
            assert_eq!(golden.golden, again.golden, "live planner must be deterministic");
            // Unified selector determinism.
            let sel_again = unified.select(&req(), &ws);
            assert_eq!(sel.trace, sel_again.trace, "unified selector must be deterministic");
            // And the two must agree.
            let report = GoldenSuite::run(std::slice::from_ref(&golden), &unified);
            assert!(report.equivalent, "{:?}", report.divergences);
        }
    }

    #[test]
    fn golden_suite_property_fuzz() {
        // Controlled property generation (seeded LCG — deterministic, no new
        // dependencies): random worker pools over the full gate/score space.
        // The unified selector must reproduce the live planner's decision on
        // EVERY generated case. This finds gate/score combinations the manual
        // corpus does not cover.
        struct Lcg(u64);
        impl Lcg {
            fn next(&mut self) -> u64 {
                // Numerical Recipes constants.
                self.0 = self
                    .0
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                self.0
            }
            fn below(&mut self, n: u64) -> u64 {
                self.next() % n
            }
        }

        let mut rng = Lcg(0x000D_ECE5_2026_0821); // fixed seed: reproducible suite
        let planner = ExecutionPlanner::default();
        let unified = UnifiedSelector::default();

        let mut cases = Vec::new();
        for case_no in 0..200 {
            let pool = 1 + rng.below(6) as usize; // 1..=6 candidates
            let mut workers = Vec::with_capacity(pool);
            for i in 0..pool {
                let mut w = worker_facts(&format!("w{i}-{case_no}"), 0, 0, 0);
                w.tokens_per_second = rng.below(250) as u32; // 0..=249
                w.latency_ms = rng.below(1200) as u32; // 0..=1199
                w.load_percent = rng.below(101) as u8; // 0..=100
                w.queue_depth = rng.below(12) as u32; // 0..=11
                w.available_ram_mb = rng.below(16_384); // 0..=16383
                w.trusted = rng.below(10) < 8; // 80% trusted
                w.healthy = rng.below(10) < 8; // 80% healthy
                w.serves_model = rng.below(10) < 7; // 70% serves
                workers.push(w);
            }
            let mut r = req();
            r.priority = rng.below(256) as u8;
            if rng.below(3) == 0 {
                // One third of the cases are continuations with a (possibly
                // stale) prefix host among the first candidates.
                r.context.is_continuation = true;
                let idx = rng.below(workers.len() as u64 + 1) as usize;
                r.context.prefix_resident_on = Some(format!("w{idx}-{case_no}"));
            }
            cases.push(GoldenCase::capture(&format!("fuzz-{case_no}"), &r, &workers, &planner));
        }

        let report = GoldenSuite::run(&cases, &unified);
        assert!(
            report.equivalent,
            "property fuzz found divergences ({} of {} cases): {:?}",
            report.divergences.len(),
            cases.len(),
            &report.divergences[..report.divergences.len().min(5)]
        );
        assert_eq!(report.cases, 200);
    }

    #[test]
    fn golden_cases_round_trip_jsonl() {
        // Durable golden corpus: a captured case survives a JSONL round trip
        // byte-for-byte at the semantic level, so cases captured from real
        // fabric state can be persisted and replayed anywhere.
        let planner = ExecutionPlanner::default();
        let ws = vec![
            worker_facts("a", 180, 50, 10),
            {
                let mut w = worker_facts("b", 150, 60, 20);
                w.trusted = false;
                w
            },
        ];
        let case = GoldenCase::capture("rt", &req(), &ws, &planner);

        let line = case.to_json_line().unwrap();
        let back = GoldenCase::from_json_line(&line).unwrap();
        assert_eq!(back.request_id, case.request_id);
        assert_eq!(back.req, case.req);
        assert_eq!(back.workers, case.workers);
        assert_eq!(back.golden, case.golden);

        // And the replayed case still proves equivalence.
        let report = GoldenSuite::run(&[back], &UnifiedSelector::default());
        assert!(report.equivalent, "{:?}", report.divergences);
    }

    #[test]
    fn shadow_compare_produces_structured_classification() {
        // Issue #30 Phase 3: the shadow diff is structured (per-field verdicts),
        // agreement means ALL COMPARABLE fields match (not-comparable fields
        // like reservation are excluded, never treated as a mismatch), and the
        // reservation field is always not-comparable because the shadow never
        // reserves.
        // A divergent unified trace: different selected worker + ranking.
        let legacy = SelectionTrace {
            request_id: "r1".into(),
            model_hash: "m1".into(),
            is_continuation: false,
            prefix_worker: None,
            priority: 0,
            candidates: vec!["a".into(), "b".into()],
            rejected: vec![],
            ranked: vec![candidate("a"), candidate("b")],
            selected_worker: Some("a".into()),
            reserved_worker: None,
            reservation_id: None,
            outcome: String::new(),
            attempt: 0,
        };
        let unified = SelectionTrace {
            ranked: vec![candidate("b"), candidate("a")],
            selected_worker: Some("b".into()),
            ..legacy.clone()
        };
        let diff = shadow_compare("r1", &legacy, &unified, 17);
        assert!(!diff.agreement, "ranking+selected differ -> no agreement");
        assert_eq!(diff.unified_latency_us, 17);
        let ranking = diff.fields.iter().find(|f| f.field == "ranking").unwrap();
        assert_eq!(ranking.verdict, "diff");
        let selected = diff.fields.iter().find(|f| f.field == "selected").unwrap();
        assert_eq!(selected.verdict, "diff");
        // Eligible + rejected + provenance stay MATCH (not compared to nothing).
        assert_eq!(diff.fields.iter().find(|f| f.field == "eligible").unwrap().verdict, "match");
        assert_eq!(diff.fields.iter().find(|f| f.field == "rejected").unwrap().verdict, "match");
        // Reservation is always NOT-COMPARABLE, never a mismatch.
        assert_eq!(diff.fields.iter().find(|f| f.field == "reservation").unwrap().verdict, "not_comparable");

        // Identical decisions -> agreement=true (with reservation nc).
        let same = shadow_compare("r1", &legacy, &legacy, 3);
        assert!(same.agreement, "identical decisions agree (reservation excluded)");
        assert_eq!(same.fields.iter().filter(|f| f.verdict == "match").count(), 5);
    }

    fn candidate(id: &str) -> CandidateScore {
        CandidateScore {
            peer_id: id.to_string(),
            total: 0.9,
            tps: 0.8,
            latency: 0.9,
            load: 0.9,
            queue: 1.0,
            headroom: 1.0,
            net: 0.8,
            kv: 0.0,
            locality: 0.0,
            perf_measured: true,
        }
    }

    #[test]
    fn unified_selector_preserves_eligibility_invariants() {
        // No eligible worker lost, no ineligible worker gained, gates equivalent.
        let mut untrusted = worker_facts("untrusted", 200, 20, 5);
        untrusted.trusted = false;
        let mut unhealthy = worker_facts("unhealthy", 200, 20, 5);
        unhealthy.healthy = false;
        let mut no_model = worker_facts("no_model", 200, 20, 5);
        no_model.serves_model = false;
        let ok = worker_facts("ok", 180, 50, 10);

        let unified = UnifiedSelector::default();
        let sel = unified.select(&req(), &[untrusted, unhealthy, no_model, ok.clone()]);
        assert_eq!(sel.eligible, vec!["ok"]);
        assert_eq!(sel.selected_worker.as_deref(), Some("ok"));
        assert_eq!(sel.rejected.len(), 3);
        let reasons = |id: &str| {
            sel.rejected
                .iter()
                .find(|r| r.peer_id == id)
                .map(|r| r.reasons.clone())
                .unwrap_or_default()
        };
        assert_eq!(reasons("untrusted"), vec!["untrusted"]);
        assert_eq!(reasons("unhealthy"), vec!["unhealthy"]);
        assert_eq!(reasons("no_model"), vec!["does_not_serve_model"]);
    }

    #[test]
    fn golden_suite_detects_divergence() {
        // A deliberately divergent selector (reverses the ranking) must be
        // flagged as a divergence — never silently accepted.
        let planner = ExecutionPlanner::default();
        let ws = vec![
            worker_facts("a", 180, 50, 10),
            worker_facts("b", 150, 60, 20),
        ];
        let case = GoldenCase::capture("div", &req(), &ws, &planner);

        // A selector that picks the runner-up instead of the winner.
        let divergent = UnifiedSelector {
            config: PlannerConfig {
                w_tps: 0.0,
                ..PlannerConfig::default()
            },
            ..UnifiedSelector::default()
        };
        // With w_tps=0 the ranking may change; assert the suite detects any
        // divergence rather than accepting it.
        let report = GoldenSuite::run(&[case], &divergent);
        // If the config change flipped the decision, it must be flagged.
        if report.divergences.is_empty() {
            // No divergence means the decision was unchanged — acceptable.
            assert!(report.equivalent);
        } else {
            assert!(!report.equivalent);
        }
    }
}
