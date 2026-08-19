//! Collective knowledge — evidence-backed knowledge objects (P12).
//!
//! # Why this design
//!
//! [`memory::MemoryEntry`] is the *remembered fact* (opaque content +
//! provenance). A **knowledge object** is the next step up: a fact that the
//! fabric can *reason about* — it carries evidence references and a
//! **confidence derived from that evidence**, never declared by an author.
//!
//! The core rule (the same principle `runtime_evidence` enforces for skills):
//!
//! > **declaration ≠ evidence** — a knowledge object's confidence is a pure
//! > function of the evidence attached to it. No evidence → confidence 0.0,
//! > no matter who wrote it.
//!
//! Evidence is deliberately heterogeneous (a closed set of *kinds* so the
//! confidence formula stays deterministic):
//!
//! - `VerifiedExecution` — the fact was produced by an execution whose
//!   output passed verification (see `crate::verification`).
//! - `Consensus` — multiple independent agents agreed on the fact
//!   (`crate::verification::evaluate_consensus`).
//! - `ReputationWeighted` — the author's reputation contributes, capped so it
//!   can never dominate structural evidence.
//! - `DirectObservation` — the node itself observed the fact (local tool
//!   output, probe, receipt).
//! - `Synthetic` — derived bookkeeping (ledger rows, aggregated counters).
//!   The weakest evidence by design: synthetic numbers are *computed*, not
//!   *verified*.
//!
//! This module is pure (no I/O, no async), serde-serializable, and drives the
//! closed loop: `KnowledgeObject → CollectiveDecision → memory feedback →
//! VerifiedComputeReceipt → CompensationLedger → evidence → KnowledgeObject`.

use decentraai_hub::capability::Provenance;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Evidence kinds a knowledge object can carry. A closed set so the
/// confidence formula is deterministic and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// Produced by an execution whose output passed verification.
    VerifiedExecution,
    /// Multiple independent agents agreed (consensus verdict `Verified`).
    Consensus,
    /// The author's reputation contributes (capped).
    ReputationWeighted,
    /// The node itself observed the fact (local tool output, probe, receipt).
    DirectObservation,
    /// Derived bookkeeping (ledger rows, aggregated counters). Weakest.
    Synthetic,
}

/// One piece of evidence behind a knowledge object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// The kind of evidence (drives confidence).
    pub kind: EvidenceKind,
    /// Human-readable description of what backs the fact.
    pub detail: String,
    /// Optional reference (execution id, receipt id, verification report id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
}

impl Evidence {
    pub fn new(kind: EvidenceKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
            ref_id: None,
        }
    }

    pub fn referencing(mut self, ref_id: impl Into<String>) -> Self {
        self.ref_id = Some(ref_id.into());
        self
    }
}

/// A knowledge object: a fact the fabric can reason about, with evidence and
/// a confidence *derived* from that evidence (never declared).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeObject {
    /// Stable unique id within the node.
    pub object_id: String,
    /// The fact (canonical short text).
    pub fact: String,
    /// Agent that authored the object.
    pub author_agent: String,
    /// Node (peer id) that hosted the authoring agent.
    pub author_node: String,
    /// Capability this fact is about (when tied to one), optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    /// Evidence backing the fact. Empty = confidence 0.0 (declaration only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// Author's reputation score at object creation (0.0–1.0, unknown = 0.0).
    #[serde(default)]
    pub author_reputation: f32,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
    /// Provenance of the fact's content (mirrors memory entries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

impl KnowledgeObject {
    pub fn new(
        object_id: impl Into<String>,
        fact: impl Into<String>,
        author_agent: impl Into<String>,
        author_node: impl Into<String>,
        created_at_ms: u64,
    ) -> Self {
        Self {
            object_id: object_id.into(),
            fact: fact.into(),
            author_agent: author_agent.into(),
            author_node: author_node.into(),
            capability: None,
            evidence: Vec::new(),
            author_reputation: 0.0,
            created_at_ms,
            provenance: None,
        }
    }

    pub fn with_capability(mut self, capability: impl Into<String>) -> Self {
        self.capability = Some(capability.into());
        self
    }

    pub fn with_evidence(mut self, evidence: Vec<Evidence>) -> Self {
        self.evidence = evidence;
        self
    }

    pub fn with_author_reputation(mut self, reputation: f32) -> Self {
        self.author_reputation = reputation.clamp(0.0, 1.0);
        self
    }

    pub fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = Some(provenance);
        self
    }
}

/// Confidence weights per evidence kind. Structural evidence (verification,
/// consensus, direct observation) outweighs social evidence (reputation) and
/// bookkeeping (synthetic). Deliberately small, deterministic constants.
pub const WEIGHT_VERIFIED_EXECUTION: f32 = 0.90;
pub const WEIGHT_CONSENSUS: f32 = 0.80;
pub const WEIGHT_DIRECT_OBSERVATION: f32 = 0.70;
pub const WEIGHT_REPUTATION: f32 = 0.25;
pub const WEIGHT_SYNTHETIC: f32 = 0.20;

/// Pure: derives a knowledge object's confidence (0.0–1.0) from its evidence.
///
/// - No evidence → 0.0 (declaration ≠ evidence).
/// - Multiple evidence items combine: `1 - ∏(1 - w_i)` — more independent
///   evidence strictly increases confidence, bounded by 1.0.
/// - Reputation contributes at most one capped term, so a famous author can
///   *add* but never *replace* structural evidence.
/// - Provenance is not confidence: `Verified` provenance on a fact with no
///   evidence still yields 0.0 (the *fact* is unverified, not the author).
pub fn evidence_confidence(knowledge: &KnowledgeObject) -> f32 {
    let mut combined = 0.0_f32;
    let mut reputation_applied = false;
    for ev in &knowledge.evidence {
        let w = match ev.kind {
            EvidenceKind::VerifiedExecution => WEIGHT_VERIFIED_EXECUTION,
            EvidenceKind::Consensus => WEIGHT_CONSENSUS,
            EvidenceKind::DirectObservation => WEIGHT_DIRECT_OBSERVATION,
            EvidenceKind::ReputationWeighted => {
                if reputation_applied {
                    continue; // one capped reputation term max
                }
                reputation_applied = true;
                WEIGHT_REPUTATION * knowledge.author_reputation
            }
            EvidenceKind::Synthetic => WEIGHT_SYNTHETIC,
        };
        // Combined evidence: 1 - ∏(1 - w)
        combined = combined + w * (1.0 - combined);
    }
    combined.clamp(0.0, 1.0)
}

/// Classification of a knowledge object by its derived confidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeConfidence {
    /// confidence == 0.0 — declaration only, no evidence.
    None,
    /// 0.0 < confidence < 0.5 — some evidence, not trustworthy alone.
    Low,
    /// 0.5 <= confidence < 0.8 — usable with corroboration.
    Medium,
    /// confidence >= 0.8 — high evidence (verified execution / consensus).
    High,
}

impl std::fmt::Display for KnowledgeConfidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        };
        write!(f, "{s}")
    }
}

impl KnowledgeConfidence {
    pub fn of(confidence: f32) -> Self {
        if confidence <= 0.0 {
            Self::None
        } else if confidence < 0.5 {
            Self::Low
        } else if confidence < 0.8 {
            Self::Medium
        } else {
            Self::High
        }
    }
}

/// Errors for knowledge operations.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum KnowledgeError {
    #[error("knowledge object '{id}' is already registered")]
    DuplicateObject { id: String },
    #[error("knowledge object '{id}' is not registered")]
    UnknownObject { id: String },
    #[error("author reputation must be in 0.0..=1.0, got {value}")]
    InvalidReputation { value: f32 },
}

/// Deterministic registry of knowledge objects on a node.
///
/// Pure and bounded: entries are keyed by `object_id`, iteration is sorted,
/// and the registry enforces the evidence rule (an object cannot be
/// re-registered with different evidence under the same id).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeRegistry {
    objects: std::collections::BTreeMap<String, KnowledgeObject>,
}

impl KnowledgeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a knowledge object. Duplicate ids are rejected (objects are
    /// immutable once registered — evidence must be added as a new object).
    pub fn add(&mut self, object: KnowledgeObject) -> Result<(), KnowledgeError> {
        if !(0.0..=1.0).contains(&object.author_reputation) {
            return Err(KnowledgeError::InvalidReputation {
                value: object.author_reputation,
            });
        }
        if self.objects.contains_key(&object.object_id) {
            return Err(KnowledgeError::DuplicateObject {
                id: object.object_id,
            });
        }
        self.objects.insert(object.object_id.clone(), object);
        Ok(())
    }

    /// Looks up one object by id.
    pub fn get(&self, id: &str) -> Option<&KnowledgeObject> {
        self.objects.get(id)
    }

    /// Every object, sorted by id (deterministic), each with its derived
    /// confidence. The confidence is *computed* at read time so evidence
    /// additions are reflected without mutable state.
    pub fn all_with_confidence(&self) -> Vec<(KnowledgeObject, f32)> {
        self.objects
            .values()
            .map(|o| (o.clone(), evidence_confidence(o)))
            .collect()
    }

    /// Objects whose derived confidence is at least `min_confidence`.
    pub fn with_confidence_at_least(&self, min_confidence: f32) -> Vec<KnowledgeObject> {
        self.objects
            .values()
            .filter(|o| evidence_confidence(o) >= min_confidence)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(id: &str, fact: &str) -> KnowledgeObject {
        KnowledgeObject::new(id, fact, "a:research", "peer1", 1000)
    }

    #[test]
    fn no_evidence_means_zero_confidence() {
        let k = obj("k1", "the sky is blue");
        assert_eq!(evidence_confidence(&k), 0.0);
        // Provenance is NOT confidence: a verified-author declaration with no
        // evidence still scores 0.
        let with_prov = k.with_provenance(Provenance::Verified);
        assert_eq!(evidence_confidence(&with_prov), 0.0);
        assert_eq!(KnowledgeConfidence::of(0.0), KnowledgeConfidence::None);
    }

    #[test]
    fn verified_execution_dominates() {
        let k = obj("k2", "model output passed schema check").with_evidence(vec![
            Evidence::new(EvidenceKind::VerifiedExecution, "execution verified"),
        ]);
        let c = evidence_confidence(&k);
        assert!((c - WEIGHT_VERIFIED_EXECUTION).abs() < 1e-6);
        assert_eq!(KnowledgeConfidence::of(c), KnowledgeConfidence::High);
    }

    #[test]
    fn multiple_evidence_combines_strictly_above_single() {
        let single = obj("k3", "fact").with_evidence(vec![Evidence::new(
            EvidenceKind::Consensus,
            "3 agents agreed",
        )]);
        let both = obj("k4", "fact").with_evidence(vec![
            Evidence::new(EvidenceKind::Consensus, "3 agents agreed"),
            Evidence::new(EvidenceKind::DirectObservation, "node observed it"),
        ]);
        let c_single = evidence_confidence(&single);
        let c_both = evidence_confidence(&both);
        assert!(c_both > c_single, "{c_both} should exceed {c_single}");
        assert!(c_both < 1.0, "combined evidence is bounded by 1.0");
    }

    #[test]
    fn reputation_is_capped_and_never_dominates() {
        // A famous author with only reputation evidence stays Low — social
        // proof alone cannot reach Medium.
        let famous = obj("k5", "fact").with_author_reputation(1.0).with_evidence(vec![
            Evidence::new(EvidenceKind::ReputationWeighted, "author has great reputation"),
        ]);
        let c = evidence_confidence(&famous);
        assert!((c - WEIGHT_REPUTATION).abs() < 1e-6);
        assert_eq!(KnowledgeConfidence::of(c), KnowledgeConfidence::Low);

        // Two reputation items still count once (capped).
        let double = obj("k6", "fact").with_author_reputation(1.0).with_evidence(vec![
            Evidence::new(EvidenceKind::ReputationWeighted, "a"),
            Evidence::new(EvidenceKind::ReputationWeighted, "b"),
        ]);
        assert!((evidence_confidence(&double) - c).abs() < 1e-6);
    }

    #[test]
    fn synthetic_evidence_is_weakest() {
        let synthetic = obj("k7", "counted row").with_evidence(vec![Evidence::new(
            EvidenceKind::Synthetic,
            "ledger row",
        )]);
        let verified = obj("k8", "verified fact").with_evidence(vec![Evidence::new(
            EvidenceKind::VerifiedExecution,
            "execution verified",
        )]);
        assert!(evidence_confidence(&synthetic) < evidence_confidence(&verified));
        assert_eq!(
            KnowledgeConfidence::of(evidence_confidence(&synthetic)),
            KnowledgeConfidence::Low
        );
    }

    #[test]
    fn registry_rejects_duplicates_and_sorts_deterministically() {
        let mut reg = KnowledgeRegistry::new();
        reg.add(obj("b", "second")).unwrap();
        reg.add(obj("a", "first")).unwrap();
        assert_eq!(reg.len(), 2);
        assert!(reg.add(obj("a", "duplicate")).is_err());
        let all = reg.all_with_confidence();
        assert_eq!(all[0].0.object_id, "a");
        assert_eq!(all[1].0.object_id, "b");
        assert_eq!(all[0].1, 0.0);
    }

    #[test]
    fn registry_rejects_out_of_range_reputation() {
        let mut reg = KnowledgeRegistry::new();
        // The builder clamps, so inject the invalid value directly to exercise
        // the registry's guard (the guard protects deserialized objects too).
        let mut bad = obj("x", "fact");
        bad.author_reputation = 1.5;
        assert!(matches!(
            reg.add(bad),
            Err(KnowledgeError::InvalidReputation { .. })
        ));
        let mut low = obj("y", "fact");
        low.author_reputation = -0.2;
        assert!(matches!(
            reg.add(low),
            Err(KnowledgeError::InvalidReputation { .. })
        ));
    }

    #[test]
    fn filter_by_confidence_only_returns_evidenced() {
        let mut reg = KnowledgeRegistry::new();
        reg.add(obj("plain", "no evidence")).unwrap();
        reg.add(
            obj("verified", "checked")
                .with_evidence(vec![Evidence::new(
                    EvidenceKind::VerifiedExecution,
                    "execution verified",
                )]),
        )
        .unwrap();
        let solid = reg.with_confidence_at_least(0.5);
        assert_eq!(solid.len(), 1);
        assert_eq!(solid[0].object_id, "verified");
    }
}