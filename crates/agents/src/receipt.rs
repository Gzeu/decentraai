//! Verified compute receipts — the evidence that closes the collective loop (P12).
//!
//! # Why this design
//!
//! The P12 loop is:
//!
//! ```text
//! KnowledgeObject → CollectiveDecision → memory feedback →
//! VerifiedComputeReceipt → CompensationLedger → evidence → KnowledgeObject
//! ```
//!
//! A **verified compute receipt** is the runtime's proof that one workload was
//! executed *and its output passed verification*. It is the single record that
//! feeds both sides of the loop:
//!
//! - **Compensation**: the receipt is the idempotency-safe trigger for
//!   [`CompensationLedger::credit`] — a worker earns contribution credits only
//!   for *verified* work, and only once per execution.
//! - **Knowledge**: the receipt becomes evidence for a knowledge object about
//!   the workload's outcome (the fact "execution X verified" is itself a
//!   knowledge fact backed by `VerifiedExecution` evidence).
//!
//! The receipt itself is pure data (serde-serializable for transport/audit).
//! Applying it to a ledger is a pure operation; the runtime supplies the
//! worker's [`ContributionProfile`] at apply time so the credit honestly
//! reflects the reputation the worker had when it served the request (same
//! principle as [`CompensationLedger::credit`]).

use decentraai_compute::{CompensationLedger, ContributionProfile};
use decentraai_hub::capability::Provenance;
use serde::{Deserialize, Serialize};

use crate::knowledge::{Evidence, EvidenceKind, KnowledgeObject};

/// The final verdict of a verified execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptVerdict {
    /// The output passed verification — the work counts.
    Verified,
    /// The output failed verification — the work does not count.
    Failed,
}

/// A verified compute receipt: proof that one workload ran and its output was
/// verified. Idempotent by `execution_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifiedComputeReceipt {
    /// Stable unique id of the execution (idempotency key for compensation).
    pub execution_id: String,
    /// The workload's id (when known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_id: Option<String>,
    /// The worker node (peer id) that executed the workload.
    pub worker_node: String,
    /// The worker agent that executed the workload.
    pub worker_agent: String,
    /// The workload kind / capability exercised.
    pub capability: String,
    /// Execution duration (ms).
    pub duration_ms: u64,
    /// The verdict.
    pub verdict: ReceiptVerdict,
    /// Hash (BLAKE3 hex) of the verified output, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<String>,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
}

impl VerifiedComputeReceipt {
    pub fn new(
        execution_id: impl Into<String>,
        worker_node: impl Into<String>,
        worker_agent: impl Into<String>,
        capability: impl Into<String>,
        duration_ms: u64,
        verdict: ReceiptVerdict,
        created_at_ms: u64,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            workload_id: None,
            worker_node: worker_node.into(),
            worker_agent: worker_agent.into(),
            capability: capability.into(),
            duration_ms,
            verdict,
            output_hash: None,
            created_at_ms,
        }
    }

    pub fn with_workload_id(mut self, workload_id: impl Into<String>) -> Self {
        self.workload_id = Some(workload_id.into());
        self
    }

    pub fn with_output_hash(mut self, output_hash: impl Into<String>) -> Self {
        self.output_hash = Some(output_hash.into());
        self
    }

    /// Pure: applies this receipt to a [`CompensationLedger`], crediting the
    /// worker **exactly once** per `execution_id` (the ledger's idempotency
    /// guard). Only `Verified` receipts can credit; `Failed` receipts credit
    /// nothing by design (you earn by verified service — see the compensation
    /// module docs). Returns the credits credited (0 for duplicates/failures).
    pub fn apply_compensation(
        &self,
        ledger: &mut CompensationLedger,
        profile: &ContributionProfile,
    ) -> u64 {
        if self.verdict != ReceiptVerdict::Verified {
            return 0;
        }
        ledger.credit(&self.worker_node, &self.execution_id, profile)
    }

    /// Pure: converts this receipt into a knowledge object about the executed
    /// workload's outcome. The object's evidence is `VerifiedExecution` with a
    /// reference to this receipt — so `evidence_confidence` scores it at the
    /// full structural weight (0.9). The returned object is the *evidence
    /// half* of the loop: it can be registered, then consumed by
    /// [`crate::decision::decide_collectively`].
    pub fn to_knowledge_object(&self, object_id: &str, fact: &str) -> KnowledgeObject {
        let evidence = if self.verdict == ReceiptVerdict::Verified {
            vec![
                Evidence::new(
                    EvidenceKind::VerifiedExecution,
                    format!("execution {} verified", self.execution_id),
                )
                .referencing(self.execution_id.clone()),
            ]
        } else {
            vec![
                Evidence::new(
                    EvidenceKind::Synthetic,
                    format!("execution {} failed verification", self.execution_id),
                )
                .referencing(self.execution_id.clone()),
            ]
        };
        KnowledgeObject::new(
            object_id,
            fact,
            self.worker_agent.clone(),
            self.worker_node.clone(),
            self.created_at_ms,
        )
        .with_capability(self.capability.clone())
        .with_evidence(evidence)
        .with_provenance(Provenance::Verified)
    }
}

/// Deterministic registry of receipts (bounded, read-only after registration).
/// Kept so the runtime can expose the receipt trail and answer "was this
/// execution already compensated?" before re-sending work.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReceiptRegistry {
    receipts: std::collections::BTreeMap<String, VerifiedComputeReceipt>,
}

impl ReceiptRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a receipt. Duplicate execution ids are rejected (a receipt is
    /// immutable once recorded — same rule as knowledge objects).
    pub fn add(&mut self, receipt: VerifiedComputeReceipt) -> Result<(), ReceiptError> {
        if self.receipts.contains_key(&receipt.execution_id) {
            return Err(ReceiptError::DuplicateReceipt {
                id: receipt.execution_id,
            });
        }
        self.receipts.insert(receipt.execution_id.clone(), receipt);
        Ok(())
    }

    pub fn get(&self, execution_id: &str) -> Option<&VerifiedComputeReceipt> {
        self.receipts.get(execution_id)
    }

    pub fn contains(&self, execution_id: &str) -> bool {
        self.receipts.contains_key(execution_id)
    }

    /// Every receipt, sorted by execution id (deterministic).
    pub fn all(&self) -> Vec<VerifiedComputeReceipt> {
        self.receipts.values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.receipts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptError {
    #[error("receipt for execution '{id}' is already registered")]
    DuplicateReceipt { id: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::evidence_confidence;

    fn verified_receipt(id: &str) -> VerifiedComputeReceipt {
        VerifiedComputeReceipt::new(
            id,
            "peer-worker",
            "a:worker",
            "inference",
            120,
            ReceiptVerdict::Verified,
            2000,
        )
        .with_output_hash("blake3:abc")
    }

    fn failed_receipt(id: &str) -> VerifiedComputeReceipt {
        VerifiedComputeReceipt::new(
            id,
            "peer-worker",
            "a:worker",
            "inference",
            95,
            ReceiptVerdict::Failed,
            2000,
        )
    }

    fn profile() -> ContributionProfile {
        ContributionProfile {
            cpu_cores: 4,
            ram_mb: 8192,
            vram_mb: 0,
            online_seconds: 3600,
            verified_requests: 10,
            failed_requests: 1,
        }
    }

    #[test]
    fn verified_receipt_credits_worker() {
        let mut ledger = CompensationLedger::default();
        let r = verified_receipt("e1");
        let credits = r.apply_compensation(&mut ledger, &profile());
        assert!(credits > 0, "verified work should credit");
        let acc = ledger.account("peer-worker").unwrap();
        assert_eq!(acc.earned, credits);
    }

    #[test]
    fn failed_receipt_never_credits() {
        let mut ledger = CompensationLedger::default();
        let r = failed_receipt("e2");
        assert_eq!(r.apply_compensation(&mut ledger, &profile()), 0);
        assert!(ledger.account("peer-worker").is_none());
    }

    #[test]
    fn compensation_is_idempotent_per_execution() {
        let mut ledger = CompensationLedger::default();
        let r = verified_receipt("e3");
        let first = r.apply_compensation(&mut ledger, &profile());
        let second = r.apply_compensation(&mut ledger, &profile());
        assert!(first > 0);
        assert_eq!(second, 0, "same execution id must never double-credit");
        assert_eq!(ledger.account("peer-worker").unwrap().earned, first);
    }

    #[test]
    fn receipt_becomes_high_confidence_knowledge() {
        let r = verified_receipt("e4");
        let ko = r.to_knowledge_object("k:receipt:e4", "inference e4 verified");
        assert_eq!(ko.capability.as_deref(), Some("inference"));
        assert_eq!(ko.author_node, "peer-worker");
        // VerifiedExecution evidence → full structural weight.
        assert!((evidence_confidence(&ko) - 0.90).abs() < 1e-6);
        assert!(ko.evidence[0].ref_id.is_some());
    }

    #[test]
    fn failed_receipt_knowledge_has_synthetic_evidence() {
        let r = failed_receipt("e5");
        let ko = r.to_knowledge_object("k:receipt:e5", "inference e5 failed");
        // Synthetic evidence is the weakest kind — the failed receipt cannot
        // claim verified confidence about the outcome.
        assert_eq!(ko.evidence[0].kind, EvidenceKind::Synthetic);
        assert!(evidence_confidence(&ko) < 0.3);
    }

    #[test]
    fn registry_rejects_duplicate_executions() {
        let mut reg = ReceiptRegistry::new();
        reg.add(verified_receipt("e1")).unwrap();
        assert!(reg.add(verified_receipt("e1")).is_err());
        assert_eq!(reg.len(), 1);
        assert!(reg.contains("e1"));
    }
}
