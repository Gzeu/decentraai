//! Agent Hub — pure market for tasks, bids, proposals, teams, settlement.
//! Reuses QuotaLedger, Evidence, Reputation; no new identity/scheduler.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,
    Bidding,
    Assigned,
    InProgress,
    Completed,
    Settled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubTask {
    pub id: String,
    pub issuer: String,
    pub title: String,
    pub description: String,
    pub reward: u64,
    pub required_capability: Option<String>,
    pub status: TaskStatus,
    pub created_tick: u64,
    pub deadline_tick: Option<u64>,
    /// Tick at which the task settled (`None` = not settled). Set together
    /// with `evidence_id` — the pair makes the evidence preimage
    /// (`hub:<task>:<actor>:<tick>`) recomputable by anyone, which passport
    /// anchoring depends on. Old records without these fields still load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_tick: Option<u64>,
    /// BLAKE3 evidence id minted at settle time (`None` = not settled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<String>,
    /// Actor the evidence was computed for (request actor at execute time).
    /// Stored so the preimage is authoritative, never inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_by: Option<String>,
    /// Optional hash of the work deliverable (64 hex chars), bound at
    /// execute time. Binds the settlement to an artifact for passports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliverable_hash: Option<String>,
    /// Optional evolution anchor (artifact/parent/bench hashes) bound at
    /// execute time. Turns a client-side generation ledger into a public
    /// chain anchored in the fabric. Absent if no evolution field was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evolution: Option<EvolutionTag>,
}

/// Evolution anchor carried on a settled task's receipt. Each hash is a
/// 64-hex digest over the canonical preimage (§BACKEND_EVOLUTION.md §2/§11);
/// `*_alg` defaults to `blake3-256`. Only hash fields actually sent are
/// present, so receipts stay additive (no empty `{}` when nothing was sent).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionTag {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_alg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_alg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bench_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bench_alg: Option<String>,
}

impl EvolutionTag {
    /// Build a block from raw optional args, defaulting each `*_alg` to
    /// `blake3-256` when its hash is present. Returns `None` when no hash
    /// field was provided, so the receipt never carries an empty block.
    pub fn from_args(
        artifact_hash: Option<String>,
        artifact_alg: Option<String>,
        parent_hash: Option<String>,
        parent_alg: Option<String>,
        bench_hash: Option<String>,
        bench_alg: Option<String>,
    ) -> Option<EvolutionTag> {
        if artifact_hash.is_none() && parent_hash.is_none() && bench_hash.is_none() {
            return None;
        }
        Some(EvolutionTag {
            artifact_hash: artifact_hash.clone(),
            artifact_alg: artifact_hash
                .is_some()
                .then(|| artifact_alg.unwrap_or_else(|| "blake3-256".to_string())),
            parent_hash: parent_hash.clone(),
            parent_alg: parent_hash
                .is_some()
                .then(|| parent_alg.unwrap_or_else(|| "blake3-256".to_string())),
            bench_hash: bench_hash.clone(),
            bench_alg: bench_hash
                .is_some()
                .then(|| bench_alg.unwrap_or_else(|| "blake3-256".to_string())),
        })
    }

    /// True when any hash field is present (the block is non-empty).
    pub fn is_present(&self) -> bool {
        self.artifact_hash.is_some() || self.parent_hash.is_some() || self.bench_hash.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bid {
    pub id: String,
    pub task_id: String,
    pub bidder: String,
    pub price: u64,
    pub rationale: String,
    pub created_tick: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    Accepted,
    Rejected,
    Counter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub from: String,
    pub to: String,
    pub task_id: String,
    pub offer_price: u64,
    pub workshare: u8,
    pub status: ProposalStatus,
    pub created_tick: u64,
    pub expires_tick: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub task_id: String,
    pub members: Vec<(String, u8)>,
    pub created_tick: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubEvent {
    pub tick: u64,
    pub kind: String,
    pub detail: String,
    pub task_id: Option<String>,
    pub evidence_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubState {
    pub tick: u64,
    pub tasks: BTreeMap<String, HubTask>,
    pub bids: BTreeMap<String, Bid>,
    pub proposals: BTreeMap<String, Proposal>,
    pub teams: BTreeMap<String, Team>,
    pub events: VecDeque<HubEvent>,
    pub max_events: usize,
}

impl Default for HubState {
    fn default() -> Self {
        Self {
            tick: 0,
            tasks: BTreeMap::new(),
            bids: BTreeMap::new(),
            proposals: BTreeMap::new(),
            teams: BTreeMap::new(),
            events: VecDeque::new(),
            max_events: 1000,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HubError {
    #[error("task not found")]
    TaskNotFound,
    #[error("task not open")]
    TaskNotOpen,
    #[error("bid price exceeds reward")]
    PriceTooHigh,
    #[error("self_bid_forbidden: bidder and task issuer must differ")]
    SelfBid,
    #[error("proposal not found")]
    ProposalNotFound,
    #[error("not proposal recipient")]
    NotRecipient,
    #[error("team already exists")]
    TeamExists,
    #[error("insufficient members")]
    InsufficientMembers,
    #[error("already exists")]
    AlreadyExists,
}

impl HubState {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_id(&self, prefix: &str) -> String {
        let n = match prefix {
            "task" => self.tasks.len(),
            "bid" => self.bids.len(),
            "prop" => self.proposals.len(),
            "team" => self.teams.len(),
            _ => self.events.len(),
        };
        format!("{}-{:04}", prefix, n + 1)
    }

    fn push_event(
        &mut self,
        kind: &str,
        detail: String,
        task_id: Option<String>,
        evidence_id: Option<String>,
    ) {
        self.events.push_back(HubEvent {
            tick: self.tick,
            kind: kind.to_string(),
            detail,
            task_id,
            evidence_id,
        });
        while self.events.len() > self.max_events {
            self.events.pop_front();
        }
    }

    pub fn publish_task(
        &mut self,
        issuer: String,
        title: String,
        description: String,
        reward: u64,
        required_capability: Option<String>,
    ) -> HubTask {
        let id = self.next_id("task");
        let task = HubTask {
            id: id.clone(),
            issuer: issuer.clone(),
            title: title.clone(),
            description,
            reward,
            required_capability,
            status: TaskStatus::Open,
            created_tick: self.tick,
            deadline_tick: None,
            settled_tick: None,
            evidence_id: None,
            settled_by: None,
            deliverable_hash: None,
            evolution: None,
        };
        self.tasks.insert(id.clone(), task.clone());
        self.push_event(
            "task_published",
            format!("{} published '{}' reward {}", issuer, title, reward),
            Some(id),
            None,
        );
        task
    }

    /// Identity equality for self-dealing checks: trimmed, exact. Empty
    /// ids never match (a missing bidder/issuer fails validation
    /// elsewhere, never the sybil guard). Intentional mirror of the M18
    /// `same_party` rule — the crates share no dependency, so the 3-line
    /// rule lives in both rather than growing a new leaf crate.
    pub fn same_party(a: &str, b: &str) -> bool {
        let a = a.trim();
        let b = b.trim();
        !a.is_empty() && !b.is_empty() && a == b
    }

    /// Self-dealing award check: true when the task's best bid was placed
    /// by the task issuer (the issuer accepting their own bid). The no-bid
    /// issuer fallback is NOT covered — settling a task nobody bid on is
    /// not "accepting your own bid" (no bid exists). Execute paths refuse
    /// on true; nothing is mutated by the check itself.
    pub fn self_bid_award(&self, task_id: &str) -> bool {
        match (self.tasks.get(task_id), self.best_bid(task_id)) {
            (Some(task), Some(best)) => Self::same_party(&best.bidder, &task.issuer),
            _ => false,
        }
    }

    pub fn place_bid(
        &mut self,
        bidder: String,
        task_id: String,
        price: u64,
        rationale: String,
    ) -> Result<Bid, HubError> {
        let task = self.tasks.get(&task_id).ok_or(HubError::TaskNotFound)?;
        if task.status != TaskStatus::Open && task.status != TaskStatus::Bidding {
            return Err(HubError::TaskNotOpen);
        }
        if Self::same_party(&bidder, &task.issuer) {
            return Err(HubError::SelfBid);
        }
        if price > task.reward {
            return Err(HubError::PriceTooHigh);
        }
        let id = self.next_id("bid");
        let bid = Bid {
            id: id.clone(),
            task_id: task_id.clone(),
            bidder: bidder.clone(),
            price,
            rationale: rationale.clone(),
            created_tick: self.tick,
        };
        self.bids.insert(id.clone(), bid.clone());
        // mark task as bidding
        if let Some(t) = self.tasks.get_mut(&task_id) {
            t.status = TaskStatus::Bidding;
        }
        self.push_event(
            "bid_placed",
            format!("{} bid {} on {}", bidder, price, task_id),
            Some(task_id),
            None,
        );
        Ok(bid)
    }

    pub fn best_bid(&self, task_id: &str) -> Option<&Bid> {
        let mut best: Option<&Bid> = None;
        for bid in self.bids.values().filter(|b| b.task_id == task_id) {
            match best {
                None => best = Some(bid),
                Some(cur) => {
                    if bid.price < cur.price || (bid.price == cur.price && bid.bidder < cur.bidder)
                    {
                        best = Some(bid);
                    }
                }
            }
        }
        best
    }

    pub fn propose(
        &mut self,
        from: String,
        to: String,
        task_id: String,
        offer_price: u64,
        workshare: u8,
    ) -> Result<Proposal, HubError> {
        if !self.tasks.contains_key(&task_id) {
            return Err(HubError::TaskNotFound);
        }
        let id = self.next_id("prop");
        let prop = Proposal {
            id: id.clone(),
            from: from.clone(),
            to: to.clone(),
            task_id: task_id.clone(),
            offer_price,
            workshare,
            status: ProposalStatus::Pending,
            created_tick: self.tick,
            expires_tick: self.tick + 100,
        };
        self.proposals.insert(id.clone(), prop.clone());
        self.push_event(
            "proposal_sent",
            format!("{} -> {} for {} offer {}", from, to, task_id, offer_price),
            Some(task_id),
            None,
        );
        Ok(prop)
    }

    pub fn decide_proposal(
        &mut self,
        proposal_id: &str,
        actor: &str,
        accept: bool,
    ) -> Result<Proposal, HubError> {
        let task_id = {
            let prop = self
                .proposals
                .get_mut(proposal_id)
                .ok_or(HubError::ProposalNotFound)?;
            if prop.to != actor {
                return Err(HubError::NotRecipient);
            }
            prop.status = if accept {
                ProposalStatus::Accepted
            } else {
                ProposalStatus::Rejected
            };
            prop.task_id.clone()
        };
        let prop_clone = self.proposals.get(proposal_id).unwrap().clone();
        self.push_event(
            if accept {
                "proposal_accepted"
            } else {
                "proposal_rejected"
            },
            format!(
                "{} {} proposal {}",
                actor,
                if accept { "accepted" } else { "rejected" },
                proposal_id
            ),
            Some(task_id),
            None,
        );
        Ok(prop_clone)
    }

    pub fn form_team(
        &mut self,
        task_id: String,
        members: Vec<(String, u8)>,
    ) -> Result<Team, HubError> {
        if !self.tasks.contains_key(&task_id) {
            return Err(HubError::TaskNotFound);
        }
        if members.len() < 2 {
            return Err(HubError::InsufficientMembers);
        }
        let sum: u16 = members.iter().map(|(_, s)| *s as u16).sum();
        if sum != 100 {
            return Err(HubError::InsufficientMembers);
        }
        if self.teams.values().any(|t| t.task_id == task_id) {
            return Err(HubError::TeamExists);
        }
        let id = self.next_id("team");
        let team = Team {
            id: id.clone(),
            task_id: task_id.clone(),
            members: members.clone(),
            created_tick: self.tick,
        };
        self.teams.insert(id.clone(), team.clone());
        if let Some(task) = self.tasks.get_mut(&task_id) {
            task.status = TaskStatus::Assigned;
        }
        self.push_event(
            "team_formed",
            format!("team {} for {} with {} members", id, task_id, members.len()),
            Some(task_id),
            None,
        );
        Ok(team)
    }

    pub fn mark_executing(&mut self, task_id: &str) {
        if let Some(t) = self.tasks.get_mut(task_id) {
            t.status = TaskStatus::InProgress;
        }
        self.push_event(
            "execution_started",
            format!("execution started for {}", task_id),
            Some(task_id.to_string()),
            None,
        );
    }

    pub fn settle(&mut self, task_id: &str, evidence_id: Option<String>) {
        if let Some(t) = self.tasks.get_mut(task_id) {
            t.status = TaskStatus::Settled;
        }
        self.push_event(
            "settlement_done",
            format!("settlement for {}", task_id),
            Some(task_id.to_string()),
            evidence_id,
        );
    }

    /// Authoritative settlement record: status + tick + evidence + actor +
    /// optional deliverable hash, all on the task itself. The evidence
    /// preimage (`hub:<task>:<actor>:<tick>`) becomes recomputable from the
    /// receipt — passports anchor on this, never on inference.
    pub fn record_settlement(
        &mut self,
        task_id: &str,
        evidence_id: String,
        actor: String,
        tick: u64,
        deliverable_hash: Option<String>,
    ) {
        self.record_settlement_with_evolution(
            task_id,
            evidence_id,
            actor,
            tick,
            deliverable_hash,
            None,
        );
    }

    /// Like [`record_settlement`], but also binds an optional [`EvolutionTag`]
    /// to the settled task, anchoring a client generation ledger in the fabric.
    pub fn record_settlement_with_evolution(
        &mut self,
        task_id: &str,
        evidence_id: String,
        actor: String,
        tick: u64,
        deliverable_hash: Option<String>,
        evolution: Option<EvolutionTag>,
    ) {
        if let Some(t) = self.tasks.get_mut(task_id) {
            t.status = TaskStatus::Settled;
            t.settled_tick = Some(tick);
            t.evidence_id = Some(evidence_id.clone());
            t.settled_by = Some(actor);
            t.deliverable_hash = deliverable_hash;
            if evolution.is_none() || evolution.as_ref().is_some_and(|e| e.is_present()) {
                t.evolution = evolution;
            }
        }
        self.push_event(
            "settlement_done",
            format!("settlement for {}", task_id),
            Some(task_id.to_string()),
            Some(evidence_id),
        );
    }

    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    /// Recover the evidence actor for legacy records (settled before
    /// receipt fields existed): the unique candidate whose
    /// `blake3("hub:<task>:<actor>:<tick>")` matches the evidence. A match
    /// is a cryptographic verification, not an inference; `None` stays null.
    /// Callers assemble candidates from team members, bidders, issuer, and
    /// the well-known fallback actors.
    pub fn recover_evidence_actor(
        task_id: &str,
        evidence: &str,
        tick: u64,
        candidates: &[String],
    ) -> Option<String> {
        candidates.iter().find(|cand| {
            blake3::hash(format!("hub:{task_id}:{cand}:{tick}").as_bytes())
                .to_hex()
                .to_string()
                == evidence
        }).cloned()
    }

    /// Feed window: events at/after `since`, OLDEST-first (append-safe),
    /// but capped to the NEWEST `limit` — the tail, not the head. (A
    /// head-window starves followers once the log exceeds `limit`.)
    pub fn events_since(&self, since: u64, limit: usize) -> Vec<HubEvent> {
        let filtered: Vec<_> = self
            .events
            .iter()
            .filter(|e| e.tick >= since)
            .cloned()
            .collect();
        let skip = filtered.len().saturating_sub(limit);
        filtered.into_iter().skip(skip).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recover_evidence_actor_verifies_not_guesses() {
        // Real vectors: task-0097 (twin probe) and task-0096-adjacent shape.
        let ev97 = blake3::hash(b"hub:task-0097:agent:pylon-verify:334")
            .to_hex()
            .to_string();
        let cands = vec![
            "operator".to_string(),
            "agent:pylon-verify".to_string(),
            "open".to_string(),
        ];
        assert_eq!(
            HubState::recover_evidence_actor("task-0097", &ev97, 334, &cands),
            Some("agent:pylon-verify".to_string())
        );
        // Wrong tick or unknown actor pool → None (stays null, never inferred).
        assert_eq!(
            HubState::recover_evidence_actor("task-0097", &ev97, 335, &cands),
            None
        );
        assert_eq!(
            HubState::recover_evidence_actor(
                "task-0097",
                &ev97,
                334,
                &["operator".to_string()]
            ),
            None
        );
    }

    #[test]
    fn record_settlement_binds_receipt_fields_and_single_event() {
        let mut hub = HubState::new();
        let task = hub.publish_task(
            "alice".into(),
            "t".into(),
            "d".into(),
            100,
            None,
        );
        let before = hub.events.len();
        hub.record_settlement(
            &task.id,
            "ev123".into(),
            "bob".into(),
            7,
            Some("ab".repeat(32)),
        );
        let t = hub.tasks.get(&task.id).unwrap();
        assert_eq!(t.status, TaskStatus::Settled);
        assert_eq!(t.settled_tick, Some(7));
        assert_eq!(t.evidence_id.as_deref(), Some("ev123"));
        assert_eq!(t.settled_by.as_deref(), Some("bob"));
        let expect_dh = "ab".repeat(32);
        assert_eq!(t.deliverable_hash.as_deref(), Some(expect_dh.as_str()));
        // Exactly one settlement event (record subsumes settle).
        assert_eq!(hub.events.len(), before + 1);
        let ev = hub.events.back().unwrap();
        assert_eq!(ev.kind, "settlement_done");
        assert_eq!(ev.evidence_id.as_deref(), Some("ev123"));
    }

    #[test]
    fn record_settlement_with_evolution_binds_anchor() {
        let mut hub = HubState::new();
        let task = hub.publish_task("alice".into(), "t".into(), "d".into(), 100, None);
        let dh = "ab".repeat(32);
        let art = "11".repeat(32);
        let par = "22".repeat(32);
        let ben = "33".repeat(32);
        let evo = EvolutionTag::from_args(
            Some(art.clone()),
            None,
            Some(par.clone()),
            None,
            Some(ben.clone()),
            None,
        )
        .expect("all three hashes present");
        hub.record_settlement_with_evolution(
            &task.id,
            "ev123".into(),
            "bob".into(),
            7,
            Some(dh),
            Some(evo),
        );
        let t = hub.tasks.get(&task.id).unwrap();
        let bound = t.evolution.as_ref().expect("evolution anchored");
        assert_eq!(bound.artifact_hash.as_deref(), Some(art.as_str()));
        assert_eq!(bound.parent_hash.as_deref(), Some(par.as_str()));
        assert_eq!(bound.bench_hash.as_deref(), Some(ben.as_str()));
        // *_alg defaults to blake3-256 when hash present.
        assert_eq!(bound.artifact_alg.as_deref(), Some("blake3-256"));
        assert_eq!(bound.parent_alg.as_deref(), Some("blake3-256"));
        assert_eq!(bound.bench_alg.as_deref(), Some("blake3-256"));
    }

    #[test]
    fn evolution_tag_from_args_omits_empty_block() {
        // No hash field → None (receipt never carries an empty evolution {}).
        assert!(EvolutionTag::from_args(None, None, None, None, None, None).is_none());
        // Only artifact supplied → other alg fields stay None.
        let t = EvolutionTag::from_args(
            Some("11".repeat(32)),
            None,
            None,
            None,
            None,
            None,
        )
        .expect("artifact present");
        assert!(t.is_present());
        assert_eq!(t.artifact_alg.as_deref(), Some("blake3-256"));
        assert_eq!(t.parent_hash, None);
        assert_eq!(t.bench_hash, None);
        // Honored explicit alg.
        let t2 = EvolutionTag::from_args(
            Some("11".repeat(32)),
            Some("sha-256".into()),
            None,
            None,
            None,
            None,
        )
        .expect("artifact present");
        assert_eq!(t2.artifact_alg.as_deref(), Some("sha-256"));
    }

    #[test]
    fn events_since_returns_newest_window_oldest_first() {        // Regression: the feed must serve the TAIL, not the head — a
        // head-window starves followers once the log exceeds `limit`.
        let mut hub = HubState::new();
        for i in 0..5 {
            hub.tick = i;
            hub.push_event("t", format!("e{i}"), None, None);
        }
        let win = hub.events_since(0, 2);
        assert_eq!(win.len(), 2);
        assert_eq!(win[0].detail, "e3");
        assert_eq!(win[1].detail, "e4");
        // `since` still filters, then the tail applies within it.
        let win = hub.events_since(4, 10);
        assert_eq!(win.len(), 1);
        assert_eq!(win[0].detail, "e4");
        // Zero limit = empty, never a panic.
        assert!(hub.events_since(0, 0).is_empty());
    }

    #[test]
    fn self_bid_rejected_and_award_detected() {
        let mut hub = HubState::new();
        let task = hub.publish_task(
            "alice".into(),
            "T".into(),
            "d".into(),
            500,
            Some("analysis".into()),
        );
        // Issuer bidding on her own task: refused, nothing recorded.
        let err = hub
            .place_bid("alice".into(), task.id.clone(), 100, "me".into())
            .unwrap_err();
        assert!(matches!(err, HubError::SelfBid));
        assert!(err.to_string().contains("self_bid_forbidden"));
        assert!(hub.bids.is_empty());
        assert!(!hub.self_bid_award(&task.id), "no bids: not an own-bid award");
        // Padded identity still matches (normalized, non-empty rule).
        assert!(HubState::same_party(" alice ", "alice"));
        assert!(!HubState::same_party("", ""));
        // Honest bid: no refusal, no award flag.
        hub.place_bid("beta".into(), task.id.clone(), 100, "ok".into())
            .unwrap();
        assert!(!hub.self_bid_award(&task.id));
        // Legacy self-bid (predates the place_bid gate, e.g. from disk):
        // the award detector still catches it at execute time.
        hub.bids.insert(
            "bid-legacy".to_string(),
            crate::Bid {
                id: "bid-legacy".to_string(),
                task_id: task.id.clone(),
                bidder: "alice".to_string(),
                price: 50,
                rationale: "legacy".to_string(),
                created_tick: 0,
            },
        );
        assert!(hub.self_bid_award(&task.id));
    }

    #[test]
    fn task_bid_team_settle_flow() {
        let mut hub = HubState::new();
        let task = hub.publish_task(
            "alice".into(),
            "Analyze docs".into(),
            "2000 docs".into(),
            500,
            Some("analysis".into()),
        );
        assert_eq!(task.status, TaskStatus::Open);
        let _b1 = hub
            .place_bid("beta".into(), task.id.clone(), 450, "can do".into())
            .unwrap();
        let b2 = hub
            .place_bid("gamma".into(), task.id.clone(), 350, "better".into())
            .unwrap();
        let best = hub.best_bid(&task.id).unwrap();
        assert_eq!(best.bidder, "gamma");
        assert_eq!(best.price, 350);
        let prop = hub
            .propose("alice".into(), "gamma".into(), task.id.clone(), 400, 100)
            .unwrap();
        hub.decide_proposal(&prop.id, "gamma", true).unwrap();
        let team = hub
            .form_team(
                task.id.clone(),
                vec![("beta".into(), 40), ("gamma".into(), 60)],
            )
            .unwrap();
        assert_eq!(team.members.len(), 2);
        hub.mark_executing(&task.id);
        assert_eq!(hub.tasks[&task.id].status, TaskStatus::InProgress);
        hub.settle(&task.id, Some("ev123".into()));
        assert_eq!(hub.tasks[&task.id].status, TaskStatus::Settled);
        assert!(hub.events.iter().any(|e| e.kind == "settlement_done"));
        let _ = b2;
    }
}
