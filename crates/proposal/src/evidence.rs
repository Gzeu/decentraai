//! Evidence — hash-sealed, chained execution facts (Execution → Evidence).
//!
//! [`ExperimentEvidence`] commits to one execution report plus its outcome;
//! [`EvidenceLog`] is the append-only, hash-chained store with pure
//! [`EvidenceLog::verify_chain`]. Seals use BLAKE3 over canonical JSON.
//!
//! Honesty rule (same as the fabric evidence index): entries carry facts
//! (what ran, what was measured, what the outcome was). Prompts and model
//! outputs are never evidence material.

use serde::{Deserialize, Serialize};

use crate::budget::TestnetAsset;
use crate::error::ProposalError;
use crate::policy::ExecutionMode;
use crate::sandbox::ExecutionReport;

/// Schema version for sealed evidence.
pub const EVIDENCE_VERSION: u32 = 1;

/// What the experiment concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentOutcome {
    /// All steps succeeded and the hypothesis held.
    Success,
    /// Some steps succeeded; hypothesis partially held.
    Partial,
    /// Steps ran but the hypothesis did not hold.
    Failed,
    /// Could not conclude (not an error — a recorded fact).
    Inconclusive,
}

/// Sealed evidence for one executed proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentEvidence {
    /// Schema version.
    pub version: u32,
    /// Stable id (`ev:<proposal_id>:<sealed_at_ms>`).
    pub id: String,
    /// Proposal this evidence belongs to.
    pub proposal_id: String,
    /// Lane it executed in.
    pub mode: ExecutionMode,
    /// Step results digest (BLAKE3 over the canonical report JSON).
    pub results_digest: [u8; 32],
    /// Concluded outcome.
    pub outcome: ExperimentOutcome,
    /// Seal time (unix ms, caller-provided).
    pub sealed_at_ms: u64,
    /// Previous entry's seal (None for genesis). Tamper-evident chaining.
    pub prev_hash: Option<[u8; 32]>,
    /// Testnet execution facts, when the evidence backs a live experiment.
    /// `None` for sandbox/read-only runs. `#[serde(default)]` keeps older
    /// sealed JSON readable (missing field → `None`, seal re-checks fail
    /// closed on any mutation as before).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub testnet: Option<TestnetEvidence>,
    /// Seal: BLAKE3 over canonical JSON with `hash` zeroed.
    pub hash: [u8; 32],
}

/// On-chain facts for one testnet experiment. Every field is a recorded
/// fact: the tx hash is external verifiable evidence (look it up on the
/// testnet explorer), never a claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestnetEvidence {
    /// Experiment id.
    pub experiment_id: String,
    /// Chain (always [`crate::budget::TESTNET_CHAIN_ID`]).
    pub chain_id: String,
    /// Asset moved.
    pub asset: TestnetAsset,
    /// Amount moved (wei).
    pub amount_wei: u64,
    /// Amount the budget authorized (wei) — `amount_wei <= authorized`.
    pub authorized_wei: u64,
    /// Destination (budget-allowlisted).
    pub destination: String,
    /// Chain tx hash.
    pub tx_hash: String,
    /// Policy lane that allowed it (always `testnet` here).
    pub policy_mode: ExecutionMode,
}

impl ExperimentEvidence {
    /// Build + seal evidence for a report. The seal commits to every field
    /// above it, so any later mutation breaks verification.
    #[must_use]
    pub fn seal(
        report: &ExecutionReport,
        outcome: ExperimentOutcome,
        sealed_at_ms: u64,
        prev_hash: Option<[u8; 32]>,
    ) -> Self {
        let results_digest =
            *blake3::hash(&serde_json::to_vec(report).unwrap_or_default()).as_bytes();
        let mut ev = Self {
            version: EVIDENCE_VERSION,
            id: format!("ev:{}:{sealed_at_ms}", report.proposal_id),
            proposal_id: report.proposal_id.clone(),
            mode: report.mode,
            results_digest,
            outcome,
            sealed_at_ms,
            prev_hash,
            testnet: None,
            hash: [0u8; 32],
        };
        ev.hash = *blake3::hash(&serde_json::to_vec(&ev).unwrap_or_default()).as_bytes();
        ev
    }

    /// Build + seal evidence for a BOUNDED TESTNET execution report.
    /// Commits to the tx hash, the authorized budget and the actual amount,
    /// so under-reporting the spend breaks the seal.
    #[must_use]
    pub fn seal_testnet(
        report: &crate::testnet::TestnetReport,
        outcome: ExperimentOutcome,
        authorized_wei: u64,
        sealed_at_ms: u64,
        prev_hash: Option<[u8; 32]>,
    ) -> Self {
        let results_digest =
            *blake3::hash(&serde_json::to_vec(report).unwrap_or_default()).as_bytes();
        let mut ev = Self {
            version: EVIDENCE_VERSION,
            id: format!("ev:{}:{sealed_at_ms}", report.proposal_id),
            proposal_id: report.proposal_id.clone(),
            mode: ExecutionMode::Testnet,
            results_digest,
            outcome,
            sealed_at_ms,
            prev_hash,
            testnet: Some(TestnetEvidence {
                experiment_id: report.experiment_id.clone(),
                chain_id: report.chain_id.clone(),
                asset: report.asset.clone(),
                amount_wei: report.amount_wei,
                authorized_wei,
                destination: report.destination.clone(),
                tx_hash: report.tx_hash.clone(),
                policy_mode: ExecutionMode::Testnet,
            }),
            hash: [0u8; 32],
        };
        ev.hash = *blake3::hash(&serde_json::to_vec(&ev).unwrap_or_default()).as_bytes();
        ev
    }

    /// Recompute the seal and compare. Pure; no I/O.
    #[must_use]
    pub fn verify_seal(&self) -> bool {
        let mut cento = self.clone();
        cento.hash = [0u8; 32];
        let expect = blake3::hash(&serde_json::to_vec(&cento).unwrap_or_default());
        expect.as_bytes() == &self.hash
    }
}

/// Append-only evidence store with hash chaining.
#[derive(Debug, Default, Clone)]
pub struct EvidenceLog {
    entries: Vec<ExperimentEvidence>,
}

impl EvidenceLog {
    /// Empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append evidence sealed against the current tip. Rejects evidence
    /// whose `prev_hash` does not match the tip (no silent forks).
    pub fn append(&mut self, ev: ExperimentEvidence) -> Result<(), ProposalError> {
        let tip = self.entries.last().map(|e| e.hash);
        if ev.prev_hash != tip {
            return Err(ProposalError::ChainBroken(format!(
                "prev_hash mismatch for {}",
                ev.id
            )));
        }
        if !ev.verify_seal() {
            return Err(ProposalError::ChainBroken(format!(
                "bad seal for {}",
                ev.id
            )));
        }
        self.entries.push(ev);
        Ok(())
    }

    /// All entries, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[ExperimentEvidence] {
        &self.entries
    }

    /// Full re-verification: every seal plus every link. Pure.
    #[must_use]
    pub fn verify_chain(&self) -> bool {
        let mut prev: Option<[u8; 32]> = None;
        for e in &self.entries {
            if e.prev_hash != prev || !e.verify_seal() {
                return false;
            }
            prev = Some(e.hash);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    const NOW: u64 = 1_780_000_000;
    use super::*;
    use crate::economic::DenyAllEconomicAuthorization;
    use crate::policy::decide;
    use crate::protocol::parse_proposal;
    use crate::sandbox::execute;

    fn report() -> ExecutionReport {
        let p = parse_proposal(&crate::protocol::sandbox_proposal_json()).unwrap();
        let d = decide(&p, &DenyAllEconomicAuthorization, NOW);
        execute(&p, &d, 1_700_000_000_000).unwrap()
    }

    #[test]
    fn seal_verifies_and_chain_links() {
        let r = report();
        let e0 = ExperimentEvidence::seal(&r, ExperimentOutcome::Success, 1, None);
        assert!(e0.verify_seal());
        let e1 = ExperimentEvidence::seal(&r, ExperimentOutcome::Partial, 2, Some(e0.hash));
        let mut log = EvidenceLog::new();
        log.append(e0).unwrap();
        log.append(e1).unwrap();
        assert!(log.verify_chain());
        assert_eq!(log.entries().len(), 2);
    }

    #[test]
    fn tampered_entry_breaks_chain() {
        let r = report();
        let mut log = EvidenceLog::new();
        log.append(ExperimentEvidence::seal(
            &r,
            ExperimentOutcome::Success,
            1,
            None,
        ))
        .unwrap();
        let mut bad = log.entries()[0].clone();
        bad.outcome = ExperimentOutcome::Failed;
        assert!(!bad.verify_seal());
        let mut log2 = EvidenceLog::new();
        assert!(log2.append(bad).is_err());
    }

    #[test]
    fn fork_prev_hash_rejected() {
        let r = report();
        let mut log = EvidenceLog::new();
        log.append(ExperimentEvidence::seal(
            &r,
            ExperimentOutcome::Success,
            1,
            None,
        ))
        .unwrap();
        let fork = ExperimentEvidence::seal(&r, ExperimentOutcome::Success, 2, None);
        assert!(log.append(fork).is_err());
    }
}
