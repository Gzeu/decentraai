//! v0.5 — Persistent Research Memory: the line of research across restarts.
//!
//! The journal is the agent's longitudinal memory of INVESTIGATIONS —
//! distinct from [`crate::store::ExperimentStore`] (execution accounting)
//! and [`crate::curiosity::CuriosityState`] (numerical beliefs). The
//! journal answers: WHAT was tried, WHAT was learned, WHAT was refuted,
//! and WHAT REMAINS UNKNOWN. It survives restarts and steers
//! [`crate::research::construct_candidates`]:
//!
//! - a family with ≥2 refuted members is AVOIDED (the line is dead);
//! - a family with only inconclusive members is OPEN (priority target);
//! - a family with a supported member is CLOSED (settled).
//!
//! Append-only, deterministic order = insertion order, JSON-serializable,
//! operator-owned file (default `~/.decentraai/experiments/research-journal.json`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::learning::HypothesisVerdict;

/// One line of the research record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEntry {
    /// Cycle that produced it.
    pub cycle_id: String,
    /// Hypothesis under test (`fam:<family>:…`).
    pub hypothesis_id: String,
    /// Family extracted from the hypothesis id.
    pub family: String,
    /// Inferred verdict (never operator-declared).
    pub verdict: HypothesisVerdict,
    /// Evidence id on disk.
    pub evidence_id: String,
    /// Completion timestamp (ms).
    pub completed_at_ms: u64,
}

/// The research journal. Append-only; queries are pure.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchJournal {
    /// Entries in completion order.
    pub entries: Vec<JournalEntry>,
}

/// Extract `family` from a `fam:<family>:<rest>` hypothesis id.
#[must_use]
pub fn family_of(hypothesis_id: &str) -> String {
    hypothesis_id
        .strip_prefix("fam:")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or("")
        .to_string()
}

impl ResearchJournal {
    /// Empty journal — maximal ignorance, maximal honesty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a completed cycle's verdict.
    pub fn record(
        &mut self,
        cycle_id: &str,
        hypothesis_id: &str,
        verdict: HypothesisVerdict,
        evidence_id: &str,
        completed_at_ms: u64,
    ) {
        self.entries.push(JournalEntry {
            cycle_id: cycle_id.to_string(),
            hypothesis_id: hypothesis_id.to_string(),
            family: family_of(hypothesis_id),
            verdict,
            evidence_id: evidence_id.to_string(),
            completed_at_ms,
        });
    }

    /// Per-family verdict tallies. Deterministic (BTreeMap).
    #[must_use]
    pub fn family_tallies(&self) -> BTreeMap<String, (u32, u32, u32)> {
        let mut t: BTreeMap<String, (u32, u32, u32)> = BTreeMap::new();
        for e in &self.entries {
            let slot = t.entry(e.family.clone()).or_default();
            match e.verdict {
                HypothesisVerdict::Supported => slot.0 += 1,
                HypothesisVerdict::Refuted => slot.1 += 1,
                HypothesisVerdict::Inconclusive => slot.2 += 1,
            }
        }
        t
    }

    /// A family is DEAD (avoided) once refuted twice — two independent
    /// refutations close the line definitively.
    #[must_use]
    pub fn family_is_dead(&self, family: &str) -> bool {
        self.family_tallies()
            .get(family)
            .is_some_and(|&(_, r, _)| r >= 2)
    }

    /// A family is OPEN when tried but never supported (researchable).
    #[must_use]
    pub fn family_is_open(&self, family: &str) -> bool {
        self.family_tallies()
            .get(family)
            .is_some_and(|&(s, _, _)| s == 0)
            && self.entries.iter().any(|e| e.family == family)
    }

    /// Human/JSON report: what was tried / supported / refuted / open.
    #[must_use]
    pub fn research_report(&self) -> serde_json::Value {
        let tallies = self.family_tallies();
        let open: Vec<&str> = tallies
            .iter()
            .filter(|(f, t)| t.0 == 0 && !self.family_is_dead(f))
            .map(|(f, _)| f.as_str())
            .collect();
        serde_json::json!({
            "tried": self.entries.len(),
            "supported": tallies.values().map(|&(s, _, _)| s).sum::<u32>(),
            "refuted": tallies.values().map(|&(_, r, _)| r).sum::<u32>(),
            "open_families": open,
        })
    }

    /// Serialize.
    pub fn to_json(&self) -> Result<String, crate::error::ProposalError> {
        serde_json::to_string(self).map_err(|e| crate::error::ProposalError::Bound(e.to_string()))
    }

    /// Reload; unknown fields fail closed.
    pub fn from_json(json: &str) -> Result<Self, crate::error::ProposalError> {
        serde_json::from_str(json).map_err(|e| crate::error::ProposalError::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j() -> ResearchJournal {
        let mut j = ResearchJournal::new();
        j.record(
            "c1",
            "fam:transfer-health:probe-100",
            HypothesisVerdict::Supported,
            "ev:1",
            1,
        );
        j.record(
            "c2",
            "fam:signal-delta:minted",
            HypothesisVerdict::Inconclusive,
            "ev:2",
            2,
        );
        j.record(
            "c3",
            "fam:fee-model:a",
            HypothesisVerdict::Refuted,
            "ev:3",
            3,
        );
        j.record(
            "c4",
            "fam:fee-model:b",
            HypothesisVerdict::Refuted,
            "ev:4",
            4,
        );
        j
    }

    #[test]
    fn family_extraction() {
        assert_eq!(
            family_of("fam:transfer-health:probe-100"),
            "transfer-health"
        );
        assert_eq!(family_of("other"), "");
    }

    #[test]
    fn tallies_and_dead_lines() {
        let j = j();
        let t = j.family_tallies();
        assert_eq!(t["transfer-health"], (1, 0, 0));
        assert_eq!(t["signal-delta"], (0, 0, 1));
        assert_eq!(t["fee-model"], (0, 2, 0));
        assert!(j.family_is_dead("fee-model"));
        assert!(!j.family_is_dead("signal-delta"));
        assert!(j.family_is_open("signal-delta"));
        assert!(!j.family_is_open("transfer-health"));
    }

    #[test]
    fn report_roundtrip() {
        let j = j();
        let back = ResearchJournal::from_json(&j.to_json().unwrap()).unwrap();
        assert_eq!(j, back);
        let rep = j.research_report();
        assert_eq!(rep["tried"], 4);
        assert_eq!(rep["supported"], 1);
        assert_eq!(rep["refuted"], 2);
        assert!(
            rep["open_families"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "signal-delta")
        );
    }
}
