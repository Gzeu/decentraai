//! The agent record — identity + capabilities + policies of one logical agent.
//!
//! An `AgentRecord` is the unit the collective fabric reasons about. It is
//! deliberately node-agnostic: the record names *what the agent is allowed to
//! do* (semantic capabilities, models, tools) and *how it is governed*
//! (policies), while the physical capacity of the hosting node stays in the
//! node's own `ComputeAdvertisement` (see [`matcher`] for the composition).
//!
//! Roles are free-form strings with well-known constants (extensible by
//! design — the architecture explicitly forbids a fixed role list).

use decentraai_hub::capability::{CapabilityClaim, CapabilityKind, Provenance};
use serde::{Deserialize, Serialize};

use crate::tool::ToolDescriptor;

/// Well-known agent roles. Free-form by design: any string is a valid role.
pub const ROLE_GENERALIST: &str = "generalist";
pub const ROLE_SPECIALIST: &str = "specialist";
pub const ROLE_PLANNER: &str = "planner";
pub const ROLE_EXECUTOR: &str = "executor";
pub const ROLE_RESEARCHER: &str = "researcher";
pub const ROLE_CRITIC: &str = "critic";
pub const ROLE_VERIFIER: &str = "verifier";
pub const ROLE_COORDINATOR: &str = "coordinator";
pub const ROLE_MEMORY: &str = "memory";
pub const ROLE_ROUTER: &str = "router";
pub const ROLE_TOOL: &str = "tool";
pub const ROLE_INFRASTRUCTURE: &str = "infrastructure";

/// Lifecycle state of a logical agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// The agent exists but is not yet accepting work.
    #[default]
    Registered,
    /// The agent accepts work.
    Ready,
    /// The agent is executing a task.
    Busy,
    /// The agent is intentionally paused (policies/sandbox review).
    Suspended,
    /// The agent has been retired; records may linger for audit.
    Retired,
}

/// Sandbox mode for controlled exploration (§14 of the architecture).
///
/// More capability never grants more permission; the sandbox is a *hard*
/// boundary that scales with how much the operator lets the agent explore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Default: the agent is limited to its declared policies.
    Normal,
    /// The agent may test new models/tools/workflows inside a measured
    /// sandbox (audited, quota-capped).
    Exploration,
    /// The agent may run experimental combinations, local node only.
    Experimental,
}

/// Agent-level governance. Resource *budgets* are enforced by the fabric
/// (reservation + admission); these policies express the agent's own limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPolicies {
    /// Maximum concurrent tasks this agent may run (0 = node default).
    pub max_concurrent_tasks: u32,
    /// Sandbox mode (Normal | Exploration | Experimental).
    pub sandbox: SandboxMode,
    /// Whether remote peers may delegate work to this agent. This is the
    /// agent-level remote opt-in; the node-level `accepts_remote_inference`
    /// gate still applies at the fabric layer.
    pub allow_remote: bool,
}

impl Default for AgentPolicies {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 1,
            sandbox: SandboxMode::Normal,
            allow_remote: false,
        }
    }
}

/// One logical agent on a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRecord {
    /// Stable identifier unique within the node, e.g. `"dca-abc123:generalist"`.
    pub agent_id: String,
    /// Human-friendly name, e.g. `"Generalist"`.
    pub name: String,
    /// Free-form role (see `ROLE_*` constants).
    pub role: String,
    /// Short human-readable description of what the agent does.
    pub description: String,
    /// Semantic capability claims (hub taxonomy) with provenance.
    pub semantic_capabilities: Vec<CapabilityClaim>,
    /// Model hashes this agent may use (subset of the node's models).
    pub allowed_models: Vec<String>,
    /// Tools this agent exposes.
    pub tools: Vec<ToolDescriptor>,
    /// Names of memory scopes owned by this agent (future: collective memory).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_scopes: Vec<String>,
    /// Governance limits.
    #[serde(default)]
    pub policies: AgentPolicies,
    /// Lifecycle state.
    #[serde(default)]
    pub state: AgentState,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
}

impl AgentRecord {
    /// A minimal agent with the given id, name and role.
    pub fn new(agent_id: impl Into<String>, name: impl Into<String>, role: &str) -> Self {
        Self {
            agent_id: agent_id.into(),
            name: name.into(),
            role: role.to_string(),
            description: String::new(),
            semantic_capabilities: Vec::new(),
            allowed_models: Vec::new(),
            tools: Vec::new(),
            memory_scopes: Vec::new(),
            policies: AgentPolicies::default(),
            state: AgentState::Registered,
            created_at_ms: 0,
        }
    }

    /// Sets a human-readable description.
    pub fn described(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Adds one semantic capability claim.
    pub fn with_claim(mut self, claim: CapabilityClaim) -> Self {
        if !self.semantic_capabilities.contains(&claim) {
            self.semantic_capabilities.push(claim);
        }
        self
    }

    /// Adds a semantic capability with a given provenance.
    pub fn with_capability(self, capability: CapabilityKind, provenance: Provenance) -> Self {
        self.with_claim(CapabilityClaim {
            capability,
            provenance,
        })
    }

    /// Allows the agent to use a model (by hash).
    pub fn with_model(mut self, model_hash: impl Into<String>) -> Self {
        let hash = model_hash.into();
        if !self.allowed_models.contains(&hash) {
            self.allowed_models.push(hash);
        }
        self
    }

    /// Adds a tool the agent exposes.
    pub fn with_tool(mut self, tool: ToolDescriptor) -> Self {
        if !self.tools.iter().any(|t| t.name == tool.name) {
            self.tools.push(tool);
        }
        self
    }

    /// Sets governance policies.
    pub fn with_policies(mut self, policies: AgentPolicies) -> Self {
        self.policies = policies;
        self
    }

    /// Adds an owned memory scope name.
    pub fn with_memory_scope(mut self, scope: impl Into<String>) -> Self {
        let scope = scope.into();
        if !self.memory_scopes.contains(&scope) {
            self.memory_scopes.push(scope);
        }
        self
    }

    /// Sets the lifecycle state.
    pub fn set_state(&mut self, state: AgentState) {
        self.state = state;
    }

    /// Whether the agent claims a semantic capability (any provenance).
    pub fn has_capability(&self, capability: CapabilityKind) -> bool {
        self.semantic_capabilities
            .iter()
            .any(|c| c.capability == capability)
    }

    /// The strongest claim for a capability, if any.
    pub fn capability_claim(&self, capability: CapabilityKind) -> Option<&CapabilityClaim> {
        self.semantic_capabilities
            .iter()
            .find(|c| c.capability == capability)
    }

    /// Whether the agent is allowed to use the model with the given hash.
    pub fn has_model(&self, model_hash: &str) -> bool {
        self.allowed_models.iter().any(|m| m == model_hash)
    }

    /// Whether the agent exposes a tool with the given name.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name == name)
    }

    /// Whether the agent currently accepts work.
    pub fn can_accept_work(&self) -> bool {
        matches!(self.state, AgentState::Ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_hub::capability::CapabilityKind;

    fn record() -> AgentRecord {
        AgentRecord::new("dca-test:generalist", "Generalist", ROLE_GENERALIST)
            .described("handles general chat and reasoning")
            .with_capability(CapabilityKind::Chat, Provenance::Inferred)
            .with_capability(CapabilityKind::Reasoning, Provenance::Inferred)
            .with_model("abc123")
            .with_tool(ToolDescriptor::new("registry.lookup", crate::TOOL_KIND_BUILTIN))
            .with_memory_scope("generalist.notes")
    }

    #[test]
    fn record_round_trips_over_wire() {
        let rec = record();
        let json = serde_json::to_string(&rec).unwrap();
        let back: AgentRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(rec, back);
    }

    #[test]
    fn builder_deduplicates_claims_models_and_tools() {
        let rec = record()
            .with_capability(CapabilityKind::Chat, Provenance::Inferred)
            .with_model("abc123")
            .with_tool(ToolDescriptor::new("registry.lookup", crate::TOOL_KIND_BUILTIN));
        assert_eq!(rec.semantic_capabilities.len(), 2);
        assert_eq!(rec.allowed_models.len(), 1);
        assert_eq!(rec.tools.len(), 1);
    }

    #[test]
    fn capability_queries_are_honest() {
        let rec = record();
        assert!(rec.has_capability(CapabilityKind::Chat));
        assert!(!rec.has_capability(CapabilityKind::Ocr));
        assert!(rec.has_model("abc123"));
        assert!(!rec.has_model("nope"));
        assert!(rec.has_tool("registry.lookup"));
        assert!(!rec.has_tool("mcp.filesystem"));
        // New agents start Registered — not accepting work until the node
        // flips them to Ready.
        assert!(!rec.can_accept_work());
    }

    #[test]
    fn default_policies_are_conservative() {
        let rec = record();
        assert_eq!(rec.policies.max_concurrent_tasks, 1);
        assert_eq!(rec.policies.sandbox, SandboxMode::Normal);
        assert!(!rec.policies.allow_remote, "agents do not opt into remote use by default");
    }

    #[test]
    fn old_records_without_policies_deserialize_safely() {
        // Forward/backward compatibility: a record serialized before the
        // policies field existed must parse with defaults.
        let json = r#"{
            "agent_id":"a","name":"A","role":"generalist","description":"",
            "semantic_capabilities":[],"allowed_models":[],
            "tools":[],"created_at_ms":0
        }"#;
        let rec: AgentRecord = serde_json::from_str(json).unwrap();
        assert_eq!(rec.policies, AgentPolicies::default());
        assert_eq!(rec.state, AgentState::Registered);
    }
}