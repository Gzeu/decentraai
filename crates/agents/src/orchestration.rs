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
    pub replicas: u32,
    pub requester: String,
    pub state: AssignmentState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Per-replica state within a multi-replica stage assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaAssignment {
    pub plan_id: String,
    pub stage_id: String,
    pub replica: u32,
    pub executor: String,
    pub state: AssignmentState,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Collected output from a replica for consensus voting.
#[derive(Debug, Clone)]
pub struct ReplicaResult {
    pub plan_id: String,
    pub stage_id: String,
    pub replica: u32,
    pub executor: String,
    pub output: serde_json::Value,
    pub confidence: f32,
    pub submitted_at_ms: u64,
}

/// Consensus outcome for a multi-replica stage.
#[derive(Debug, Clone)]
pub struct ConsensusOutcome {
    pub plan_id: String,
    pub stage_id: String,
    pub verdict: crate::verification::VerificationVerdict,
    pub outputs: Vec<ReplicaResult>,
    pub resolved_at_ms: u64,
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
    /// Per-replica state machine: `{(plan_id, stage_id): Vec<ReplicaAssignment>}`.
    replica_items: HashMap<(String, String), Vec<ReplicaAssignment>>,
    /// Submitted replica outputs keyed by `{(plan_id, stage_id)}`.
    replica_results: HashMap<(String, String), Vec<ReplicaResult>>,
    /// Resolved consensus outcomes keyed by `{(plan_id, stage_id)}`.
    consensus_outcomes: HashMap<(String, String), ConsensusOutcome>,
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
        self.propose_with_replicas(plan_id, stage_id, capability, price, requester, 1, now_ms)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn propose_with_replicas(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        capability: &str,
        price: u64,
        requester: &str,
        replicas: u32,
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        Self::check_ids(plan_id, stage_id)?;
        if !bounded(capability) || !bounded(requester) {
            return Err(AssignmentError::Malformed);
        }
        if replicas == 0 || replicas > MAX_REPLICAS {
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
                replicas,
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

    // -------------------------------------------------------------------------
    // N-of-M replica management
    // -------------------------------------------------------------------------

    /// Initialize per-replica assignments for a multi-replica stage.
    /// Called once during propose when `replicas > 1`.
    pub fn init_replicas(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        replica_count: u32,
        executors: &[String],
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        if replica_count == 0 || replica_count as usize > executors.len() {
            return Err(AssignmentError::Malformed);
        }
        let key = (plan_id.to_string(), stage_id.to_string());
        if self.replica_items.contains_key(&key) {
            return Err(AssignmentError::BadTransition); // already initialized
        }
        let replicas: Vec<ReplicaAssignment> = (0..replica_count)
            .map(|r| ReplicaAssignment {
                plan_id: plan_id.to_string(),
                stage_id: stage_id.to_string(),
                replica: r,
                executor: executors[r as usize].clone(),
                state: AssignmentState::Assigned {
                    executor: executors[r as usize].clone(),
                },
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            })
            .collect();
        self.replica_items.insert(key, replicas);
        Ok(())
    }

    /// Transition one replica from Assigned → Claimed.
    pub fn claim_replica(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        replica: u32,
        executor: &str,
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        let key = (plan_id.to_string(), stage_id.to_string());
        let replicas = self
            .replica_items
            .get_mut(&key)
            .ok_or(AssignmentError::Unknown)?;
        let r = replicas
            .get_mut(replica as usize)
            .ok_or(AssignmentError::Unknown)?;
        match &r.state {
            AssignmentState::Assigned { executor: e } if e == executor => {
                r.state = AssignmentState::Claimed {
                    executor: executor.to_string(),
                    claimed_at_ms: now_ms,
                };
                r.updated_at_ms = now_ms;
                Ok(())
            }
            _ => Err(AssignmentError::BadTransition),
        }
    }

    /// Submit a replica's output → Claimed → Settled.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_replica(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        replica: u32,
        executor: &str,
        output: serde_json::Value,
        confidence: f32,
        now_ms: u64,
    ) -> Result<(), AssignmentError> {
        let key = (plan_id.to_string(), stage_id.to_string());
        let replicas = self
            .replica_items
            .get_mut(&key)
            .ok_or(AssignmentError::Unknown)?;
        let r = replicas
            .get_mut(replica as usize)
            .ok_or(AssignmentError::Unknown)?;
        match &r.state {
            AssignmentState::Claimed { executor: e, .. } if e == executor => {
                r.state = AssignmentState::Settled {
                    executor: executor.to_string(),
                };
                r.updated_at_ms = now_ms;

                let result = ReplicaResult {
                    plan_id: plan_id.to_string(),
                    stage_id: stage_id.to_string(),
                    replica,
                    executor: executor.to_string(),
                    output,
                    confidence,
                    submitted_at_ms: now_ms,
                };
                self.replica_results
                    .entry(key)
                    .or_default()
                    .push(result);
                Ok(())
            }
            _ => Err(AssignmentError::BadTransition),
        }
    }

    /// Try to resolve consensus for a multi-replica stage once all replicas
    /// have submitted (or enough for quorum). Returns the outcome if resolved.
    pub fn try_consensus(
        &mut self,
        plan_id: &str,
        stage_id: &str,
        required_agents: u32,
        agreement_threshold: f32,
        now_ms: u64,
    ) -> Option<ConsensusOutcome> {
        let key = (plan_id.to_string(), stage_id.to_string());
        // Already resolved?
        if self.consensus_outcomes.contains_key(&key) {
            return self.consensus_outcomes.get(&key).cloned();
        }
        // Need enough results to form a quorum.
        let results = self.replica_results.get(&key)?;
        if (results.len() as u32) < required_agents {
            return None;
        }

        let outputs: Vec<ReplicaOutput> = results
            .iter()
            .map(|r| ReplicaOutput {
                agent_id: r.executor.clone(),
                value: r.output.clone(),
                confidence: r.confidence,
            })
            .collect();
        let verdict = consensus_verdict(&outputs, required_agents, agreement_threshold);
        let outcome = ConsensusOutcome {
            plan_id: plan_id.to_string(),
            stage_id: stage_id.to_string(),
            verdict,
            outputs: results.clone(),
            resolved_at_ms: now_ms,
        };
        self.consensus_outcomes.insert(key.clone(), outcome.clone());
        self.consensus_outcomes.get(&key).cloned()
    }

    /// Read-only accessors for status reporting.
    pub fn replicas(&self, plan_id: &str, stage_id: &str) -> Vec<&ReplicaAssignment> {
        self.replica_items
            .get(&(plan_id.to_string(), stage_id.to_string()))
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    pub fn consensus(&self, plan_id: &str, stage_id: &str) -> Option<&ConsensusOutcome> {
        self.consensus_outcomes
            .get(&(plan_id.to_string(), stage_id.to_string()))
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

// ---------------------------------------------------------------------------
// N-of-M consensus (v0.1) — replicated stage execution, ONE consensus
// language (`crate::verification::evaluate_consensus`), no second engine.
// ---------------------------------------------------------------------------

/// Replicas bound: a requester may ask for 1..=3 providers per stage.
pub const MAX_REPLICAS: u32 = 3;

/// Idempotency keys per replica (`{replica}` is the 0-based ordinal).
pub fn reservation_id_replica(plan_id: &str, stage_id: &str, replica: u32) -> String {
    format!("m17res:{plan_id}:{stage_id}:r{replica}")
}

/// Per-replica credit key (`m17cr:{plan}:{stage}:rN`).
pub fn credit_ref_replica(plan_id: &str, stage_id: &str, replica: u32) -> String {
    format!("m17cr:{plan_id}:{stage_id}:r{replica}")
}

/// Canonical serialization used to cluster replica outputs: plain JSON
/// `to_string` of the value with object keys in BTreeMap order — serde_json
/// preserves map order for `Map` (BTreeMap when `preserve_order` is off);
/// to stay deterministic regardless of feature flags we canonicalize
/// recursively here (never from the raw output bytes).
pub fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .iter()
                .map(|k| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap(),
                        canonical_json(&map[k.as_str()])
                    )
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        serde_json::Value::Array(arr) => {
            let inner: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// One replica's submitted output for the consensus vote.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplicaOutput {
    pub agent_id: String,
    /// The output value (compared by canonical form, never fuzzy).
    pub value: serde_json::Value,
    /// The verifier's per-replica confidence `0.0..=1.0` (from the existing
    /// verification path; a failed `verify_value` yields 0.0).
    pub confidence: f32,
}

/// Errors for replica fan-out planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicaPlanError {
    /// `replicas` outside 1..=MAX_REPLICAS.
    ReplicaBound,
    /// replicas × max_price exceeds the requester budget cap.
    BudgetExceeded,
    /// Fewer distinct providers available than replicas requested.
    InsufficientDistinctProviders,
}

/// Select `n` DISTINCT providers for one stage: run the single-provider
/// selection, then remove the winner and repeat. Deterministic.
pub fn select_providers<'a>(
    ads: &'a [ProviderAd],
    req: &StageRequirement,
    replicas: u32,
    budget_cap: u64,
    now_ms: u64,
) -> Result<Vec<&'a ProviderAd>, ReplicaPlanError> {
    if replicas == 0 || replicas > MAX_REPLICAS {
        return Err(ReplicaPlanError::ReplicaBound);
    }
    // Bound spend BEFORE selection: replicas × stage price cap ≤ budget.
    if req.max_price.saturating_mul(replicas as u64) > budget_cap {
        return Err(ReplicaPlanError::BudgetExceeded);
    }
    let mut excluded: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut out: Vec<&'a ProviderAd> = Vec::new();
    while (out.len() as u32) < replicas {
        let owned: Vec<ProviderAd> = ads
            .iter()
            .filter(|a| !excluded.contains(a.agent_id.as_str()))
            .cloned()
            .collect();
        let pick = select_provider(&owned, req, now_ms)
            .map_err(|_| ReplicaPlanError::InsufficientDistinctProviders)?;
        let pid = pick.agent_id.clone();
        // Safe: `pick` is a clone of an element of `ads`; re-borrow it.
        let orig = ads
            .iter()
            .find(|a| a.agent_id == pid)
            .expect("clone source");
        out.push(orig);
        excluded.insert(orig.agent_id.as_str());
    }
    Ok(out)
}

/// Consensus verdict over replica outputs using the ONE consensus language.
///
/// How a vote is formed: outputs are clustered by canonical bytes; the
/// largest cluster (TIE-BREAK: lexicographically smallest canonical string,
/// deterministic) is the candidate answer; members of that cluster vote
/// `agrees=true` with their own confidence, all others vote against.
/// The verdict comes from `evaluate_consensus` — never fabricated here.
pub fn consensus_verdict(
    outputs: &[ReplicaOutput],
    required_agents: u32,
    agreement_threshold: f32,
) -> crate::verification::VerificationVerdict {
    use crate::verification::{ConsensusPolicy, ConsensusResult, evaluate_consensus};

    // Deterministic ordering before clustering (agent_id asc).
    let mut ordered: Vec<&ReplicaOutput> = outputs.iter().collect();
    ordered.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

    // Cluster by canonical form.
    let mut clusters: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, o) in ordered.iter().enumerate() {
        let canon = canonical_json(&o.value);
        match clusters.iter_mut().find(|(c, _)| *c == canon) {
            Some((_, members)) => members.push(i),
            None => clusters.push((canon, vec![i])),
        }
    }
    // Winner: largest cluster; tie → lexicographically smallest canonical.
    let best = clusters
        .iter()
        .max_by(|a, b| {
            a.1.len().cmp(&b.1.len()).then_with(|| b.0.cmp(&a.0)) // smaller canonical wins ties
        })
        .map(|(c, _)| c.clone());

    let Some(winner) = best else {
        return crate::verification::VerificationVerdict::Uncertain {
            reason: "no replica outputs to vote on".to_string(),
        };
    };

    let votes: Vec<ConsensusResult> = ordered
        .iter()
        .map(|o| ConsensusResult {
            agent_id: o.agent_id.clone(),
            agrees: canonical_json(&o.value) == winner,
            confidence: o.confidence.clamp(0.0, 1.0),
        })
        .collect();

    evaluate_consensus(
        &votes,
        &ConsensusPolicy {
            required_agents,
            agreement_threshold,
            require_schema: false,
        },
    )
}

/// Per-replica settlement decision: verified replicas settle+credit with
/// replica-namespaced keys; unverified replicas release. Idempotent via the
/// ledger on each key — retrying this decision for the same replica can
/// never double-pay.
pub fn decide_settle_replica(
    plan_id: &str,
    stage_id: &str,
    replica: u32,
    verified: bool,
    price: u64,
    measures: VerifiedMeasures,
) -> SettleDecision {
    if verified {
        SettleDecision::SettleAndCredit {
            reservation_id: reservation_id_replica(plan_id, stage_id, replica),
            credit_ref: credit_ref_replica(plan_id, stage_id, replica),
            price,
            measures,
        }
    } else {
        SettleDecision::Release {
            reservation_id: reservation_id_replica(plan_id, stage_id, replica),
        }
    }
}

#[cfg(test)]
mod nofm_tests {
    use super::*;
    use crate::verification::VerificationVerdict;

    fn out(agent: &str, v: serde_json::Value, conf: f32) -> ReplicaOutput {
        ReplicaOutput {
            agent_id: agent.to_string(),
            value: v,
            confidence: conf,
        }
    }

    #[test]
    fn canonical_json_orders_object_keys() {
        let a = serde_json::json!({"b":1,"a":2});
        let b = serde_json::json!({"a":2,"b":1});
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_ne!(
            canonical_json(&a),
            canonical_json(&serde_json::json!({"a":2,"b":3}))
        );
    }

    #[test]
    fn consensus_two_of_three_identical_outputs_verified() {
        let v = serde_json::json!({"answer": "42"});
        let outputs = vec![
            out("a", v.clone(), 0.9),
            out("b", serde_json::json!({"answer":"42"}), 0.8), // key order differs
            out("c", serde_json::json!({"answer": "43"}), 0.1),
        ];
        assert_eq!(
            consensus_verdict(&outputs, 3, 0.5),
            VerificationVerdict::Verified
        );
    }

    #[test]
    fn consensus_majority_disagrees_rejected() {
        let outputs = vec![
            out("a", serde_json::json!(1), 0.9),
            out("b", serde_json::json!(2), 0.9),
            out("c", serde_json::json!(3), 0.9),
        ];
        assert!(matches!(
            consensus_verdict(&outputs, 3, 0.8),
            VerificationVerdict::Rejected { .. }
        ));
    }

    #[test]
    fn consensus_insufficient_outputs_is_uncertain() {
        let outputs = vec![out("a", serde_json::json!(1), 0.9)];
        assert!(matches!(
            consensus_verdict(&outputs, 2, 0.5),
            VerificationVerdict::Uncertain { .. }
        ));
    }

    #[test]
    fn consensus_tie_break_deterministic() {
        // 1-1 tie: lexicographically smallest canonical wins the candidate
        // slot; threshold 0.5 → Verified for the winner regardless of input order.
        let mk = |first: &str| {
            vec![
                out(first, serde_json::json!({"k":"x"}), 0.5),
                out(
                    if first == "a" { "c" } else { "a" },
                    serde_json::json!({"k":"y"}),
                    0.5,
                ),
            ]
        };
        assert_eq!(
            consensus_verdict(&mk("a"), 2, 0.5),
            consensus_verdict(&mk("c"), 2, 0.5)
        );
    }

    #[test]
    fn replica_config_enforces_bounds_and_budget() {
        let ads = vec![
            ad_for("p1", 5.0, 1000),
            ad_for("p2", 5.0, 1000),
            ad_for("p3", 5.0, 1000),
        ];
        let req = StageRequirement {
            capability: "chat".into(),
            max_price: 10,
        };
        // 4 replicas > bound
        assert_eq!(
            select_providers(&ads, &req, 4, 100, 1000),
            Err(ReplicaPlanError::ReplicaBound)
        );
        // 3 replicas × 10 > budget 25
        assert_eq!(
            select_providers(&ads, &req, 3, 25, 1000),
            Err(ReplicaPlanError::BudgetExceeded)
        );
        // 3 distinct available → ok, and all distinct
        let got = select_providers(&ads, &req, 3, 100, 1000).unwrap();
        let ids: std::collections::HashSet<_> = got.iter().map(|p| p.agent_id.clone()).collect();
        assert_eq!(ids.len(), 3);
        // 3 replicas but only 2 distinct providers
        let two = vec![ad_for("p1", 5.0, 1000), ad_for("p2", 5.0, 1000)];
        assert_eq!(
            select_providers(&two, &req, 3, 100, 1000),
            Err(ReplicaPlanError::InsufficientDistinctProviders)
        );
    }

    #[test]
    fn per_replica_settle_keys_are_namespaced_and_stable() {
        assert_eq!(reservation_id_replica("p", "s", 0), "m17res:p:s:r0");
        assert_eq!(credit_ref_replica("p", "s", 2), "m17cr:p:s:r2");
        let m = VerifiedMeasures {
            tokens_used: Some(1),
            processing_ms: Some(1),
        };
        match decide_settle_replica("p", "s", 1, true, 5, m) {
            SettleDecision::SettleAndCredit {
                reservation_id: r,
                credit_ref: c,
                ..
            } => {
                assert_eq!(r, "m17res:p:s:r1");
                assert_eq!(c, "m17cr:p:s:r1");
            }
            _ => panic!("expected settle"),
        }
        assert_eq!(
            decide_settle_replica("p", "s", 1, false, 5, m),
            SettleDecision::Release {
                reservation_id: "m17res:p:s:r1".into()
            }
        );
    }

    fn ad_for(id: &str, _p: f32, seen: u64) -> ProviderAd {
        ProviderAd {
            agent_id: id.to_string(),
            origin: ProviderOrigin::External,
            capabilities: vec!["chat".to_string()],
            price: 10,
            contribution_balance: 0,
            last_seen_ms: seen,
        }
    }

    #[test]
    fn replica_lifecycle_init_claim_submit_consensus() {
        let mut store = AssignmentStore::new();
        let now = 1000;

        // Propose a 2-replica stage.
        store
            .propose_with_replicas("p1", "s1", "chat", 10, "requester", 2, now)
            .unwrap();

        // Initialize replicas.
        let executors = vec!["prov-a".into(), "prov-b".into()];
        store.init_replicas("p1", "s1", 2, &executors, now).unwrap();

        // Verify replicas are in Assigned state.
        let reps = store.replicas("p1", "s1");
        assert_eq!(reps.len(), 2);
        assert!(matches!(reps[0].state, AssignmentState::Assigned { .. }));
        assert!(matches!(reps[1].state, AssignmentState::Assigned { .. }));

        // Claim replica 0.
        store.claim_replica("p1", "s1", 0, "prov-a", now + 100).unwrap();
        let reps = store.replicas("p1", "s1");
        assert!(matches!(reps[0].state, AssignmentState::Claimed { .. }));

        // Claim wrong executor fails.
        assert_eq!(
            store.claim_replica("p1", "s1", 0, "prov-wrong", now + 100),
            Err(AssignmentError::BadTransition)
        );

        // Submit replica 0 output.
        store
            .submit_replica("p1", "s1", 0, "prov-a", serde_json::json!("answer-42"), 0.9, now + 200)
            .unwrap();

        // Claim + submit replica 1 with same output.
        store.claim_replica("p1", "s1", 1, "prov-b", now + 150).unwrap();
        store
            .submit_replica("p1", "s1", 1, "prov-b", serde_json::json!("answer-42"), 0.8, now + 250)
            .unwrap();

        // Consensus should be resolved now (2 of 2 submitted).
        let outcome = store.try_consensus("p1", "s1", 2, 0.5, now + 300);
        assert!(outcome.is_some(), "consensus should resolve");
        let c = outcome.unwrap();
        assert!(matches!(
            c.verdict,
            crate::verification::VerificationVerdict::Verified
        ));
        assert_eq!(c.outputs.len(), 2);

        // Re-querying returns cached outcome.
        let c2 = store.try_consensus("p1", "s1", 2, 0.5, now + 400).unwrap();
        assert_eq!(c.resolved_at_ms, c2.resolved_at_ms);
    }

    #[test]
    fn replica_disagreement_rejected() {
        let mut store = AssignmentStore::new();
        let now = 1000;

        // 3 replicas, 2 different outputs, 1 agrees with candidate.
        store
            .propose_with_replicas("p1", "s1", "chat", 10, "requester", 3, now)
            .unwrap();
        let executors = vec!["prov-a".into(), "prov-b".into(), "prov-c".into()];
        store.init_replicas("p1", "s1", 3, &executors, now).unwrap();

        // prov-a and prov-b say "yes", prov-c says "no" — but use threshold 0.8
        // so 2/3 = 0.667 < 0.8 → rejected.
        store.claim_replica("p1", "s1", 0, "prov-a", now + 100).unwrap();
        store
            .submit_replica("p1", "s1", 0, "prov-a", serde_json::json!("yes"), 0.9, now + 200)
            .unwrap();
        store.claim_replica("p1", "s1", 1, "prov-b", now + 150).unwrap();
        store
            .submit_replica("p1", "s1", 1, "prov-b", serde_json::json!("yes"), 0.9, now + 250)
            .unwrap();
        store.claim_replica("p1", "s1", 2, "prov-c", now + 170).unwrap();
        store
            .submit_replica("p1", "s1", 2, "prov-c", serde_json::json!("no"), 0.9, now + 270)
            .unwrap();

        // With threshold 0.8, 2/3 = 0.667 is below threshold → Rejected.
        let outcome = store.try_consensus("p1", "s1", 3, 0.8, now + 300).unwrap();
        assert!(matches!(
            outcome.verdict,
            crate::verification::VerificationVerdict::Rejected { .. }
        ));
    }

    #[test]
    fn replica_uninitialized_errors() {
        let mut store = AssignmentStore::new();
        let now = 1000;
        store
            .propose_with_replicas("p1", "s1", "chat", 10, "requester", 2, now)
            .unwrap();
        // No init_replicas → claim should fail.
        assert_eq!(
            store.claim_replica("p1", "s1", 0, "prov-a", now),
            Err(AssignmentError::Unknown)
        );
    }

    #[test]
    fn init_replicas_rejects_duplicate() {
        let mut store = AssignmentStore::new();
        let now = 1000;
        store
            .propose_with_replicas("p1", "s1", "chat", 10, "requester", 2, now)
            .unwrap();
        let executors = vec!["prov-a".into(), "prov-b".into()];
        store.init_replicas("p1", "s1", 2, &executors, now).unwrap();
        // Double init → error.
        assert_eq!(
            store.init_replicas("p1", "s1", 2, &executors, now),
            Err(AssignmentError::BadTransition)
        );
    }
}
