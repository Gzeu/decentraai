//! Agent reputation — per-(agent, capability) factors, not one opaque score.
//!
//! The architecture (`docs/COLLECTIVE_INTELLIGENCE.md` §4.5) is explicit:
//! the collective fabric already tracks trust several ways (TrustStore EMA,
//! CircuitBreaker, CompensationLedger, ContributionProfile), and P6 unifies
//! them into an explainable [`AgentReputation`] whose building blocks are
//! isolated, purely-testable factors. Each factor has one definition of
//! calculation, so the planner can weight and explain them separately.
//!
//! # Design decisions
//!
//! - **Scores are per (agent, capability).** An agent can be excellent at OCR
//!   and mediocre at coding; a single global score hides exactly that. The
//!   reputation key is `(agent_id, capability)` and the aggregate
//!   [`AgentReputation::score`] is only ever a projection of the factors.
//! - **Only cryptographic/policy violations touch [`ReputationFactor::Safety`]**
//!   (fabric invariant: network errors never punish peers). The *caller*
//!   decides what counts as a violation; [`safety_penalty`] turns the count
//!   into a factor value.
//! - **Unknown is not bad.** A factor with no samples is *unknown*, never
//!   assumed 0 or 1 ([`FactorScore::is_meaningful`]). An agent with no
//!   meaningful factors scores 0.0 meaning "we don't know" — explicitly NOT a
//!   penalty. Reputation feeds the planner as a **configurable weight** (see
//!   [`default_weights`]), never as a hard filter; hard filters stay trust +
//!   capability match.
//! - **Deterministic and bounded.** The store is a `BTreeMap` keyed
//!   `(agent_id, capability)`, ranking is tie-broken by `agent_id` ascending,
//!   and observation smoothing is a fixed EMA, so the same inputs always give
//!   the same store.
//!
//! The wire types (`AgentReputation`, `ReputationUpdate`) carry no runtime
//! state and derive serde so observations can travel between nodes and feed
//! the local store.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The reputation factor of one (agent, capability) pair.
///
/// Extensible by design: the enum is exhaustive today, but each variant maps
/// to an isolated calculation the planner can weight independently, so a new
/// factor never changes how existing ones are computed or explained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReputationFactor {
    /// Verified success rate (verified / total) on executed work.
    Reliability,
    /// Average verification verdict over the results that passed review.
    Quality,
    /// Execution latency percentile — lower is better, inverted to 0..=1.
    Latency,
    /// Availability of the agent inside the observation window.
    Uptime,
    /// Zero policy/sandbox/cryptographic violations. Only these count; network
    /// errors never feed this factor.
    Safety,
    /// Ratio of capability claims that were verified vs merely inferred.
    Provenance,
}

impl ReputationFactor {
    /// Human label used in [`AgentReputation::reasons`].
    fn label(&self) -> &'static str {
        match self {
            ReputationFactor::Reliability => "reliability",
            ReputationFactor::Quality => "quality",
            ReputationFactor::Latency => "latency",
            ReputationFactor::Uptime => "uptime",
            ReputationFactor::Safety => "safety",
            ReputationFactor::Provenance => "provenance",
        }
    }
}

/// One measured factor value for an (agent, capability) pair.
///
/// The value is clamped to `0..=1` by the constructor. `samples` is what makes
/// a score *known*: with zero samples the value is meaningless (it could be a
/// seed or a default), so decision code must consult [`FactorScore::is_meaningful`]
/// instead of trusting the raw number.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FactorScore {
    /// The factor value in `0..=1`, clamped by [`FactorScore::new`].
    pub value: f32,
    /// Number of observations behind this value (0 = unknown).
    pub samples: u64,
    /// Last update time (unix ms).
    pub updated_at_ms: u64,
}

impl FactorScore {
    /// A factor score; `value` is clamped to `0..=1` so callers can feed raw
    /// measurements without worrying about out-of-range inputs.
    pub fn new(value: f32, samples: u64, updated_at_ms: u64) -> Self {
        Self {
            value: value.clamp(0.0, 1.0),
            samples,
            updated_at_ms,
        }
    }

    /// Whether this score is backed by at least `min_samples` observations.
    /// With no samples a score is *unknown*, never assumed 0 or 1.
    pub fn is_meaningful(&self, min_samples: u64) -> bool {
        self.samples >= min_samples
    }
}

/// Default minimum sample count before a factor is treated as meaningful.
/// `1` is the honest default: a single real observation is information, while
/// zero samples means "unknown".
pub const DEFAULT_MIN_SAMPLES: u64 = 1;

/// Default factor weights for [`AgentReputation::score`] — a documented policy
/// table so the aggregate reads as policy, not magic numbers, and the planner
/// can substitute its own configurable weights when it consumes the factors.
/// The weights sum to 1.0.
pub fn default_weights() -> BTreeMap<ReputationFactor, f32> {
    [
        (ReputationFactor::Reliability, 0.30),
        (ReputationFactor::Quality, 0.25),
        (ReputationFactor::Latency, 0.15),
        (ReputationFactor::Uptime, 0.10),
        (ReputationFactor::Safety, 0.10),
        (ReputationFactor::Provenance, 0.10),
    ]
    .into_iter()
    .collect()
}

/// The explainable reputation of one (agent, capability) pair.
///
/// `capability` is a free-form string (typically the snake_case form of a hub
/// `CapabilityKind`, but kept as `String` for extensibility beyond the fixed
/// taxonomy). Factors are stored keyed by [`ReputationFactor`], and the
/// aggregate [`AgentReputation::score`] is a weighted projection over only the
/// factors that are currently meaningful.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReputation {
    /// The logical agent this reputation belongs to.
    pub agent_id: String,
    /// The capability this reputation is scoped to (snake_case or free-form).
    pub capability: String,
    /// Factor scores, keyed by factor type.
    pub factors: BTreeMap<ReputationFactor, FactorScore>,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
}

impl AgentReputation {
    /// A reputation with no factors yet (score unknown, not bad).
    pub fn new(agent_id: impl Into<String>, capability: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            capability: capability.into(),
            factors: BTreeMap::new(),
            created_at_ms: 0,
        }
    }

    /// Records or replaces one factor score.
    pub fn set_factor(&mut self, factor: ReputationFactor, score: FactorScore) {
        self.factors.insert(factor, score);
    }

    /// Looks up one factor's score.
    pub fn factor(&self, factor: ReputationFactor) -> Option<&FactorScore> {
        self.factors.get(&factor)
    }

    /// Looks up one factor's raw value.
    pub fn factor_value(&self, factor: ReputationFactor) -> Option<f32> {
        self.factors.get(&factor).map(|s| s.value)
    }

    /// Whether any factor is backed by at least `min_samples` observations.
    pub fn is_meaningful(&self, min_samples: u64) -> bool {
        self.factors
            .values()
            .any(|score| score.is_meaningful(min_samples))
    }

    /// The aggregate score: a weighted average over *meaningful* factors only.
    ///
    /// If NO factor is meaningful the result is `0.0` — which here means
    /// **unknown, not bad**. Callers that use this as a planner weight should
    /// treat a 0.0 from an unknown reputation as "no signal" rather than
    /// "penalize the agent", and rely on [`is_meaningful`][Self::is_meaningful]
    /// to distinguish the two.
    pub fn score(&self) -> f32 {
        self.score_with_min(DEFAULT_MIN_SAMPLES)
    }

    fn score_with_min(&self, min_samples: u64) -> f32 {
        let weights = default_weights();
        let mut weighted_sum = 0.0f64;
        let mut weight_total = 0.0f64;
        for (factor, score) in &self.factors {
            if !score.is_meaningful(min_samples) {
                continue;
            }
            let w = weights.get(factor).copied().unwrap_or(0.0) as f64;
            weighted_sum += w * score.value as f64;
            weight_total += w;
        }
        if weight_total == 0.0 {
            return 0.0;
        }
        (weighted_sum / weight_total).clamp(0.0, 1.0) as f32
    }

    /// Human-readable per-factor lines ("factor: value (n samples)") for
    /// dashboards and audit — the *explainability* half of the design.
    pub fn reasons(&self) -> Vec<String> {
        self.factors
            .iter()
            .map(|(factor, score)| {
                format!(
                    "{}: {:.2} ({} samples)",
                    factor.label(),
                    score.value,
                    score.samples
                )
            })
            .collect()
    }
}

/// One observation feeding a reputation factor.
///
/// Wire-safe so remote verifiers/coordinators can submit observations of an
/// agent's work without owning the store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReputationUpdate {
    /// The logical agent being scored.
    pub agent_id: String,
    /// The capability the observation applies to.
    pub capability: String,
    /// Which factor this observation measures.
    pub factor: ReputationFactor,
    /// The measured value in `0..=1` (clamped on ingest).
    pub value: f32,
    /// Confidence weight of this observation (defaults to 1).
    #[serde(default = "default_sample_weight")]
    pub sample_weight: u64,
    /// When the observation happened (unix ms).
    pub observed_at_ms: u64,
}

fn default_sample_weight() -> u64 {
    1
}

impl ReputationUpdate {
    /// A weight-1 observation.
    pub fn new(
        agent_id: impl Into<String>,
        capability: impl Into<String>,
        factor: ReputationFactor,
        value: f32,
        observed_at_ms: u64,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            capability: capability.into(),
            factor,
            value,
            sample_weight: 1,
            observed_at_ms,
        }
    }

    /// Sets the confidence weight of this observation.
    pub fn sample_weight(mut self, weight: u64) -> Self {
        self.sample_weight = weight;
        self
    }
}

/// EMA smoothing constant for a weight-1 observation. Heavier observations
/// move the estimate more: alpha scales linearly with `sample_weight` and is
/// clamped to 1.0, so a weight ≥ 5 observation replaces the estimate outright.
fn ema_alpha(sample_weight: u64) -> f64 {
    (0.2 * sample_weight as f64).min(1.0)
}

/// The deterministic, bounded reputation store, keyed `(agent_id, capability)`.
///
/// This is the pure bookkeeping half (mirroring [`crate::registry::AgentRegistry`]);
/// the runtime half that ships observations over the wire lives elsewhere.
#[derive(Debug, Clone, Default)]
pub struct ReputationStore {
    reputations: BTreeMap<(String, String), AgentReputation>,
}

impl ReputationStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingests one observation, upserting the (agent, capability) reputation.
    ///
    /// The factor is updated with an exponential moving average
    /// (`new = old * (1 - alpha) + value * alpha`, `alpha` from
    /// [`ema_alpha`]), `samples` accumulates by `sample_weight`, and the value
    /// is clamped to `0..=1`. Always returns `Some` (the updated reputation)
    /// today; the `Option` keeps room for a future refusal policy (e.g.
    /// rejecting observations older than a horizon) without breaking callers.
    pub fn observe(&mut self, update: ReputationUpdate) -> Option<AgentReputation> {
        let ReputationUpdate {
            agent_id,
            capability,
            factor,
            value,
            sample_weight,
            observed_at_ms,
        } = update;
        let key = (agent_id.clone(), capability.clone());
        let entry = self
            .reputations
            .entry(key)
            .or_insert_with(|| AgentReputation {
                agent_id: agent_id.clone(),
                capability: capability.clone(),
                factors: BTreeMap::new(),
                created_at_ms: observed_at_ms,
            });

        let alpha = ema_alpha(sample_weight);
        let new_value = match entry.factors.get(&factor) {
            Some(existing) => {
                let old = existing.value as f64;
                (old * (1.0 - alpha) + value as f64 * alpha).clamp(0.0, 1.0) as f32
            }
            // First observation seeds the factor with the observed value.
            None => value.clamp(0.0, 1.0),
        };
        let prev_samples = entry.factors.get(&factor).map(|s| s.samples).unwrap_or(0);
        entry.factors.insert(
            factor,
            FactorScore::new(new_value, prev_samples + sample_weight, observed_at_ms),
        );

        Some(entry.clone())
    }

    /// Looks up one (agent, capability) reputation.
    pub fn get(&self, agent_id: &str, capability: &str) -> Option<&AgentReputation> {
        self.reputations
            .get(&(agent_id.to_string(), capability.to_string()))
    }

    /// All reputations across every (agent, capability), sorted by
    /// (agent_id, capability) — deterministic, for dashboards/export.
    pub fn all(&self) -> Vec<AgentReputation> {
        self.reputations.values().cloned().collect()
    }

    /// All reputations of one agent, sorted by capability (deterministic).
    pub fn for_agent(&self, agent_id: &str) -> Vec<AgentReputation> {
        let mut reps: Vec<AgentReputation> = self
            .reputations
            .iter()
            .filter(|((a, _), _)| a == agent_id)
            .map(|(_, rep)| rep.clone())
            .collect();
        reps.sort_by(|a, b| a.capability.cmp(&b.capability));
        reps
    }

    /// The best-known agent for a capability among reputations that meet
    /// `min_samples`, as `(agent_id, aggregate score)`. Ties break
    /// deterministically on `agent_id` ascending. Agents whose reputation is
    /// unknown (nothing meaningful) never compete.
    pub fn best_for_capability(&self, capability: &str, min_samples: u64) -> Option<(String, f32)> {
        self.reputations
            .values()
            .filter(|rep| rep.capability == capability && rep.is_meaningful(min_samples))
            .map(|rep| (rep.agent_id.clone(), rep.score_with_min(min_samples)))
            .max_by(|(a_id, a_score), (b_id, b_score)| {
                a_score
                    .partial_cmp(b_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    // Equal scores: prefer the smaller agent_id.
                    .then_with(|| b_id.cmp(a_id))
            })
    }

    /// All capabilities seen in the store, sorted and deduplicated.
    pub fn capabilities(&self) -> Vec<String> {
        let mut caps: Vec<String> = self.reputations.keys().map(|(_, c)| c.clone()).collect();
        caps.sort();
        caps.dedup();
        caps
    }

    /// Number of (agent, capability) reputations tracked.
    pub fn count(&self) -> usize {
        self.reputations.len()
    }

    /// Drops reputations where no factor is meaningful at `min_samples` — an
    /// unknown reputation is removed, not scored as bad. Returns the number
    /// of reputations pruned.
    pub fn prune_below(&mut self, min_samples: u64) -> usize {
        let before = self.reputations.len();
        self.reputations
            .retain(|_, rep| rep.is_meaningful(min_samples));
        before - self.reputations.len()
    }
}

/// Maps a count of policy/cryptographic violations to a safety factor value.
///
/// `0` violations → `1.0`; each violation halves the remaining headroom
/// (`1.0 - 0.5 * violations`), clamped to `0.0`. Only the *caller* decides
/// what counts as a violation — but the fabric invariant is absolute: network
/// errors and execution failures NEVER feed this factor, so a flaky link can
/// never damage an agent's safety reputation.
pub fn safety_penalty(violations: u64) -> f32 {
    if violations == 0 {
        return 1.0;
    }
    (1.0 - 0.5 * violations as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reliability(agent: &str, value: f32, weight: u64, at: u64) -> ReputationUpdate {
        ReputationUpdate::new(agent, "ocr", ReputationFactor::Reliability, value, at)
            .sample_weight(weight)
    }

    #[test]
    fn factor_score_clamps_to_unit_range_and_needs_samples() {
        let over = FactorScore::new(1.7, 5, 100);
        assert_eq!(over.value, 1.0);
        let under = FactorScore::new(-0.3, 5, 100);
        assert_eq!(under.value, 0.0);
        let empty = FactorScore::new(0.5, 0, 100);
        assert!(!empty.is_meaningful(1));
        assert!(empty.is_meaningful(0));
        let sampled = FactorScore::new(0.5, 4, 100);
        assert!(sampled.is_meaningful(4));
        assert!(!sampled.is_meaningful(5));
    }

    #[test]
    fn unknown_reputation_scores_zero_and_is_not_a_penalty() {
        let rep = AgentReputation::new("dca-a:ocr", "ocr");
        assert!(!rep.is_meaningful(1));
        assert_eq!(rep.score(), 0.0);
        assert!(rep.reasons().is_empty());
    }

    #[test]
    fn aggregate_score_is_weighted_average_over_meaningful_factors() {
        let mut rep = AgentReputation::new("dca-a:ocr", "ocr");
        rep.set_factor(ReputationFactor::Reliability, FactorScore::new(1.0, 10, 1));
        rep.set_factor(ReputationFactor::Quality, FactorScore::new(0.5, 10, 1));
        rep.set_factor(ReputationFactor::Latency, FactorScore::new(0.8, 10, 1));
        rep.set_factor(ReputationFactor::Uptime, FactorScore::new(0.9, 10, 1));
        rep.set_factor(ReputationFactor::Safety, FactorScore::new(1.0, 10, 1));
        rep.set_factor(ReputationFactor::Provenance, FactorScore::new(0.6, 10, 1));
        let expected = 0.30 * 1.0 + 0.25 * 0.5 + 0.15 * 0.8 + 0.10 * 0.9 + 0.10 * 1.0 + 0.10 * 0.6;
        assert!((rep.score() - expected).abs() < 1e-6);
    }

    #[test]
    fn aggregate_score_renormalizes_when_some_factors_are_unknown() {
        let mut rep = AgentReputation::new("dca-a:ocr", "ocr");
        // Unknown (0 samples) factor must not drag the aggregate down.
        rep.set_factor(ReputationFactor::Reliability, FactorScore::new(0.0, 0, 1));
        rep.set_factor(ReputationFactor::Quality, FactorScore::new(1.0, 3, 1));
        rep.set_factor(ReputationFactor::Latency, FactorScore::new(1.0, 3, 1));
        rep.set_factor(ReputationFactor::Uptime, FactorScore::new(1.0, 3, 1));
        rep.set_factor(ReputationFactor::Safety, FactorScore::new(1.0, 3, 1));
        rep.set_factor(ReputationFactor::Provenance, FactorScore::new(1.0, 3, 1));
        // The 0.70 of weight behind meaningful factors is renormalized to 1.0.
        assert!((rep.score() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ema_observe_moves_score_toward_observations_and_accumulates_samples() {
        let mut store = ReputationStore::new();
        // First observation seeds the factor.
        store.observe(ReputationUpdate::new(
            "dca-a:ocr",
            "ocr",
            ReputationFactor::Reliability,
            1.0,
            1000,
        ));
        let rep = store.get("dca-a:ocr", "ocr").unwrap();
        assert_eq!(
            rep.factor_value(ReputationFactor::Reliability).unwrap(),
            1.0
        );
        assert_eq!(rep.factors[&ReputationFactor::Reliability].samples, 1);
        // A low observation drags the estimate down by alpha = 0.2.
        store.observe(ReputationUpdate::new(
            "dca-a:ocr",
            "ocr",
            ReputationFactor::Reliability,
            0.0,
            1001,
        ));
        let rep = store.get("dca-a:ocr", "ocr").unwrap();
        let v = rep.factor_value(ReputationFactor::Reliability).unwrap();
        assert!((v - 0.8).abs() < 1e-6);
        assert_eq!(rep.factors[&ReputationFactor::Reliability].samples, 2);
        // Repeated observations converge toward the new value and stay in [0,1].
        for i in 0..50u64 {
            store.observe(ReputationUpdate::new(
                "dca-a:ocr",
                "ocr",
                ReputationFactor::Reliability,
                0.0,
                2000 + i,
            ));
        }
        let v = store
            .get("dca-a:ocr", "ocr")
            .unwrap()
            .factor_value(ReputationFactor::Reliability)
            .unwrap();
        assert!((0.0..0.01).contains(&v));
    }

    #[test]
    fn heavier_observations_move_the_estimate_more() {
        let mut store = ReputationStore::new();
        store.observe(ReputationUpdate::new(
            "a",
            "ocr",
            ReputationFactor::Reliability,
            1.0,
            1,
        ));
        // sample_weight 2 -> alpha 0.4: 1.0 moves to 0.6.
        store.observe(reliability("a", 0.0, 2, 2));
        let v = store
            .get("a", "ocr")
            .unwrap()
            .factor_value(ReputationFactor::Reliability)
            .unwrap();
        assert!((v - 0.6).abs() < 1e-6);
        // sample_weight >= 5 -> alpha 1.0: replaces the estimate.
        store.observe(reliability("a", 0.9, 5, 3));
        let v = store
            .get("a", "ocr")
            .unwrap()
            .factor_value(ReputationFactor::Reliability)
            .unwrap();
        assert!((v - 0.9).abs() < 1e-6);
        assert_eq!(
            store.get("a", "ocr").unwrap().factors[&ReputationFactor::Reliability].samples,
            8
        );
    }

    #[test]
    fn store_observations_upsert_and_query() {
        let mut store = ReputationStore::new();
        assert_eq!(store.count(), 0);
        store.observe(ReputationUpdate::new(
            "a",
            "ocr",
            ReputationFactor::Quality,
            0.5,
            100,
        ));
        assert_eq!(store.count(), 1);
        let first = store.get("a", "ocr").expect("stored");
        assert_eq!(first.agent_id, "a");
        assert_eq!(first.capability, "ocr");
        assert_eq!(first.created_at_ms, 100);
        // A second observation updates the same (agent, capability), not a new row.
        store.observe(ReputationUpdate::new(
            "a",
            "ocr",
            ReputationFactor::Reliability,
            0.9,
            200,
        ));
        assert_eq!(store.count(), 1);
        let rep = store.get("a", "ocr").unwrap();
        assert!(rep.factor_value(ReputationFactor::Quality).is_some());
        assert!(rep.factor_value(ReputationFactor::Reliability).is_some());
        assert!(store.get("a", "coding").is_none());
    }

    #[test]
    fn for_agent_returns_reputations_sorted_by_capability() {
        let mut store = ReputationStore::new();
        store.observe(ReputationUpdate::new(
            "a",
            "coding",
            ReputationFactor::Reliability,
            0.5,
            1,
        ));
        store.observe(ReputationUpdate::new(
            "a",
            "ocr",
            ReputationFactor::Reliability,
            0.8,
            1,
        ));
        store.observe(ReputationUpdate::new(
            "b",
            "ocr",
            ReputationFactor::Reliability,
            0.7,
            1,
        ));
        let reps = store.for_agent("a");
        assert_eq!(reps.len(), 2);
        assert_eq!(reps[0].capability, "coding");
        assert_eq!(reps[1].capability, "ocr");
    }

    #[test]
    fn best_for_capability_is_deterministic_and_respects_min_samples() {
        let mut store = ReputationStore::new();
        store.observe(ReputationUpdate::new(
            "a",
            "ocr",
            ReputationFactor::Reliability,
            0.5,
            1,
        ));
        store.observe(ReputationUpdate::new(
            "b",
            "ocr",
            ReputationFactor::Reliability,
            0.9,
            1,
        ));
        let best = store.best_for_capability("ocr", 1);
        assert_eq!(best, Some(("b".to_string(), 0.9)));
        // Tie on score -> agent_id ascending wins.
        store.observe(ReputationUpdate::new(
            "c",
            "ocr",
            ReputationFactor::Reliability,
            0.9,
            1,
        ));
        let best = store.best_for_capability("ocr", 1);
        assert_eq!(best, Some(("b".to_string(), 0.9)));
        // Below the sample floor nothing competes (unknown is not a candidate).
        assert_eq!(store.best_for_capability("ocr", 2), None);
        // Other capabilities do not leak in.
        assert_eq!(store.best_for_capability("coding", 1), None);
    }

    #[test]
    fn capabilities_are_sorted_and_deduped() {
        let mut store = ReputationStore::new();
        store.observe(ReputationUpdate::new(
            "a",
            "ocr",
            ReputationFactor::Reliability,
            0.5,
            1,
        ));
        store.observe(ReputationUpdate::new(
            "b",
            "coding",
            ReputationFactor::Reliability,
            0.5,
            1,
        ));
        store.observe(ReputationUpdate::new(
            "c",
            "ocr",
            ReputationFactor::Reliability,
            0.5,
            1,
        ));
        assert_eq!(
            store.capabilities(),
            vec!["coding".to_string(), "ocr".to_string()]
        );
    }

    #[test]
    fn prune_below_drops_only_unknown_reputations() {
        let mut store = ReputationStore::new();
        // Known: two observations -> 2 samples, survives the floor.
        store.observe(ReputationUpdate::new(
            "known",
            "ocr",
            ReputationFactor::Reliability,
            0.5,
            1,
        ));
        store.observe(ReputationUpdate::new(
            "known",
            "ocr",
            ReputationFactor::Reliability,
            0.6,
            2,
        ));
        // Unknown: factors exist but were never observed -> samples 0.
        let mut unknown = AgentReputation::new("unknown", "coding");
        unknown.set_factor(ReputationFactor::Safety, FactorScore::new(1.0, 0, 1));
        store
            .reputations
            .insert(("unknown".into(), "coding".into()), unknown);
        let mut unknown2 = AgentReputation::new("unknown2", "vision");
        unknown2.set_factor(ReputationFactor::Quality, FactorScore::new(0.8, 0, 1));
        store
            .reputations
            .insert(("unknown2".into(), "vision".into()), unknown2);

        assert_eq!(store.count(), 3);
        let dropped = store.prune_below(2);
        assert_eq!(dropped, 2);
        assert_eq!(store.count(), 1);
        assert!(store.get("known", "ocr").is_some());
        assert!(store.get("unknown", "coding").is_none());
        assert!(store.get("unknown2", "vision").is_none());
    }

    #[test]
    fn safety_penalty_counts_only_policy_or_crypto_violations() {
        assert_eq!(safety_penalty(0), 1.0);
        assert_eq!(safety_penalty(1), 0.5);
        assert_eq!(safety_penalty(2), 0.0);
        assert_eq!(safety_penalty(100), 0.0);
    }

    #[test]
    fn reputation_round_trips_over_wire() {
        let mut rep = AgentReputation::new("dca-a:ocr", "ocr");
        rep.set_factor(
            ReputationFactor::Reliability,
            FactorScore::new(0.93, 12, 1234),
        );
        rep.set_factor(ReputationFactor::Safety, FactorScore::new(1.0, 3, 1234));
        let json = serde_json::to_string(&rep).unwrap();
        let back: AgentReputation = serde_json::from_str(&json).unwrap();
        assert_eq!(rep, back);
        // snake_case factor names on the wire.
        assert!(json.contains("\"reliability\""));
        assert!(json.contains("\"capability\":\"ocr\""));

        let update =
            ReputationUpdate::new("a", "ocr", ReputationFactor::Latency, 0.7, 9).sample_weight(2);
        let json = serde_json::to_string(&update).unwrap();
        let back: ReputationUpdate = serde_json::from_str(&json).unwrap();
        assert_eq!(update, back);
        assert_eq!(back.sample_weight, 2);
    }

    #[test]
    fn reputation_update_sample_weight_defaults_to_one_on_the_wire() {
        let json = r#"{"agent_id":"a","capability":"ocr","factor":"uptime","value":0.6,"observed_at_ms":5}"#;
        let back: ReputationUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(back.sample_weight, 1);
    }

    #[test]
    fn reasons_are_non_empty_and_human_readable() {
        let mut rep = AgentReputation::new("dca-a:ocr", "ocr");
        rep.set_factor(
            ReputationFactor::Reliability,
            FactorScore::new(0.8, 12, 100),
        );
        rep.set_factor(ReputationFactor::Quality, FactorScore::new(0.5, 7, 100));
        let reasons = rep.reasons();
        assert_eq!(reasons.len(), 2);
        assert!(
            reasons
                .iter()
                .all(|line| line.contains(':') && line.contains("samples"))
        );
        assert!(reasons[0].starts_with("reliability"));
    }

    #[test]
    fn default_weights_are_a_documented_policy_table() {
        let weights = default_weights();
        let total: f32 = weights.values().sum();
        assert!((total - 1.0).abs() < 1e-6);
        assert_eq!(weights.len(), 6);
    }
}
