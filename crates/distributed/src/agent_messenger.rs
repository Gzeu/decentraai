//! AgentMessenger — the P2P transport half of agent-to-agent messaging (P2).
//!
//! The pure message model lives in `decentraai-agents` (`AgentMessage`,
//! `AgentInbox`); this module bridges it to the existing libp2p
//! request/response channel:
//!
//! - `send(peer, msg)` — delivers an `AgentMessage` to a specific peer and
//!   awaits the transport-level acknowledgement (the Noise-authenticated
//!   connection establishes *who* sent it; the message's `nonce` guards
//!   replay at the application layer).
//! - inbound messages land in a per-recipient bounded [`AgentInbox`] that
//!   the host node drains for its local agents.
//!
//! This is the substrate P3 delegation uses to hand sub-tasks to remote
//! agents (`AgentMessage::Delegate`).

use anyhow::{Context, Result};
use decentraai_agents::{AgentInbox, AgentMessage};
use decentraai_p2p::P2PNode;
use decentraai_protocol::{deserialize_message, serialize_message};
use libp2p::PeerId;
use std::sync::{Arc, Mutex};

/// Sends agent messages and holds the inbound inbox.
#[derive(Clone)]
pub struct AgentMessenger {
    /// The P2P transport. Interior-mutable because construction is circular
    /// (the handler needs the messenger, the messenger needs the node): the
    /// messenger starts on a placeholder node and is re-pointed at the real,
    /// handler-bearing node via [`AgentMessenger::set_transport`] after it
    /// exists.
    p2p: Arc<Mutex<P2PNode>>,
    inbox: Arc<Mutex<AgentInbox>>,
}

impl AgentMessenger {
    /// Wraps the node's P2P transport with a per-recipient bounded inbox.
    pub fn new(p2p: P2PNode) -> Self {
        Self {
            p2p: Arc::new(Mutex::new(p2p)),
            inbox: Arc::new(Mutex::new(AgentInbox::new(64))),
        }
    }

    /// Points the messenger at the transport that carries its handler.
    pub fn set_transport(&self, p2p: P2PNode) {
        *self.p2p.lock().unwrap() = p2p;
    }

    /// Delivers a message to a peer over the transport. Returns `Ok(())`
    /// once the receiving node has *accepted* the frame (transport-level
    /// acknowledgement — not agent-level processing; agents drain their
    /// inbox asynchronously).
    pub async fn send(&self, peer: PeerId, message: AgentMessage) -> Result<()> {
        decentraai_agents::validate_message(&message)
            .context("refusing to send an invalid agent message")?;
        let bytes = serialize_message(&message)?;
        let p2p = self.p2p.lock().unwrap().clone();
        p2p.request(peer, bytes).await.context("agent message transport")?;
        Ok(())
    }

    /// Records an inbound message into the recipient's inbox. `false` when
    /// the recipient's inbox is full (the frame is dropped — the transport
    /// never blocks or grows unbounded).
    pub fn push_inbound(&self, message: AgentMessage) -> bool {
        let to = message.to_agent.clone();
        self.inbox.lock().unwrap().push(&to, message)
    }

    /// Pops the oldest pending message for an agent (FIFO).
    pub fn pop(&self, agent_id: &str) -> Option<AgentMessage> {
        self.inbox.lock().unwrap().pop(agent_id)
    }

    /// Number of pending messages for an agent.
    pub fn pending(&self, agent_id: &str) -> usize {
        self.inbox.lock().unwrap().pending(agent_id)
    }

    /// Whether any message is pending for an agent.
    pub fn has_pending(&self, agent_id: &str) -> bool {
        self.pending(agent_id) > 0
    }
}

/// Parses an inbound frame as an `AgentMessage` (used by the P2P handler).
pub fn parse_agent_message(bytes: &[u8]) -> Result<AgentMessage> {
    deserialize_message(bytes, decentraai_p2p::DEFAULT_MAX_MESSAGE_BYTES)
        .map_err(|e| anyhow::anyhow!("not an agent message: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_agents::MessageKind;
    use decentraai_identity::Identity;

    fn msg(from: &str, to: &str) -> AgentMessage {
        AgentMessage::new("m-1", from, to, MessageKind::Ask)
            .with_nonce(1)
            .with_created_at_ms(1_700_000_000_000)
    }

    fn dead_node() -> P2PNode {
        P2PNode::new(
            &Identity::generate(),
            decentraai_p2p::DEFAULT_MAX_MESSAGE_BYTES,
            decentraai_p2p::DEFAULT_MAX_CHUNK_MESSAGE_BYTES,
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn inbox_push_pop_pending() {
        let messenger = AgentMessenger::new(dead_node());
        assert!(messenger.push_inbound(msg("a:1", "b:1")));
        assert!(messenger.push_inbound(msg("a:2", "b:1")));
        assert_eq!(messenger.pending("b:1"), 2);
        assert!(messenger.has_pending("b:1"));
        let first = messenger.pop("b:1").unwrap();
        assert_eq!(first.from_agent, "a:1");
        assert_eq!(messenger.pending("b:1"), 1);
        assert!(!messenger.has_pending("nobody"));
    }

    #[tokio::test]
    async fn inbox_recipient_isolation() {
        let messenger = AgentMessenger::new(dead_node());
        messenger.push_inbound(msg("a:1", "b:1"));
        messenger.push_inbound(msg("a:2", "c:1"));
        assert_eq!(messenger.pending("b:1"), 1);
        assert_eq!(messenger.pending("c:1"), 1);
        // Popping b does not touch c.
        messenger.pop("b:1");
        assert_eq!(messenger.pending("b:1"), 0);
        assert_eq!(messenger.pending("c:1"), 1);
    }

    #[tokio::test]
    async fn inbox_bound_is_capped() {
        let messenger = AgentMessenger::new(dead_node());
        for i in 0..64 {
            assert!(
                messenger.push_inbound(msg(&format!("a:{i}"), "b:1")),
                "queue must accept up to its capacity"
            );
        }
        assert!(
            !messenger.push_inbound(msg("a:65", "b:1")),
            "overflow is dropped, never grown"
        );
        assert_eq!(messenger.pending("b:1"), 64);
    }

    #[test]
    fn parse_agent_message_round_trips() {
        let m = msg("a:1", "b:1");
        let bytes = serialize_message(&m).unwrap();
        let back = parse_agent_message(&bytes).unwrap();
        assert_eq!(back.message_id, "m-1");
        assert_eq!(back.kind, MessageKind::Ask);
        assert!(parse_agent_message(b"not a message").is_err());
    }
}