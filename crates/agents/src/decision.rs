//! Collective decisions — consensus over knowledge objects with feedback (P12).
//!
//! # Why this design
//!
//! [`crate::verification::evaluate_consensus`] produces a *verdict* over
//! opinions. A **collective decision** is the next step: it consumes knowledge
//! objects, weights their derived confidence, applies a consensus policy, and
//! produces a decision record that can be written back to memory as evidence.
//!
//! The decision is the fabric's "we decided" moment:
//!
//! ```text
//! KnowledgeObject(s) ─► weight by confidence ─► ConsensusPolicy
//!         ▲                                          │
//!         │                                     decision record
//!         │                                          │
//!         └── evidence ←── memory feedback (write_decision_to_memory)
//! ```
//!
//! Rules (all pure):
//!
//! - A decision consumes **at least one** knowledge object (nothing to decide
//!   without a fact).
//! - Each object contributes its derived confidence (`evidence_confidence`),
//!   **never a declared score** — an un-evidenced object has confidence 0.0
//!   and cannot pull a decision over the line by itself.
//! - The vote is delegated to the existing `evaluate_consensus` so the fabric
//!   has ONE consensus language (P4), not a second one.
//! - The decision record carries the aggregated confidence and the full
//!   evidence trail (object ids + vote results) so it can be re-audited.

use crate::knowledge::{KnowledgeObject, evidence_confidence};
use crate::verification::{
    ConsensusPolicy, ConsensusResult, VerificationVerdict, evaluate_consensus,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A vote by one agent on a candidate knowledge object (its *opinion* about
/// whether the object's fact is correct).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeVote {
    /// The voting agent.
    pub agent_id: String,
    /// Whether this agent agrees the fact is correct.
    pub agrees: bool,
    /// The agent's confidence in its own judgment, `0.0..=1.0`.
    pub confidence: f32,
}

impl From<KnowledgeVote> for ConsensusResult {
    fn from(v: KnowledgeVote) -> Self {
        Self {
            agent_id: v.agent_id,
            agrees: v.agrees,
            confidence: v.confidence,
        }
    }
}

/// The outcome of a collective decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionVerdict {
    /// The fabric decided the fact holds (consensus Verified).
    Adopted,
    /// The fabric decided the fact does not hold.
    Rejected,
    /// Not enough evidence or no consensus — no decision.
    Deferred { reason: String },
}

/// One knowledge object considered by a decision, with its derived confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsideredObject {
    /// The object's id.
    pub object_id: String,
    /// The derived confidence (0.0–1.0), computed at decision time.
    pub confidence: f32,
}

/// A collective decision record: what was decided, over which objects, with
/// which evidence. Immutable once produced (a re-decision is a new record).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectiveDecision {
    /// Stable unique id (e.g. `"d:2026-08-19:1"`).
    pub decision_id: String,
    /// Short human summary of what was decided.
    pub summary: String,
    /// The knowledge objects considered, with their derived confidence.
    pub considered: Vec<ConsideredObject>,
    /// The verdict.
    pub verdict: DecisionVerdict,
    /// Aggregated confidence of the considered objects (max of their derived
    /// confidence — the strongest evidence the decision could rely on).
    pub aggregated_confidence: f32,
    /// The consensus verdict that produced the outcome (when a vote ran).
    pub consensus: VerificationVerdict,
    /// Agent that initiated the decision (the coordinator).
    pub initiator_agent: String,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
}

/// Errors for collective decisions.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum DecisionError {
    #[error("a collective decision needs at least one knowledge object")]
    NoObjects,
    #[error("decision '{id}' is already registered")]
    DuplicateDecision { id: String },
}

/// Pure: runs a collective decision over knowledge objects.
///
/// - Requires at least one object (`NoObjects` otherwise).
/// - Computes each object's derived confidence (`evidence_confidence`).
/// - Builds a consensus vote from the objects' evidence (an object with a
///   `VerifiedExecution` or `Consensus` evidence item votes *agrees* with
///   confidence = its derived confidence; otherwise it votes with its derived
///   confidence as agreement — the formula below is deterministic).
/// - Delegates to `evaluate_consensus` with the given policy.
/// - `Verified` → `Adopted`, `Rejected` → `Rejected`, `Uncertain` → `Deferred`.
pub fn decide_collectively(
    decision_id: &str,
    summary: &str,
    initiator_agent: &str,
    created_at_ms: u64,
    objects: &[KnowledgeObject],
    policy: &ConsensusPolicy,
) -> Result<CollectiveDecision, DecisionError> {
    if objects.is_empty() {
        return Err(DecisionError::NoObjects);
    }
    let considered: Vec<ConsideredObject> = objects
        .iter()
        .map(|o| ConsideredObject {
            object_id: o.object_id.clone(),
            confidence: evidence_confidence(o),
        })
        .collect();
    let aggregated_confidence = considered
        .iter()
        .map(|c| c.confidence)
        .fold(0.0_f32, f32::max);

    // Deterministic vote construction from the objects' own evidence: an
    // object backed by VerifiedExecution or Consensus evidence votes agrees
    // with its derived confidence; other objects vote with their confidence
    // as-is (the policy threshold still gates the outcome).
    let votes: Vec<ConsensusResult> = objects
        .iter()
        .map(|o| {
            let confidence = evidence_confidence(o);
            let backed = o.evidence.iter().any(|e| {
                matches!(
                    e.kind,
                    crate::knowledge::EvidenceKind::VerifiedExecution
                        | crate::knowledge::EvidenceKind::Consensus
                )
            });
            ConsensusResult {
                agent_id: o.author_agent.clone(),
                agrees: backed || confidence > 0.0,
                confidence,
            }
        })
        .collect();
    let consensus = evaluate_consensus(&votes, policy);
    let verdict = match &consensus {
        VerificationVerdict::Verified => DecisionVerdict::Adopted,
        VerificationVerdict::Rejected { .. } => DecisionVerdict::Rejected,
        VerificationVerdict::Uncertain { reason } => DecisionVerdict::Deferred {
            reason: reason.clone(),
        },
    };
    Ok(CollectiveDecision {
        decision_id: decision_id.to_string(),
        summary: summary.to_string(),
        considered,
        verdict,
        aggregated_confidence,
        consensus,
        initiator_agent: initiator_agent.to_string(),
        created_at_ms,
    })
}

/// Deterministic registry of collective decisions (bounded, read-only after
/// registration). Kept so the runtime can expose the decision trail.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DecisionRegistry {
    decisions: std::collections::BTreeMap<String, CollectiveDecision>,
}

impl DecisionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, decision: CollectiveDecision) -> Result<(), DecisionError> {
        if self.decisions.contains_key(&decision.decision_id) {
            return Err(DecisionError::DuplicateDecision {
                id: decision.decision_id,
            });
        }
        self.decisions
            .insert(decision.decision_id.clone(), decision);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&CollectiveDecision> {
        self.decisions.get(id)
    }

    /// Every decision, sorted by id (deterministic).
    pub fn all(&self) -> Vec<CollectiveDecision> {
        self.decisions.values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.decisions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.decisions.is_empty()
    }
}

/// Pure: converts an adopted collective decision into a memory entry's
/// content (the feedback half of the loop). The decision becomes a knowledge
/// fact backed by `VerifiedExecution`-grade evidence (it passed consensus).
///
/// Returns the entry content (a single canonical line) plus the object id the
/// caller should use when persisting it back to the knowledge registry.
pub fn decision_feedback_entry(
    decision: &CollectiveDecision,
    feedback_scope: &str,
) -> (String, String) {
    let object_id = format!("k:decision:{}", decision.decision_id);
    let content = format!(
        "[collective-decision] {summary} — verdict={verdict:?}, confidence={confidence:.2}, objects=[{objects}]",
        summary = decision.summary,
        verdict = decision.verdict,
        confidence = decision.aggregated_confidence,
        objects = decision
            .considered
            .iter()
            .map(|c| c.object_id.clone())
            .collect::<Vec<_>>()
            .join(","),
    );
    let _ = feedback_scope; // scope is chosen by the runtime, not encoded here
    (content, object_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{Evidence, EvidenceKind, KnowledgeObject};

    fn obj(id: &str, fact: &str) -> KnowledgeObject {
        KnowledgeObject::new(id, fact, "a:research", "peer1", 1000)
    }

    fn verified_obj(id: &str, fact: &str) -> KnowledgeObject {
        obj(id, fact).with_evidence(vec![Evidence::new(
            EvidenceKind::VerifiedExecution,
            "execution verified",
        )])
    }

    #[test]
    fn no_objects_is_an_error() {
        let policy = ConsensusPolicy::default();
        assert!(matches!(
            decide_collectively("d1", "nothing", "a:coord", 1, &[], &policy),
            Err(DecisionError::NoObjects)
        ));
    }

    #[test]
    fn unverified_objects_defer() {
        // A single un-evidenced object cannot reach consensus (confidence 0).
        let policy = ConsensusPolicy::default();
        let d = decide_collectively("d1", "fact", "a:coord", 1, &[obj("k1", "plain")], &policy)
            .unwrap();
        assert!(matches!(d.verdict, DecisionVerdict::Deferred { .. }));
        assert_eq!(d.aggregated_confidence, 0.0);
        assert!(matches!(d.consensus, VerificationVerdict::Uncertain { .. }));
    }

    #[test]
    fn verified_objects_are_adopted() {
        let policy = ConsensusPolicy {
            required_agents: 1,
            agreement_threshold: 0.5,
            require_schema: false,
        };
        let d = decide_collectively(
            "d2",
            "fact",
            "a:coord",
            1,
            &[verified_obj("k1", "checked")],
            &policy,
        )
        .unwrap();
        assert_eq!(d.verdict, DecisionVerdict::Adopted);
        assert!(d.aggregated_confidence >= 0.8);
        assert_eq!(d.consensus, VerificationVerdict::Verified);
    }

    #[test]
    fn threshold_gates_adoption() {
        // Two objects but only one backed by strong evidence; threshold 1.0
        // means the weak vote blocks adoption (agreement 0.5 < 1.0).
        let policy = ConsensusPolicy {
            required_agents: 2,
            agreement_threshold: 1.0,
            require_schema: false,
        };
        let d = decide_collectively(
            "d3",
            "fact",
            "a:coord",
            1,
            &[verified_obj("k1", "strong"), obj("k2", "weak")],
            &policy,
        )
        .unwrap();
        assert!(matches!(d.verdict, DecisionVerdict::Rejected));
    }

    #[test]
    fn decision_registry_rejects_duplicates() {
        let mut reg = DecisionRegistry::new();
        let policy = ConsensusPolicy {
            required_agents: 1,
            agreement_threshold: 0.5,
            require_schema: false,
        };
        let d = decide_collectively(
            "d1",
            "fact",
            "a:coord",
            1,
            &[verified_obj("k1", "x")],
            &policy,
        )
        .unwrap();
        reg.add(d.clone()).unwrap();
        assert!(reg.add(d).is_err());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn feedback_entry_contains_verdict_and_confidence() {
        let policy = ConsensusPolicy {
            required_agents: 1,
            agreement_threshold: 0.5,
            require_schema: false,
        };
        let d = decide_collectively(
            "d9",
            "model X is safe for the team",
            "a:coord",
            1,
            &[verified_obj("k1", "checked")],
            &policy,
        )
        .unwrap();
        let (content, object_id) = decision_feedback_entry(&d, "team.decisions");
        assert!(content.contains("model X is safe"));
        assert!(content.contains("Adopted"));
        assert!(object_id == "k:decision:d9");
    }
}
