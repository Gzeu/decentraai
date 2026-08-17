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
pub mod capability;
pub mod matcher;
pub mod registry;
pub mod task;
pub mod tool;

pub use advertisement::AgentAdvertisement;
pub use agent::{
    AgentPolicies, AgentRecord, AgentState, ROLE_COORDINATOR, ROLE_CRITIC, ROLE_EXECUTOR,
    ROLE_GENERALIST, ROLE_INFRASTRUCTURE, ROLE_MEMORY, ROLE_PLANNER, ROLE_RESEARCHER,
    ROLE_ROUTER, ROLE_SPECIALIST, ROLE_TOOL, ROLE_VERIFIER, SandboxMode,
};
pub use capability::{AgentCapability, model_capabilities_from_claims};
pub use matcher::{
    AgentMatchOutcome, AgentMatchReason, AgentRequirement, match_agent, match_agent_semantic,
};
pub use registry::{AgentRegistry, AgentRegistryError};
pub use task::{AgentTask, AgentWorkloadRequirement, TaskVerification};
pub use tool::{
    ToolDescriptor, TOOL_KIND_BUILTIN, TOOL_KIND_CUSTOM, TOOL_KIND_HTTP, TOOL_KIND_MCP,
};

/// Current protocol version carried by agent advertisements.
pub const AGENT_ADVERTISEMENT_VERSION: u16 = 1;