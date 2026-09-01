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

pub mod mcp;
pub mod reputation;
pub mod rules;
pub mod state;

pub use mcp::{
    ToolDef, build_contributions_response, build_decision_hints_response, build_outcomes_response,
    build_relationships_response, build_reputation_response, build_society_state_response,
    build_trust_response, society_contributions_request, society_decision_hints_request,
    society_outcomes_request, society_relationships_request, society_reputation_request,
    society_state_request, society_tools, society_trust_request,
};
pub use reputation::{ReputationSignal, ReputationStore, SocialReputation};
pub use rules::{DecisionContext, DecisionHint, SocietyRules};
pub use state::{
    ContributionRecord, RelationshipKind, RewardDistribution, ShareBasis, SocialRelationship,
    SocietyState, TaskOutcome, TaskOutcomeStatus,
};
pub use state::{ReputationEvent, ReputationEventType};

/// Re-exports for convenience
pub use decentraai_agent_hub::{
    Bid, HubError, HubEvent, HubState, HubTask, Proposal, ProposalStatus, TaskStatus, Team,
};
pub use decentraai_arena::{ActionKind, ArenaAgent};

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
