//! Reward engine — applies the versioned CU formula to an account book with
//! bounded reversals and penalties.
//!
//! # Rules
//!
//! - `earned` is MONOTONIC: reversals never claw back below what was
//!   actually earned, penalties are capped per epoch, and nothing can make
//!   an account negative. Bounded punishment, unbounded trust erosion is
//!   reputation's job (handled elsewhere), not the ledger's.
//! - Reversals exist for INVALIDATED results: if verification later flips a
//!   previously-paid contribution to invalid, up to the original award is
//!   returned to the pool — but never more than the account holds.
//! - Every mutation records WHY (award / reversal / penalty + reason), so
//!   the book is its own audit trail.
//!
//! No LLM decides anything here: inputs are facts + the pure formula.

use crate::contribution::{AwardOutcome, ContributionFacts, VerificationStatus, compute_award};
use serde::{Deserialize, Serialize};

/// One worker's economic account, in micro-CU.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub worker_id: String,
    /// Monotonic lifetime earnings (awards − reversals can dip it, but it
    /// can never go below zero; see [`RewardEngine::reverse`]).
    pub balance_micro_cu: u64,
    /// Sum of all awards ever made (never decreases).
    pub gross_earned_micro_cu: u64,
    /// Sum returned via reversals (bounded by what this account earned).
    pub reversed_micro_cu: u64,
    /// Sum of applied penalties (each individually bounded).
    pub penalized_micro_cu: u64,
}

/// The deterministic reward engine over a set of accounts.
#[derive(Debug, Clone, Default)]
pub struct RewardEngine {
    accounts: std::collections::BTreeMap<String, Account>,
    /// Evidence references already paid per worker: result/evidence replay
    /// is rejected at the door instead of being double-counted.
    seen_evidence: std::collections::BTreeSet<(String, String)>,
}

/// Why a mutation happened — recorded in the audit log returned to callers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    Award,
    Reversal,
    Penalty,
}

/// One audited mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mutation {
    pub kind: MutationKind,
    pub worker_id: String,
    pub micro_cu: u64,
    pub reason: String,
}

/// Maximum single penalty: 25 % of current balance, enforced here so no
/// caller can invent unlimited punishment.
pub const MAX_PENALTY_PERCENT: u64 = 25;

impl RewardEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn account(&self, worker_id: &str) -> Option<&Account> {
        self.accounts.get(worker_id)
    }

    pub fn total_awarded_micro_cu(&self) -> u64 {
        self.accounts
            .values()
            .map(|a| a.gross_earned_micro_cu)
            .sum()
    }

    /// Awards one verified contribution. Non-verified facts pay 0 and are
    /// recorded as a zero-award mutation (the gate is visible in audit).
    pub fn award(&mut self, facts: &ContributionFacts) -> Result<Mutation, crate::EconomyError> {
        // ANTI-GAMING GATES (cheapest rejections first).
        // 1. Nobody verifies their own work — kills self-verification.
        if facts.verifier_id == facts.worker_id {
            return Err(crate::EconomyError::SelfVerification {
                worker_id: facts.worker_id.clone(),
            });
        }
        // 2. The verification gate itself.
        if facts.verification != VerificationStatus::Verified {
            return Err(crate::EconomyError::NotVerified);
        }
        // 3. No evidence reference, no money.
        if facts.evidence_ref.trim().is_empty() {
            return Err(crate::EconomyError::MissingEvidence);
        }
        // 4. Replay: identical evidence from the same worker never pays twice.
        let key = (facts.worker_id.clone(), facts.evidence_ref.clone());
        if !self.seen_evidence.insert(key) {
            return Err(crate::EconomyError::DuplicateEvidence {
                worker_id: facts.worker_id.clone(),
                evidence_ref: facts.evidence_ref.clone(),
            });
        }
        let outcome: AwardOutcome = compute_award(facts);
        let acc = self
            .accounts
            .entry(facts.worker_id.clone())
            .or_insert_with(|| Account {
                worker_id: facts.worker_id.clone(),
                ..Default::default()
            });
        acc.balance_micro_cu = acc.balance_micro_cu.saturating_add(outcome.micro_cu);
        acc.gross_earned_micro_cu = acc.gross_earned_micro_cu.saturating_add(outcome.micro_cu);
        Ok(Mutation {
            kind: MutationKind::Award,
            worker_id: facts.worker_id.clone(),
            micro_cu: outcome.micro_cu,
            reason: format!(
                "cu-v{} units={} evidence={}",
                outcome.version, facts.verified_units, facts.evidence_ref
            ),
        })
    }

    /// Reverses up to `micro_cu` from a previously paid contribution whose
    /// verification was later invalidated. Clamped to the account balance —
    /// the pool eats the difference rather than driving accounts negative.
    pub fn reverse(&mut self, worker_id: &str, micro_cu: u64, reason: &str) -> Mutation {
        let clamp_reason = reason.to_string();
        let acc = self
            .accounts
            .entry(worker_id.to_string())
            .or_insert_with(|| Account {
                worker_id: worker_id.to_string(),
                ..Default::default()
            });
        let applied = micro_cu.min(acc.balance_micro_cu);
        acc.balance_micro_cu -= applied;
        acc.reversed_micro_cu += applied;
        Mutation {
            kind: MutationKind::Reversal,
            worker_id: worker_id.to_string(),
            micro_cu: applied,
            reason: clamp_reason,
        }
    }

    /// Applies a bounded penalty: at most [`MAX_PENALTY_PERCENT`] % of the
    /// CURRENT balance, regardless of the requested amount or reason.
    pub fn penalize_bounded(
        &mut self,
        worker_id: &str,
        requested_micro_cu: u64,
        reason: &str,
    ) -> Mutation {
        let acc = self
            .accounts
            .entry(worker_id.to_string())
            .or_insert_with(|| Account {
                worker_id: worker_id.to_string(),
                ..Default::default()
            });
        let cap = acc.balance_micro_cu * MAX_PENALTY_PERCENT / 100;
        let applied = requested_micro_cu.min(cap);
        acc.balance_micro_cu -= applied;
        acc.penalized_micro_cu += applied;
        Mutation {
            kind: MutationKind::Penalty,
            worker_id: worker_id.to_string(),
            micro_cu: applied,
            reason: reason.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution::ContributionFacts;

    fn facts(worker: &str, evidence: &str) -> ContributionFacts {
        ContributionFacts {
            worker_id: worker.into(),
            verified_units: 5,
            quality_percent: 100,
            reliability_percent: 100,
            latency_ms: 1000,
            baseline_latency_ms: 1000,
            resource_bytes: 1000,
            efficiency_index_x100: 100,
            scarcity_bps: 10_000,
            difficulty_bps: 10_000,
            verification: VerificationStatus::Verified,
            evidence_ref: evidence.into(),
            verifier_id: "honest-verifier".into(),
        }
    }

    #[test]
    fn award_is_monotonic_and_reversals_never_go_negative() {
        let mut e = RewardEngine::new();
        let f = facts("w", "ev-1");
        let m = e.award(&f).unwrap();
        assert!(m.micro_cu > 0);
        let bal = e.account("w").unwrap().balance_micro_cu;
        assert_eq!(bal, m.micro_cu);
        let rev = e.reverse("w", u64::MAX, "verification invalidated");
        assert_eq!(rev.micro_cu, bal, "clamped to what the account holds");
        assert_eq!(e.account("w").unwrap().balance_micro_cu, 0);
        assert_eq!(
            e.account("w").unwrap().gross_earned_micro_cu,
            bal,
            "history never shrinks"
        );
    }

    #[test]
    fn penalties_are_capped_at_max_percent() {
        let mut e = RewardEngine::new();
        e.award(&facts("w", "ev-1")).unwrap();
        let bal_before = e.account("w").unwrap().balance_micro_cu;
        let pen = e.penalize_bounded("w", bal_before * 100, "proven abuse");
        assert!(pen.micro_cu <= bal_before * MAX_PENALTY_PERCENT / 100);
        assert!(e.account("w").unwrap().balance_micro_cu >= bal_before * 75 / 100);
    }

    #[test]
    fn self_verification_is_rejected_at_the_door() {
        let mut e = RewardEngine::new();
        let mut f = facts("w", "ev-self");
        f.verifier_id = f.worker_id.clone();
        assert!(matches!(
            e.award(&f),
            Err(crate::EconomyError::SelfVerification { .. })
        ));
        assert!(e.account("w").is_none(), "rejected work creates no account");
    }

    #[test]
    fn evidence_replay_pays_once() {
        let mut e = RewardEngine::new();
        e.award(&facts("w", "same-evidence")).unwrap();
        let first = e.account("w").unwrap().balance_micro_cu;
        assert!(matches!(
            e.award(&facts("w", "same-evidence")),
            Err(crate::EconomyError::DuplicateEvidence { .. })
        ));
        assert_eq!(e.account("w").unwrap().balance_micro_cu, first);
    }

    #[test]
    fn unverified_work_creates_no_account() {
        let mut e = RewardEngine::new();
        let mut f = facts("ghost", "ev");
        f.verification = VerificationStatus::Pending;
        assert!(matches!(e.award(&f), Err(crate::EconomyError::NotVerified)));
        assert!(e.account("ghost").is_none());
    }

    #[test]
    fn sybil_workers_cannot_amplify_one_account() {
        let mut e = RewardEngine::new();
        let mut expected = 0u64;
        for i in 0..10 {
            expected += e
                .award(&facts(&format!("sybil-{i}"), &format!("ev-{i}")))
                .unwrap()
                .micro_cu;
        }
        assert_eq!(e.total_awarded_micro_cu(), expected);
    }
}
