//! Agent Society Rules v0.1 — pure layer over Hub + Arena
//!
//! This crate introduces a coherent layer of social rules that affect agent decisions:
//! - Social relationship memory (worked_with, accepted, rejected, countered, successful, failed, trust_signal)
//! - Reputation/trust signals from verified results
//! - Refusal as valid behavior (not error)
//! - Counter-offer flexibility (price/workshare/deadline within task bounds)
//! - Team contribution: verified results affect final reward distribution
//! - Social memory: observable history
//! - Reputation changes through results, not simple cooperate
//! - Autonomy invariant: agents choose from real state, no hardcoded sequences
//!
//! Reuses: Hub (task market), Arena (agent world), Agents (reputation/economy), Compute (quota/evidence)


pub mod rules;
pub mod state;
pub mod mcp;
pub mod reputation;

pub use rules::{SocietyRules, DecisionContext, DecisionHint};
pub use state::{SocietyState, SocialRelationship, RelationshipKind, ContributionRecord, TaskOutcome, TaskOutcomeStatus, RewardDistribution, ShareBasis};
pub use state::{ReputationEvent, ReputationEventType};
pub use reputation::{SocialReputation, ReputationSignal, ReputationStore};
pub use mcp::{ToolDef, society_tools, society_state_request, society_trust_request, society_reputation_request, society_relationships_request, society_contributions_request, society_outcomes_request, society_decision_hints_request, build_society_state_response, build_trust_response, build_reputation_response, build_relationships_response, build_contributions_response, build_outcomes_response, build_decision_hints_response};

/// Re-exports for convenience
pub use decentraai_agent_hub::{HubState, HubTask, Bid, Proposal, ProposalStatus, Team, HubEvent, HubError, TaskStatus};
pub use decentraai_arena::{ArenaAgent, ActionKind};

/// Agent Society error types
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SocietyError {
    #[error("relationship not found")]
    RelationshipNotFound,
    #[error("invalid counter-offer: {0}")]
    InvalidCounterOffer(String),
    #[error("refusal not allowed in this context: {0}")]
    RefusalNotAllowed(String),
    #[error("contribution verification failed: {0}")]
    ContributionVerificationFailed(String),
    #[error("insufficient reputation for action: {0}")]
    InsufficientReputation(String),
}

/// Tick timestamp (unix ms)
pub type Tick = u64;

/// Agent identifier
pub type AgentId = String;

/// Task identifier  
pub type TaskId = String;

/// Proposal identifier
pub type ProposalId = String;

/// Team identifier
pub type TeamId = String;
