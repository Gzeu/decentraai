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

    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    pub fn events_since(&self, since: u64, limit: usize) -> Vec<HubEvent> {
        self.events
            .iter()
            .filter(|e| e.tick >= since)
            .take(limit)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
