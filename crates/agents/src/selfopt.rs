//! P10 self-optimization: the pure policy loop that proposes how to run the
//! fabric better under its operating constraints.
//!
//! # Why this module exists
//!
//! A distributed fabric does not have a single operator watching every
//! resource. P10 gives the node a *decision core*: it ingests measured
//! observations of how the fabric is actually running and emits concrete,
//! ranked `OptimizationSuggestion`s. It is deliberately a PURE policy loop —
//! it only *decides* what to suggest; the runtime applies the suggestions.
//! Keeping it pure (no I/O, no async) means it is trivially testable with
//! synthetic inputs, deterministic (sorted output), and safe to run on the
//! control plane without ever blocking a request.
//!
//! # The threat model / why constraints are folded in
//!
//! An optimization that trades away *reliability*, *security* or *privacy*
//! to squeeze out a little more capacity is a net loss for a fabric whose
//! whole premise is trust. So those three dimensions are treated as **hard
//! ceilings**: exceeding their `max_value` caps the suggestion score and
//! marks it high-risk. `Quality`, `Cost` and `Latency` are **soft targets** —
//! worth tuning, but overshooting them is recoverable.

use serde::{Deserialize, Serialize};

/// Which aspect of the fabric an observation (or a suggestion) concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationDimension {
    /// Where models are placed across nodes.
    ModelPlacement,
    /// How many workers the fabric runs.
    WorkerCount,
    /// How many logical agents run concurrently.
    AgentCount,
    /// Which tools are exposed to agents.
    ActiveTools,
    /// Whether tasks are fanned out (split) across workers.
    TaskSplit,
    /// How much inter-agent collaboration happens.
    Collaboration,
    /// How aggressively compute resources are used.
    ComputeUsage,
}

/// Which direction a suggestion wants to nudge the dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Raise the amount / usage of the dimension.
    Increase,
    /// Lower the amount / usage of the dimension.
    Decrease,
    /// Redistribute without changing the total (e.g. rebalance placement).
    Rebalance,
}

/// How risky a suggestion is to apply. `Low < Medium < High` (declaration
/// order drives the derived ordering, so risk can be compared/sorted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

/// A single measured signal about one dimension, e.g. `ModelPlacement`
/// value `0.85` meaning the current placement is well-fit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationObservation {
    /// The dimension being measured.
    pub dimension: OptimizationDimension,
    /// Normalized measurement in `[0, 1]` (higher = "more" of the dimension).
    pub value: f64,
    /// Confidence/importance of this measurement, clamped to `[0, 1]`.
    #[serde(default = "default_weight")]
    pub weight: f64,
    /// Wall-clock timestamp (ms) of when the observation was taken.
    pub observed_at_ms: u64,
}

fn default_weight() -> f64 {
    1.0
}

impl OptimizationObservation {
    /// Builds an observation, clamping `weight` into `[0, 1]`.
    pub fn new(dimension: OptimizationDimension, value: f64, weight: f64, observed_at_ms: u64) -> Self {
        Self {
            dimension,
            value,
            weight: weight.clamp(0.0, 1.0),
            observed_at_ms,
        }
    }
}

/// The kind of an operating constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    /// Output quality floor (soft target).
    Quality,
    /// Resource cost budget (soft target).
    Cost,
    /// Response latency budget (soft target).
    Latency,
    /// Availability/dependability floor (hard ceiling).
    Reliability,
    /// Security posture floor (hard ceiling).
    Security,
    /// Privacy posture floor (hard ceiling).
    Privacy,
}

/// A named constraint: a max value the fabric should not exceed for `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Constraint {
    /// Which dimension of operation this constrains.
    pub kind: ConstraintKind,
    /// The ceiling on the (normalized) measured value for this kind.
    pub max_value: f64,
    /// Relative importance of this constraint when folding into scores.
    pub weight: f64,
}

impl Constraint {
    /// Whether this constraint is a hard ceiling (overshoot is risky) rather
    /// than a soft target (overshoot is merely suboptimal).
    pub fn is_hard_ceiling(&self) -> bool {
        matches!(
            self.kind,
            ConstraintKind::Reliability | ConstraintKind::Security | ConstraintKind::Privacy
        )
    }
}

/// A proposed optimization, ranked by `score`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationSuggestion {
    /// The dimension this suggestion changes.
    pub dimension: OptimizationDimension,
    /// Which direction to move the dimension.
    pub direction: Direction,
    /// Benefit under constraints in `[0, 1]`; higher is more beneficial.
    pub score: f64,
    /// Short human-readable reason.
    pub reason: String,
    /// Rough cost of applying the change (arbitrary unit).
    pub cost_estimate: f64,
    /// How risky the change is.
    pub risk: RiskLevel,
    /// Concrete actions the runtime could take.
    pub actions: Vec<String>,
}

/// Thresholds used by [`SelfOptimizer::evaluate`].
const LOW_MEAN: f64 = 0.35;
const HIGH_MEAN: f64 = 0.70;

/// The self-optimization policy engine.
#[derive(Debug, Clone)]
pub struct SelfOptimizer {
    constraints: Vec<Constraint>,
    min_observations: u32,
}

impl Default for SelfOptimizer {
    fn default() -> Self {
        Self {
            constraints: Vec::new(),
            min_observations: 2,
        }
    }
}

impl SelfOptimizer {
    /// Adds a constraint to fold into subsequent evaluations.
    pub fn add_constraint(&mut self, c: Constraint) {
        self.constraints.push(c);
    }

    /// Sets the minimum number of observations a dimension needs before a
    /// suggestion is produced for it.
    pub fn with_min_observations(mut self, n: u32) -> Self {
        self.min_observations = n;
        self
    }

    /// Evaluates observations and produces ranked, deterministic suggestions.
    ///
    /// Observations are grouped by dimension; a dimension must have at least
    /// `min_observations` to produce a suggestion. The weighted mean value
    /// drives the direction:
    /// - mean `< 0.35` → `Decrease` ("under-utilized")
    /// - mean `> 0.70` → `Increase` ("saturated")
    /// - otherwise → `Rebalance`
    ///
    /// The base score is a distance from the neutral band, normalized into
    /// `[0, 1]`, then folded against the constraints (see `score_with_constraints`).
    pub fn evaluate(&self, observations: &[OptimizationObservation]) -> Vec<OptimizationSuggestion> {
        let mut by_dimension: Vec<(OptimizationDimension, Vec<&OptimizationObservation>)> =
            Vec::new();
        for obs in observations {
            if let Some(entry) = by_dimension.iter_mut().find(|(d, _)| *d == obs.dimension) {
                entry.1.push(obs);
            } else {
                by_dimension.push((obs.dimension, vec![obs]));
            }
        }

        let mut suggestions = Vec::new();
        for (dimension, group) in by_dimension {
            if (group.len() as u32) < self.min_observations {
                continue;
            }
            let mean = weighted_mean(&group);
            suggestions.push(self.suggest_for_dimension(dimension, mean));
        }

        // Deterministic: score desc, then dimension asc.
        suggestions.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.dimension.cmp(&b.dimension))
        });
        suggestions
    }

    /// Builds a single suggestion for one dimension from its weighted mean.
    fn suggest_for_dimension(&self, dimension: OptimizationDimension, mean: f64) -> OptimizationSuggestion {
        let (direction, raw_score, reason) = if mean < LOW_MEAN {
            // Normalize the deficit (LOW_MEAN - mean) ∈ (0, LOW_MEAN] into [0, 1].
            (
                Direction::Decrease,
                ((LOW_MEAN - mean) / LOW_MEAN).clamp(0.0, 1.0),
                "under-utilized",
            )
        } else if mean > HIGH_MEAN {
            // Normalize the surplus (mean - HIGH_MEAN) ∈ (0, 1 - HIGH_MEAN] into [0, 1].
            (
                Direction::Increase,
                ((mean - HIGH_MEAN) / (1.0 - HIGH_MEAN)).clamp(0.0, 1.0),
                "saturated",
            )
        } else {
            // Mid band: rebalance, moderate score (distance from band center).
            let center = (LOW_MEAN + HIGH_MEAN) / 2.0;
            let distance = (mean - center).abs() / (HIGH_MEAN - center);
            (Direction::Rebalance, distance.clamp(0.0, 1.0), "rebalance")
        };

        let (score, risk, actions) = self.fold_constraints(dimension, mean, raw_score);
        let reason = reason.to_string();
        OptimizationSuggestion {
            dimension,
            direction,
            score,
            reason,
            cost_estimate: cost_estimate_for(dimension, direction),
            risk,
            actions,
        }
    }

    /// Folds operating constraints into the raw score.
    ///
    /// `soft_max` is the largest weight among soft-target constraints; if the
    /// observation exceeds it the suggestion is dampened proportionally.
    /// `hard_breached` is true when the observation exceeds a hard-ceiling
    /// constraint (Reliability/Security/Privacy) — overshoot there caps the
    /// score at 0.5 and raises risk to `High`.
    fn fold_constraints(
        &self,
        dimension: OptimizationDimension,
        mean: f64,
        raw_score: f64,
    ) -> (f64, RiskLevel, Vec<String>) {
        let mut soft_max: f64 = 0.0;
        let mut hard_breached = false;

        for c in &self.constraints {
            if c.is_hard_ceiling() {
                if mean > c.max_value {
                    hard_breached = true;
                }
            } else if mean > c.max_value {
                soft_max = soft_max.max(c.weight);
            }
        }

        // Dampen by any soft-target overshoot, scaled by its weight.
        let mut score = raw_score * (1.0 - soft_max.clamp(0.0, 1.0));
        let mut risk = RiskLevel::Low;
        let mut actions = Vec::new();

        if hard_breached {
            // Hard ceiling overshoot: this dimension is endangering a
            // non-negotiable (reliability/security/privacy). Cap the score and
            // flag high risk; never reward a change that breaches the ceiling.
            score = (score * 0.5).min(0.5);
            risk = RiskLevel::High;
            actions.push(format!(
                "breaches a hard ceiling ({} constraint); verify before applying",
                "reliability/security/privacy"
            ));
        } else if soft_max > 0.0 {
            risk = RiskLevel::Medium;
            actions.push("overshoots a soft target constraint".to_string());
        } else if score >= 0.5 {
            actions.push("low-risk, constraint-respecting change".to_string());
        }

        actions.push(default_action_for(dimension, hard_breached));
        (score.clamp(0.0, 1.0), risk, actions)
    }

    /// Convenience: suggests how to tune `ComputeUsage` based on memory use.
    ///
    /// Returns `None` when the fabric is comfortably inside its budget. The
    /// returned suggestion, when present, targets the [`OptimizationDimension::ComputeUsage`]
    /// dimension with `cost_estimate` 0.
    pub fn suggest_compute(
        &self,
        _observations: &[OptimizationObservation],
        budget_mb: u64,
        used_mb: u64,
    ) -> Option<OptimizationSuggestion> {
        if budget_mb == 0 {
            return None;
        }
        let ratio = used_mb as f64 / budget_mb as f64;

        let suggestion = if ratio < 0.5 {
            OptimizationSuggestion {
                dimension: OptimizationDimension::ComputeUsage,
                direction: Direction::Decrease,
                score: 1.0 - ratio / 0.5, // 0..1, higher when very under-used
                reason: "under-used compute".to_string(),
                cost_estimate: 0.0,
                risk: RiskLevel::Low,
                actions: vec![
                    "reduce compute allocation".to_string(),
                    default_action_for(OptimizationDimension::ComputeUsage, false),
                ],
            }
        } else if ratio > 0.9 {
            OptimizationSuggestion {
                dimension: OptimizationDimension::ComputeUsage,
                direction: Direction::Rebalance,
                score: (ratio - 0.9) / 0.1, // 0..1, higher when nearer the cap
                reason: "near budget".to_string(),
                cost_estimate: 0.0,
                risk: RiskLevel::Medium,
                actions: vec![
                    "keep current compute allocation".to_string(),
                    default_action_for(OptimizationDimension::ComputeUsage, true),
                ],
            }
        } else {
            return None;
        };

        Some(suggestion)
    }
}

fn weighted_mean(group: &[&OptimizationObservation]) -> f64 {
    let mut sum: f64 = 0.0;
    let mut wsum: f64 = 0.0;
    for o in group {
        sum += o.value * o.weight;
        wsum += o.weight;
    }
    if wsum == 0.0 {
        0.0
    } else {
        sum / wsum
    }
}

fn cost_estimate_for(dimension: OptimizationDimension, direction: Direction) -> f64 {
    // Rebalancing and splitting are more expensive than monotonic bumps.
    let base = match dimension {
        OptimizationDimension::ModelPlacement | OptimizationDimension::TaskSplit => 8.0,
        OptimizationDimension::WorkerCount | OptimizationDimension::AgentCount => 6.0,
        _ => 4.0,
    };
    if direction == Direction::Rebalance {
        base * 1.5
    } else {
        base
    }
}

fn default_action_for(dimension: OptimizationDimension, hard_breached: bool) -> String {
    if hard_breached {
        return "re-audit before any change".to_string();
    }
    match dimension {
        OptimizationDimension::ModelPlacement => "re-evaluate model placement".to_string(),
        OptimizationDimension::WorkerCount => "adjust worker count".to_string(),
        OptimizationDimension::AgentCount => "adjust concurrent agent count".to_string(),
        OptimizationDimension::ActiveTools => "re-tune active tool set".to_string(),
        OptimizationDimension::TaskSplit => "re-tune task fan-out".to_string(),
        OptimizationDimension::Collaboration => "re-tune collaboration depth".to_string(),
        OptimizationDimension::ComputeUsage => "re-tune compute usage".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: a single observation with default weight 1.
    fn obs(dim: OptimizationDimension, value: f64) -> OptimizationObservation {
        OptimizationObservation::new(dim, value, 1.0, 0)
    }

    /// Helper: a plain hard-ceiling constraint.
    fn hard_constraint(kind: ConstraintKind, max: f64) -> Constraint {
        Constraint {
            kind,
            max_value: max,
            weight: 1.0,
        }
    }

    #[test]
    fn below_min_observations_produces_no_suggestion() {
        let opt = SelfOptimizer::default().with_min_observations(3);
        let suggestions = opt.evaluate(&[obs(OptimizationDimension::WorkerCount, 0.8)]);
        assert!(suggestions.is_empty());
    }

    #[test]
    fn low_mean_suggests_decrease() {
        let opt = SelfOptimizer::default();
        let suggestions = opt.evaluate(&[
            obs(OptimizationDimension::WorkerCount, 0.1),
            obs(OptimizationDimension::WorkerCount, 0.2),
        ]);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].direction, Direction::Decrease);
        assert!(suggestions[0].reason.contains("under-utilized"));
    }

    #[test]
    fn high_mean_suggests_increase() {
        let opt = SelfOptimizer::default();
        let suggestions = opt.evaluate(&[
            obs(OptimizationDimension::AgentCount, 0.95),
            obs(OptimizationDimension::AgentCount, 0.9),
        ]);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].direction, Direction::Increase);
        assert!(suggestions[0].reason.contains("saturated"));
    }

    #[test]
    fn mid_mean_suggests_rebalance() {
        let opt = SelfOptimizer::default();
        let suggestions = opt.evaluate(&[
            obs(OptimizationDimension::TaskSplit, 0.5),
            obs(OptimizationDimension::TaskSplit, 0.55),
        ]);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].direction, Direction::Rebalance);
        assert!(suggestions[0].reason.contains("rebalance"));
    }

    #[test]
    fn heavy_weight_observation_dominates() {
        // Two observations: the 0.9-weight low value outweighs the 0.1-weight
        // high value, dragging the weighted mean below the Decrease threshold.
        let low = OptimizationObservation::new(OptimizationDimension::ComputeUsage, 0.1, 0.9, 0);
        let high = OptimizationObservation::new(OptimizationDimension::ComputeUsage, 0.9, 0.1, 0);
        let opt = SelfOptimizer::default();
        let suggestions = opt.evaluate(&[low, high]);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].direction, Direction::Decrease);
    }

    #[test]
    fn exceeded_security_constraint_caps_score_and_raises_risk() {
        let mut opt = SelfOptimizer::default();
        opt.add_constraint(hard_constraint(ConstraintKind::Security, 0.6));
        let suggestions = opt.evaluate(&[
            obs(OptimizationDimension::ModelPlacement, 0.95),
            obs(OptimizationDimension::ModelPlacement, 0.9),
        ]);
        let s = &suggestions[0];
        assert_eq!(s.risk, RiskLevel::High);
        assert!(s.score <= 0.5);
    }

    #[test]
    fn suggestions_sorted_by_score_then_dimension() {
        let opt = SelfOptimizer::default();
        let suggestions = opt.evaluate(&[
            obs(OptimizationDimension::ComputeUsage, 0.05),
            obs(OptimizationDimension::ComputeUsage, 0.05),
            obs(OptimizationDimension::WorkerCount, 0.95),
            obs(OptimizationDimension::WorkerCount, 0.95),
            obs(OptimizationDimension::AgentCount, 0.3),
            obs(OptimizationDimension::AgentCount, 0.3),
        ]);
        // Three distinct dimensions; scores descending, ties broken by dim asc.
        let scores: Vec<f64> = suggestions.iter().map(|s| s.score).collect();
        let mut sorted = scores.clone();
        sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
        assert_eq!(scores, sorted);

        // Verify ties are broken by dimension ascending.
        for pair in suggestions.windows(2) {
            if pair[0].score == pair[1].score {
                assert!(pair[0].dimension <= pair[1].dimension);
            }
        }
    }

    #[test]
    fn suggest_compute_under_used_is_decrease() {
        let opt = SelfOptimizer::default();
        let s = opt
            .suggest_compute(&[obs(OptimizationDimension::ComputeUsage, 0.1)], 1000, 200)
            .unwrap();
        assert_eq!(s.direction, Direction::Decrease);
        assert_eq!(s.risk, RiskLevel::Low);
        assert_eq!(s.dimension, OptimizationDimension::ComputeUsage);
    }

    #[test]
    fn suggest_compute_mid_is_none() {
        let opt = SelfOptimizer::default();
        assert!(opt.suggest_compute(&[], 1000, 700).is_none());
    }

    #[test]
    fn suggest_compute_near_budget_warns() {
        let opt = SelfOptimizer::default();
        let s = opt
            .suggest_compute(&[obs(OptimizationDimension::ComputeUsage, 0.9)], 1000, 950)
            .unwrap();
        assert_eq!(s.direction, Direction::Rebalance);
        assert_eq!(s.risk, RiskLevel::Medium);
        assert!(s.reason.contains("near budget"));
    }

    #[test]
    fn enums_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&OptimizationDimension::ModelPlacement).unwrap(),
            "\"model_placement\""
        );
        assert_eq!(
            serde_json::to_string(&Direction::Rebalance).unwrap(),
            "\"rebalance\""
        );
        assert_eq!(
            serde_json::to_string(&RiskLevel::High).unwrap(),
            "\"high\""
        );
        assert_eq!(
            serde_json::to_string(&ConstraintKind::Privacy).unwrap(),
            "\"privacy\""
        );
    }

    #[test]
    fn wire_round_trip_over_serde_json() {
        let mut opt = SelfOptimizer::default();
        opt.add_constraint(hard_constraint(ConstraintKind::Latency, 0.9));
        let suggestions = opt.evaluate(&[
            obs(OptimizationDimension::Collaboration, 0.2),
            obs(OptimizationDimension::Collaboration, 0.25),
        ]);
        let json = serde_json::to_string(&suggestions).unwrap();
        let back: Vec<OptimizationSuggestion> = serde_json::from_str(&json).unwrap();
        assert_eq!(suggestions, back);
    }

    #[test]
    fn risk_level_orders_low_below_medium_below_high() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
        assert!(RiskLevel::Low < RiskLevel::High);
    }

    #[test]
    fn weight_clamps_to_unit_interval() {
        let o = OptimizationObservation::new(
            OptimizationDimension::ActiveTools,
            0.5,
            5.0,
            0,
        );
        assert_eq!(o.weight, 1.0);
        let o = OptimizationObservation::new(OptimizationDimension::ActiveTools, 0.5, -1.0, 0);
        assert_eq!(o.weight, 0.0);
    }
}
