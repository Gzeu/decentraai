//! Experiment store — restart-safe bounded-execution state.
//!
//! [`ExperimentStore`] records every attempt, submission and outcome keyed
//! by experiment id. It serializes to plain JSON ([`ExperimentStore::to_json`]
//! / [`from_json`]) so the runtime can persist it to disk: after a restart
//! the store reloads and in-flight experiments resume WITHOUT double-spend
//! (submitted experiments replay their cached tx hash, never re-execute).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::budget::TestnetAsset;
use crate::error::ProposalError;
use crate::protocol::ExperimentProposal;

/// Lifecycle of one experiment. Terminal states never leave the store
/// (audit trail), but only non-terminal states may execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ExperimentStatus {
    /// Authorized, nothing submitted yet.
    Authorized,
    /// Broadcast, awaiting confirmation.
    Submitted {
        /// Chain tx hash.
        tx_hash: String,
    },
    /// Chain-confirmed.
    Confirmed {
        /// Chain tx hash.
        tx_hash: String,
    },
    /// Failed (executor error or denial after authorization).
    Failed {
        /// Reason.
        reason: String,
    },
}

/// One experiment's durable record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentRecord {
    /// Idempotency key.
    pub experiment_id: String,
    /// Proposal it runs.
    pub proposal_id: String,
    /// Budget backing it.
    pub budget_id: String,
    /// Asset (for exact replay reports).
    pub asset: TestnetAsset,
    /// Destination (for exact replay reports).
    pub destination: String,
    /// Amount authorized (wei).
    pub amount_wei: u64,
    /// Current lifecycle state.
    pub status: ExperimentStatus,
    /// Attempts used (retry accounting survives restarts).
    pub attempts_used: u32,
    /// Tx hash once submitted (replay source).
    pub tx_hash: Option<String>,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
    /// Last update (unix ms).
    pub updated_at_ms: u64,
}

/// Attempt metadata bundled for [`ExperimentStore::record_attempt`]
/// (keeps the call under the argument-count lint).
pub struct AttemptInfo<'a> {
    /// Proposal running.
    pub proposal: &'a ExperimentProposal,
    /// Budget backing it.
    pub budget_id: &'a str,
    /// Asset (for exact replay reports).
    pub asset: &'a TestnetAsset,
    /// Destination (for exact replay reports).
    pub destination: &'a str,
    /// Amount authorized (wei).
    pub amount_wei: u64,
    /// Attempts used including this one.
    pub attempts_used: u32,
    /// Now (unix ms).
    pub now_ms: u64,
}

/// In-memory store with JSON persistence. Deterministic iteration
/// (BTreeMap) so reports and digests are stable.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentStore {
    records: BTreeMap<String, ExperimentRecord>,
}

impl ExperimentStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up an experiment by id.
    #[must_use]
    pub fn get(&self, experiment_id: &str) -> Option<&ExperimentRecord> {
        self.records.get(experiment_id)
    }

    /// How many experiments are tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// True when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Record an attempt (creates the record on first attempt, bumps the
    /// counter after). Crash-safe accounting: the attempt is stored BEFORE
    /// the executor runs, so a crash-then-restart never loses count.
    pub fn record_attempt(&mut self, experiment_id: &str, info: AttemptInfo<'_>) {
        let AttemptInfo {
            proposal,
            budget_id,
            asset,
            destination,
            amount_wei,
            attempts_used,
            now_ms,
        } = info;
        self.records
            .entry(experiment_id.to_string())
            .and_modify(|r| {
                r.attempts_used = attempts_used;
                r.updated_at_ms = now_ms;
            })
            .or_insert_with(|| ExperimentRecord {
                experiment_id: experiment_id.to_string(),
                proposal_id: proposal.id.clone(),
                budget_id: budget_id.to_string(),
                asset: asset.clone(),
                destination: destination.to_string(),
                amount_wei,
                status: ExperimentStatus::Authorized,
                attempts_used,
                tx_hash: None,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            });
    }

    /// Mark broadcast. Sets the replayable tx hash.
    pub fn mark_submitted(&mut self, experiment_id: &str, tx_hash: &str, now_ms: u64) {
        if let Some(r) = self.records.get_mut(experiment_id) {
            r.status = ExperimentStatus::Submitted {
                tx_hash: tx_hash.to_string(),
            };
            r.tx_hash = Some(tx_hash.to_string());
            r.updated_at_ms = now_ms;
        }
    }

    /// Mark chain-confirmed.
    pub fn mark_confirmed(&mut self, experiment_id: &str, tx_hash: &str, now_ms: u64) {
        if let Some(r) = self.records.get_mut(experiment_id) {
            r.status = ExperimentStatus::Confirmed {
                tx_hash: tx_hash.to_string(),
            };
            r.tx_hash = Some(tx_hash.to_string());
            r.updated_at_ms = now_ms;
        }
    }

    /// Mark failed with reason.
    pub fn mark_failed(&mut self, experiment_id: &str, reason: &str, now_ms: u64) {
        if let Some(r) = self.records.get_mut(experiment_id) {
            r.status = ExperimentStatus::Failed {
                reason: reason.to_string(),
            };
            r.updated_at_ms = now_ms;
        }
    }

    /// Serialize the whole store (what the runtime persists to disk).
    pub fn to_json(&self) -> Result<String, ProposalError> {
        serde_json::to_string(self).map_err(|e| ProposalError::Bound(e.to_string()))
    }

    /// Reload a persisted store (restart recovery). Unknown fields fail
    /// closed — a corrupt or foreign store never loads silently.
    pub fn from_json(json: &str) -> Result<Self, ProposalError> {
        serde_json::from_str(json).map_err(|e| ProposalError::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_preserves_state() {
        let mut s = ExperimentStore::new();
        let proposal = ExperimentProposal {
            version: 1,
            id: "prop:r".to_string(),
            idea_id: "idea:r".to_string(),
            risk: crate::risk::ExperimentRiskClass::TestnetEconomic,
            commitment: crate::risk::ResourceCommitment::Cr,
            budget: None,
            steps: vec![],
            created_by: "t".to_string(),
        };
        s.record_attempt(
            "exp:1",
            AttemptInfo {
                proposal: &proposal,
                budget_id: "budget:1",
                asset: &TestnetAsset::Xegld,
                destination: "erd1x",
                amount_wei: 1_000,
                attempts_used: 1,
                now_ms: 100,
            },
        );
        s.mark_submitted("exp:1", "hash:abc", 101);
        let json = s.to_json().unwrap();
        let back = ExperimentStore::from_json(&json).unwrap();
        assert_eq!(s, back);
        assert_eq!(back.get("exp:1").unwrap().attempts_used, 1);
        assert_eq!(
            back.get("exp:1").unwrap().tx_hash.as_deref(),
            Some("hash:abc")
        );
    }

    #[test]
    fn corrupt_store_fails_closed() {
        assert!(ExperimentStore::from_json("{not json").is_err());
        assert!(ExperimentStore::from_json(r#"{"records":{"x":{"nope":1}}}"#).is_err());
    }
}
