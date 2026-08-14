//! Contribution scoring and tier suggestion (M17).
//!
//! DecentraAI's subscription model is "everything is free; your tier
//! reflects your contribution". This module turns **measured compute
//! served** — the single most honest signal a node provides — into a
//! suggested tier. It is pure and I/O-free so unit tests drive it with
//! synthetic inputs and the coordinator can score every known worker with
//! real measurements.
//!
//! # What counts as contribution
//!
//! A node earns contribution by serving verified inference on hardware it
//! contributes to the swarm. Three quantities combine multiplicatively:
//!
//! - **Hardware** (`cpu_cores × ram_mb × vram_mb` exposure): GPU memory is
//!   the scarcest and most valuable resource, so VRAM is weighted most.
//! - **Availability hours**: how long the node has been online and
//!   advertising. An always-on server legitimately out-earns an occasional
//!   laptop, even with the same hardware.
//! - **Verified requests**: how many requests it actually served to a
//!   terminal, verified completion. This is the proof-of-work: hardware
//!   could idly sit on the network, but only served compute earns.
//!
//! Failures subtract: a node that accepts work but keeps failing is
//! penalized, discouraging gatekeeping/over-advertising.
//!
//! # Tier thresholds
//!
//! The [`suggest_tier`] mapping is deliberately coarse and documented so it
//! reads like policy, not magic numbers:
//!
//! | Tier | Meaning | Score threshold |
//! |---|---|---|
//! | 1 Guest | invited, small/public models, tight limits | `< 1.0` |
//! | 2 Contributor | shares ≥1 verified model / capacity | `>= 1.0` |
//! | 3 Core | serves large/multiple models, clean record | `>= 10.0` |
//!
//! Thresholds are constants (`CONTRIBUTOR_SCORE`, `CORE_SCORE`) so the
//! admin can tune them from one place.

use serde::{Deserialize, Serialize};

/// Hardware+service profile of one contributing node (M17).
///
/// Every field is a plain measurement the coordinator already has after M16:
/// capability (CPU/RAM/VRAM) comes from the advertisement, hours come from
/// heartbeat accounting, verified requests come from the routing outcome
/// path. The score is a pure function of these, so the same inputs always
/// yield the same tier suggestion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub struct ContributionProfile {
    pub cpu_cores: u16,
    pub ram_mb: u64,
    pub vram_mb: u64,
    /// Cumulative time the node has been online and advertising (seconds).
    pub online_seconds: u64,
    /// Requests served to a terminal, verified completion.
    pub verified_requests: u64,
    /// Requests the node accepted but failed to serve.
    pub failed_requests: u64,
}

/// Score below which a node is a Guest (invited, not yet contributing).
pub const CONTRIBUTOR_SCORE: f64 = 1.0;
/// Score at which a node earns Core (large/multi-model, clean record).
pub const CORE_SCORE: f64 = 6.0;

/// A dimensionless hardware exposure score. VRAM is weighted most because
/// GPU memory is the scarcest shared resource; then RAM, then CPU.
fn hardware_score(p: &ContributionProfile) -> f64 {
    let vram_gib = p.vram_mb as f64 / 1024.0;
    let ram_gib = p.ram_mb as f64 / 1024.0;
    // Each 8 GiB of VRAM ≈ 2.0, each 8 GiB of RAM ≈ 0.5, each 8 cores ≈ 0.5.
    let vram = 2.0 * (vram_gib / 8.0);
    let ram = 0.5 * (ram_gib / 8.0);
    let cpu = 0.5 * (p.cpu_cores as f64 / 8.0);
    (vram + ram + cpu).clamp(0.0, 6.0)
}

/// Availability multiplier: rewards long-lived, always-on contributors and
/// damps sporadic ones. Starts at 1.0 and rises toward 2.0 as uptime grows
/// (roughly plateauing after a day), so a persistent node earns up to a 2x
/// boost over a fresh one of the same hardware.
fn availability_factor(online_seconds: u64) -> f64 {
    let hours = online_seconds as f64 / 3600.0;
    1.0 + (1.0 - (-hours / 24.0).exp()) // 1 + (1 - e^(-hours/24)): → 2.0
}

/// Verified-work multiplier: starts at 1.0 and grows toward 2.0 with each
/// served request, so serving work reliably doubles a Contributor into Core
/// range while an idle node of equal hardware stays a plain Contributor.
fn verified_factor(verified: u64) -> f64 {
    1.0 + (1.0 - (-(verified as f64) / 1000.0).exp())
}

/// Reliability penalty in `(0, 1]`: 1.0 when flawless, approaching 0.2 as
/// the failure rate climbs. Purely failure-ratio driven; a node that simply
/// does little work is not penalized by this factor (its other factors are
/// already low).
fn reliability_factor(failed: u64, verified: u64) -> f64 {
    let total = failed + verified;
    if total == 0 {
        return 1.0;
    }
    let rate = failed as f64 / total as f64;
    (1.0 - 0.8 * rate).max(0.2)
}

/// Full contribution score for a node (M17).
///
/// `hardware × availability × verified` — three dimensions that are all
/// necessary and orthogonal — scaled by reliability. Returns a non-negative
/// `f64`; see [`suggest_tier`] for the policy mapping to tiers.
pub fn contribution_score(p: &ContributionProfile) -> f64 {
    let hw = hardware_score(p);
    let availability = availability_factor(p.online_seconds);
    let verified = verified_factor(p.verified_requests);
    let reliability = reliability_factor(p.failed_requests, p.verified_requests);
    (hw * availability * verified) * reliability
}

/// Maps a contribution score to a subscription tier (1/2/3). This mirrors the
/// token crate's `Tier` (GUEST=1, CONTRIBUTOR=2, CORE=3); the coordinates
/// live here so the mapping reads as policy and the token crate stays free
/// of compute-coupled logic. Guests start at 1 and earn their way up as they
/// serve compute.
///
/// A node with zero verified work always receives Guest regardless of
/// hardware, because an idle/aspirational node has not yet contributed.
pub fn suggest_tier(p: &ContributionProfile) -> u8 {
    if p.verified_requests == 0 {
        return 1;
    }
    let score = contribution_score(p);
    if score >= CORE_SCORE {
        3
    } else if score >= CONTRIBUTOR_SCORE {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_work_is_required_for_contribution() {
        // Huge hardware, huge uptime, but zero served requests.
        let idle = ContributionProfile {
            cpu_cores: 64,
            ram_mb: 512 * 1024,
            vram_mb: 128 * 1024,
            online_seconds: 30 * 86400,
            verified_requests: 0,
            failed_requests: 0,
        };
        assert_eq!(suggest_tier(&idle), 1, "zero verified work -> Guest");
    }

    #[test]
    fn consistent_contributor_beats_core_threshold() {
        // 8-core, 32 GiB RAM, 16 GiB VRAM, online a day, 2000 served, clean.
        let healthy = ContributionProfile {
            cpu_cores: 8,
            ram_mb: 32 * 1024,
            vram_mb: 16 * 1024,
            online_seconds: 24 * 3600,
            verified_requests: 2000,
            failed_requests: 0,
        };
        let score = contribution_score(&healthy);
        assert!(score >= CORE_SCORE, "healthy contributor score={score:.2}");
        assert_eq!(suggest_tier(&healthy), 3);
    }

    #[test]
    fn recurring_failures_demote_contribution() {
        let good = ContributionProfile {
            cpu_cores: 8,
            ram_mb: 32 * 1024,
            vram_mb: 16 * 1024,
            online_seconds: 24 * 3600,
            verified_requests: 1000,
            failed_requests: 0,
        };
        let flaky = ContributionProfile {
            failed_requests: 5000,
            ..good
        };
        let good_score = contribution_score(&good);
        let flaky_score = contribution_score(&flaky);
        assert!(
            flaky_score < good_score,
            "flaky node ({flaky_score:.2}) must score below ({good_score:.2})"
        );
    }

    #[test]
    fn more_served_compute_means_higher_tier() {
        let base = ContributionProfile {
            cpu_cores: 4,
            ram_mb: 16 * 1024,
            vram_mb: 8 * 1024,
            online_seconds: 12 * 3600,
            verified_requests: 100,
            failed_requests: 0,
        };
        let contributor = base;
        let core = ContributionProfile {
            online_seconds: 30 * 86400,
            verified_requests: 100_000,
            ..base
        };
        assert_eq!(suggest_tier(&contributor), 2);
        assert_eq!(suggest_tier(&core), 3);
    }

    #[test]
    fn always_zero_for_empty_profile() {
        let empty = ContributionProfile::default();
        assert_eq!(contribution_score(&empty), 0.0);
        assert_eq!(suggest_tier(&empty), 1);
    }
}
