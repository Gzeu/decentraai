//! Social state: relationships, contributions, task outcomes

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use super::{AgentId, TaskId, Tick};

/// Kind of social relationship between two agents
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    /// Agents have worked together on a task
    WorkedWith,
    /// Agent A accepted Agent B's proposal/bid
    Accepted,
    /// Agent A rejected Agent B's proposal/bid
    Rejected,
    /// Agent A countered Agent B's offer
    Countered,
    /// Collaboration succeeded (verified)
    Successful,
    /// Collaboration failed (verified)
    Failed,
    /// Positive trust signal (reliability, quality, etc.)
    TrustSignal,
    /// Negative trust signal
    DistrustSignal,
}

impl RelationshipKind {
    /// Weight for reputation calculation
    pub fn weight(&self) -> f32 {
        match self {
            RelationshipKind::Successful => 1.0,
            RelationshipKind::Failed => -1.0,
            RelationshipKind::TrustSignal => 0.5,
            RelationshipKind::DistrustSignal => -0.5,
            RelationshipKind::Accepted => 0.3,
            RelationshipKind::Rejected => -0.2,
            RelationshipKind::Countered => 0.1,
            RelationshipKind::WorkedWith => 0.2,
        }
    }
    
    /// Whether this is a positive signal
    pub fn is_positive(&self) -> bool {
        self.weight() > 0.0
    }
}

/// A directed social relationship from observer -> subject
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialRelationship {
    /// The agent observing/recording this relationship
    pub observer: AgentId,
    /// The agent being observed
    pub subject: AgentId,
    /// Kind of relationship
    pub kind: RelationshipKind,
    /// Task context (optional)
    pub task_id: Option<TaskId>,
    /// Tick when recorded
    pub tick: Tick,
    /// Optional detail/rationale
    pub detail: Option<String>,
    /// Strength (0.0 to 1.0, or -1.0 to 0.0 for negative)
    pub strength: f32,
}

impl SocialRelationship {
    pub fn new(observer: AgentId, subject: AgentId, kind: RelationshipKind, tick: Tick) -> Self {
        Self {
            observer,
            subject,
            kind,
            task_id: None,
            tick,
            detail: None,
            strength: kind.weight().abs().min(1.0),
        }
    }
    
    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }
    
    pub fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail);
        self
    }
    
    pub fn with_strength(mut self, strength: f32) -> Self {
        self.strength = strength.clamp(-1.0, 1.0);
        self
    }
}

/// Record of an agent's contribution to a team task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributionRecord {
    /// Task this contribution belongs to
    pub task_id: TaskId,
    /// Agent who contributed
    pub agent_id: AgentId,
    /// Planned workshare (from team formation)
    pub planned_share: u8,
    /// Verified contribution (0.0 to 1.0, 1.0 = full delivery)
    pub verified_contribution: Option<f32>,
    /// Evidence ID if verified
    pub evidence_id: Option<String>,
    /// Tick when contribution was recorded
    pub recorded_tick: Tick,
    /// Tick when verified (if verified)
    pub verified_tick: Option<Tick>,
    /// Quality score of the contribution (0.0 to 1.0)
    pub quality: Option<f32>,
    /// Whether contribution met SLA
    pub met_sla: Option<bool>,
}

impl ContributionRecord {
    pub fn new(task_id: TaskId, agent_id: AgentId, planned_share: u8, tick: Tick) -> Self {
        Self {
            task_id,
            agent_id,
            planned_share,
            verified_contribution: None,
            evidence_id: None,
            recorded_tick: tick,
            verified_tick: None,
            quality: None,
            met_sla: None,
        }
    }
    
    pub fn verify(mut self, contribution: f32, evidence_id: String, quality: f32, met_sla: bool, tick: Tick) -> Self {
        self.verified_contribution = Some(contribution.clamp(0.0, 1.0));
        self.evidence_id = Some(evidence_id);
        self.quality = Some(quality.clamp(0.0, 1.0));
        self.met_sla = Some(met_sla);
        self.verified_tick = Some(tick);
        self
    }
    
    /// Effective share for reward distribution
    pub fn effective_share(&self) -> f32 {
        self.verified_contribution.unwrap_or(self.planned_share as f32 / 100.0)
    }
}

/// Outcome of a task for social memory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub task_id: TaskId,
    pub issuer: AgentId,
    pub team_members: Vec<AgentId>,
    pub status: TaskOutcomeStatus,
    pub evidence_id: Option<String>,
    pub settled_tick: Tick,
    pub total_reward: u64,
    pub distributions: Vec<RewardDistribution>,
    pub contributor_records: Vec<ContributionRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcomeStatus {
    Completed,
    Settled,
    Failed,
    Disputed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardDistribution {
    pub agent_id: AgentId,
    pub amount: u64,
    pub share_basis: ShareBasis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareBasis {
    Planned,
    Verified,
    Hybrid,
}

/// Complete society state
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SocietyState {
    /// Social relationships: observer -> (subject -> [relationships])
    pub relationships: HashMap<AgentId, HashMap<AgentId, Vec<SocialRelationship>>>,
    /// Contribution records by task
    pub contributions: HashMap<TaskId, Vec<ContributionRecord>>,
    /// Task outcomes
    pub outcomes: HashMap<TaskId, TaskOutcome>,
    /// Reputation events per agent
    pub reputation: HashMap<AgentId, Vec<ReputationEvent>>,
    /// Current tick
    pub tick: Tick,
    /// Max history per relationship
    pub max_history_per_pair: usize,
}

impl SocietyState {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn with_tick(tick: Tick) -> Self {
        Self { tick, ..Default::default() }
    }
    
    /// Record a social relationship
    pub fn record_relationship(&mut self, rel: SocialRelationship) {
        let observer = rel.observer.clone();
        let subject = rel.subject.clone();
        let entry = self.relationships
            .entry(observer)
            .or_default()
            .entry(subject)
            .or_default();
        entry.push(rel);
        // Trim history
        if entry.len() > self.max_history_per_pair {
            let excess = entry.len() - self.max_history_per_pair;
            entry.drain(0..excess);
        }
    }
    
    /// Get relationships from observer to subject
    pub fn get_relationships(&self, observer: &AgentId, subject: &AgentId) -> Vec<&SocialRelationship> {
        self.relationships
            .get(observer)
            .and_then(|m| m.get(subject))
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }
    
    /// Get all relationships where observer is involved
    pub fn get_all_for_agent(&self, agent: &AgentId) -> Vec<&SocialRelationship> {
        self.relationships
            .get(agent)
            .map(|m| m.values().flatten().collect())
            .unwrap_or_default()
    }
    
    /// Get relationships where agent is subject
    pub fn get_about_agent(&self, agent: &AgentId) -> Vec<&SocialRelationship> {
        self.relationships
            .values()
            .flat_map(|m| m.get(agent))
            .flatten()
            .collect()
    }
    
    /// Compute trust score from observer to subject based on relationship history
    pub fn trust_score(&self, observer: &AgentId, subject: &AgentId) -> f32 {
        let rels = self.get_relationships(observer, subject);
        if rels.is_empty() {
            return 0.0; // Unknown = neutral
        }
        
        let total: f32 = rels.iter()
            .map(|r| r.kind.weight() * r.strength)
            .sum();
        let count = rels.len() as f32;
        
        (total / count).clamp(-1.0, 1.0)
    }
    
    /// Record a contribution
    pub fn record_contribution(&mut self, contrib: ContributionRecord) {
        self.contributions
            .entry(contrib.task_id.clone())
            .or_default()
            .push(contrib);
    }
    
    /// Record task outcome
    pub fn record_outcome(&mut self, outcome: TaskOutcome) {
        self.outcomes.insert(outcome.task_id.clone(), outcome);
    }
    
    /// Record reputation event
    pub fn record_reputation_event(&mut self, event: ReputationEvent) {
        self.reputation
            .entry(event.agent_id.clone())
            .or_default()
            .push(event);
    }
    
    /// Get recent outcomes for an agent
    pub fn recent_outcomes(&self, agent: &AgentId, limit: usize) -> Vec<&TaskOutcome> {
        let mut outcomes: Vec<_> = self.outcomes
            .values()
            .filter(|o| o.team_members.contains(agent) || o.issuer == *agent)
            .collect();
        
        // Sort by tick descending
        outcomes.sort_by_key(|b| std::cmp::Reverse(b.settled_tick));
        outcomes.into_iter().take(limit).collect()
    }
    
    /// Get contribution for agent on task
    pub fn get_contribution(&self, task_id: &TaskId, agent: &AgentId) -> Option<&ContributionRecord> {
        self.contributions
            .get(task_id)
            .and_then(|v| v.iter().find(|c| c.agent_id == *agent))
    }
    
    /// Advance tick
    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }
}

/// Reputation event from verified outcomes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationEvent {
    pub agent_id: AgentId,
    pub event_type: ReputationEventType,
    pub task_id: Option<TaskId>,
    pub delta: f32, // -1.0 to 1.0
    pub tick: Tick,
    pub evidence_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReputationEventType {
    TaskCompleted,
    TaskFailed,
    QualityHigh,
    QualityLow,
    SlaMet,
    SlaMissed,
    ContributionVerified,
    ContributionMissing,
    ProposalAccepted,
    ProposalRejected,
    BidAccepted,
    BidRejected,
}
