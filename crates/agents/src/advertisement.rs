//! Agent advertisement — the wire shape that tells the fabric which logical
//! agents a node hosts.
//!
//! This mirrors the fabric's `ComputeAdvertisement` (physical node facts)
//! but carries the *logical* layer: the node's agent records with their
//! semantic claims, model allowlists, tools and policies. The two
//! advertisements are complementary — a coordinator combines a peer's
//! compute advertisement (capacity) with its agent advertisement (agents)
//! through the unified matcher in [`crate::matcher`].
//!
//! On the wire the advertisement is wrapped in
//! `decentraai_protocol::SignedAgentAdvertisement` (opaque bytes +
//! signature), exactly like compute advertisements.

use libp2p::PeerId;
use serde::{Deserialize, Serialize};

use crate::agent::AgentRecord;
use crate::AGENT_ADVERTISEMENT_VERSION;

/// The set of logical agents a node advertises at one moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAdvertisement {
    /// Protocol version of the advertisement shape.
    pub protocol_version: u16,
    /// The hosting node's peer id.
    pub peer_id: PeerId,
    /// Human node name (same as the compute advertisement's).
    pub node_name: String,
    /// Wall-clock time of the advertisement (unix ms).
    pub announced_at_ms: u64,
    /// The node's logical agents.
    pub agents: Vec<AgentRecord>,
}

impl AgentAdvertisement {
    /// A new advertisement for the given node and agents.
    pub fn new(peer_id: PeerId, node_name: impl Into<String>, agents: Vec<AgentRecord>) -> Self {
        Self {
            protocol_version: AGENT_ADVERTISEMENT_VERSION,
            peer_id,
            node_name: node_name.into(),
            announced_at_ms: 0,
            agents,
        }
    }

    /// Sets the announcement timestamp (unix ms).
    pub fn announced_at(mut self, announced_at_ms: u64) -> Self {
        self.announced_at_ms = announced_at_ms;
        self
    }

    /// Number of agents in the advertisement.
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    /// Total semantic capability claims across all agents.
    pub fn total_capability_claims(&self) -> usize {
        self.agents
            .iter()
            .map(|a| a.semantic_capabilities.len())
            .sum()
    }

    /// Total tools across all agents.
    pub fn total_tools(&self) -> usize {
        self.agents.iter().map(|a| a.tools.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentRecord, ROLE_GENERALIST};
    use crate::tool::ToolDescriptor;
    use decentraai_hub::capability::{CapabilityKind, Provenance};
    use libp2p::identity::Keypair;

    fn peer() -> PeerId {
        PeerId::from(Keypair::generate_ed25519().public())
    }

    #[test]
    fn advertisement_round_trips_over_wire() {
        let adv = AgentAdvertisement::new(
            peer(),
            "node-1",
            vec![
                AgentRecord::new("n1:generalist", "Generalist", ROLE_GENERALIST)
                    .with_capability(CapabilityKind::Chat, Provenance::Inferred),
                AgentRecord::new("n1:ocr", "OCR", crate::agent::ROLE_SPECIALIST)
                    .with_capability(CapabilityKind::Ocr, Provenance::Verified)
                    .with_tool(ToolDescriptor::new("ocr.api", crate::TOOL_KIND_HTTP)),
            ],
        )
        .announced_at(1_700_000_000_000);

        let json = serde_json::to_string(&adv).unwrap();
        let back: AgentAdvertisement = serde_json::from_str(&json).unwrap();
        assert_eq!(adv, back);
        assert_eq!(back.agent_count(), 2);
        assert_eq!(back.total_capability_claims(), 2);
        assert_eq!(back.total_tools(), 1);
    }

    #[test]
    fn empty_node_advertises_no_agents() {
        let adv = AgentAdvertisement::new(peer(), "node-2", vec![]);
        assert_eq!(adv.agent_count(), 0);
        assert_eq!(adv.total_capability_claims(), 0);
    }
}