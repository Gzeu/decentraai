//! M17 collective orchestration — pure core (no I/O, no time source).
//!
//! The fabric plans and delegates (existing `AgentOrchestrator`); this module
//! owns the three decisions the orchestrator does NOT make:
//! 1. **provider selection** across LOCAL + P2P-remote + EXTERNAL (`dga_`)
//!    providers with deterministic fairness (hard gates, then score, then
//!    `agent_id` ascending — the same discipline as DFCP);
//! 2. **settlement decision** on stage completion (settle + credit on
//!    verified output, release on failure), idempotent by construction;
//! 3. **assignment lifecycle** (propose → assign → claim → settle/fail) for
//!    the pull model external executors use (they cannot receive `Delegate`
//!    messages, so they claim open assignments and submit outputs).
//!
//! All time is injected (`now_ms`) so every decision is unit-testable.
//! Ledger mutation happens OUTSIDE (runtime, via `QuotaLedger`), keyed by
//! the idempotency keys built here.

use std::collections::HashMap;

/// Mirrors the API workflow bound: a plan never exceeds 8 stages.
pub const MAX_ORCHESTRATION_STAGES: usize = 8;
/// Upper bound for a single stage price (quota credits).
pub const MAX_STAGE_PRICE: u64 = 10_000;
/// Provider advertisements older than this are excluded (stale).
pub const PROVIDER_STALE_MS: u64 = 60_000;
/// An `Assigned` stage unclaimed for longer than this expires.
pub const ASSIGNMENT_TTL_MS: u64 = 3_600_000;
/// Max length for plan/stage/agent/capability identifiers on this path.
pub const MAX_ID_LEN: usize = 128;

/// Reservation id for the requester's locked price. Format is part of the
/// idempotency contract: `m17res:{plan}:{stage}`.
pub fn reservation_id(plan_id: &str, stage_id: &str) -> String {
    format!("m17res:{plan_id}:{stage_id}")
}

/// Credit reference for the executor award. `m17cr:{plan}:{stage}`.
pub fn credit_ref(plan_id: &str, stage_id: &str) -> String {
    format!("m17cr:{plan_id}:{stage_id}")
}

fn bounded(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN
}

/// Ranking key for provider selection (smaller wins): price asc, then
/// contribution balance desc (clamped to ±15), then agent_id asc.
type RankKey<'a> = (u64, std::cmp::Reverse<i64>, &'a str);

/// Where a provider runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOrigin {
    /// A worker on this node.
    Local,
    /// A peer agent reachable over P2P.
    Remote,
    /// An external agent holding a `dga_` credential.
    External,
}

/// A provider advertisement: what an agent offers, at what price, how fresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAd {
    /// Stable agent id (tie-break key — must be unique per provider).
    pub agent_id: String,
    /// Where it runs.
    pub origin: ProviderOrigin,
    /// Capability labels (hub taxonomy `label()` strings) it serves.
    pub capabilities: Vec<String>,
    /// Asking price in quota credits per stage execution.
    pub price: u64,
    /// Contribution balance: biases ties by at most ±0.15 (fairness is a
    /// bias, never a dictator — same rule as DFCP offer selection).
    pub contribution_balance: i64,
    /// Wall-clock ms of the last heartbeat/advertisement.
    pub last_seen_ms: u64,
}

/// What one stage needs from a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRequirement {
    /// Capability label the stage needs.
    pub capability: String,
    /// Hard ceiling the requester will pay (quota credits).
    pub max_price: u64,
}

/// Why no provider was selected. Denials carry no oracle detail outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionError {
    /// No advertisement serves the capability.
    NoCapableProvider,
    /// Providers serve it but all are stale, overpriced, or malformed.
    NoEligibleProvider,
}

/// Deterministic provider selection: hard gates first (capability fit,
/// freshness ≤ 60s, price ≤ requester bound, well-formed ids), then score
/// (`-price` primary, contribution bias clamped to ±0.15), ties broken by
/// `agent_id` ascending. No randomness anywhere.
pub fn select_provider<'a>(
    ads: &'a [ProviderAd],
    req: &StageRequirement,
    now_ms: u64,
) -> Result<&'a ProviderAd, SelectionError> {
    if req.capability.is_empty() || req.capability.len() > MAX_ID_LEN {
        return Err(SelectionError::NoCapableProvider);
    }
    let mut capable = false;
    let mut best: Option<(&'a ProviderAd, RankKey<'_>)> = None;
    for ad in ads {
        if !bounded(&ad.agent_id) {
            continue;
        }
        if !ad.capabilities.iter().any(|c| c == &req.capability) {
            continue;
        }
        capable = true;
        // Hard gates: freshness, price bound.
        if now_ms.saturating_sub(ad.last_seen_ms) > PROVIDER_STALE_MS {
            continue;
        }
        if ad.price > req.max_price || ad.price > MAX_STAGE_PRICE {
            continue;
        }
        // Score: lower price wins; on a price tie the higher contribution
        // balance wins (clamped to ±15 so fairness stays a bias); final tie
        // broken by agent_id ascending. Smaller tuple wins.
        let bias = ad.contribution_balance.clamp(-15, 15);
        let key = (ad.price, std::cmp::Reverse(bias), ad.agent_id.as_str());
        match &best {
            // Lower price first; on price tie higher bias; then agent_id asc.
            Some((_, best_key)) if *best_key <= key => {}
            _ => best = Some((ad, key)),
        }
    }
    match best {
        Some((ad, _)) => Ok(ad),
        None if capable => Err(SelectionError::NoEligibleProvider),
        None => Err(SelectionError::NoCapableProvider),
    }
}

/// Verified stage measurements feeding the executor credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedMeasures {
    pub tokens_used: Option<u32>,
    pub processing_ms: Option<u32>,
}

/// What the fabric does with a finished stage's locked price.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettleDecision {
    /// Requester pays (`settle` reservation), executor earns (`credit`).
    /// Both ledger calls are idempotent on their keys — applying the same
    /// decision twice moves value once.
    SettleAndCredit {
        reservation_id: String,
        credit_ref: String,
        price: u64,
        measures: VerifiedMeasures,
    },
    /// Requester is refunded (`release` reservation); executor gets nothing.
    Release { reservation_id: String },
}

/// Pure settlement decision. `verified=true` MUST only be passed for output
/// that passed the existing verification path — this function trusts its
/// caller on that point and only decides the money movement.
pub fn decide_settle(
    plan_id: &str,
    stage_id: &str,
    verified: bool,
    price: u64,
    measures: VerifiedMeasures,
) -> SettleDecision {
    if verified {
        SettleDecision::SettleAndCredit {
            reservation_id: reservation_id(plan_id, stage_id),
            credit_ref: credit_ref(plan_id, stage_id),
            price,
            measures,
        }
    } else {
        SettleDecision::Release {
            reservation_id: reservation_id(plan_id, stage_id),
        }
    }
}

/// Assignment lifecycle for the pull model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentState {
    /// Proposed by a requester, awaiting fabric assignment.
    Proposed,
    /// Assigned to an executor, awaiting claim.
    Assigned { executor: String },
    /// Claimed by the executor, work in progress.
    Claimed {
        executor: String,
        claimed_at_ms: u64,
    },
    /// Verified output submitted, settlement applied.
    Settled { executor: String },
    /// Failed (no provider, verification failed, expired…).
    Failed { reason: String },
}

/// One stage assignment: the unit external executors pull.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub plan_id: String,
    pub stage_id: String,
    pub capability: String,
    pub price: u64,
    pub requester: String,
    pub state: AssignmentState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Assignment store errors (no oracle detail leaves the module).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssignmentError {
    Unknown,
    BadTransition,
    StageLimit,
    PriceBound,
    Malformed,
}

type Key = (String, String);

/// In-memory assignment store. Bounded by plan size (≤8 stages); the runtime
/// wraps it in a mutex and sweeps `expire` on a slow tick.
#[derive(Debug, Default)]
pub struct AssignmentStore {
    plans: HashMap<String, Vec<Key>>,
    items: HashMap<Key, Assignment>,
}

impl AssignmentStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn check_ids(plan_id: &str, stage_id: &str) -> Result<(), AssignmentError> {
        if !bounded(plan_id) || !bounded(stage_id) {
            return Err(AssignmentError::Malformed);
        }
        Ok(())
    }

    /// Requester proposes one stage. Fails closed on stage limit / price.
    #[allow(clippy::too_many_arguments)]
    pub fn propose(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        capability: &str,
        price: u64,
        requester: &str,
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        Self::check_ids(plan_id, stage_id)?;
        if !bounded(capability) || !bounded(requester) {
            return Err(AssignmentError::Malformed);
        }
        if price > MAX_STAGE_PRICE {
            return Err(AssignmentError::PriceBound);
        }
        let key = (plan_id.to_string(), stage_id.to_string());
        if self.items.contains_key(&key) {
            return Err(AssignmentError::BadTransition);
        }
        let entry = self.plans.entry(plan_id.to_string()).or_default();
        if entry.len() >= MAX_ORCHESTRATION_STAGES {
            return Err(AssignmentError::StageLimit);
        }
        entry.push(key.clone());
        self.items.insert(
            key,
            Assignment {
                plan_id: plan_id.to_string(),
                stage_id: stage_id.to_string(),
                capability: capability.to_string(),
                price,
                requester: requester.to_string(),
                state: AssignmentState::Proposed,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            },
        );
        Ok(())
    }

    /// Fabric assigns a proposed stage to a selected executor.
    pub fn assign(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        executor: &str,
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        Self::check_ids(plan_id, stage_id)?;
        if !bounded(executor) {
            return Err(AssignmentError::Malformed);
        }
        let a = self
            .items
            .get_mut(&(plan_id.to_string(), stage_id.to_string()))
            .ok_or(AssignmentError::Unknown)?;
        match a.state {
            AssignmentState::Proposed => {
                a.state = AssignmentState::Assigned {
                    executor: executor.to_string(),
                };
                a.updated_at_ms = now_ms;
                Ok(())
            }
            _ => Err(AssignmentError::BadTransition),
        }
    }

    /// The assigned executor claims the stage (pull model).
    pub fn claim(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        executor: &str,
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        let a = self
            .items
            .get_mut(&(plan_id.to_string(), stage_id.to_string()))
            .ok_or(AssignmentError::Unknown)?;
        match &a.state {
            AssignmentState::Assigned { executor: e } if e == executor => {
                a.state = AssignmentState::Claimed {
                    executor: executor.to_string(),
                    claimed_at_ms: now_ms,
                };
                a.updated_at_ms = now_ms;
                Ok(())
            }
            _ => Err(AssignmentError::BadTransition),
        }
    }

    /// Verified output submitted: terminal Settled (idempotency: only from
    /// Claimed; re-submit is a BadTransition, and the ledger credit key
    /// makes double-application a no-op downstream).
    pub fn submit_verified(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        executor: &str,
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        let a = self
            .items
            .get_mut(&(plan_id.to_string(), stage_id.to_string()))
            .ok_or(AssignmentError::Unknown)?;
        match &a.state {
            AssignmentState::Claimed { executor: e, .. } if e == executor => {
                a.state = AssignmentState::Settled {
                    executor: executor.to_string(),
                };
                a.updated_at_ms = now_ms;
                Ok(())
            }
            _ => Err(AssignmentError::BadTransition),
        }
    }

    /// Terminal failure from any non-terminal state.
    pub fn mark_failed(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        reason: &str,
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        let a = self
            .items
            .get_mut(&(plan_id.to_string(), stage_id.to_string()))
            .ok_or(AssignmentError::Unknown)?;
        match a.state {
            AssignmentState::Settled { .. } | AssignmentState::Failed { .. } => {
                Err(AssignmentError::BadTransition)
            }
            _ => {
                a.state = AssignmentState::Failed {
                    reason: reason.chars().take(64).collect(),
                };
                a.updated_at_ms = now_ms;
                Ok(())
            }
        }
    }

    /// Sweeps `Assigned` stages older than the claim TTL → `Failed{expired}`.
    /// Returns the expired keys. Called on a slow tick, never hot path.
    pub fn expire(&mut self, now_ms: u64) -> Vec<Key> {
        let mut out = Vec::new();
        for (k, a) in self.items.iter_mut() {
            if matches!(a.state, AssignmentState::Assigned { .. })
                && now_ms.saturating_sub(a.updated_at_ms) > ASSIGNMENT_TTL_MS
            {
                a.state = AssignmentState::Failed {
                    reason: "claim window expired".to_string(),
                };
                a.updated_at_ms = now_ms;
                out.push(k.clone());
            }
        }
        out
    }

    pub fn get(&self, plan_id: &str, stage_id: &str) -> Option<&Assignment> {
        self.items.get(&(plan_id.to_string(), stage_id.to_string()))
    }

    /// All stages of a plan, in proposal order (read-only status surface).
    pub fn plan(&self, plan_id: &str) -> Vec<&Assignment> {
        self.plans
            .get(plan_id)
            .map(|keys| keys.iter().filter_map(|k| self.items.get(k)).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ad(id: &str, caps: &[&str], price: u64, bal: i64, seen: u64) -> ProviderAd {
        ProviderAd {
            agent_id: id.to_string(),
            origin: ProviderOrigin::External,
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            price,
            contribution_balance: bal,
            last_seen_ms: seen,
        }
    }

    fn req(cap: &str, max: u64) -> StageRequirement {
        StageRequirement {
            capability: cap.to_string(),
            max_price: max,
        }
    }

    #[test]
    fn select_prefers_lower_price() {
        let ads = vec![
            ad("b", &["chat"], 50, 0, 1000),
            ad("a", &["chat"], 20, 0, 1000),
        ];
        assert_eq!(
            select_provider(&ads, &req("chat", 100), 1100)
                .unwrap()
                .agent_id,
            "a"
        );
    }

    #[test]
    fn select_tie_breaks_by_agent_id_asc() {
        let ads = vec![
            ad("b", &["chat"], 20, 0, 1000),
            ad("a", &["chat"], 20, 0, 1000),
        ];
        assert_eq!(
            select_provider(&ads, &req("chat", 100), 1100)
                .unwrap()
                .agent_id,
            "a"
        );
    }

    #[test]
    fn select_contribution_bias_breaks_price_tie() {
        let ads = vec![
            ad("b", &["chat"], 20, 0, 1000),
            ad("a", &["chat"], 20, 10, 1000),
        ];
        assert_eq!(
            select_provider(&ads, &req("chat", 100), 1100)
                .unwrap()
                .agent_id,
            "a"
        );
    }

    #[test]
    fn select_contribution_cannot_beat_cheaper_price() {
        // Fairness is a bias, never a dictator: +15 bias < any price gap ≥ 1.
        let ads = vec![
            ad("rich", &["chat"], 21, 1000, 1000),
            ad("cheap", &["chat"], 20, -1000, 1000),
        ];
        assert_eq!(
            select_provider(&ads, &req("chat", 100), 1100)
                .unwrap()
                .agent_id,
            "cheap"
        );
    }

    #[test]
    fn select_rejects_stale_and_overpriced() {
        let ads = vec![
            ad("stale", &["chat"], 10, 0, 0),
            ad("pricey", &["chat"], 500, 0, 1000),
        ];
        assert_eq!(
            select_provider(&ads, &req("chat", 100), 100_000),
            Err(SelectionError::NoEligibleProvider)
        );
    }

    #[test]
    fn select_unknown_capability_is_no_capable() {
        let ads = vec![ad("a", &["chat"], 10, 0, 1000)];
        assert_eq!(
            select_provider(&ads, &req("nope", 100), 1100),
            Err(SelectionError::NoCapableProvider)
        );
    }

    #[test]
    fn settle_keys_are_stable_and_namespaced() {
        assert_eq!(reservation_id("p", "s"), "m17res:p:s");
        assert_eq!(credit_ref("p", "s"), "m17cr:p:s");
        let m = VerifiedMeasures {
            tokens_used: Some(10),
            processing_ms: Some(5),
        };
        match decide_settle("p", "s", true, 7, m) {
            SettleDecision::SettleAndCredit {
                reservation_id: r,
                credit_ref: c,
                price: 7,
                ..
            } => {
                assert_eq!(r, "m17res:p:s");
                assert_eq!(c, "m17cr:p:s");
            }
            _ => panic!("expected settle"),
        }
        match decide_settle("p", "s", false, 7, m) {
            SettleDecision::Release { reservation_id: r } => assert_eq!(r, "m17res:p:s"),
            _ => panic!("expected release"),
        }
    }

    #[test]
    fn assignment_full_lifecycle() {
        let mut s = AssignmentStore::new();
        s.propose("p", "s1", "chat", 10, "req-a", 1000).unwrap();
        s.assign("p", "s1", "exec-x", 1001).unwrap();
        s.claim("p", "s1", "exec-x", 1002).unwrap();
        s.submit_verified("p", "s1", "exec-x", 1003).unwrap();
        assert!(matches!(
            s.get("p", "s1").unwrap().state,
            AssignmentState::Settled { .. }
        ));
        // Re-submit is rejected (ledger idempotency key covers the money).
        assert_eq!(
            s.submit_verified("p", "s1", "exec-x", 1004),
            Err(AssignmentError::BadTransition)
        );
    }

    #[test]
    fn assignment_wrong_executor_cannot_claim() {
        let mut s = AssignmentStore::new();
        s.propose("p", "s1", "chat", 10, "req-a", 1000).unwrap();
        s.assign("p", "s1", "exec-x", 1001).unwrap();
        assert_eq!(
            s.claim("p", "s1", "intruder", 1002),
            Err(AssignmentError::BadTransition)
        );
    }

    #[test]
    fn assignment_enforces_stage_limit_and_price_bound() {
        let mut s = AssignmentStore::new();
        for i in 0..MAX_ORCHESTRATION_STAGES {
            s.propose("p", &format!("s{i}"), "chat", 10, "req", 1000)
                .unwrap();
        }
        assert_eq!(
            s.propose("p", "sX", "chat", 10, "req", 1000),
            Err(AssignmentError::StageLimit)
        );
        assert_eq!(
            s.propose("q", "s1", "chat", MAX_STAGE_PRICE + 1, "req", 1000),
            Err(AssignmentError::PriceBound)
        );
    }

    #[test]
    fn assignment_expiry_sweeps_stale_assigned() {
        let mut s = AssignmentStore::new();
        s.propose("p", "s1", "chat", 10, "req", 1000).unwrap();
        s.assign("p", "s1", "exec-x", 1000).unwrap();
        let expired = s.expire(1000 + ASSIGNMENT_TTL_MS + 1);
        assert_eq!(expired.len(), 1);
        assert!(matches!(
            s.get("p", "s1").unwrap().state,
            AssignmentState::Failed { .. }
        ));
    }

    #[test]
    fn failed_is_terminal_but_claimed_can_fail() {
        let mut s = AssignmentStore::new();
        s.propose("p", "s1", "chat", 10, "req", 1000).unwrap();
        s.assign("p", "s1", "exec-x", 1001).unwrap();
        s.claim("p", "s1", "exec-x", 1002).unwrap();
        s.mark_failed("p", "s1", "bad output", 1003).unwrap();
        assert_eq!(
            s.mark_failed("p", "s1", "again", 1004),
            Err(AssignmentError::BadTransition)
        );
    }
}
