//! Reputation-based compensation for worker contributions (M9-9).
//!
//! DecentraAI is **not** a payment platform (see AGENTS.md: payments are out
//! of scope). "Compensation" here is a deterministic ledger of **contribution
//! credits** that lets the operator see, in a single number, how much a worker
//! earned from the compute it served — the same signal M17 uses for tiers, now
//! with an explicit reputation dimension on top. The credits are synthetic
//! bookkeeping only; they mint or move no money, and nothing here touches the
//! token registry.
//!
//! # Why reputation
//!
//! The [contribution score][`crate::contribution_score`] already rewards
//! *hardware × availability × verified work*. M9-9 adds the **reputation**
//! axis: two workers of identical hardware and uptime who served the same
//! number of requests should not be credited identically if one did so with a
//! clean record and the other with a trail of failures/gatekeeping. A
//! reputation-sackled multiplier scales credited work down as the failure
//! ratio climbs, so compensation tracks *trusted, verifiably-good* service.
//!
//! The reputation signal is derived purely from the contribution ledger's own
//! verified/failed counts — there is no separate "reputation score" fed in.
//! This is a deliberate choice that keeps the policy pure and testable, and it
//! is the honest one for earnings: a worker whose verified-transfer
//! reputation is bad (cryptographic-integrity failure) is already **banned and
//! therefore never routed work**, so it earns nothing regardless. What remains
//! for compensation to reward is how cleanly the work a worker *did* serve was
//! completed — exactly what `verified_requests` vs `failed_requests` records.
//!
//! # Policy
//!
//! ```text
//! base           = verified_requests * tokens_per_verified_request
//! quality_factor = contribution_score / CONTRIBUTOR_SCORE        (clamped to [q_min, q_max])
//! reputation_mult = clean_ratio.PowRep (clean_ratio in [0,1])     (>= 1.0 when clean, < 1.0 as it fails)
//! credits        = round(base * quality_factor * reputation_mult)
//! ```
//!
//! The `clean_ratio` is `verified / (verified + failed)`; raising it to a
//! positive power keeps it near 1.0 until the failure rate grows, then damps
//! quickly. All terms are `f64` on named constants so the policy reads as
//! policy, not magic numbers, and every function is I/O-free.

use serde::{Deserialize, Serialize};

use crate::contribution::{contribution_score, ContributionProfile};
use crate::contribution::CONTRIBUTOR_SCORE;

/// The contribution-reward policy (M9-9). Plain data, serde-serializable for
/// transport/config, with named constants so the admin tunes it in one place.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct RewardPolicy {
    /// Credits granted per verified request, before quality/reputation terms.
    pub tokens_per_verified_request: u64,
    /// Quality is contribution_score / CONTRIBUTOR_SCORE clamped to
    /// `[quality_min, quality_max]`.
    pub quality_min: f64,
    pub quality_max: f64,
    /// Exponent on the clean-ratio reputation term. >1 rewards clean service
    /// more steeply; 1.0 makes it linear.
    pub reputation_power: f64,
}

impl Default for RewardPolicy {
    fn default() -> Self {
        Self {
            tokens_per_verified_request: 1,
            quality_min: 0.5,
            quality_max: 2.0,
            reputation_power: 2.0,
        }
    }
}

impl RewardPolicy {
    /// The reputation multiplier for a worker with `verified` successes and
    /// `failed` failures. `clean_ratio = verified/(verified+failed)`, raised to
    /// `reputation_power`. A flawless worker gets 1.0; a perfect failure gets
    /// 0.0 (and no credits). An idle worker (no attempts) is treated as 1.0 so
    /// earning nothing comes from the `verified_requests == 0` base, not from a
    /// spurious zero.
    pub fn reputation_multiplier(&self, verified: u64, failed: u64) -> f64 {
        let total = verified + failed;
        if total == 0 {
            return 1.0;
        }
        let clean_ratio = verified as f64 / total as f64;
        clean_ratio.powf(self.reputation_power)
    }
}

/// Computes a deterministic credit reward for one worker (M9-9).
///
/// Honest degradation rules:
/// - Zero verified work → exactly 0 credits, regardless of hardware/uptime
///   (you earn by *serving*, mirroring `suggest_tier`).
/// - A perfect failure record → 0 credits (the load was never completed).
/// - Otherwise `verified * rate` scaled by contribution quality and the
///   reputation multiplier; the result is rounded to a whole number of
///   credits, so the ledger stays integer-valued.
pub fn reward_tokens(
    profile: &ContributionProfile,
    policy: &RewardPolicy,
) -> u64 {
    let RewardPolicy {
        tokens_per_verified_request,
        quality_min,
        quality_max,
        reputation_power,
    } = *policy;

    if profile.verified_requests == 0 {
        return 0;
    }

    let base = profile.verified_requests.saturating_mul(tokens_per_verified_request) as f64;
    // Scale raw hardware×uptime×work down to a quality factor around 1.0.
    let quality = (contribution_score(profile) / CONTRIBUTOR_SCORE).clamp(quality_min, quality_max);
    // Reputation term.
    let rep = if total_attempts(profile) > 0 {
        let clean = profile.verified_requests as f64
            / (profile.verified_requests + profile.failed_requests) as f64;
        clean.powf(reputation_power)
    } else {
        1.0
    };

    let credits = base * quality * rep;
    credits.round().max(0.0) as u64
}

/// Total serving attempts (verified + failed) for a profile.
pub fn total_attempts(profile: &ContributionProfile) -> u64 {
    profile.verified_requests.saturating_add(profile.failed_requests)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> ContributionProfile {
        ContributionProfile {
            cpu_cores: 8,
            ram_mb: 32 * 1024,
            vram_mb: 16 * 1024,
            online_seconds: 24 * 3600,
            verified_requests: 1000,
            failed_requests: 0,
        }
    }

    #[test]
    fn zero_verified_work_earns_nothing() {
        let idle = ContributionProfile {
            verified_requests: 0,
            failed_requests: 0,
            ..healthy()
        };
        let p = RewardPolicy::default();
        assert_eq!(reward_tokens(&idle, &p), 0, "idle node earns nothing");
    }

    #[test]
    fn a_complete_failure_record_earns_nothing() {
        // Verified nothing and only failed: the base term is zero and the early
        // return (and the reputation term's clean ratio of 0) both land on 0.
        let failing = ContributionProfile {
            verified_requests: 0,
            failed_requests: 2000,
            ..healthy()
        };
        let p = RewardPolicy::default();
        assert_eq!(reward_tokens(&failing, &p), 0);
    }

    #[test]
    fn reward_is_monotone_in_verified_work() {
        let p = RewardPolicy::default();
        let light = ContributionProfile {
            verified_requests: 10,
            ..healthy()
        };
        let heavy = ContributionProfile {
            verified_requests: 2000,
            ..healthy()
        };
        let light_earnings = reward_tokens(&light, &p);
        let heavy_earnings = reward_tokens(&heavy, &p);
        assert!(
            heavy_earnings > light_earnings,
            "2000 verified ({heavy_earnings}cr) > 10 verified ({light_earnings}cr)"
        );
    }

    #[test]
    fn reputation_demotes_compensation_for_failures() {
        let p = RewardPolicy::default();
        let clean = reward_tokens(&healthy(), &p);
        let sloppy = ContributionProfile {
            failed_requests: healthy().verified_requests, // 50% failure
            ..healthy()
        };
        let sloppy_earnings = reward_tokens(&sloppy, &p);
        assert!(
            sloppy_earnings < clean,
            "50% failure ({sloppy_earnings}cr) must credit less than clean ({clean}cr)"
        );
    }

    #[test]
    fn policy_rate_scales_earnings() {
        let p1 = RewardPolicy {
            tokens_per_verified_request: 1,
            ..Default::default()
        };
        let p10 = RewardPolicy {
            tokens_per_verified_request: 10,
            ..Default::default()
        };
        let r1 = reward_tokens(&healthy(), &p1);
        let r10 = reward_tokens(&healthy(), &p10);
        assert_eq!(r10, r1 * 10, "doubling the rate must scale credits 10x");
    }

    #[test]
    fn reputation_multiplier_boundaries() {
        let p = RewardPolicy::default();
        assert!((p.reputation_multiplier(100, 0) - 1.0).abs() < 1e-9, "flawless -> 1.0");
        assert!((p.reputation_multiplier(0, 100) - 0.0).abs() < 1e-9, "all fail -> 0.0");
        assert!((p.reputation_multiplier(0, 0) - 1.0).abs() < 1e-9, "idle -> 1.0");
    }
}