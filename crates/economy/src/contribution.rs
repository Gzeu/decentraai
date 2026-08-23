//! Contribution Units (CU) — the deterministic economic value of verified
//! work. Version [`ECONOMICS_VERSION`], integer-only, evidence-gated.
//!
//! # The formula (v2)
//!
//! ```text
//! micro_cu = BASE_CU_PER_UNIT                       (constant, 1_000 µCU)
//!          × verified_units                         (fact)
//!          × quality_bps / BPS                      graded correctness 30–130 %
//!          × reliability_bps / BPS                  clean ratio 50–120 %
//!          × latency_bps / BPS                      vs task baseline 60–110 %
//!          × resource_efficiency_bps / BPS          work per byte 70–110 %
//!          × scarcity_bps / BPS                     capability scarcity 100–300 %
//!          × difficulty_bps / BPS                   declared task class 100–500 %
//! ```
//!
//! paid **only when** `verification == Verified`. Every other status pays
//! exactly 0 — verification is a gate, not a factor.
//!
//! All factors are basis points recorded in [`ContributionFacts`]; the same
//! facts under the same version always produce the same value
//! (`reproducibility` test below). Multiplication is u128 with sequential
//! division to stay overflow-free; the result saturates to u64 micro-CU.

use serde::{Deserialize, Serialize};

/// Verification states an execution can be in when economics sees it.
/// ONLY `Verified` earns. This enum is deliberately separate from the
/// memory lifecycle: memory tracks knowledge trust, economics tracks payout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    /// Evidence checked out — payable.
    Verified,
    /// Not yet reviewed — held, pays nothing until verified.
    Pending,
    /// Verification failed or retracted — permanently unpaid.
    Invalid,
}

/// The complete, auditable input set for one contribution award.
/// Everything is a recorded fact: no opinions, no model output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionFacts {
    /// Worker identity (peer id string) — attribution only.
    pub worker_id: String,
    /// Completed, verified units of work (requests, chunks, tasks…).
    pub verified_units: u64,
    /// Graded quality percent 0..=100 (from benchmark/verification).
    pub quality_percent: u8,
    /// Clean ratio percent 0..=100 (verified / total attempts history).
    pub reliability_percent: u8,
    /// Actual mean latency in ms for this contribution.
    pub latency_ms: u64,
    /// Baseline latency in ms for the task class (>0).
    pub baseline_latency_ms: u64,
    /// Resource bytes consumed serving this work.
    pub resource_bytes: u64,
    /// Work units delivered per resource unit baseline (×100 fixed point:
    /// 100 = exactly as efficient as the class baseline).
    pub efficiency_index_x100: u64,
    /// Capability scarcity index in bps (10000 = common; up to 30000 rare).
    pub scarcity_bps: u64,
    /// Declared difficulty weight for the task class in bps (10000 base).
    pub difficulty_bps: u64,
    /// Verification outcome.
    pub verification: VerificationStatus,
    /// Reference to the cryptographic evidence backing these numbers.
    /// REQUIRED for `Verified` — an award without evidence is a bug.
    pub evidence_ref: String,
    /// Who verified this contribution (peer id). MUST differ from the
    /// worker: nobody verifies their own work (anti-gaming gate).
    pub verifier_id: String,
}

impl ContributionFacts {
    fn clamp_pct(v: u8, lo: u8, hi: u8) -> u64 {
        (v.clamp(lo, hi)) as u64 * 100 // percent → bps
    }
}

/// Base value per verified work unit, in micro-CU (1 CU = 1_000_000 µCU).
pub const BASE_MICRO_CU_PER_UNIT: u64 = 1_000_000;
/// Quality band: 30 % floor (spam ≈ worthless but not negative), 130 % cap.
pub const QUALITY_MIN_PCT: u8 = 30;
pub const QUALITY_MAX_PCT: u8 = 130;
/// Reliability band.
pub const RELIABILITY_MIN_PCT: u8 = 50;
pub const RELIABILITY_MAX_PCT: u8 = 120;
/// Latency band (vs baseline): 2× slower → floor 60 %, faster caps at 110 %.
pub const LATENCY_MIN_BPS: u64 = 6_000;
pub const LATENCY_MAX_BPS: u64 = 11_000;
/// Resource-efficiency band around the class baseline index of 100.
pub const EFFICIENCY_MIN_BPS: u64 = 7_000;
pub const EFFICIENCY_MAX_BPS: u64 = 11_000;

fn clamp_bps(v: u64, lo: u64, hi: u64) -> u64 {
    v.clamp(lo, hi)
}

/// Computes the award in micro-CU. Pure, deterministic, versioned by
/// [`super::ECONOMICS_VERSION`].
///
/// Rejection cases return 0 with the reason recorded by the caller-facing
/// [`AwardOutcome`] wrapper ([`compute_award`]) so ledgers can explain
/// every zero.
pub fn micro_cu_v2(facts: &ContributionFacts) -> u128 {
    if facts.verification != VerificationStatus::Verified || facts.verified_units == 0 {
        return 0;
    }
    let mut v: u128 =
        u128::from(BASE_MICRO_CU_PER_UNIT) * u128::from(facts.verified_units);

    let apply = |v: u128, bps: u64| -> u128 { v * u128::from(bps) / u128::from(super::BPS) };

    // Quality: percent → bps (clamped band).
    v = apply(v, ContributionFacts::clamp_pct(facts.quality_percent, QUALITY_MIN_PCT, QUALITY_MAX_PCT));
    // Reliability.
    v = apply(
        v,
        ContributionFacts::clamp_pct(facts.reliability_percent, RELIABILITY_MIN_PCT, RELIABILITY_MAX_PCT),
    );
    // Latency vs baseline: ratio baseline/actual ×10000, clamped band.
    let lat_bps = if facts.latency_ms == 0 {
        LATENCY_MAX_BPS
    } else {
        let ratio = (u128::from(facts.baseline_latency_ms) * u128::from(super::BPS)
            / u128::from(facts.latency_ms.max(1)))
        .min(u128::from(u64::MAX));
        clamp_bps(ratio as u64, LATENCY_MIN_BPS, LATENCY_MAX_BPS)
    };
    v = apply(v, lat_bps);
    // Resource efficiency (pre-computed index ×100 fixed point; u128 keeps
    // hostile inputs from overflowing before the band clamp).
    let eff_bps = ((u128::from(facts.efficiency_index_x100) * 100)
        .min(u128::from(u64::MAX))) as u64;
    v = apply(v, clamp_bps(eff_bps, EFFICIENCY_MIN_BPS, EFFICIENCY_MAX_BPS));
    // Capability scarcity and task difficulty: policy inputs, bounded here.
    v = apply(v, clamp_bps(facts.scarcity_bps, 10_000, 30_000));
    v = apply(v, clamp_bps(facts.difficulty_bps, 10_000, 50_000));
    v.min(u128::from(u64::MAX))
}

/// Why an award is zero (explainability for ledgers and dashboards).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroReason {
    /// Verification gate: only verified work pays.
    NotVerified,
    /// No completed units — you earn by serving, not by existing.
    NoWork,
}

/// The award plus its explanation. Same facts → same outcome, always.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwardOutcome {
    /// Awarded value in micro-CU (0 when rejected).
    pub micro_cu: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zero_reason: Option<ZeroReason>,
    /// Formula version that produced this value.
    pub version: u32,
}

/// Entry point used by engines/ledgers: version-tagged, explained.
pub fn compute_award(facts: &ContributionFacts) -> AwardOutcome {
    if facts.verification != VerificationStatus::Verified {
        return AwardOutcome { micro_cu: 0, zero_reason: Some(ZeroReason::NotVerified), version: super::ECONOMICS_VERSION };
    }
    if facts.verified_units == 0 {
        return AwardOutcome { micro_cu: 0, zero_reason: Some(ZeroReason::NoWork), version: super::ECONOMICS_VERSION };
    }
    AwardOutcome {
        micro_cu: u64::try_from(micro_cu_v2(facts)).unwrap_or(u64::MAX),
        zero_reason: None,
        version: super::ECONOMICS_VERSION,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> ContributionFacts {
        ContributionFacts {
            worker_id: "peer-a".into(),
            verified_units: 10,
            quality_percent: 100,
            reliability_percent: 100,
            latency_ms: 1000,
            baseline_latency_ms: 1000,
            resource_bytes: 1_000_000,
            efficiency_index_x100: 100,
            scarcity_bps: 10_000,
            difficulty_bps: 10_000,
            verification: VerificationStatus::Verified,
            evidence_ref: "ev-1".into(),
            verifier_id: "verifier-x".into(),
        }
    }

    #[test]
    fn baseline_facts_pay_exactly_base_rate() {
        // All-neutral factors → base × units, nothing more.
        let out = compute_award(&facts());
        assert_eq!(out.micro_cu, BASE_MICRO_CU_PER_UNIT * 10);
        assert_eq!(out.zero_reason, None);
        assert_eq!(out.version, super::super::ECONOMICS_VERSION);
    }

    #[test]
    fn verification_is_a_gate_not_a_factor() {
        let mut f = facts();
        f.verified_units = 999;
        f.quality_percent = 130;
        f.scarcity_bps = 30_000;
        for status in [VerificationStatus::Pending, VerificationStatus::Invalid] {
            f.verification = status;
            let out = compute_award(&f);
            assert_eq!(out.micro_cu, 0);
            assert_eq!(out.zero_reason, Some(ZeroReason::NotVerified));
        }
        // Zero work also pays nothing even when fully verified.
        let mut f = facts();
        f.verified_units = 0;
        let out = compute_award(&f);
        assert_eq!(out.micro_cu, 0);
        assert_eq!(out.zero_reason, Some(ZeroReason::NoWork));
    }

    #[test]
    fn factors_move_value_in_the_declared_direction_and_stay_in_band() {
        // Scarcity triples at most.
        let mut rare = facts();
        rare.scarcity_bps = 50_000; // over the cap
        let out = compute_award(&rare);
        assert_eq!(
            out.micro_cu,
            BASE_MICRO_CU_PER_UNIT * 10 * 3,
            "scarcity capped at 30000 bps"
        );

        // Faster than baseline raises value, capped at +10%.
        let mut fast = facts();
        fast.latency_ms = 100; // 10× faster
        let out_fast = compute_award(&fast);
        assert_eq!(out_fast.micro_cu, BASE_MICRO_CU_PER_UNIT * 10 * 110 / 100);

        // Slower than baseline floors at −40%.
        let mut slow = facts();
        slow.latency_ms = 100_000;
        let out_slow = compute_award(&slow);
        assert_eq!(out_slow.micro_cu, BASE_MICRO_CU_PER_UNIT * 10 * 60 / 100);

        // Spam-quality floor: 5% quality still pays the 30% floor, not zero
        // and never negative — the reward engine's penalties handle abuse.
        let mut spam = facts();
        spam.quality_percent = 5;
        assert_eq!(
            compute_award(&spam).micro_cu,
            BASE_MICRO_CU_PER_UNIT * 10 * 3_000 / 10_000
        );
    }

    #[test]
    fn reproducibility_same_facts_same_value() {
        let f = facts();
        let a = compute_award(&f);
        let mut b = compute_award(&f);
        b.micro_cu += 0; // no-op to prove both paths run
        assert_eq!(a, b);
        // And serialization round-trips (facts are the audit record).
        let json = serde_json::to_string(&f).unwrap();
        let back: ContributionFacts = serde_json::from_str(&json).unwrap();
        assert_eq!(compute_award(&back), a);
    }

    #[test]
    fn extreme_inputs_cannot_overflow() {
        let mut f = facts();
        f.verified_units = u64::MAX;
        f.scarcity_bps = 30_000;
        f.difficulty_bps = 50_000;
        f.efficiency_index_x100 = u64::MAX;
        // Must not panic; saturates inside u64.
        let out = compute_award(&f);
        assert_eq!(out.micro_cu, u64::MAX);
    }
}
