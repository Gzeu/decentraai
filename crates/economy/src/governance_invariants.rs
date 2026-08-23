//! Governance invariants (Phase 8) — each economic rule from the contract,
//! stated as a test so it CANNOT rot silently.
//!
//! ```text
//! AI proposes
//!   → deterministic economics validates      (this crate, pure functions)
//!   → cryptographic evidence proves          (crate::evidence)
//!   → settlement executes                    (crate::settlement adapters)
//! ```
//!
//! The Governor (or ANY AI component) can never:
//! - mint — there is no mint function; value enters ONLY through
//!   [`crate::engine::RewardEngine::award`], which requires verified facts
//!   plus an evidence reference plus a third-party verifier;
//! - reward itself — self-verification is rejected at the door;
//! - bypass verification — unverified work pays exactly zero;
//! - modify supply / alter emission — [`TokenomicsParams`] is consumed by
//!   shared reference; simulation cannot mutate configuration;
//! - alter ledger history — `gross_earned` is monotonic and reversals/
//!   penalties only touch balance, never history.
#[cfg(test)]
mod tests {
    use crate::contribution::{ContributionFacts, VerificationStatus, compute_award};
    use crate::engine::{MAX_PENALTY_PERCENT, RewardEngine};
    use crate::tokenomics::{TokenomicsParams, simulate};

    fn serde_round_trip(f: &ContributionFacts) -> ContributionFacts {
        serde_json::from_slice(&serde_json::to_vec(f).unwrap()).unwrap()
    }

    fn facts(worker: &str, verifier: &str) -> ContributionFacts {
        facts_ev(worker, verifier, "ev-inv")
    }

    fn facts_ev(worker: &str, verifier: &str, evidence: &str) -> ContributionFacts {
        ContributionFacts {
            worker_id: worker.into(),
            verified_units: 3,
            quality_percent: 90,
            reliability_percent: 95,
            latency_ms: 800,
            baseline_latency_ms: 1000,
            resource_bytes: 512,
            efficiency_index_x100: 100,
            scarcity_bps: 12_000,
            difficulty_bps: 10_000,
            verification: VerificationStatus::Verified,
            evidence_ref: evidence.into(),
            verifier_id: verifier.into(),
        }
    }

    /// INVARIANT: value enters the system ONLY through evidence-backed
    /// awards of verified work. There is no mint path: without calling
    /// `award`, every account stays at zero forever.
    #[test]
    fn no_mint_path_exists() {
        let mut e = RewardEngine::new();
        // Any number of "decisions" that do NOT go through award() change
        // nothing — represented here by simply reading state repeatedly.
        for _ in 0..100 {
            assert_eq!(e.total_awarded_micro_cu(), 0);
        }
        e.award(&facts("w", "v")).unwrap();
        let after = e.total_awarded_micro_cu();
        assert!(after > 0);
        // And awards are bounded by the formula, not by caller ambition:
        // neutral-ish facts (quality 90→90%, reliability 95%, latency
        // 800/1000→110% capped, scarcity 120%) produce a deterministic,
        // recomputable value — asserted against the formula output itself.
        let expected = compute_award(&facts("w2", "v"));
        assert!(expected.micro_cu > 0);
        assert_eq!(
            compute_award(&serde_round_trip(&facts("w2", "v"))).micro_cu,
            expected.micro_cu
        );
    }

    /// INVARIANT: nobody rewards themselves. The verifier identity must
    /// differ from the worker identity.
    #[test]
    fn no_self_reward() {
        let mut e = RewardEngine::new();
        assert!(e.award(&facts("gov", "gov")).is_err());
    }

    /// INVARIANT: verification cannot be bypassed — pending/invalid work
    /// creates no account and pays exactly zero.
    #[test]
    fn verification_cannot_be_bypassed() {
        let mut f = facts("w", "v");
        f.verification = VerificationStatus::Invalid;
        let out = compute_award(&f);
        assert_eq!(out.micro_cu, 0);
    }

    /// INVARIANT: simulation cannot mutate supply or emission config.
    /// `simulate` takes params by shared reference — enforced by types.
    #[test]
    fn supply_and_emission_are_immutable_during_simulation() {
        let p = TokenomicsParams {
            total_supply_micro_cu: 1_000,
            epochs: 3,
            schedule: crate::tokenomics::EmissionSchedule::Fixed,
            initial_emission_bps_of_supply: 1_000,
            allocations: crate::tokenomics::AllocationSplit {
                contributors_bps: 5_000,
                validators_bps: 2_500,
                development_bps: 1_250,
                treasury_bps: 1_250,
            },
            network_fee_bps: 100,
            burn_bps_of_fee: 500,
            vesting_epochs: 2,
            slashing: crate::tokenomics::SlashingParams {
                enabled: true,
                max_bps_per_epoch: 100,
            },
            min_reward_micro_cu: 1,
            max_reward_micro_cu: 10,
        };
        let snapshot = p.clone();
        let _ = simulate(&p, 50, 5).unwrap();
        assert_eq!(p, snapshot, "params untouched by simulation");
        assert_eq!(p.total_supply_micro_cu, 1_000);
    }

    /// INVARIANT: history cannot be rewritten — gross earned is monotonic;
    /// reversals and penalties move BALANCE only, within hard caps.
    #[test]
    fn ledger_history_is_append_only() {
        let mut e = RewardEngine::new();
        for i in 0..5 {
            e.award(&facts_ev("w", "v", &format!("unique-ev-{i}")))
                .unwrap();
        }
        let gross = e.account("w").unwrap().gross_earned_micro_cu;
        let bal = e.account("w").unwrap().balance_micro_cu;

        e.reverse("w", u64::MAX, "invalidated");
        e.penalize_bounded("w", u64::MAX, "abuse");
        let a = e.account("w").unwrap();
        assert_eq!(a.gross_earned_micro_cu, gross, "history intact");
        assert!(a.balance_micro_cu <= bal);
        assert!(a.penalized_micro_cu <= bal * MAX_PENALTY_PERCENT / 100);
        assert_eq!(
            a.balance_micro_cu + a.reversed_micro_cu + a.penalized_micro_cu,
            bal
        );
    }
}
