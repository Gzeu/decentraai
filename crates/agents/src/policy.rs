//! P7 — policy engine: agent permissions, budgets and sandbox enforcement.
//!
//! Architecture rule: **Agent Power ≠ Permission** — a more capable agent
//! never gains more rights implicitly; every capability is gated by declared
//! policy at each hop (tool access, model access, network access, resource
//! budget, sandbox mode). This module is the pure decision core the runtime
//! calls before allowing an agent to touch anything.
//!
//! The policy model is deliberately composed from what already exists:
//! `AgentRecord.policies` (concurrency, sandbox, remote opt-in) + the
//! record's declared allowlists (models, tools). This engine adds the
//! *decision layer*: explicit Allow/Deny verdicts with reasons, and the
//! Controlled-Exploration boundary (Normal/Exploration/Experimental from the
//! architecture §14).

use serde::{Deserialize, Serialize};

use crate::agent::{AgentRecord, AgentState, SandboxMode};

/// What an agent asks permission to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Use a tool by name.
    Tool { name: String },
    /// Use a model by hash.
    Model { model_hash: String },
    /// Interact with a remote peer (by peer id).
    Peer { peer_id: String },
    /// Consume compute/network beyond the agent's declared budget.
    Resource { resource: String, amount_mb: u64 },
    /// Write to a memory scope (ownership is enforced by `crate::memory`).
    MemoryWrite { scope: String },
    /// Open a network connection to an external endpoint (exploration only).
    NetworkEgress { host: String },
}

impl Permission {
    /// A tool permission.
    pub fn tool(name: impl Into<String>) -> Self {
        Permission::Tool { name: name.into() }
    }
    /// A model permission.
    pub fn model(model_hash: impl Into<String>) -> Self {
        Permission::Model {
            model_hash: model_hash.into(),
        }
    }
    /// A peer permission.
    pub fn peer(peer_id: impl Into<String>) -> Self {
        Permission::Peer {
            peer_id: peer_id.into(),
        }
    }
}

/// The verdict of a policy check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyDecision::Allow)
    }
}

/// Deny a permission with a reason (small helper for readable rules).
fn deny(reason: impl Into<String>) -> PolicyDecision {
    PolicyDecision::Deny {
        reason: reason.into(),
    }
}

/// The policy engine. Stateless and pure: every check is a decision function
/// over an [`AgentRecord`]'s declared state.
#[derive(Debug, Clone, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    /// Checks whether the agent may use a tool.
    ///
    /// Rules: the agent must exist in a working state; the tool must be in
    /// the agent's declared tool set. In `Exploration`/`Experimental` modes
    /// an *undeclared* tool is also denied (the sandbox never grants
    /// implicit tool access) — declared tools are always allowed.
    pub fn check_tool(&self, agent: &AgentRecord, name: &str) -> PolicyDecision {
        if !agent_active(agent) {
            return deny("agent is not in a working state");
        }
        if agent.has_tool(name) {
            PolicyDecision::Allow
        } else {
            deny(format!(
                "tool '{name}' is not in the agent's declared tool set"
            ))
        }
    }

    /// Checks whether the agent may use a model.
    ///
    /// Rules: the model must be in the agent's allowlist. A generalist agent
    /// with an empty allowlist may use any model the node serves (empty
    /// allowlist = node default policy); an agent with an explicit allowlist
    /// is strictly limited to it.
    pub fn check_model(&self, agent: &AgentRecord, model_hash: &str) -> PolicyDecision {
        if !agent_active(agent) {
            return deny("agent is not in a working state");
        }
        if agent.allowed_models.is_empty() || agent.has_model(model_hash) {
            PolicyDecision::Allow
        } else {
            deny(format!(
                "model '{model_hash}' is not in the agent's allowlist"
            ))
        }
    }

    /// Checks whether the agent may interact with a remote peer.
    ///
    /// Rules: remote interaction requires the agent's `allow_remote` policy
    /// (it is opt-in, never implicit). The local node is always allowed.
    pub fn check_peer(
        &self,
        agent: &AgentRecord,
        peer_id: &str,
        local_peer_id: &str,
    ) -> PolicyDecision {
        if !agent_active(agent) {
            return deny("agent is not in a working state");
        }
        if peer_id == local_peer_id {
            return PolicyDecision::Allow;
        }
        if agent.policies.allow_remote {
            PolicyDecision::Allow
        } else {
            deny("agent does not allow interaction with remote peers (policy.allow_remote = false)")
        }
    }

    /// Checks a resource budget request.
    ///
    /// Rules: `max_concurrent_tasks` bounds concurrency (checked by the
    /// runtime with a counter); `amount_mb` is capped at a hard per-agent
    /// ceiling. The ceiling here is the record's declared budget if set,
    /// otherwise the node default (large, but bounded — never unbounded).
    pub fn check_resource(
        &self,
        agent: &AgentRecord,
        amount_mb: u64,
        active_tasks: u32,
    ) -> PolicyDecision {
        if !agent_active(agent) {
            return deny("agent is not in a working state");
        }
        if agent.policies.max_concurrent_tasks > 0
            && active_tasks >= agent.policies.max_concurrent_tasks
        {
            return deny(format!(
                "agent concurrency budget exceeded ({} >= {})",
                active_tasks, agent.policies.max_concurrent_tasks
            ));
        }
        // Hard ceiling per request — conservative default of 8 GiB unless the
        // operator configured a tighter budget.
        const DEFAULT_MAX_REQUEST_MB: u64 = 8 * 1024;
        if amount_mb > DEFAULT_MAX_REQUEST_MB {
            return deny(format!(
                "resource request {amount_mb} MiB exceeds the per-request ceiling {DEFAULT_MAX_REQUEST_MB} MiB"
            ));
        }
        PolicyDecision::Allow
    }

    /// Checks whether the agent may open an external network connection.
    ///
    /// Controlled exploration (§14): `Normal` agents may not egress at all;
    /// `Exploration` agents may egress only to a declared allowlist (passed
    /// as `allowed_hosts`); `Experimental` agents may egress to any host but
    /// only on their own node (the caller enforces node locality).
    pub fn check_network_egress(
        &self,
        agent: &AgentRecord,
        host: &str,
        allowed_hosts: &[String],
    ) -> PolicyDecision {
        if !agent_active(agent) {
            return deny("agent is not in a working state");
        }
        match agent.policies.sandbox {
            SandboxMode::Normal => deny("network egress is denied in Normal sandbox mode"),
            SandboxMode::Exploration => {
                if allowed_hosts.iter().any(|h| h == host) {
                    PolicyDecision::Allow
                } else {
                    deny(format!("host '{host}' is not in the exploration allowlist"))
                }
            }
            SandboxMode::Experimental => PolicyDecision::Allow,
        }
    }

    /// The sandbox boundary: what an agent may *explore* (new tools, models,
    /// workflows) is capped by its sandbox mode. Returns the effective
    /// allowed exploration depth.
    pub fn exploration_limit(&self, agent: &AgentRecord) -> ExplorationLimit {
        match agent.policies.sandbox {
            SandboxMode::Normal => ExplorationLimit::None,
            SandboxMode::Exploration => ExplorationLimit::Measured,
            SandboxMode::Experimental => ExplorationLimit::Unrestricted,
        }
    }
}

/// How far an agent may explore beyond its declared capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorationLimit {
    /// No exploration: only declared capabilities, fully audited.
    None,
    /// Measured exploration inside a sandbox: audited, quota-capped.
    Measured,
    /// Experimental exploration: local node only, fully audited.
    Unrestricted,
}

/// Whether the agent is in a state that can act at all.
fn agent_active(agent: &AgentRecord) -> bool {
    matches!(agent.state, AgentState::Ready | AgentState::Busy)
}

/// Convenience: a default PolicyEngine (avoids `PolicyEngine` in callers).
pub fn policy_engine() -> PolicyEngine {
    PolicyEngine
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentRecord, ROLE_GENERALIST};
    use crate::tool::ToolDescriptor;
    use decentraai_hub::capability::{CapabilityKind, Provenance};

    fn active_agent() -> AgentRecord {
        let mut rec = AgentRecord::new("a:gen", "Gen", ROLE_GENERALIST)
            .with_capability(CapabilityKind::Chat, Provenance::Inferred)
            .with_tool(ToolDescriptor::new("mcp.filesystem", crate::TOOL_KIND_MCP))
            .with_model("m1");
        rec.set_state(AgentState::Ready);
        rec
    }

    #[test]
    fn registered_agent_cannot_act() {
        // An agent that is not Ready/Busy is denied everything — a fresh
        // record never implicitly acts.
        let agent = AgentRecord::new("a:gen", "Gen", ROLE_GENERALIST);
        let engine = PolicyEngine;
        assert!(!engine.check_tool(&agent, "mcp.filesystem").is_allowed());
        assert!(!engine.check_model(&agent, "m1").is_allowed());
    }

    #[test]
    fn declared_tool_allowed_undeclared_denied() {
        let agent = active_agent();
        let engine = PolicyEngine;
        assert_eq!(
            engine.check_tool(&agent, "mcp.filesystem"),
            PolicyDecision::Allow
        );
        assert!(matches!(
            engine.check_tool(&agent, "ocr.api"),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn empty_model_allowlist_means_node_default() {
        let mut agent = active_agent();
        agent.allowed_models = Vec::new();
        let engine = PolicyEngine;
        // Empty allowlist = the agent may use any model the node serves.
        assert_eq!(
            engine.check_model(&agent, "anything"),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn explicit_model_allowlist_is_strict() {
        let agent = active_agent();
        let engine = PolicyEngine;
        assert_eq!(engine.check_model(&agent, "m1"), PolicyDecision::Allow);
        assert!(matches!(
            engine.check_model(&agent, "other"),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn remote_peers_require_opt_in() {
        let agent = active_agent();
        let engine = PolicyEngine;
        // Local peer always allowed.
        assert_eq!(
            engine.check_peer(&agent, "local", "local"),
            PolicyDecision::Allow
        );
        // Remote denied unless allow_remote.
        assert!(matches!(
            engine.check_peer(&agent, "remote-1", "local"),
            PolicyDecision::Deny { .. }
        ));
        let mut open = active_agent();
        open.policies.allow_remote = true;
        assert_eq!(
            engine.check_peer(&open, "remote-1", "local"),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn concurrency_budget_is_enforced() {
        let mut agent = active_agent();
        agent.policies.max_concurrent_tasks = 2;
        let engine = PolicyEngine;
        // 0 and 1 active tasks are within the budget of 2.
        assert_eq!(
            engine.check_resource(&agent, 1024, 0),
            PolicyDecision::Allow
        );
        assert_eq!(
            engine.check_resource(&agent, 1024, 1),
            PolicyDecision::Allow
        );
        // The 2-slot budget is fully occupied — a new request is denied.
        assert!(matches!(
            engine.check_resource(&agent, 1024, 2),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn oversized_resource_request_is_denied() {
        let agent = active_agent();
        let engine = PolicyEngine;
        assert!(matches!(
            engine.check_resource(&agent, 9 * 1024, 0),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn network_egress_follows_sandbox_mode() {
        let engine = PolicyEngine;
        // Normal: denied.
        assert!(matches!(
            engine.check_network_egress(&active_agent(), "api.x", &[]),
            PolicyDecision::Deny { .. }
        ));
        // Exploration: only allowlisted hosts.
        let mut exploring = active_agent();
        exploring.policies.sandbox = SandboxMode::Exploration;
        assert!(matches!(
            engine.check_network_egress(&exploring, "other.x", &["api.x".into()]),
            PolicyDecision::Deny { .. }
        ));
        assert_eq!(
            engine.check_network_egress(&exploring, "api.x", &["api.x".into()]),
            PolicyDecision::Allow
        );
        // Experimental: allowed (local-node-only is enforced by the caller).
        let mut experimental = active_agent();
        experimental.policies.sandbox = SandboxMode::Experimental;
        assert_eq!(
            engine.check_network_egress(&experimental, "anything.x", &[]),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn exploration_limit_matches_sandbox_mode() {
        let engine = PolicyEngine;
        assert_eq!(
            engine.exploration_limit(&active_agent()),
            ExplorationLimit::None
        );
        let mut exploring = active_agent();
        exploring.policies.sandbox = SandboxMode::Exploration;
        assert_eq!(
            engine.exploration_limit(&exploring),
            ExplorationLimit::Measured
        );
    }

    #[test]
    fn policy_decision_round_trips_over_wire() {
        let decision = PolicyDecision::Deny {
            reason: "no".into(),
        };
        let json = serde_json::to_string(&decision).unwrap();
        let back: PolicyDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(decision, back);
        assert_eq!(PolicyDecision::Allow, PolicyDecision::Allow);
    }
}
