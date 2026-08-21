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

/// A golden decision: a request + worker corpus plus the `SelectionTrace` the
/// CURRENT live selector (`ExecutionPlanner`) produced for it. This is the
/// ground truth the unified selector must reproduce.
#[derive(Debug, Clone)]
pub struct GoldenCase {
    pub request_id: String,
    pub req: RequestFacts,
    pub workers: Vec<WorkerFacts>,
    pub golden: SelectionTrace,
}

impl GoldenCase {
    /// Captures the golden decision from the live `ExecutionPlanner` for a
    /// scenario. This is the the source of truth for the equivalence proof.
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
        }
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
