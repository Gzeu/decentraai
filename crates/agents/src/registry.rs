//! Agent registry — the local, pure bookkeeping of logical agents on a node.
//!
//! One node hosts many logical agents; this registry tracks them
//! deterministically (sorted by id). It is the local half of the collective
//! picture: remote agents live in the runtime `AgentManager`
//! (`decentraai-distributed`), which feeds this same shape to the dashboard.

use decentraai_hub::capability::CapabilityKind;
use std::collections::BTreeMap;
use std::fmt;

use crate::agent::AgentRecord;

/// Registry errors — all recoverable and explainable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentRegistryError {
    /// An agent with the same id is already registered.
    DuplicateAgentId { agent_id: String },
}

impl fmt::Display for AgentRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentRegistryError::DuplicateAgentId { agent_id } => {
                write!(f, "agent '{agent_id}' is already registered")
            }
        }
    }
}

impl std::error::Error for AgentRegistryError {}

/// A deterministic registry of local agents, keyed by `agent_id`.
#[derive(Debug, Clone, Default)]
pub struct AgentRegistry {
    agents: BTreeMap<String, AgentRecord>,
}

impl AgentRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an agent. Fails on duplicate ids so callers notice
    /// collisions instead of silently overwriting an existing agent.
    pub fn register(&mut self, record: AgentRecord) -> Result<(), AgentRegistryError> {
        let id = record.agent_id.clone();
        if self.agents.contains_key(&id) {
            return Err(AgentRegistryError::DuplicateAgentId { agent_id: id });
        }
        self.agents.insert(id, record);
        Ok(())
    }

    /// Registers an agent, replacing any existing agent with the same id
    /// (used by the runtime to refresh an agent's state).
    pub fn register_or_replace(&mut self, record: AgentRecord) {
        self.agents.insert(record.agent_id.clone(), record);
    }

    /// Removes an agent; returns whether it existed.
    pub fn unregister(&mut self, agent_id: &str) -> bool {
        self.agents.remove(agent_id).is_some()
    }

    /// Looks up an agent by id.
    pub fn get(&self, agent_id: &str) -> Option<&AgentRecord> {
        self.agents.get(agent_id)
    }

    /// All agents, sorted by id (deterministic).
    pub fn list(&self) -> Vec<AgentRecord> {
        self.agents.values().cloned().collect()
    }

    /// Number of registered agents.
    pub fn count(&self) -> usize {
        self.agents.len()
    }

    /// Agents that claim a given semantic capability (any provenance).
    pub fn with_capability(&self, capability: CapabilityKind) -> Vec<AgentRecord> {
        self.list()
            .into_iter()
            .filter(|a| a.has_capability(capability))
            .collect()
    }

    /// Agents with the given role.
    pub fn by_role(&self, role: &str) -> Vec<AgentRecord> {
        self.list().into_iter().filter(|a| a.role == role).collect()
    }

    /// Agents that are ready to accept work.
    pub fn ready(&self) -> Vec<AgentRecord> {
        self.list()
            .into_iter()
            .filter(|a| a.can_accept_work())
            .collect()
    }

    /// Whether the registry holds no agents.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentRecord, ROLE_GENERALIST};
    use decentraai_hub::capability::{CapabilityKind, Provenance};

    fn agent(id: &str) -> AgentRecord {
        AgentRecord::new(id, id, ROLE_GENERALIST)
            .with_capability(CapabilityKind::Chat, Provenance::Inferred)
    }

    #[test]
    fn register_list_and_lookup() {
        let mut reg = AgentRegistry::new();
        assert!(reg.is_empty());
        reg.register(agent("a:1")).unwrap();
        reg.register(agent("a:2")).unwrap();
        assert_eq!(reg.count(), 2);
        assert_eq!(reg.list().len(), 2);
        assert!(reg.get("a:1").is_some());
        assert!(reg.get("nope").is_none());
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let mut reg = AgentRegistry::new();
        reg.register(agent("a:1")).unwrap();
        let err = reg.register(agent("a:1")).unwrap_err();
        assert_eq!(
            err,
            AgentRegistryError::DuplicateAgentId {
                agent_id: "a:1".into()
            }
        );
    }

    #[test]
    fn replace_and_unregister() {
        let mut reg = AgentRegistry::new();
        reg.register(agent("a:1")).unwrap();
        reg.register_or_replace(agent("a:1").described("v2"));
        assert_eq!(reg.get("a:1").unwrap().description, "v2");
        assert!(reg.unregister("a:1"));
        assert!(!reg.unregister("a:1"));
        assert!(reg.is_empty());
    }

    #[test]
    fn filters_are_deterministic() {
        let mut reg = AgentRegistry::new();
        let mut ocr = agent("b:ocr");
        ocr.role = crate::agent::ROLE_SPECIALIST.into();
        ocr = ocr.with_capability(CapabilityKind::Ocr, Provenance::Verified);
        reg.register(ocr).unwrap();
        reg.register(agent("a:chat")).unwrap();
        let ocr_agents = reg.with_capability(CapabilityKind::Ocr);
        assert_eq!(ocr_agents.len(), 1);
        assert_eq!(ocr_agents[0].agent_id, "b:ocr");
        let specialists = reg.by_role(crate::agent::ROLE_SPECIALIST);
        assert_eq!(specialists.len(), 1);
        // list() is sorted by id
        assert_eq!(reg.list()[0].agent_id, "a:chat");
    }
}
