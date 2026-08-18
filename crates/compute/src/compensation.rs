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

use crate::contribution::CONTRIBUTOR_SCORE;
use crate::contribution::{ContributionProfile, contribution_score};

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
pub fn reward_tokens(profile: &ContributionProfile, policy: &RewardPolicy) -> u64 {
    let RewardPolicy {
        tokens_per_verified_request,
        quality_min,
        quality_max,
        reputation_power,
    } = *policy;

    if profile.verified_requests == 0 {
        return 0;
    }

    let base = profile
        .verified_requests
        .saturating_mul(tokens_per_verified_request) as f64;
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
    profile
        .verified_requests
        .saturating_add(profile.failed_requests)
}

/// Lifetime compensation credits accumulated by one worker.
///
/// The invariant is simple: `earned` is monotonic — it never decreases, so a
/// compensation ledger cannot "lose" credits when the reputation signal
/// improves. Each credit is computed from the contribution profile **at the
/// moment it was earned**, and the profile is frozen into the audit event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CompensationAccount {
    /// Total credits ever earned (monotonic).
    pub earned: u64,
}

/// One auditable compensation credit (provenance).
///
/// Unlike the quota ledger, the *profile at credit time* is frozen into the
/// event so an operator can always explain a given credit: "at request X the
/// worker had 1000 verified / 5 failed, which under this reward policy earned
/// 970 credits".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompensationEvent {
    /// The operation: always `credit` today (reserved for future ops).
    pub op: String,
    /// The account (worker peer id) affected.
    pub account: String,
    /// Credits involved.
    pub amount: u64,
    /// The idempotency key (execution/request id) this refers to.
    pub ref_id: String,
    /// The reward policy that governed the credit (frozen for explainability).
    pub policy: RewardPolicy,
    /// The worker's verified/failed counts when the credit was computed.
    pub verified_requests: u64,
    pub failed_requests: u64,
}

/// The deterministic reputation-compensation core (M9-9).
///
/// Wrap this behind a `Mutex` (never `await` under the lock). All operations
/// are pure, idempotent by `ref_id`, and audited — mirroring `QuotaLedger`.
#[derive(Debug, Default)]
pub struct CompensationLedger {
    /// Per-account balances.
    accounts: std::collections::HashMap<String, CompensationAccount>,
    /// Idempotency: set of `(op, ref_id)` tuples already applied.
    applied: std::collections::HashSet<(String, String)>,
    /// Append-only audit trail (provenance). Bounded to avoid unbounded growth.
    events: std::collections::VecDeque<CompensationEvent>,
    /// The active reward policy.
    policy: RewardPolicy,
}

/// The max number of compensation events retained in memory (bounded provenance).
const MAX_COMPENSATION_EVENTS: usize = 4096;

impl CompensationLedger {
    /// A fresh ledger with the given reward policy.
    pub fn new(policy: RewardPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    /// The active reward policy (read-only).
    pub fn policy(&self) -> RewardPolicy {
        self.policy
    }

    /// Swaps the active reward policy **in place**, preserving all historical
    /// balances and the audit trail. Future credits use the new policy;
    /// already-recorded events keep the policy that produced them.
    pub fn set_policy(&mut self, policy: RewardPolicy) {
        self.policy = policy;
    }

    /// Current compensation balance of `account`, or `None` if no record.
    pub fn account(&self, account: &str) -> Option<CompensationAccount> {
        self.accounts.get(account).copied()
    }

    /// Snapshot of every account balance (read-only, deterministic order).
    pub fn accounts(&self) -> std::collections::BTreeMap<String, CompensationAccount> {
        self.accounts.iter().map(|(k, v)| (k.clone(), *v)).collect()
    }

    /// The audit trail so far (provenance). Read-only.
    pub fn events(&self) -> &std::collections::VecDeque<CompensationEvent> {
        &self.events
    }

    /// Credits reputation-adjusted compensation for one verified execution,
    /// **exactly once** per `ref_id`.
    ///
    /// `profile` is the worker's contribution profile **at credit time** — the
    /// same verified/failed counters `reward_tokens` consumes, so the reward
    /// honestly reflects the reputation the worker had when it served the
    /// request. Returns the credits credited (0 when the worker had no verified
    /// work yet — you earn by *serving*, exactly as the module docs promise).
    pub fn credit(&mut self, account: &str, ref_id: &str, profile: &ContributionProfile) -> u64 {
        if !self
            .applied
            .insert(("credit".to_string(), ref_id.to_string()))
        {
            return 0; // duplicate: already credited this ref_id exactly once.
        }
        let amount = reward_tokens(profile, &self.policy);
        if amount == 0 {
            return 0;
        }
        let acc = self.accounts.entry(account.to_string()).or_default();
        acc.earned = acc.earned.saturating_add(amount);
        if self.events.len() >= MAX_COMPENSATION_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(CompensationEvent {
            op: "credit".to_string(),
            account: account.to_string(),
            amount,
            ref_id: ref_id.to_string(),
            policy: self.policy,
            verified_requests: profile.verified_requests,
            failed_requests: profile.failed_requests,
        });
        amount
    }
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
        assert!(
            (p.reputation_multiplier(100, 0) - 1.0).abs() < 1e-9,
            "flawless -> 1.0"
        );
        assert!(
            (p.reputation_multiplier(0, 100) - 0.0).abs() < 1e-9,
            "all fail -> 0.0"
        );
        assert!(
            (p.reputation_multiplier(0, 0) - 1.0).abs() < 1e-9,
            "idle -> 1.0"
        );
    }

    // ---- CompensationLedger ----

    #[test]
    fn ledger_credits_verified_work_and_is_idempotent() {
        let mut ledger = CompensationLedger::new(RewardPolicy::default());
        let first = ledger.credit("peer-a", "req-1", &healthy());
        assert!(first > 0, "verified work earns credits");
        assert_eq!(ledger.account("peer-a").unwrap().earned, first);

        // Re-crediting the same ref_id must be a no-op (idempotency).
        let again = ledger.credit("peer-a", "req-1", &healthy());
        assert_eq!(again, 0, "same ref_id is never double-credited");
        assert_eq!(ledger.account("peer-a").unwrap().earned, first);
    }

    #[test]
    fn ledger_failed_work_earns_nothing() {
        let mut ledger = CompensationLedger::new(RewardPolicy::default());
        let failing = ContributionProfile {
            verified_requests: 0,
            failed_requests: 50,
            ..healthy()
        };
        let amount = ledger.credit("peer-b", "req-fail", &failing);
        assert_eq!(amount, 0, "a worker that only failed earns nothing");
        assert!(
            ledger.account("peer-b").is_none(),
            "no record for zero earnings"
        );
    }

    #[test]
    fn ledger_accumulates_across_requests_and_orders_accounts() {
        let mut ledger = CompensationLedger::new(RewardPolicy::default());
        let mut prof = healthy();
        prof.verified_requests = 5;
        let a1 = ledger.credit("peer-c", "r1", &prof);
        prof.verified_requests = 10;
        let a2 = ledger.credit("peer-c", "r2", &prof);
        assert!(a2 > a1, "more verified work earns more on the next credit");
        let total = ledger.account("peer-c").unwrap().earned;
        assert_eq!(total, a1 + a2, "credits accumulate monotonically");

        let accounts = ledger.accounts();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts.get("peer-c").unwrap().earned, total);
        assert_eq!(ledger.events().len(), 2, "one audited event per credit");
        let ev = ledger.events().back().unwrap();
        assert_eq!(ev.op, "credit");
        assert_eq!(ev.ref_id, "r2");
        assert_eq!(ev.verified_requests, 10);
        assert_eq!(ev.failed_requests, 0);
    }

    #[test]
    fn ledger_policy_swap_affects_only_future_credits() {
        let mut ledger = CompensationLedger::new(RewardPolicy::default());
        let p1 = ledger.credit("peer-d", "r1", &healthy());
        let richer = RewardPolicy {
            tokens_per_verified_request: 10,
            ..Default::default()
        };
        ledger.set_policy(richer);
        let p2 = ledger.credit("peer-d", "r2", &healthy());
        assert!(p2 > p1, "a richer policy yields more credits going forward");
        // Historical event keeps the old policy (explainability).
        let first_event = ledger.events().front().unwrap();
        assert_eq!(first_event.policy, RewardPolicy::default());
        assert_eq!(ledger.policy(), richer);
        assert_eq!(ledger.account("peer-d").unwrap().earned, p1 + p2);
    }
}
