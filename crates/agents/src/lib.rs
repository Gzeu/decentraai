//! DecentraAI collective-intelligence agent model (P0 of the Collective
//! Intelligence foundation, see `docs/COLLECTIVE_INTELLIGENCE.md`).
//!
//! # What this crate is
//!
//! The pure, decision-only domain model for **logical agents**. An agent is
//! NOT a new process or a new node type: it is a *logical execution context*
//! that runs on an existing node. A node hosts one or more agents, and an
//! agent wraps three things the fabric already models separately:
//!
//! - a **semantic** capability set (the hub taxonomy: OCR, coding, vision…),
//! - an **execution** capability set (models the agent may use, tools it
//!   exposes),
//! - **policies** (concurrency budget, sandbox mode, remote opt-in).
//!
//! Unifying the two capability languages (physical `ComputeCapability` from
//! `decentraai-compute` and semantic `CapabilityKind`/`CapabilityClaim` from
//! `decentraai-hub`) is this crate's core job: the unified matcher
//! [`matcher::match_agent`] returns one compositional verdict (semantic +
//! physical) instead of two parallel ones.
//!
//! # Why pure (no I/O, no async)
//!
//! Following the `compute`/`fabric` pattern: every type is serde-serializable
//! so agent records and advertisements can travel over the P2P request/
//! response channel, and every decision is a pure function that unit tests
//! can drive with synthetic inputs. The stateful runtime half (remote agent
//! registry, signing, broadcasting) lives in `decentraai-distributed`
//! (`AgentManager`).
//!
//! # Scope of P0
//!
//! This crate defines the *shapes*: [`agent::AgentRecord`],
//! [`registry::AgentRegistry`], [`advertisement::AgentAdvertisement`],
//! [`task::AgentTask`] (generic task contract — routed in a later phase),
//! [`tool::ToolDescriptor`] and the unified matcher. It does NOT yet route
//! tasks or execute agents; that is the delegation phase (P3).

pub mod advertisement;
pub mod agent;
pub mod benchmark;
pub mod capability;
pub mod dataset;
pub mod decision;
pub mod delegation;
pub mod economy;
pub mod evidence;
pub mod knowledge;
pub mod matcher;
pub mod memory;
pub mod message;
pub mod policy;
pub mod registry;
pub mod reputation;
pub mod retrieval;
pub mod receipt;
pub mod signed_receipt;
pub mod selfopt;
pub mod talent_tree;
pub mod task;
pub mod tool;
pub mod verification;
pub mod workflow;

pub use advertisement::AgentAdvertisement;
pub use agent::{
    AgentPolicies, AgentRecord, AgentState, ROLE_COORDINATOR, ROLE_CRITIC, ROLE_EXECUTOR,
    ROLE_GENERALIST, ROLE_INFRASTRUCTURE, ROLE_MEMORY, ROLE_PLANNER, ROLE_RESEARCHER, ROLE_ROUTER,
    ROLE_SPECIALIST, ROLE_TOOL, ROLE_VERIFIER, SandboxMode,
};
pub use benchmark::{
    BenchmarkMode, BenchmarkRegistry, BenchmarkRun, BenchmarkTask, BenchmarkVerdict, MIN_MARGIN,
    MIN_SAMPLES, ModeAggregate, ModeComparison, RunMetrics, aggregate, compare_modes,
    grade_answer, normalize_answer,
};
pub use capability::{AgentCapability, model_capabilities_from_claims};
pub use dataset::{
    CapabilityBuild, DatasetDescriptor, DatasetError, DatasetKind, SkillDescriptor, SkillRegistry,
    build_agent_capabilities, demo_skill_registry,
};
pub use decision::{
    CollectiveDecision, ConsideredObject, DecisionError, DecisionRegistry, DecisionVerdict,
    KnowledgeVote, decide_collectively, decision_feedback_entry,
};
pub use delegation::{
    DelegationError, DelegationPlan, DelegationPlanner, DelegationResult, DelegationStage,
    DelegationVerdict, StageAssignment, StageResult, execute_plan,
};
pub use economy::{
    BookingRequest, BookingVerdict, CapabilityOffer, EconomyError, EconomyLedger, MAX_OFFERS,
    OfferStatus, negotiate,
};
pub use evidence::{
    EvidenceEntry, EvidenceFamily, EvidenceHit, EvidenceIndex, EvidenceSummary, Lesson, lessons,
};
pub use knowledge::{
    Evidence, EvidenceKind, KnowledgeConfidence, KnowledgeError, KnowledgeObject, KnowledgeRegistry,
    WEIGHT_CONSENSUS, WEIGHT_DIRECT_OBSERVATION, WEIGHT_REPUTATION, WEIGHT_SYNTHETIC,
    WEIGHT_VERIFIED_EXECUTION, evidence_confidence,
};
pub use matcher::{
    AgentMatchOutcome, AgentMatchReason, AgentRequirement, match_agent, match_agent_semantic,
};
pub use memory::{
    MemoryAccess, MemoryAccessDecision, MemoryEntry, MemoryError, MemoryLevel, MemoryPolicy,
    MemoryRegistry, MemoryScope, can_read, can_write, enforce_retention, entry_expired,
};
pub use message::{
    AgentInbox, AgentMessage, MessageKind, MessageValidationError, validate_message,
};
pub use policy::{ExplorationLimit, Permission, PolicyDecision, PolicyEngine, policy_engine};
pub use receipt::{
    ReceiptError, ReceiptRegistry, ReceiptVerdict, VerifiedComputeReceipt,
};
pub use registry::{AgentRegistry, AgentRegistryError};
pub use reputation::{
    AgentReputation, DEFAULT_MIN_SAMPLES, FactorScore, ReputationFactor, ReputationStore,
    ReputationUpdate, default_weights, safety_penalty,
};
pub use retrieval::{IndexedDocument, RetrievalIndex, RetrievalResult, cosine_similarity};
pub use selfopt::{
    Constraint, ConstraintKind, Direction, OptimizationDimension, OptimizationObservation,
    OptimizationSuggestion, RiskLevel, SelfOptimizer,
};
pub use talent_tree::{TalentError, TalentNode, TalentTree, seed_talent_tree};
pub use task::{AgentTask, AgentWorkloadRequirement, TaskVerification};
pub use tool::{
    TOOL_KIND_BUILTIN, TOOL_KIND_CUSTOM, TOOL_KIND_HTTP, TOOL_KIND_MCP, ToolDescriptor,
};
pub use verification::{
    CheckKind, ConsensusPolicy, ConsensusResult, DisagreementResolution, VerificationCheck,
    VerificationError, VerificationLedger, VerificationReport, VerificationVerdict,
    check_output_schema, clamped_confidence, evaluate_consensus, resolve_disagreement,
};
pub use workflow::{
    WorkflowError, WorkflowOutcome, WorkflowStep, WorkflowTemplate, research_report_template,
    run_workflow,
};

/// Current protocol version carried by agent advertisements.
pub const AGENT_ADVERTISEMENT_VERSION: u16 = 1;
