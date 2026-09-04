//! Agent Proposal & Experiment Protocol v0.1 — DECOUPLED FIRST.
//!
//! The cognitive chain this crate implements (pure, deterministic, no I/O):
//!
//! ```text
//! Observation → Question → Hypothesis → AgentIdea → ExperimentProposal
//!   → Policy (Sandbox Policy, deterministic)
//!   → Sandbox Execution (Sandbox / ReadOnly only)
//!   → Evidence (hash-sealed, chained)
//!   → Outcome → Learning (derived from evidence, never invented)
//! ```
//!
//! # v0.1 hard gates (enforced by construction, tested per rule)
//!
//! - Only [`risk::ExperimentRiskClass::Sandbox`] and `ReadOnly` proposals can
//!   be allowed. `Economic` risk is denied deterministically.
//! - Only [`risk::ResourceCommitment::None`] is usable. Any commitment of Cr,
//!   DCAI or escrow is denied — and the denial flows through the
//!   [`economic::EconomicAuthorization`] seam, which in v0.1 has exactly one
//!   implementation: [`economic::DenyAllEconomicAuthorization`].
//! - Economic actions ([`action::ProposedAction`] variants
//!   `EconomicStateMutation`, `FundTransfer`, `SignerChange`, `DCAIMint`) are
//!   denied by policy AND rejected again at the executor boundary
//!   (defense in depth: unreachable code path, tested).
//! - The live economy is a *future adapter*, never part of the cognitive
//!   core: no Cr, no M18 escrow, no DCAI, no transfers, no mint/burn, no
//!   settlement, no operator funds anywhere in this crate.
//!
//! # Separation (explicit modules, no layering violations)
//!
//! 1. Cognitive protocol — [`protocol`]
//! 2. Experiment execution — [`sandbox`]
//! 3. Evidence — [`evidence`]
//! 4. Learning — [`learning`]
//! 5. Economic authorization (future seam, deny-all in v0.1) — [`economic`]
//!
//! Risk/commitment taxonomy lives in [`risk`], the action vocabulary in
//! [`action`], the deterministic gate in [`policy`].
//!
//! # Untrusted input
//!
//! Proposals arrive as JSON from AI output and are therefore untrusted:
//! [`protocol::parse_proposal`] enforces a closed schema
//! (`deny_unknown_fields`) plus hard bounds before anything else runs.

pub mod action;
pub mod budget;
pub mod curiosity;
pub mod economic;
pub mod error;
pub mod evidence;
pub mod learning;
pub mod policy;
pub mod protocol;
pub mod risk;
pub mod sandbox;
pub mod selection;
pub mod store;
pub mod testnet;

pub use action::ProposedAction;
pub use budget::{
    DCAI_TESTNET_ID, ExperimentBudget, MAX_BUDGET_ACTIONS, MAX_BUDGET_RETRIES, MAX_BUDGET_WEI,
    MAX_GAS_LIMIT, TestnetAsset,
};
pub use curiosity::{CuriosityState, HypothesisBelief};
pub use economic::{
    DenyAllEconomicAuthorization, EconomicAuthError, EconomicAuthorization, TestnetApproval,
    TestnetAuthConfig, TestnetAuthRequest, TestnetEconomicAuthorization,
};
pub use error::ProposalError;
pub use evidence::{EvidenceLog, ExperimentEvidence, ExperimentOutcome, TestnetEvidence};
pub use learning::{
    ExperimentLearning, HypothesisVerdict, LearningEntry, assess, derive_learnings,
};
pub use policy::{DenyReason, ExecutionMode, PolicyDecision, decide};
pub use protocol::{
    AgentIdea, ExperimentProposal, ExperimentStep, Hypothesis, Observation, PROTOCOL_VERSION,
    ResearchQuestion, parse_proposal,
};
pub use risk::{ExperimentRiskClass, ResourceCommitment};
pub use sandbox::{ExecutionReport, StepResult, execute};
pub use selection::{
    CandidateExperiment, CandidateRejection, CycleState, MIN_EXECUTABLE_SCORE, ScoreBreakdown,
    ScoredCandidate, ScoredSummary, SelectionRecord, TESTNET_RISK_PENALTY, action_signature,
    generate_candidates, novelty_bp, score_candidate, select_experiment,
};
pub use store::{AttemptInfo, ExperimentRecord, ExperimentStatus, ExperimentStore};
pub use testnet::{
    AuthorizedTransfer, TestnetExecutor, TestnetReport, execute_testnet_experiment, transfer_totals,
};
