//! P2 agent messaging — the wire/message layer for agent-to-agent
//! communication (see `docs/COLLECTIVE_INTELLIGENCE.md` §P2).
//!
//! # Why this layer exists
//!
//! Agents on different nodes must talk before delegation (P3) can happen.
//! This module defines the *envelope* they exchange: a [`MessageKind`]
//! (what kind of exchange), an [`AgentMessage`] (who says what to whom,
//! with an opaque payload and schema hints), and an [`AgentInbox`] (bounded
//! per-agent FIFO queues the runtime half feeds from the transport).
//!
//! # Design decisions
//!
//! - **Opaque payload**: `payload` is `serde_json::Value`, never a
//!   type-specific shape — callers interpret it by `kind` + the schema
//!   hints. This keeps the crate pure and the format extensible. It is why
//!   `serde_json` is a runtime dependency here (it was dev-only before P2).
//! - **Bounded inbox, no blocking**: a full recipient queue makes `push`
//!   return `false`; it never grows unbounded and never blocks the caller.
//!   The transport decides whether to drop the overflow or refuse the
//!   sender.
//! - **Validation is a pure gate**: [`validate_message`] rejects malformed
//!   envelopes before they are admitted to an inbox or sent. An agent must
//!   address a specific recipient; targeting "any" (empty `to_agent`) is a
//!   transport concern resolved *before* validation.
//! - **Replay protection**: `nonce` is a seed the transport mixes into the
//!   signed envelope; the fabric never deduplicates on it.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use thiserror::Error;

/// The kind of exchange an [`AgentMessage`] represents.
///
/// The kind drives how the recipient (or the fabric) handles the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// Request information or help from another agent.
    Ask,
    /// Assign a sub-task to another agent (delegation handoff).
    Delegate,
    /// Return a result (answer to an `Ask`, completion of a `Delegate`).
    Reply,
    /// Request verification or critique of a result.
    Verify,
    /// Liveness / round-trip probe.
    Ping,
}

/// One agent-to-agent message envelope.
///
/// The envelope is deliberately generic: it carries routing fields
/// (`from_agent`, `to_agent`), an exchange kind, an optional linked
/// [`crate::AgentTask`] id, an opaque `payload`, schema hints and a
/// `nonce` seed for replay protection. It is wire-safe
/// (Serialize + Deserialize) so the transport can forward it over the P2P
/// channel; the transport signs the envelope (including `nonce` and
/// `created_at_ms`) so the fields are tamper-evident end to end.
///
/// The struct is `PartialEq` but not `Eq` because `serde_json::Value` may
/// contain floats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentMessage {
    /// Unique message id, supplied by the caller (e.g. a uuid).
    pub message_id: String,
    /// `agent_id` of the sender.
    pub from_agent: String,
    /// `agent_id` of the recipient; empty means "any capable agent" and is
    /// resolved by the transport (see [`validate_message`]).
    pub to_agent: String,
    /// What kind of exchange this is.
    pub kind: MessageKind,
    /// Linked [`crate::AgentTask`] id, when the message belongs to a task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Opaque payload; callers interpret it by `kind` + the schema hints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// JSON-schema hint for the input payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<String>,
    /// JSON-schema hint for the expected output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
    /// Replay-protection seed; the transport signs the envelope.
    pub nonce: u64,
    /// Creation time (unix ms); must be > 0 to pass [`validate_message`].
    pub created_at_ms: u64,
}

impl AgentMessage {
    /// A new message with routing and kind set; every other field defaults
    /// to empty/zero and is filled with the `with_*` builders.
    pub fn new(message_id: impl Into<String>, from: impl Into<String>, to: impl Into<String>, kind: MessageKind) -> Self {
        Self {
            message_id: message_id.into(),
            from_agent: from.into(),
            to_agent: to.into(),
            kind,
            task_id: None,
            payload: None,
            input_schema: None,
            output_schema: None,
            nonce: 0,
            created_at_ms: 0,
        }
    }

    /// Links the message to an [`crate::AgentTask`] by id.
    pub fn with_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    /// Sets the opaque payload (interpreted by `kind` + schema).
    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Sets input and output JSON-schema hints.
    pub fn with_schemas(mut self, input: impl Into<String>, output: impl Into<String>) -> Self {
        self.input_schema = Some(input.into());
        self.output_schema = Some(output.into());
        self
    }

    /// Sets the replay-protection nonce seed.
    pub fn with_nonce(mut self, nonce: u64) -> Self {
        self.nonce = nonce;
        self
    }

    /// Sets the creation time (unix ms).
    pub fn with_created_at_ms(mut self, created_at_ms: u64) -> Self {
        self.created_at_ms = created_at_ms;
        self
    }
}

/// Bounded, per-recipient FIFO queues for inbound messages.
///
/// Each recipient agent has its own queue capped at `capacity_per_agent`
/// messages. A full queue makes the next `push` return `false` instead of
/// growing: the runtime transport decides whether to drop the overflow or
/// refuse the sender. Ordering within a queue is strictly FIFO by push
/// order. There is deliberately no `Default` — a bounded inbox requires an
/// explicit capacity.
#[derive(Debug, Clone)]
pub struct AgentInbox {
    capacity_per_agent: usize,
    queues: HashMap<String, VecDeque<AgentMessage>>,
}

impl AgentInbox {
    /// A new inbox where each recipient's queue holds at most
    /// `capacity_per_agent` messages (0 disables delivery entirely).
    pub fn new(capacity_per_agent: usize) -> Self {
        Self {
            capacity_per_agent,
            queues: HashMap::new(),
        }
    }

    /// Pushes a message into the recipient's queue.
    ///
    /// Returns `false` when the queue for `to_agent` is full (the message
    /// is not stored). Never blocks and never grows the queue past the
    /// configured capacity.
    pub fn push(&mut self, to_agent: &str, msg: AgentMessage) -> bool {
        debug_assert_eq!(
            msg.to_agent, to_agent,
            "inbox key must match the message recipient"
        );
        let queue = self.queues.entry(to_agent.to_string()).or_default();
        if queue.len() >= self.capacity_per_agent {
            return false;
        }
        queue.push_back(msg);
        true
    }

    /// Pops the oldest message for an agent (FIFO).
    pub fn pop(&mut self, agent_id: &str) -> Option<AgentMessage> {
        self.queues.get_mut(agent_id).and_then(|q| q.pop_front())
    }

    /// Number of undelivered messages waiting for an agent.
    pub fn pending(&self, agent_id: &str) -> usize {
        self.queues.get(agent_id).map_or(0, |q| q.len())
    }

    /// Looks at the oldest message for an agent without consuming it.
    pub fn peek(&self, agent_id: &str) -> Option<&AgentMessage> {
        self.queues.get(agent_id).and_then(|q| q.front())
    }

    /// Discards all pending messages for an agent.
    pub fn clear(&mut self, agent_id: &str) {
        self.queues.remove(agent_id);
    }
}

/// Why a message failed [`validate_message`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MessageValidationError {
    /// `message_id` is empty.
    #[error("message_id must not be empty")]
    EmptyMessageId,
    /// `from_agent` is empty.
    #[error("from_agent must not be empty")]
    EmptyFromAgent,
    /// `to_agent` is empty — an agent must address a specific recipient.
    #[error("to_agent must not be empty")]
    EmptyToAgent,
    /// `created_at_ms` is 0.
    #[error("created_at_ms must be greater than 0")]
    InvalidTimestamp,
}

/// Validates a message before it is admitted to an inbox or sent.
///
/// Routing and identity fields must be present and the creation time must
/// be set. `to_agent` is required here even though the wire shape permits
/// empty (meaning "any capable agent"): a *specific* recipient is what makes
/// a message deliverable, so the transport resolves "any" into a concrete
/// agent before validation.
pub fn validate_message(msg: &AgentMessage) -> Result<(), MessageValidationError> {
    if msg.message_id.is_empty() {
        return Err(MessageValidationError::EmptyMessageId);
    }
    if msg.from_agent.is_empty() {
        return Err(MessageValidationError::EmptyFromAgent);
    }
    if msg.to_agent.is_empty() {
        return Err(MessageValidationError::EmptyToAgent);
    }
    if msg.created_at_ms == 0 {
        return Err(MessageValidationError::InvalidTimestamp);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A valid message addressed to `b:executor`, ready for validation.
    fn valid(kind: MessageKind) -> AgentMessage {
        AgentMessage::new("m-1", "a:planner", "b:executor", kind).with_created_at_ms(1_700_000_000_000)
    }

    /// A valid message to a chosen recipient.
    fn to(recipient: &str, kind: MessageKind) -> AgentMessage {
        AgentMessage::new("m-1", "a:planner", recipient, kind).with_created_at_ms(1_700_000_000_000)
    }

    #[test]
    fn message_round_trips_over_wire() {
        let m = AgentMessage::new("m-1", "a:planner", "b:executor", MessageKind::Delegate)
            .with_task("t-9")
            .with_payload(json!({"query": "summarize"}))
            .with_schemas(r#"{"type":"object"}"#, r#"{"type":"string"}"#)
            .with_nonce(42)
            .with_created_at_ms(1_700_000_000_000);
        let json = serde_json::to_string(&m).unwrap();
        let back: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
        // The kind travels as its snake_case name.
        assert!(json.contains(r#""kind":"delegate""#));
    }

    #[test]
    fn message_builders_fill_fields() {
        let m = AgentMessage::new("m-9", "a:planner", "b:executor", MessageKind::Delegate)
            .with_task("t-3")
            .with_payload(json!({"x": 1}))
            .with_schemas("in", "out")
            .with_nonce(7)
            .with_created_at_ms(1234);
        assert_eq!(m.task_id.as_deref(), Some("t-3"));
        assert_eq!(m.payload, Some(json!({"x": 1})));
        assert_eq!(m.input_schema.as_deref(), Some("in"));
        assert_eq!(m.output_schema.as_deref(), Some("out"));
        assert_eq!(m.nonce, 7);
        assert_eq!(m.created_at_ms, 1234);
        // Defaults before building.
        let bare = AgentMessage::new("m-0", "a", "b", MessageKind::Ping);
        assert_eq!(bare.nonce, 0);
        assert_eq!(bare.created_at_ms, 0);
        assert!(bare.task_id.is_none());
        assert!(bare.payload.is_none());
    }

    #[test]
    fn inbox_delivers_fifo_per_recipient() {
        let mut inbox = AgentInbox::new(8);
        assert!(inbox.push("b:executor", valid(MessageKind::Ask).with_task("t-1")));
        assert!(inbox.push("b:executor", valid(MessageKind::Delegate).with_task("t-2")));
        assert!(inbox.push("b:executor", valid(MessageKind::Ping)));
        assert_eq!(inbox.pending("b:executor"), 3);
        assert_eq!(inbox.pop("b:executor").unwrap().task_id.as_deref(), Some("t-1"));
        assert_eq!(inbox.pop("b:executor").unwrap().task_id.as_deref(), Some("t-2"));
        assert_eq!(inbox.pop("b:executor").unwrap().kind, MessageKind::Ping);
        assert!(inbox.pop("b:executor").is_none());
    }

    #[test]
    fn inbox_bound_drops_overflow_without_growing() {
        let mut inbox = AgentInbox::new(2);
        assert!(inbox.push("b:executor", valid(MessageKind::Ask)));
        assert!(inbox.push("b:executor", valid(MessageKind::Ask)));
        // Third push overflows: rejected, queue stays capped at capacity.
        assert!(!inbox.push("b:executor", valid(MessageKind::Ask)));
        assert_eq!(inbox.pending("b:executor"), 2);
        // The two accepted messages are still delivered FIFO.
        assert!(inbox.pop("b:executor").is_some());
        assert!(inbox.pop("b:executor").is_some());
        assert!(inbox.pop("b:executor").is_none());
        // Once drained, pushing works again.
        assert!(inbox.push("b:executor", valid(MessageKind::Ask)));
        assert_eq!(inbox.pending("b:executor"), 1);
    }

    #[test]
    fn inbox_isolates_agents() {
        let mut inbox = AgentInbox::new(4);
        assert!(inbox.push("a:planner", to("a:planner", MessageKind::Reply)));
        assert!(inbox.push("b:executor", valid(MessageKind::Delegate)));
        assert!(inbox.push("b:executor", valid(MessageKind::Delegate)));
        assert_eq!(inbox.pending("a:planner"), 1);
        assert_eq!(inbox.pending("b:executor"), 2);
        // Popping one agent never touches the other's queue.
        assert_eq!(inbox.pop("b:executor").unwrap().kind, MessageKind::Delegate);
        assert_eq!(inbox.pending("a:planner"), 1);
        assert_eq!(inbox.pending("b:executor"), 1);
        // An unknown agent has no pending messages.
        assert_eq!(inbox.pending("nobody"), 0);
        assert!(inbox.pop("nobody").is_none());
        assert!(inbox.peek("nobody").is_none());
    }

    #[test]
    fn inbox_peek_does_not_consume() {
        let mut inbox = AgentInbox::new(4);
        let m = valid(MessageKind::Ask).with_task("t-7");
        inbox.push("b:executor", m.clone());
        assert_eq!(inbox.peek("b:executor").unwrap().task_id.as_deref(), Some("t-7"));
        assert_eq!(inbox.pending("b:executor"), 1);
        assert_eq!(inbox.pop("b:executor").unwrap(), m);
        assert!(inbox.peek("b:executor").is_none());
    }

    #[test]
    fn inbox_clear_drops_pending() {
        let mut inbox = AgentInbox::new(4);
        inbox.push("b:executor", valid(MessageKind::Ask));
        inbox.push("b:executor", valid(MessageKind::Ask));
        inbox.clear("b:executor");
        assert_eq!(inbox.pending("b:executor"), 0);
        assert!(inbox.pop("b:executor").is_none());
    }

    #[test]
    fn validation_rejects_empty_fields_and_accepts_valid() {
        assert_eq!(validate_message(&valid(MessageKind::Ask)), Ok(()));
        assert_eq!(validate_message(&valid(MessageKind::Delegate)), Ok(()));

        let no_id = AgentMessage::new("", "a", "b", MessageKind::Ask).with_created_at_ms(1000);
        assert_eq!(validate_message(&no_id), Err(MessageValidationError::EmptyMessageId));

        let no_from = AgentMessage::new("m-1", "", "b", MessageKind::Ask).with_created_at_ms(1000);
        assert_eq!(validate_message(&no_from), Err(MessageValidationError::EmptyFromAgent));

        let no_to = AgentMessage::new("m-1", "a", "", MessageKind::Ask).with_created_at_ms(1000);
        assert_eq!(validate_message(&no_to), Err(MessageValidationError::EmptyToAgent));

        let no_time = AgentMessage::new("m-1", "a", "b", MessageKind::Ask);
        assert_eq!(validate_message(&no_time), Err(MessageValidationError::InvalidTimestamp));
    }

    #[test]
    fn message_kinds_serialize_to_snake_case() {
        assert_eq!(serde_json::to_string(&MessageKind::Ask).unwrap(), r#""ask""#);
        assert_eq!(
            serde_json::to_string(&MessageKind::Delegate).unwrap(),
            r#""delegate""#
        );
        assert_eq!(
            serde_json::to_string(&MessageKind::Reply).unwrap(),
            r#""reply""#
        );
        assert_eq!(
            serde_json::to_string(&MessageKind::Verify).unwrap(),
            r#""verify""#
        );
        assert_eq!(
            serde_json::to_string(&MessageKind::Ping).unwrap(),
            r#""ping""#
        );
    }
}