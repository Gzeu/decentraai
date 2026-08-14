//! Pure execution advisories (M23).
//!
//! These are *advisory only*: they answer "could a multi-stage split help?",
//! "should we hand off mid-stream?", and "is the fleet unbalanced?" The
//! coordinator is free to ignore them. None of them *execute* anything — a
//! fan-out candidacy never splits a request, and a replan handoff never moves
//! a running generation. They are pure, deterministic, and unit-testable with
//! synthetic fabric, matching the rest of the fabric crate.
//!
//! The key safety invariant lives here too: a replan advisory must never
//! recommend a handoff once any token has been emitted, because restarting on
//! another worker would duplicate the partial output.

use crate::planner::{PlannerConfig, RequestFacts, WorkerFacts};

/// Whether a request is a good candidate for a multi-worker split.
///
/// This only *reports* candidacy. It never builds a fan-out plan or moves
/// work. `allow` gates the feature (fan-out is parked for the engines
/// DecentraAI runs today), so production always passes `false` until a real
/// engine advertises staging support.
#[derive(Debug, Clone, PartialEq)]
pub enum FanOutAdvisory {
    /// Keep the request on a single worker (split is not justified or not
    /// permitted).
    RetainSingle,
    /// The request *could* be split across `targets`, saving roughly
    /// `estimated_gain_ms` over running on one worker.
    CandidateFanOut {
        targets: Vec<String>,
        estimated_gain_ms: f64,
    },
}

/// Whether a running request should stay put, move, or be abandoned.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplanDecision {
    /// Keep generating on the current worker.
    Continue,
    /// Move the *not-yet-begun* generation to a strictly better worker.
    Handoff(String),
    /// No viable worker remains and no tokens have been produced: stop.
    Abort,
}

/// Reports whether the request is a fan-out candidate across the given fabric.
///
/// Returns `RetainSingle` unless **all** of these hold:
///   - `allow` is true (feature gate),
///   - at least two eligible workers (`trusted && healthy && serves_model`)
///     advertise `supports_staging()`,
///   - there is actual parallel work to gain.
///
/// The estimated gain is a pure cost-model comparison (sequential on the best
/// single worker vs. splitting generation across the staging-capable set).
pub fn fan_out_candidacy(
    req: &RequestFacts,
    workers: &[WorkerFacts],
    _config: &PlannerConfig,
    allow: bool,
) -> FanOutAdvisory {
    if !allow {
        return FanOutAdvisory::RetainSingle;
    }

    let eligible: Vec<&WorkerFacts> = workers
        .iter()
        .filter(|w| w.trusted && w.healthy && w.serves_model && w.capabilities.supports_staging())
        .collect();

    if eligible.len() < 2 {
        return FanOutAdvisory::RetainSingle;
    }

    // Deterministic order of the staging-capable set (PeerId asc).
    let mut targets: Vec<String> = eligible.iter().map(|w| w.peer_id.clone()).collect();
    targets.sort();

    // Pure cost model, mirroring the planner's single-worker estimate.
    let best_tps = eligible
        .iter()
        .map(|w| w.tokens_per_second.max(1))
        .max()
        .unwrap_or(1);
    let avg_tps = eligible.iter().map(|w| w.tokens_per_second.max(1)).sum::<u32>() as f64
        / eligible.len() as f64;
    let total = req.context.total_slots().max(1) as f64;
    let prompt = req.context.prompt_tokens.max(1) as f64;
    let output = (total - prompt).max(1.0);

    let sequential_ms = prompt * 1000.0 / best_tps as f64 + output * 1000.0 / best_tps as f64;
    // Split decode across N workers; prompt still runs once on the fastest.
    let parallel_ms = prompt * 1000.0 / best_tps as f64
        + (output / eligible.len() as f64) * 1000.0 / avg_tps;

    let gain = sequential_ms - parallel_ms;
    if gain <= 0.0 {
        return FanOutAdvisory::RetainSingle;
    }

    FanOutAdvisory::CandidateFanOut {
        targets,
        estimated_gain_ms: gain,
    }
}

/// Decides whether an in-flight, **not-yet-emitting** request should move.
///
/// Safety invariant: once `emitted_tokens > 0`, this always returns
/// `Continue` — nothing may hand off a generation that has already produced
/// output, because moving would duplicate partial tokens to the client.
///
/// Deterministic: candidates are ranked score desc / PeerId asc, and a handoff
/// is only offered to a strictly better worker that is not the current one.
pub fn replan_decision(
    emitted_tokens: u64,
    current_worker: &str,
    fresh: &[WorkerFacts],
    budget_remaining: u32,
    _config: &PlannerConfig,
) -> ReplanDecision {
    if emitted_tokens > 0 {
        return ReplanDecision::Continue;
    }
    if budget_remaining == 0 {
        return ReplanDecision::Continue;
    }

    // Eligible: fresh workers that serve the model and are usable.
    let mut eligible: Vec<&WorkerFacts> = fresh
        .iter()
        .filter(|w| w.trusted && w.healthy && w.serves_model)
        .collect();
    if eligible.is_empty() {
        // Nothing to hand off to and no output yet: give up.
        return ReplanDecision::Abort;
    }

    // Simple deterministic proxy score so advisory.rs stays I/O-free and does
    // not depend on planner internals: descending tps, PeerId asc.
    eligible.sort_by(|a, b| {
        b.tokens_per_second
            .cmp(&a.tokens_per_second)
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });

    let best = eligible[0];
    if best.peer_id != current_worker && best.tokens_per_second * 2 > current_best_tps(fresh) {
        // Only hand off when a genuinely fresher/faster worker exists.
        return ReplanDecision::Handoff(best.peer_id.clone());
    }

    ReplanDecision::Continue
}

/// The current worker's tps for comparison in [`replan_decision`].
fn current_best_tps(fresh: &[WorkerFacts]) -> u32 {
    fresh
        .iter()
        .map(|w| w.tokens_per_second)
        .max()
        .unwrap_or(0)
}

/// Reports an ordered list of `HandoffFrom -> HandoffTo` pairings when the
/// fleet is clearly imbalanced, else an empty vector.
///
/// "Imbalance" is a load/pressure delta between the most- and least-pressured
/// workers exceeding a fixed, deterministic threshold. Handoffs flow from the
/// busiest to the least busy, ordered by descending pressure. Deterministic
/// regardless of input order.
pub fn rebalance_advisory(
    workers: &[(String, f32, u32, u32)],
    config: &PlannerConfig,
) -> Vec<(String, String)> {
    if workers.len() < 2 {
        return Vec::new();
    }

    // Pressure = weighted (normalized load) + weighted (queue+in_flight non-idle).
    let pressure = |w: &(String, f32, u32, u32)| -> f32 {
        let load = (w.1 / 100.0).clamp(0.0, 1.0);
        let backlog = ((w.2 + w.3) as f32 / 100.0).clamp(0.0, 1.0);
        (config.w_load as f32 * load) + (config.w_queue as f32 * backlog)
    };

    // Deterministic: sort by pressure desc, then PeerId asc.
    let mut sorted: Vec<(String, f32)> = workers
        .iter()
        .map(|w| (w.0.clone(), pressure(w)))
        .collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));

    let max_p = sorted[0].1;
    let min_p = sorted[sorted.len() - 1].1;
    // A "clear" imbalance is a spread of at least half the largest achievable
    // pressure (both load and backlog fully saturated). Fixed and deterministic.
    let saturated = (config.w_load + config.w_queue).max(1e-6) as f32;
    if max_p - min_p < 0.5 * saturated {
        return Vec::new();
    }

    let n = sorted.len();
    let pairs = n / 2;
    let mut out = Vec::new();
    for i in 0..pairs {
        let from = sorted[i].0.clone();
        let to = sorted[n - 1 - i].0.clone();
        out.push((from, to));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{EngineCapabilities, EngineKind};
    use crate::kv::{ContextProfile, KVCacheState};

    fn reason_worker(id: &str, tps: u32, load: f32, queue: u32) -> WorkerFacts {
        WorkerFacts {
            peer_id: id.to_string(),
            trusted: true,
            healthy: true,
            engine: EngineKind::LlamaServer,
            tokens_per_second: tps,
            latency_ms: 40,
            queue_depth: queue,
            load_percent: load as u8,
            available_ram_mb: 4096,
            available_vram_mb: 0,
            serves_model: true,
            capabilities: EngineCapabilities::conservative(),
            kv: KVCacheState::Empty,
        }
    }

    fn staging_worker(id: &str, tps: u32) -> WorkerFacts {
        let mut w = reason_worker(id, tps, 10.0, 0);
        w.engine = EngineKind::Vllm;
        w.capabilities = EngineKind::Vllm.advertised_capabilities();
        w
    }

    fn req() -> RequestFacts {
        RequestFacts {
            model_hash: "m1".into(),
            est_ram_mb: 512,
            est_vram_mb: 0,
            context: ContextProfile {
                prompt_tokens: 100,
                max_output_tokens: 200,
                is_continuation: false,
                prefix_resident_on: None,
            },
            transfer_mib: 0,
            local_peer: None,
            priority: 0,
        }
    }

    #[test]
    fn fan_out_requires_allow_gate() {
        let ws = vec![staging_worker("a", 200), staging_worker("b", 200)];
        // Gate off => never a candidate.
        assert_eq!(
            fan_out_candidacy(&req(), &ws, &PlannerConfig::default(), false),
            FanOutAdvisory::RetainSingle
        );
    }

    #[test]
    fn fan_out_requires_staging_support() {
        let ws = vec![reason_worker("a", 200, 10.0, 0), reason_worker("b", 200, 10.0, 0)];
        // llama-server (conservative) never supports staging.
        assert_eq!(
            fan_out_candidacy(&req(), &ws, &PlannerConfig::default(), true),
            FanOutAdvisory::RetainSingle
        );
    }

    #[test]
    fn fan_out_requires_two_eligible() {
        let ws = vec![staging_worker("a", 200)];
        assert_eq!(
            fan_out_candidacy(&req(), &ws, &PlannerConfig::default(), true),
            FanOutAdvisory::RetainSingle
        );
    }

    #[test]
    fn fan_out_candidacy_with_two_staging_workers() {
        let ws = vec![staging_worker("a", 300), staging_worker("b", 300)];
        match fan_out_candidacy(&req(), &ws, &PlannerConfig::default(), true) {
            FanOutAdvisory::CandidateFanOut { targets, estimated_gain_ms } => {
                assert_eq!(targets.len(), 2);
                assert!(estimated_gain_ms > 0.0);
            }
            FanOutAdvisory::RetainSingle => panic!("expected candidate fan-out"),
        }
    }

    #[test]
    fn replan_never_hands_off_after_tokens_emitted() {
        let ws = vec![reason_worker("a", 500, 10.0, 0), reason_worker("b", 50, 10.0, 0)];
        assert_eq!(
            replan_decision(1, "b", &ws, 100, &PlannerConfig::default()),
            ReplanDecision::Continue
        );
    }

    #[test]
    fn replan_continues_on_budget_exhaustion() {
        let ws = vec![reason_worker("a", 500, 10.0, 0), reason_worker("b", 50, 10.0, 0)];
        assert_eq!(
            replan_decision(0, "b", &ws, 0, &PlannerConfig::default()),
            ReplanDecision::Continue
        );
    }

    #[test]
    fn replan_aborts_with_no_fresh_workers() {
        let fresh: Vec<WorkerFacts> = Vec::new();
        assert_eq!(
            replan_decision(0, "b", &fresh, 100, &PlannerConfig::default()),
            ReplanDecision::Abort
        );
    }

    #[test]
    fn replan_hands_off_to_better_fresh_worker() {
        let cur = reason_worker("cur", 50, 10.0, 0);
        let fresh = vec![reason_worker("warp", 500, 10.0, 0), cur.clone()];
        assert_eq!(
            replan_decision(0, "cur", &fresh, 100, &PlannerConfig::default()),
            ReplanDecision::Handoff("warp".to_string())
        );
    }

    #[test]
    fn replan_continues_when_current_is_best() {
        let fresh = vec![reason_worker("warp", 50, 10.0, 0), reason_worker("cur", 500, 10.0, 0)];
        assert_eq!(
            replan_decision(0, "cur", &fresh, 100, &PlannerConfig::default()),
            ReplanDecision::Continue
        );
    }

    #[test]
    fn rebalance_empty_when_balanced() {
        let ws = vec![
            ("a".to_string(), 30.0, 1, 0),
            ("b".to_string(), 35.0, 1, 0),
        ];
        assert!(rebalance_advisory(&ws, &PlannerConfig::default()).is_empty());
    }

    #[test]
    fn rebalance_deterministic_and_ordered_on_imbalance() {
        let ws = vec![
            ("busy".to_string(), 95.0, 40, 5),
            ("very_busy".to_string(), 98.0, 45, 6),
            ("idle".to_string(), 5.0, 0, 0),
            ("other_idle".to_string(), 8.0, 0, 0),
        ];
        let out = rebalance_advisory(&ws, &PlannerConfig::default());
        assert_eq!(out.len(), 2);
        // Busiest ships to least busy, in deterministic (pressure desc) order.
        assert_eq!(out[0], ("very_busy".to_string(), "idle".to_string()));
        assert_eq!(out[1], ("busy".to_string(), "other_idle".to_string()));
    }
}