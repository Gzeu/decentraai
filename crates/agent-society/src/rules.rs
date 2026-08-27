//! Society Rules v0.1: decision logic for autonomous agents
//!
//! These rules affect agent decisions without hardcoding behaviors.
//! Agents observe state, apply rules, and choose actions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use crate::{
    AgentId, TaskId, ProposalId, Tick, SocietyError,
    ReputationEvent, ReputationEventType,
    state::{SocialRelationship, ContributionRecord, TaskOutcome, TaskOutcomeStatus, RewardDistribution, ShareBasis},
    reputation::SocialReputation,
};
use decentraai_agent_hub::{HubTask, Bid, Proposal, ProposalStatus, Team, HubEvent, TaskStatus};

/// Context for a decision - what the agent sees
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionContext {
    /// The deciding agent
    pub agent_id: AgentId,
    /// Current tick
    pub tick: Tick,
    /// Hub state snapshot
    pub hub: HubSnapshot,
    /// Society state snapshot
    pub society: SocietySnapshot,
    /// Agent's own reputation
    pub own_reputation: Option<SocialReputation>,
    /// Agent's resource state (quota, capacity)
    pub resources: ResourceState,
}

/// Snapshot of hub state for decision making
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubSnapshot {
    pub tick: Tick,
    pub open_tasks: Vec<HubTask>,
    pub my_tasks: Vec<HubTask>,
    pub my_bids: Vec<Bid>,
    pub pending_proposals: Vec<Proposal>,
    pub my_teams: Vec<Team>,
    pub recent_events: Vec<HubEvent>,
    pub total_tasks: usize,
    pub total_bids: usize,
}

/// Snapshot of society state for decision making
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocietySnapshot {
    pub tick: Tick,
    /// Trust scores toward other agents
    pub trust_scores: BTreeMap<AgentId, f32>,
    /// Reputation of other agents (capability -> reputation)
    pub other_reputations: BTreeMap<AgentId, SocialReputation>,
    /// My recent outcomes
    pub recent_outcomes: Vec<TaskOutcome>,
    /// My contribution records
    pub my_contributions: Vec<ContributionRecord>,
    /// Relationships with other agents
    pub relationships: Vec<SocialRelationship>,
}

/// Agent's resource state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceState {
    pub quota_available: u64,
    pub quota_ceiling: u64,
    pub capacity_used: f32, // 0.0 to 1.0
    pub max_concurrent_tasks: u32,
    pub current_tasks: u32,
}

/// Hint for a decision (rationale + suggested action)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionHint {
    pub action: SuggestedAction,
    pub rationale: String,
    pub confidence: f32, // 0.0 to 1.0
    pub alternatives: Vec<AlternativeAction>,
}

/// Suggested action types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestedAction {
    PublishTask { title: String, reward: u64, capability: Option<String> },
    PlaceBid { task_id: TaskId, price: u64, rationale: String },
    Propose { to: AgentId, task_id: TaskId, offer_price: u64, workshare: u8 },
    DecideProposal { proposal_id: ProposalId, accept: bool },
    FormTeam { task_id: TaskId, members: Vec<(AgentId, u8)> },
    ExecuteTask { task_id: TaskId },
    Refuse { reason: String },
    Wait,
}

/// Alternative action for decision transparency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlternativeAction {
    pub action: SuggestedAction,
    pub rationale: String,
    pub confidence: f32,
}

/// Core society rules engine
#[derive(Debug, Clone)]
pub struct SocietyRules {
    pub min_bid_ratio: f32,        // Min bid as fraction of reward (e.g., 0.5)
    pub max_bid_ratio: f32,        // Max bid as fraction of reward (e.g., 0.95)
    pub counter_offer_min_improvement: f32, // Min improvement for counter (e.g., 0.05)
    pub trust_threshold_collaborate: f32,   // Min trust to collaborate (e.g., 0.2)
    pub trust_threshold_refuse: f32,        // Below this, refuse (e.g., -0.3)
    pub reputation_weight: f32,             // Weight of reputation in decisions
    pub resource_conservation_factor: f32,  // How much to conserve resources
    pub specialization_bonus: f32,          // Bonus for known capabilities
}

impl Default for SocietyRules {
    fn default() -> Self {
        Self {
            min_bid_ratio: 0.5,
            max_bid_ratio: 0.95,
            counter_offer_min_improvement: 0.05,
            trust_threshold_collaborate: 0.2,
            trust_threshold_refuse: -0.3,
            reputation_weight: 0.3,
            resource_conservation_factor: 0.2,
            specialization_bonus: 0.15,
        }
    }
}

impl SocietyRules {
    /// Evaluate decision context and return hints for possible actions
    pub fn evaluate(&self, ctx: &DecisionContext) -> Vec<DecisionHint> {
        let mut hints = Vec::new();
        
        // Rule 1: If have open tasks with bids, consider proposing
        for task in &ctx.hub.my_tasks {
            if task.status == TaskStatus::Open || task.status == TaskStatus::Bidding {
                hints.extend(self.evaluate_propose_opportunities(ctx, task));
            }
        }
        
        // Rule 2: If have pending proposals, decide
        for prop in &ctx.hub.pending_proposals {
            hints.push(self.evaluate_proposal_decision(ctx, prop));
        }
        
        // Rule 3: If have accepted proposals without teams, form team
        for task in &ctx.hub.my_tasks {
            if task.status == TaskStatus::Assigned || task.status == TaskStatus::Bidding {
                hints.extend(self.evaluate_team_formation(ctx, task));
            }
        }
        
        // Rule 4: If have teams ready, execute
        for team in &ctx.hub.my_teams {
            hints.extend(self.evaluate_execution(ctx, team));
        }
        
        // Rule 5: Consider bidding on open tasks
        hints.extend(self.evaluate_bidding_opportunities(ctx));
        
        // Rule 6: Consider publishing new tasks
        hints.extend(self.evaluate_task_publishing(ctx));
        
        // Rule 7: Resource conservation - wait if overloaded
        if ctx.resources.capacity_used > 0.8 {
            hints.push(DecisionHint {
                action: SuggestedAction::Wait,
                rationale: "High capacity usage, conserving resources".to_string(),
                confidence: 0.8,
                alternatives: vec![],
            });
        }
        
        // Sort by confidence descending
        hints.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        hints
    }
    
    fn evaluate_propose_opportunities(&self, ctx: &DecisionContext, task: &HubTask) -> Vec<DecisionHint> {
        let mut hints = Vec::new();
        
        // Find bidders on this task
        let bidders: Vec<&Bid> = ctx.hub.my_bids.iter()
            .filter(|b| b.task_id == task.id)
            .collect();
        
        for bid in bidders {
            let trust = ctx.society.trust_scores.get(&bid.bidder).copied().unwrap_or(0.0);
            let rep = ctx.society.other_reputations.get(&bid.bidder);
            let rep_score = rep.map(|r| r.overall).unwrap_or(0.0);
            
            // Trust + reputation weighted score
            let combined = trust * (1.0 - self.reputation_weight) + rep_score * self.reputation_weight;
            
            if combined >= self.trust_threshold_collaborate {
                // Make counter-offer above bid but below reward
                let min_offer = (bid.price as f32 * (1.0 + self.counter_offer_min_improvement)) as u64;
                let max_offer = task.reward;
                let offer_price = min_offer.min(max_offer);
                
                if offer_price > bid.price {
                    hints.push(DecisionHint {
                        action: SuggestedAction::Propose {
                            to: bid.bidder.clone(),
                            task_id: task.id.clone(),
                            offer_price,
                            workshare: 100,
                        },
                        rationale: format!("Bidder {} has trust {:.2}, rep {:.2}; counter-offer {} > {}", 
                            bid.bidder, trust, rep_score, offer_price, bid.price),
                        confidence: (combined + 0.5).min(1.0),
                        alternatives: vec![AlternativeAction {
                            action: SuggestedAction::Wait,
                            rationale: "Could wait for better bids".to_string(),
                            confidence: 0.3,
                        }],
                    });
                }
            } else if combined <= self.trust_threshold_refuse {
                hints.push(DecisionHint {
                    action: SuggestedAction::Refuse {
                        reason: format!("Low trust ({:.2}) + rep ({:.2}) for {}", trust, rep_score, bid.bidder),
                    },
                    rationale: "Trust below refusal threshold".to_string(),
                    confidence: 0.7,
                    alternatives: vec![],
                });
            }
        }
        
        hints
    }
    
    fn evaluate_proposal_decision(&self, ctx: &DecisionContext, prop: &Proposal) -> DecisionHint {
        let trust = ctx.society.trust_scores.get(&prop.from).copied().unwrap_or(0.0);
        let rep = ctx.society.other_reputations.get(&prop.from);
        let rep_score = rep.map(|r| r.overall).unwrap_or(0.0);
        let combined = trust * (1.0 - self.reputation_weight) + rep_score * self.reputation_weight;
        
        // Check if offer is reasonable (above any bid, within reward)
        let task = ctx.hub.open_tasks.iter().find(|t| t.id == prop.task_id);
        let task_reward = task.map(|t| t.reward).unwrap_or(prop.offer_price);
        let best_bid = ctx.hub.my_bids.iter()
            .filter(|b| b.task_id == prop.task_id)
            .min_by_key(|b| b.price);
        
        let min_acceptable = best_bid.map(|b| b.price).unwrap_or(0) + 1;
        let is_reasonable = prop.offer_price >= min_acceptable && prop.offer_price <= task_reward;
        
        let accept = combined >= self.trust_threshold_collaborate && is_reasonable;
        
        DecisionHint {
            action: SuggestedAction::DecideProposal {
                proposal_id: prop.id.clone(),
                accept,
            },
            rationale: format!("Trust {:.2}, rep {:.2}, offer {} vs reward {} -> {}", 
                trust, rep_score, prop.offer_price, task_reward, if accept { "accept" } else { "reject" }),
            confidence: if accept { (combined + 0.5).min(1.0) } else { 0.6 },
            alternatives: vec![AlternativeAction {
                action: SuggestedAction::DecideProposal { proposal_id: prop.id.clone(), accept: !accept },
                rationale: format!("Alternative: {}", if accept { "reject" } else { "accept" }),
                confidence: 0.3,
            }],
        }
    }
    
    fn evaluate_team_formation(&self, ctx: &DecisionContext, task: &HubTask) -> Vec<DecisionHint> {
        // Check if we have accepted proposal for this task
        let accepted_prop = ctx.hub.pending_proposals.iter()
            .find(|p| p.task_id == task.id && p.status == ProposalStatus::Accepted);
        
        if let Some(prop) = accepted_prop {
            // Find existing team
            let has_team = ctx.hub.my_teams.iter().any(|t| t.task_id == task.id);
            if !has_team {
                let mut members = vec![(ctx.agent_id.clone(), 60), (prop.to.clone(), 40)];
                // Add other bidders if they exist
                for bid in &ctx.hub.my_bids {
                    if bid.task_id == task.id && bid.bidder != prop.to {
                        members.push((bid.bidder.clone(), 20));
                    }
                }
                // Normalize to 100
                let total: u16 = members.iter().map(|(_, s)| *s as u16).sum();
                if total != 100 {
                    members = members.into_iter()
                        .map(|(a, s)| (a, (s as u16 * 100 / total) as u8))
                        .collect();
                }
                
                return vec![DecisionHint {
                    action: SuggestedAction::FormTeam {
                        task_id: task.id.clone(),
                        members,
                    },
                    rationale: format!("Accepted proposal from {}, forming team", prop.to),
                    confidence: 0.9,
                    alternatives: vec![],
                }];
            }
        }
        
        vec![]
    }
    
    fn evaluate_execution(&self, ctx: &DecisionContext, team: &Team) -> Vec<DecisionHint> {
        // Check if task is assigned and team exists
        let task = ctx.hub.my_tasks.iter().find(|t| t.id == team.task_id);
        if let Some(task) = task {
            if task.status == TaskStatus::Assigned || task.status == TaskStatus::InProgress {
                // Check if we're the issuer or a team member
                let is_issuer = task.issuer == ctx.agent_id;
                let is_member = team.members.iter().any(|(a, _)| a == &ctx.agent_id);
                
                if is_issuer || is_member {
                    return vec![DecisionHint {
                        action: SuggestedAction::ExecuteTask { task_id: task.id.clone() },
                        rationale: format!("Team formed for task {}, ready to execute", task.id),
                        confidence: 0.95,
                        alternatives: vec![],
                    }];
                }
            }
        }
        vec![]
    }
    
    fn evaluate_bidding_opportunities(&self, ctx: &DecisionContext) -> Vec<DecisionHint> {
        let mut hints = Vec::new();
        
        // Only bid if we have capacity
        if ctx.resources.current_tasks >= ctx.resources.max_concurrent_tasks {
            return hints;
        }
        
        for task in &ctx.hub.open_tasks {
            if task.issuer == ctx.agent_id {
                continue; // Don't bid on own tasks
            }
            
            // Check capability match
            if let Some(_req_cap) = &task.required_capability {
                // Would check against agent's capabilities here
            }
            
            // Calculate bid price
            let min_bid = (task.reward as f32 * self.min_bid_ratio) as u64;
            let max_bid = (task.reward as f32 * self.max_bid_ratio) as u64;
            let bid_price = max_bid.min(min_bid + 50); // Competitive but not too high
            
            // Check quota
            if bid_price > ctx.resources.quota_available {
                continue;
            }
            
            // Trust in issuer
            let trust = ctx.society.trust_scores.get(&task.issuer).copied().unwrap_or(0.0);
            let rep = ctx.society.other_reputations.get(&task.issuer);
            let rep_score = rep.map(|r| r.overall).unwrap_or(0.0);
            
            if trust > self.trust_threshold_refuse && rep_score > -0.5 {
                hints.push(DecisionHint {
                    action: SuggestedAction::PlaceBid {
                        task_id: task.id.clone(),
                        price: bid_price,
                        rationale: format!("Task {} reward {}, bid {}", task.id, task.reward, bid_price),
                    },
                    rationale: format!("Open task {} reward {}, trust {:.2} with issuer {}", task.id, task.reward, trust, task.issuer),
                    confidence: (trust * 0.5 + 0.3).min(0.8),
                    alternatives: vec![AlternativeAction {
                        action: SuggestedAction::Wait,
                        rationale: "Could wait for better opportunities".to_string(),
                        confidence: 0.2,
                    }],
                });
            }
        }
        
        hints
    }
    
    fn evaluate_task_publishing(&self, ctx: &DecisionContext) -> Vec<DecisionHint> {
        // Publish if we have quota and few open tasks
        if ctx.resources.quota_available > 200 && ctx.hub.my_tasks.len() < 3 && ctx.hub.total_tasks < 20 {
            let reward = (ctx.resources.quota_available as f32 * 0.3) as u64;
            return vec![DecisionHint {
                action: SuggestedAction::PublishTask {
                    title: format!("Task from {}", ctx.agent_id),
                    reward: reward.clamp(100, 500),
                    capability: Some("analysis".to_string()),
                },
                rationale: format!("Have quota {}, few tasks, publishing new", ctx.resources.quota_available),
                confidence: 0.4,
                alternatives: vec![AlternativeAction {
                    action: SuggestedAction::Wait,
                    rationale: "Could conserve quota".to_string(),
                    confidence: 0.3,
                }],
            }];
        }
        vec![]
    }
    
    /// Validate a counter-offer
    pub fn validate_counter_offer(&self, original_price: u64, counter_price: u64, task_reward: u64) -> Result<(), SocietyError> {
        if counter_price <= original_price {
            return Err(SocietyError::InvalidCounterOffer("Counter must exceed original".to_string()));
        }
        if counter_price > task_reward {
            return Err(SocietyError::InvalidCounterOffer("Counter exceeds task reward".to_string()));
        }
        let improvement = (counter_price as f32 - original_price as f32) / original_price as f32;
        if improvement < self.counter_offer_min_improvement {
            return Err(SocietyError::InvalidCounterOffer(
                format!("Counter improvement {:.1}% below minimum {:.1}%", improvement * 100.0, self.counter_offer_min_improvement * 100.0)
            ));
        }
        Ok(())
    }
    
    /// Validate refusal is allowed
    pub fn validate_refusal(&self, trust: f32, reason: &str) -> Result<(), SocietyError> {
        if trust > self.trust_threshold_refuse && reason.is_empty() {
            return Err(SocietyError::RefusalNotAllowed("Refusal requires reason when trust is neutral/positive".to_string()));
        }
        Ok(())
    }
    
    /// Calculate reward distribution based on verified contributions
    pub fn calculate_distribution(&self, task: &HubTask, _team: &Team, contributions: &[ContributionRecord]) -> Vec<RewardDistribution> {
        let total_reward = task.reward;
        let mut distributions = Vec::new();
        
        // Sum of effective shares
        let total_effective: f32 = contributions.iter().map(|c| c.effective_share()).sum();
        
        for contrib in contributions {
            let share = if total_effective > 0.0 {
                contrib.effective_share() / total_effective
            } else {
                contrib.planned_share as f32 / 100.0
            };
            
            let amount = (total_reward as f32 * share) as u64;
            distributions.push(RewardDistribution {
                agent_id: contrib.agent_id.clone(),
                amount,
                share_basis: if contrib.verified_contribution.is_some() { ShareBasis::Verified } else { ShareBasis::Planned },
            });
        }
        
        distributions
    }
    
    /// Generate reputation events from task outcome
    pub fn generate_reputation_events(&self, outcome: &TaskOutcome, tick: Tick) -> Vec<ReputationEvent> {
        let mut events = Vec::new();
        
        for dist in &outcome.distributions {
            let contrib = outcome.contributor_records.iter()
                .find(|c| c.agent_id == dist.agent_id);
            
            match outcome.status {
                TaskOutcomeStatus::Completed | TaskOutcomeStatus::Settled => {
                    events.push(ReputationEvent {
                        agent_id: dist.agent_id.clone(),
                        event_type: ReputationEventType::TaskCompleted,
                        task_id: Some(outcome.task_id.clone()),
                        delta: 0.2,
                        tick,
                        evidence_id: outcome.evidence_id.clone(),
                        detail: format!("Task {} completed", outcome.task_id),
                    });
                    
                    if let Some(c) = contrib {
                        if c.quality.unwrap_or(0.5) > 0.7 {
                            events.push(ReputationEvent {
                                agent_id: dist.agent_id.clone(),
                                event_type: ReputationEventType::QualityHigh,
                                task_id: Some(outcome.task_id.clone()),
                                delta: 0.15,
                                tick,
                                evidence_id: outcome.evidence_id.clone(),
                                detail: format!("High quality contribution to {}", outcome.task_id),
                            });
                        }
                        if c.met_sla.unwrap_or(false) {
                            events.push(ReputationEvent {
                                agent_id: dist.agent_id.clone(),
                                event_type: ReputationEventType::SlaMet,
                                task_id: Some(outcome.task_id.clone()),
                                delta: 0.1,
                                tick,
                                evidence_id: outcome.evidence_id.clone(),
                                detail: format!("SLA met for {}", outcome.task_id),
                            });
                        }
                        if c.verified_contribution.is_some() {
                            events.push(ReputationEvent {
                                agent_id: dist.agent_id.clone(),
                                event_type: ReputationEventType::ContributionVerified,
                                task_id: Some(outcome.task_id.clone()),
                                delta: 0.1,
                                tick,
                                evidence_id: c.evidence_id.clone(),
                                detail: format!("Contribution verified for {}", outcome.task_id),
                            });
                        }
                    }
                }
                TaskOutcomeStatus::Failed => {
                    events.push(ReputationEvent {
                        agent_id: dist.agent_id.clone(),
                        event_type: ReputationEventType::TaskFailed,
                        task_id: Some(outcome.task_id.clone()),
                        delta: -0.3,
                        tick,
                        evidence_id: outcome.evidence_id.clone(),
                        detail: format!("Task {} failed", outcome.task_id),
                    });
                }
                TaskOutcomeStatus::Disputed => {
                    // Smaller penalty
                    events.push(ReputationEvent {
                        agent_id: dist.agent_id.clone(),
                        event_type: ReputationEventType::TaskFailed,
                        task_id: Some(outcome.task_id.clone()),
                        delta: -0.15,
                        tick,
                        evidence_id: outcome.evidence_id.clone(),
                        detail: format!("Task {} disputed", outcome.task_id),
                    });
                }
            }
        }
        
        events
    }
}
