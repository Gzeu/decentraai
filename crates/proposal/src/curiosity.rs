//! Curiosity state — what the agent believes and how learning moves it.
//!
//! One entry per hypothesis: uncertainty (how much remains to learn),
//! confidence (how strongly the current best guess is held), test count,
//! and the last observed outcome. [`CuriosityState::update`] is the ONLY
//! writer, and it is pure arithmetic over the observed outcome — learning
//! flows evidence → here → next selection, never backwards.
//!
//! Numbers (basis points, deterministic):
//! - unknown hypothesis: uncertainty 10000, confidence 5000, tests 0.
//! - Success (supported): uncertainty 2000, confidence +3000 (cap 10000).
//! - Failed (refuted): uncertainty 2000, confidence floored to 1000.
//! - Partial: uncertainty 6000, confidence +1000 (cap 10000).
//! - Inconclusive: uncertainty unchanged (min 8000 — still curious),
//!   confidence unchanged. The agent must try something DIFFERENT
//!   (novelty handles that in scoring).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::evidence::ExperimentOutcome;

/// Belief about one hypothesis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HypothesisBelief {
    /// Remaining uncertainty, bp (10000 = know nothing).
    pub uncertainty_bp: u32,
    /// Confidence in the current best guess, bp.
    pub confidence_bp: u32,
    /// Experiments run against it.
    pub tests: u32,
    /// Last observed outcome.
    pub last_outcome: Option<ExperimentOutcome>,
}

impl Default for HypothesisBelief {
    fn default() -> Self {
        Self {
            uncertainty_bp: 10_000,
            confidence_bp: 5_000,
            tests: 0,
            last_outcome: None,
        }
    }
}

/// The agent's belief map. Deterministic iteration (BTreeMap).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CuriosityState {
    beliefs: BTreeMap<String, HypothesisBelief>,
}

impl CuriosityState {
    /// Empty curiosity (maximal ignorance — the honest start).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Belief for a hypothesis (default when never tested).
    #[must_use]
    pub fn belief(&self, hypothesis_id: &str) -> HypothesisBelief {
        self.beliefs.get(hypothesis_id).copied().unwrap_or_default()
    }

    /// Current uncertainty (bp).
    #[must_use]
    pub fn uncertainty_bp(&self, hypothesis_id: &str) -> u32 {
        self.belief(hypothesis_id).uncertainty_bp
    }

    /// Current confidence (bp).
    #[must_use]
    pub fn confidence_bp(&self, hypothesis_id: &str) -> u32 {
        self.belief(hypothesis_id).confidence_bp
    }

    /// Last observed outcome, if any.
    #[must_use]
    pub fn last_outcome(&self, hypothesis_id: &str) -> Option<ExperimentOutcome> {
        self.belief(hypothesis_id).last_outcome
    }

    /// True when a past experiment SUPPORTED this hypothesis: new
    /// experiments on it are repetitive (anti-loop), not curious.
    #[must_use]
    pub fn is_supported(&self, hypothesis_id: &str) -> bool {
        matches!(
            self.belief(hypothesis_id).last_outcome,
            Some(ExperimentOutcome::Success)
        )
    }

    /// Record an observed outcome. Pure arithmetic, documented above.
    pub fn update(&mut self, hypothesis_id: &str, outcome: ExperimentOutcome) {
        let mut b = self.belief(hypothesis_id);
        b.tests = b.tests.saturating_add(1);
        b.last_outcome = Some(outcome);
        match outcome {
            ExperimentOutcome::Success => {
                b.uncertainty_bp = 2_000;
                b.confidence_bp = (b.confidence_bp + 3_000).min(10_000);
            }
            ExperimentOutcome::Failed => {
                b.uncertainty_bp = 2_000;
                b.confidence_bp = 1_000;
            }
            ExperimentOutcome::Partial => {
                b.uncertainty_bp = 6_000;
                b.confidence_bp = (b.confidence_bp + 1_000).min(10_000);
            }
            ExperimentOutcome::Inconclusive => {
                b.uncertainty_bp = b.uncertainty_bp.max(8_000);
            }
        }
        self.beliefs.insert(hypothesis_id.to_string(), b);
    }

    /// Serialize for persistence (operator stores the file).
    pub fn to_json(&self) -> Result<String, crate::error::ProposalError> {
        serde_json::to_string(self).map_err(|e| crate::error::ProposalError::Bound(e.to_string()))
    }

    /// Reload persisted curiosity. Unknown fields fail closed.
    pub fn from_json(json: &str) -> Result<Self, crate::error::ProposalError> {
        serde_json::from_str(json).map_err(|e| crate::error::ProposalError::Parse(e.to_string()))
    }

    /// The hypothesis id with the highest uncertainty (bp).
    /// Deterministic: max by (uncertainty_bp desc, hypothesis_id asc).
    /// Falls back to `"hyp:uninitialized"` when empty.
    #[must_use]
    pub fn detect_uncertainty(&self) -> String {
        self.beliefs
            .iter()
            .max_by(|a, b| {
                a.1.uncertainty_bp
                    .cmp(&b.1.uncertainty_bp)
                    .then_with(|| b.0.cmp(a.0))
            })
            .map(|(hid, _)| hid.clone())
            .unwrap_or_else(|| "hyp:uninitialized".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_starts_maximally_uncertain() {
        let c = CuriosityState::new();
        assert_eq!(c.uncertainty_bp("hyp:new"), 10_000);
        assert_eq!(c.confidence_bp("hyp:new"), 5_000);
        assert!(!c.is_supported("hyp:new"));
    }

    #[test]
    fn success_drops_uncertainty_and_raises_confidence() {
        let mut c = CuriosityState::new();
        c.update("h", ExperimentOutcome::Success);
        assert_eq!(c.uncertainty_bp("h"), 2_000);
        assert_eq!(c.confidence_bp("h"), 8_000);
        assert!(c.is_supported("h"));
    }

    #[test]
    fn refuted_means_certain_and_unconfident() {
        let mut c = CuriosityState::new();
        c.update("h", ExperimentOutcome::Failed);
        assert_eq!(c.uncertainty_bp("h"), 2_000);
        assert_eq!(c.confidence_bp("h"), 1_000);
        assert!(!c.is_supported("h"));
    }

    #[test]
    fn inconclusive_keeps_curiosity_high() {
        let mut c = CuriosityState::new();
        c.update("h", ExperimentOutcome::Inconclusive);
        assert!(c.uncertainty_bp("h") >= 8_000);
        assert_eq!(c.confidence_bp("h"), 5_000);
        assert!(!c.is_supported("h"));
    }

    #[test]
    fn round_trips_for_persistence() {
        let mut c = CuriosityState::new();
        c.update("h", ExperimentOutcome::Partial);
        let back = CuriosityState::from_json(&c.to_json().unwrap()).unwrap();
        assert_eq!(c, back);
    }
}
