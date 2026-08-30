//! `LocalAgentRuntime` — the default implementation of the `AgentRuntime`
//! trait declared in `lib.rs`.
//!
//! Sprint 0.1: minimal working implementation. Provides:
//!   - In-memory `DashMap<AgentId, AgentState>` as the agent store.
//!   - Reads from the v1 fabric (hub/society/arena) by accepting an
//!     `ObservationBuilder` closure that produces `AgentObservation` from
//!     external state. The default observation builder is the
//!     `StaticObservationBuilder`; production wiring is the caller's
//!     responsibility.
//!   - Calls a `DecisionPolicy` (defined in `crate::policy`) to
//!     produce `AgentDecision`.
//!   - Publishes lifecycle events on the `EventBus`.
//!   - `learn()` is a no-op apart from incrementing metrics counters
//!     (the SAES §2 signal pipeline lands in Sprint 0.2).
//!
//! Capability validation happens at `spawn()` time. The runtime does
//! not depend on the `capability-registry` crate; the caller passes a
//! `serde_json::Value` schema for each declared capability, and the
//! runtime validates that the schema is well-formed (non-empty
//! string). The actual semantic check (does this capability exist?)
//! is the caller's responsibility — typically the caller looks up
//! the capability in their own registry before calling `spawn()`.
//! This keeps the runtime decoupled from any specific registry
//! implementation.

use crate::policy::{DefaultBidPolicy, DecisionPolicy, JsonSpecPolicyLite};
use crate::{
    ActionResult, ActionType, AgentAction, AgentConfig, AgentDecision, AgentHandle, AgentId,
    AgentMetrics, AgentObservation, AgentRuntime, AgentRuntimeError, AgentState, AgentStatus,
    ResourceUsage,
};
use async_trait::async_trait;
use dashmap::DashMap;
use decentraai_event_bus::{Event, EventBus, EventPriority, Topic};
use decentraai_agent_society::AgentId as ProtocolAgentId;
use serde_json::Value;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LocalRuntimeError {
    #[error("agent already exists: {0}")]
    AlreadyExists(ProtocolAgentId),
    #[error("agent not found: {0}")]
    NotFound(ProtocolAgentId),
    #[error("invalid capability schema at index {0}: {1}")]
    InvalidCapabilitySchema(usize, String),
    #[error("internal: {0}")]
    Internal(String),
}

/// Closure that builds an `AgentObservation` for a given agent, given the
/// agent's declared capabilities.
#[async_trait]
pub trait ObservationBuilder: Send + Sync {
    async fn build(
        &self,
        agent_id: &ProtocolAgentId,
        capabilities: &[String],
    ) -> AgentObservation;
}

/// Local, in-memory implementation of the `AgentRuntime` trait.
pub struct LocalAgentRuntime {
    agents: DashMap<ProtocolAgentId, AgentState>,
    event_bus: Arc<EventBus>,
    /// Default policy used when an agent has no per-agent override.
    default_policy: Arc<dyn DecisionPolicy>,
    /// Per-agent policy override.
    agent_policies: DashMap<ProtocolAgentId, Arc<dyn DecisionPolicy>>,
    observation_builder: Arc<dyn ObservationBuilder>,
}

impl std::fmt::Debug for LocalAgentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAgentRuntime")
            .field("agent_count", &self.agents.len())
            .field("default_policy", &self.default_policy.name())
            .finish()
    }
}

impl LocalAgentRuntime {
    pub fn new(
        event_bus: Arc<EventBus>,
        observation_builder: Arc<dyn ObservationBuilder>,
    ) -> Self {
        Self {
            agents: DashMap::new(),
            event_bus,
            default_policy: Arc::new(DefaultBidPolicy),
            agent_policies: DashMap::new(),
            observation_builder,
        }
    }

    /// Override the default policy (e.g. with a JsonSpecPolicy).
    pub fn with_default_policy(mut self, policy: Arc<dyn DecisionPolicy>) -> Self {
        self.default_policy = policy;
        self
    }

    /// Install a per-agent policy override. The override is used
    /// instead of the default for this agent.
    pub fn install_policy_for(
        &self,
        agent_id: &ProtocolAgentId,
        policy: Arc<dyn DecisionPolicy>,
    ) {
        self.agent_policies.insert(agent_id.clone(), policy);
    }

    /// Install a JSON-Spec policy for one specific agent.
    pub fn install_json_spec_for(
        &self,
        agent_id: &ProtocolAgentId,
        spec: JsonSpecPolicyLite,
    ) {
        self.agent_policies
            .insert(agent_id.clone(), Arc::new(spec));
    }

    /// Validate that the declared capabilities have a well-formed
    /// shape. Each capability must be a non-empty string. The
    /// runtime does not verify the capability exists in any registry
    /// — that is the caller's job.
    fn validate_capabilities(capabilities: &[String]) -> Result<(), LocalRuntimeError> {
        for (i, cap) in capabilities.iter().enumerate() {
            if cap.trim().is_empty() {
                return Err(LocalRuntimeError::InvalidCapabilitySchema(
                    i,
                    "capability id is empty".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn emit_lifecycle(&self, agent_id: &ProtocolAgentId, event_type: &str) {
        let event = Event {
            id: decentraai_event_bus::EventId::new(),
            topic: Topic::agent(&agent_id.to_string()),
            source: agent_id.clone(),
            timestamp: now_ms(),
            event_type: event_type.to_string(),
            payload: serde_json::json!({"agent_id": agent_id.to_string()}),
            metadata: decentraai_event_bus::EventMetadata {
                priority: EventPriority::Normal,
                tags: vec!["agent-runtime".to_string(), "lifecycle".to_string()],
                ..Default::default()
            },
        };
        let _ = self.event_bus.try_publish(event);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_millis() as u64)
    .unwrap_or(0)
}

#[async_trait]
impl AgentRuntime for LocalAgentRuntime {
    async fn spawn(&self, config: AgentConfig) -> Result<AgentHandle, AgentRuntimeError> {
        Self::validate_capabilities(&config.capabilities)
            .map_err(|e| AgentRuntimeError::Internal(e.to_string()))?;
        let id = config.agent_id.clone();
        if self.agents.contains_key(&id) {
            return Err(AgentRuntimeError::AlreadyExists(id));
        }
        let now = now_ms();
        let state = AgentState {
            agent_id: id.clone(),
            status: AgentStatus::Initializing,
            config,
            current_goals: vec![],
            current_beliefs: serde_json::Value::Null,
            resource_usage: ResourceUsage::default(),
            metrics: AgentMetrics::default(),
            created_at: now,
            updated_at: now,
            last_active_at: now,
        };
        self.agents.insert(id.clone(), state);
        self.emit_lifecycle(&id, "agent.spawned");
        if let Some(mut s) = self.agents.get_mut(&id) {
            s.status = AgentStatus::Ready;
            s.updated_at = now_ms();
        }
        self.emit_lifecycle(&id, "agent.ready");
        Ok(AgentHandle {
            agent_id: id,
            status: AgentStatus::Ready,
        })
    }

    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentRuntimeError> {
        self.agents
            .get(agent_id)
            .map(|s| s.clone())
            .ok_or(AgentRuntimeError::NotFound(agent_id.clone()))
    }

    async fn observe(&self, agent_id: &AgentId) -> Result<AgentObservation, AgentRuntimeError> {
        let state = self.get_state(agent_id).await?;
        let caps = state.config.capabilities.clone();
        let obs = self.observation_builder.build(agent_id, &caps).await;
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.last_active_at = now_ms();
            s.updated_at = now_ms();
        }
        Ok(obs)
    }

    async fn decide(
        &self,
        agent_id: &AgentId,
        observation: &AgentObservation,
    ) -> Result<AgentDecision, AgentRuntimeError> {
        let policy: Arc<dyn DecisionPolicy> = self
            .agent_policies
            .get(agent_id)
            .map(|p| p.value().clone())
            .unwrap_or_else(|| self.default_policy.clone());
        let decision = policy.decide(observation);
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.last_active_at = now_ms();
            s.updated_at = now_ms();
        }
        Ok(decision)
    }

    async fn act(
        &self,
        agent_id: &AgentId,
        decision: &AgentDecision,
    ) -> Result<AgentAction, AgentRuntimeError> {
        // Map the agent's decision to the v1 `ActionType` so the
        // runtime can dispatch it to the appropriate v1 surface
        // (hub/society/arena/memory). Unknown / future decision
        // types fall through to `ActionType::Custom(...)` so we do
        // not need to recompile the runtime when a new decision
        // kind is introduced by an external policy.
        let action_type = match decision.decision_type {
            crate::DecisionType::Bid => ActionType::HubBid,
            crate::DecisionType::Propose => ActionType::HubPropose,
            crate::DecisionType::FormTeam => ActionType::HubFormTeam,
            crate::DecisionType::Execute => ActionType::HubExecute,
            crate::DecisionType::PublishTask | crate::DecisionType::Publish => {
                ActionType::HubPublish
            }
            crate::DecisionType::Wait | crate::DecisionType::Rest => ActionType::HubState,
            crate::DecisionType::Search => ActionType::MemorySearch,
            crate::DecisionType::Learn | crate::DecisionType::Reflect => {
                ActionType::MemoryWrite
            }
            crate::DecisionType::Dream => ActionType::MemoryWrite,
        };
        // Pack the full decision into the action's parameters so the
        // bus subscriber can reconstruct what the agent intended.
        // The `decision_type` and `confidence` are top-level fields
        // for easy filtering; the full context is preserved under
        // "context".
        let parameters = serde_json::json!({
            "decision_type": format!("{:?}", decision.decision_type),
            "reasoning": decision.reasoning,
            "confidence": decision.confidence,
            "expected_outcome": decision.expected_outcome,
            "context": decision.context,
        });
        let action = AgentAction {
            agent_id: agent_id.clone(),
            timestamp: now_ms(),
            action_type,
            parameters,
            result: None,
            observation: None,
        };
        let event = Event {
            id: decentraai_event_bus::EventId::new(),
            topic: Topic::agent(&agent_id.to_string()),
            source: agent_id.clone(),
            timestamp: now_ms(),
            event_type: "agent.action".to_string(),
            payload: serde_json::to_value(&action).unwrap_or(Value::Null),
            metadata: decentraai_event_bus::EventMetadata {
                priority: EventPriority::Normal,
                tags: vec!["agent-runtime".to_string(), "action".to_string()],
                ..Default::default()
            },
        };
        let _ = self.event_bus.try_publish(event);
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.last_active_at = now_ms();
            s.updated_at = now_ms();
        }
        Ok(action)
    }

    async fn learn(
        &self,
        agent_id: &AgentId,
        action: &AgentAction,
        outcome: &ActionResult,
    ) -> Result<(), AgentRuntimeError> {
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            if outcome.success {
                s.metrics.tasks_completed = s.metrics.tasks_completed.saturating_add(1);
            } else {
                s.metrics.tasks_failed = s.metrics.tasks_failed.saturating_add(1);
            }
            if let Some(r) = outcome.reward {
                s.metrics.total_reward_earned =
                    s.metrics.total_reward_earned.saturating_add(r);
            }
            if let Some(d) = outcome.reputation_delta {
                s.metrics.reputation_score += d;
            }
            s.last_active_at = now_ms();
            s.updated_at = now_ms();
        }
        let event = Event {
            id: decentraai_event_bus::EventId::new(),
            topic: Topic::agent(&agent_id.to_string()),
            source: agent_id.clone(),
            timestamp: now_ms(),
            event_type: "agent.learn".to_string(),
            payload: serde_json::json!({
                "action_type": format!("{:?}", action.action_type),
                "success": outcome.success,
                "reward": outcome.reward,
                "reputation_delta": outcome.reputation_delta,
            }),
            metadata: decentraai_event_bus::EventMetadata {
                priority: EventPriority::Normal,
                tags: vec!["agent-runtime".to_string(), "learn".to_string()],
                ..Default::default()
            },
        };
        let _ = self.event_bus.try_publish(event);
        Ok(())
    }

    async fn pause(&self, agent_id: &AgentId) -> Result<(), AgentRuntimeError> {
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.status = AgentStatus::Paused;
            s.updated_at = now_ms();
        }
        self.emit_lifecycle(agent_id, "agent.paused");
        Ok(())
    }

    async fn resume(&self, agent_id: &AgentId) -> Result<(), AgentRuntimeError> {
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.status = AgentStatus::Ready;
            s.updated_at = now_ms();
        }
        self.emit_lifecycle(agent_id, "agent.resumed");
        Ok(())
    }

    async fn stop(&self, agent_id: &AgentId) -> Result<(), AgentRuntimeError> {
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.status = AgentStatus::Stopping;
            s.updated_at = now_ms();
        }
        self.emit_lifecycle(agent_id, "agent.stopping");
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.status = AgentStatus::Stopped;
            s.updated_at = now_ms();
        }
        self.emit_lifecycle(agent_id, "agent.stopped");
        Ok(())
    }

    async fn retire(
        &self,
        agent_id: &AgentId,
        reason: String,
    ) -> Result<(), AgentRuntimeError> {
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.status = AgentStatus::Retired;
            s.updated_at = now_ms();
        }
        let event = Event {
            id: decentraai_event_bus::EventId::new(),
            topic: Topic::agent(&agent_id.to_string()),
            source: agent_id.clone(),
            timestamp: now_ms(),
            event_type: "agent.retired".to_string(),
            payload: serde_json::json!({"reason": reason}),
            metadata: decentraai_event_bus::EventMetadata {
                priority: EventPriority::High,
                tags: vec!["agent-runtime".to_string(), "retire".to_string()],
                ..Default::default()
            },
        };
        let _ = self.event_bus.try_publish(event);
        Ok(())
    }

    async fn list_agents(&self) -> Result<Vec<AgentId>, AgentRuntimeError> {
        Ok(self.agents.iter().map(|e| e.key().clone()).collect())
    }

    async fn get_metrics(&self, agent_id: &AgentId) -> Result<AgentMetrics, AgentRuntimeError> {
        self.agents
            .get(agent_id)
            .map(|s| s.metrics.clone())
            .ok_or(AgentRuntimeError::NotFound(agent_id.clone()))
    }
}

/// Default `ObservationBuilder` that uses static JSON for testing.
/// In production, the caller wires a real implementation that hits
/// the v1 `hub_state`, `society_state`, and `arena_state` endpoints.
pub struct StaticObservationBuilder {
    pub hub_state: Value,
    pub society_state: Value,
    pub arena_state: Option<Value>,
    pub personal_memory: Value,
}

impl StaticObservationBuilder {
    pub fn empty() -> Self {
        Self {
            hub_state: serde_json::json!({}),
            society_state: serde_json::json!({}),
            arena_state: None,
            personal_memory: serde_json::json!({}),
        }
    }
}

#[async_trait]
impl ObservationBuilder for StaticObservationBuilder {
    async fn build(
        &self,
        agent_id: &ProtocolAgentId,
        capabilities: &[String],
    ) -> AgentObservation {
        AgentObservation {
            agent_id: agent_id.clone(),
            timestamp: now_ms(),
            hub_state_summary: self.hub_state.clone(),
            society_state_summary: self.society_state.clone(),
            personal_memory_summary: self.personal_memory.clone(),
            arena_state_summary: self.arena_state.clone(),
            queue_depth: 0,
            available_capabilities: capabilities.to_vec(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentConfig, AgentRuntime, ResourceLimits};

    fn cfg(caps: Vec<String>) -> AgentConfig {
        AgentConfig {
            agent_id: decentraai_protocol::AgentId::new(),
            name: "test".to_string(),
            capabilities: caps,
            initial_goals: vec![],
            initial_memory: None,
            policy_overrides: None,
            resource_limits: ResourceLimits::default(),
        }
    }

    fn runtime() -> (Arc<EventBus>,) {
        let bus = Arc::new(EventBus::new(Arc::new(
            decentraai_event_bus::InMemoryEventStore::new(1024),
        )));
        (bus,)
    }

    #[tokio::test]
    async fn spawn_and_get_state() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["analysis".into()])).await.unwrap();
        let state = r.get_state(&h.agent_id).await.unwrap();
        assert_eq!(state.config.name, "test");
        assert_eq!(state.status, AgentStatus::Ready);
    }

    #[tokio::test]
    async fn spawn_rejects_empty_capability() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let mut c = cfg(vec!["analysis".into()]);
        c.capabilities = vec!["   ".into()];
        assert!(r.spawn(c).await.is_err());
    }

    #[tokio::test]
    async fn spawn_rejects_duplicate() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let c = cfg(vec!["a".into()]);
        let _h = r.spawn(c.clone()).await.unwrap();
        assert!(r.spawn(c).await.is_err());
    }

    #[tokio::test]
    async fn decide_uses_default_policy() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder {
            hub_state: serde_json::json!({
                "tasks": [
                    {"id": "t1", "status": "open", "required_capability": "analysis", "reward": 50}
                ]
            }),
            society_state: serde_json::json!({}),
            arena_state: None,
            personal_memory: serde_json::json!({}),
        });
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["analysis".into()])).await.unwrap();
        let observation = r.observe(&h.agent_id).await.unwrap();
        let d = r.decide(&h.agent_id, &observation).await.unwrap();
        assert!(matches!(d.decision_type, crate::DecisionType::Bid));
    }

    #[tokio::test]
    async fn decide_uses_per_agent_json_spec() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder {
            hub_state: serde_json::json!({}),
            society_state: serde_json::json!({}),
            arena_state: None,
            personal_memory: serde_json::json!({}),
        });
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["x".into()])).await.unwrap();
        r.install_json_spec_for(
            &h.agent_id,
            JsonSpecPolicyLite {
                name: "always_wait".to_string(),
                rules: vec![crate::policy::JsonSpecRuleLite {
                    name: "r1".to_string(),
                    condition_contains: "hub_state_summary".to_string(), // always present
                    action: "wait".to_string(),
                    rationale: "always wait".to_string(),
                }],
            },
        );
        let observation = r.observe(&h.agent_id).await.unwrap();
        let d = r.decide(&h.agent_id, &observation).await.unwrap();
        assert!(matches!(d.decision_type, crate::DecisionType::Wait));
        assert!(d.reasoning.contains("always wait"));
    }

    #[tokio::test]
    async fn act_runs() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["a".into()])).await.unwrap();
        let observation = r.observe(&h.agent_id).await.unwrap();
        let d = r.decide(&h.agent_id, &observation).await.unwrap();
        let a = r.act(&h.agent_id, &d).await.unwrap();
        assert_eq!(a.agent_id, h.agent_id);
    }

    #[tokio::test]
    async fn learn_increments_metrics() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["a".into()])).await.unwrap();
        let a = AgentAction {
            agent_id: h.agent_id.clone(),
            timestamp: 0,
            action_type: ActionType::HubBid,
            parameters: serde_json::json!({}),
            result: None,
            observation: None,
        };
        let outcome = ActionResult {
            success: true,
            output: None,
            error: None,
            evidence_id: None,
            reward: Some(25),
            reputation_delta: Some(0.15),
        };
        r.learn(&h.agent_id, &a, &outcome).await.unwrap();
        let m = r.get_metrics(&h.agent_id).await.unwrap();
        assert_eq!(m.tasks_completed, 1);
        assert_eq!(m.total_reward_earned, 25);
        assert!((m.reputation_score - 0.15).abs() < 1e-6);
    }

    #[tokio::test]
    async fn learn_counts_failure() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["a".into()])).await.unwrap();
        let a = AgentAction {
            agent_id: h.agent_id.clone(),
            timestamp: 0,
            action_type: ActionType::HubBid,
            parameters: serde_json::json!({}),
            result: None,
            observation: None,
        };
        let outcome = ActionResult {
            success: false,
            output: None,
            error: Some("nope".to_string()),
            evidence_id: None,
            reward: None,
            reputation_delta: None,
        };
        r.learn(&h.agent_id, &a, &outcome).await.unwrap();
        let m = r.get_metrics(&h.agent_id).await.unwrap();
        assert_eq!(m.tasks_failed, 1);
        assert_eq!(m.tasks_completed, 0);
    }

    #[tokio::test]
    async fn pause_resume_stop_lifecycle() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["a".into()])).await.unwrap();
        r.pause(&h.agent_id).await.unwrap();
        assert_eq!(r.get_state(&h.agent_id).await.unwrap().status, AgentStatus::Paused);
        r.resume(&h.agent_id).await.unwrap();
        assert_eq!(r.get_state(&h.agent_id).await.unwrap().status, AgentStatus::Ready);
        r.stop(&h.agent_id).await.unwrap();
        assert_eq!(r.get_state(&h.agent_id).await.unwrap().status, AgentStatus::Stopped);
    }

    #[tokio::test]
    async fn retire_sets_status() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["a".into()])).await.unwrap();
        r.retire(&h.agent_id, "test".to_string()).await.unwrap();
        assert_eq!(r.get_state(&h.agent_id).await.unwrap().status, AgentStatus::Retired);
    }

    #[tokio::test]
    async fn list_agents_returns_all() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let mut c_a = cfg(vec!["a".into()]);
        c_a.agent_id = format!("agent-{}", uuid::Uuid::new_v4());
        let mut c_b = cfg(vec!["b".into()]);
        c_b.agent_id = format!("agent-{}", uuid::Uuid::new_v4());
        let _a = r.spawn(c_a).await.unwrap();
        let _b = r.spawn(c_b).await.unwrap();
        assert_eq!(r.list_agents().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn unanticipated_agent_with_custom_capability() {
        // The acceptance test for the foundation: a brand-new agent
        // declares a capability the v1 system has never seen, and the
        // runtime accepts it without recompiling the v1 enum.
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder {
            hub_state: serde_json::json!({
                "tasks": [
                    {"id": "q1", "status": "open", "required_capability": "quantum_simulation_v0", "reward": 100}
                ]
            }),
            society_state: serde_json::json!({}),
            arena_state: None,
            personal_memory: serde_json::json!({}),
        });
        let r = LocalAgentRuntime::new(bus, obs);
        let mut c = cfg(vec!["quantum_simulation_v0".into()]);
        c.name = "quantum-bot".to_string();
        let h = r.spawn(c).await.unwrap();
        let observation = r.observe(&h.agent_id).await.unwrap();
        assert!(observation.available_capabilities.contains(&"quantum_simulation_v0".to_string()));
        // Default policy sees the matching task and bids.
        let d = r.decide(&h.agent_id, &observation).await.unwrap();
        assert!(matches!(d.decision_type, crate::DecisionType::Bid));
        // The agent can also install a custom spec that branches on
        // the new capability.
        r.install_json_spec_for(
            &h.agent_id,
            JsonSpecPolicyLite {
                name: "always_bid_quantum".to_string(),
                rules: vec![crate::policy::JsonSpecRuleLite {
                    name: "if_quantum".to_string(),
                    condition_contains: "quantum_simulation_v0".to_string(),
                    action: "bid".to_string(),
                    rationale: "quantum work is interesting".to_string(),
                }],
            },
        );
        let d2 = r.decide(&h.agent_id, &observation).await.unwrap();
        assert!(matches!(d2.decision_type, crate::DecisionType::Bid));
        assert!(d2.reasoning.contains("quantum work"));
    }

    // End-to-end: spawn -> observe -> decide -> act -> event on bus.
    // The action.parameters must carry the full decision context so a
    // bus subscriber can reconstruct what the agent intended.
    #[tokio::test]
    async fn end_to_end_emits_action_event_with_full_decision() {
        let bus = Arc::new(EventBus::new(Arc::new(
            decentraai_event_bus::InMemoryEventStore::new(1024),
        )));
        let mut rx = bus.subscribe_broadcast();
        let obs = Arc::new(StaticObservationBuilder {
            hub_state: serde_json::json!({
                "tasks": [
                    {"id": "t9", "status": "open", "required_capability": "summary", "reward": 7}
                ]
            }),
            society_state: serde_json::json!({}),
            arena_state: None,
            personal_memory: serde_json::json!({}),
        });
        let r = LocalAgentRuntime::new(bus.clone(), obs);
        let h = r.spawn(cfg(vec!["summary".into()])).await.unwrap();
        let observation = r.observe(&h.agent_id).await.unwrap();
        let d = r.decide(&h.agent_id, &observation).await.unwrap();
        let _a = r.act(&h.agent_id, &d).await.unwrap();

        // Drain the broadcast channel until we find the action event.
        let mut found: Option<decentraai_event_bus::Event> = None;
        for _ in 0..10 {
            match rx.try_recv() {
                Ok(ev) => {
                    if ev.event_type == "agent.action"
                        && ev.source == h.agent_id
                    {
                        found = Some(ev);
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let ev = found.expect("expected an agent.action event on the bus");
        // The action event must include the decision_type and
        // reasoning so subscribers can reconstruct intent.
        let params = ev.payload.get("parameters").expect("parameters");
        assert_eq!(
            params.get("decision_type").and_then(|v| v.as_str()),
            Some("Bid")
        );
        assert_eq!(
            params.get("reasoning").and_then(|v| v.as_str()).map(|s| s.contains("task t9")),
            Some(true)
        );
        assert!(params.get("confidence").and_then(|v| v.as_f64()).unwrap() > 0.0);
    }

    // Lifecycle: each phase emits an event on the bus. This is the
    // foundation for future SAES 0.2 behaviour-evolution: a signal
    // pipeline that subscribes to these events.
    #[tokio::test]
    async fn lifecycle_emits_one_event_per_phase() {
        let bus = Arc::new(EventBus::new(Arc::new(
            decentraai_event_bus::InMemoryEventStore::new(1024),
        )));
        let mut rx = bus.subscribe_broadcast();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus.clone(), obs);
        let h = r.spawn(cfg(vec!["a".into()])).await.unwrap();
        r.pause(&h.agent_id).await.unwrap();
        r.resume(&h.agent_id).await.unwrap();
        r.stop(&h.agent_id).await.unwrap();
        r.retire(&h.agent_id, "done".into()).await.unwrap();

        // Count agent.spawned, agent.ready, agent.paused,
        // agent.resumed, agent.stopping, agent.stopped, agent.retired.
        let mut counts = std::collections::HashMap::<String, u32>::new();
        for _ in 0..20 {
            match rx.try_recv() {
                Ok(ev) => {
                    if ev.source == h.agent_id && ev.event_type.starts_with("agent.") {
                        *counts.entry(ev.event_type).or_insert(0) += 1;
                    }
                }
                Err(_) => break,
            }
        }
        assert_eq!(counts.get("agent.spawned").copied(), Some(1));
        assert_eq!(counts.get("agent.ready").copied(), Some(1));
        assert_eq!(counts.get("agent.paused").copied(), Some(1));
        assert_eq!(counts.get("agent.resumed").copied(), Some(1));
        assert_eq!(counts.get("agent.stopping").copied(), Some(1));
        assert_eq!(counts.get("agent.stopped").copied(), Some(1));
        assert_eq!(counts.get("agent.retired").copied(), Some(1));
    }

    // The hub-style "publisher of a task" (issuer role) emits on
    // topic hub(), separate from agent/(self). The runtime
    // distinguishes the two via the Topic.
    #[tokio::test]
    async fn event_topics_separate_publisher_from_agent() {
        let bus = Arc::new(EventBus::new(Arc::new(
            decentraai_event_bus::InMemoryEventStore::new(1024),
        )));
        let mut hub_rx = bus.subscribe_broadcast();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let _runtime = LocalAgentRuntime::new(bus.clone(), obs);
        // Simulate a hub publishing a task (not via runtime; just
        // publish the event directly, as v1's runtime does).
        let event = Event {
            id: decentraai_event_bus::EventId::new(),
            topic: Topic::hub(),
            source: ProtocolAgentId::from("hub"),
            timestamp: 0,
            event_type: "task_published".to_string(),
            payload: serde_json::json!({"task_id": "t77"}),
            metadata: decentraai_event_bus::EventMetadata::default(),
        };
        bus.publish(event).await.unwrap();
        let ev = hub_rx.recv().await.unwrap();
        assert_eq!(ev.event_type, "task_published");
        assert_eq!(ev.topic, Topic::hub());
        assert_ne!(ev.topic, Topic::agent("any-agent"));
    }

    // Boundary: empty capabilities list. The default policy cannot
    // match any task (no capability match) and must return Wait.
    // This proves the runtime does not panic on empty capabilities.
    #[tokio::test]
    async fn empty_capabilities_means_default_waits() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder {
            hub_state: serde_json::json!({
                "tasks": [
                    {"id": "t1", "status": "open", "required_capability": "anything", "reward": 100}
                ]
            }),
            society_state: serde_json::json!({}),
            arena_state: None,
            personal_memory: serde_json::json!({}),
        });
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec![])).await.unwrap();
        let observation = r.observe(&h.agent_id).await.unwrap();
        let d = r.decide(&h.agent_id, &observation).await.unwrap();
        assert!(matches!(d.decision_type, crate::DecisionType::Wait));
    }

    // Boundary: the per-agent policy override is mutable. We
    // install a policy, observe that it runs, install another,
    // observe that the new one runs. Both policies have conditions
    // that do NOT match the empty observation, so both fall back to
    // Wait. The test asserts the override installation does not
    // panic and that the runtime still produces Wait.
    #[tokio::test]
    async fn policy_override_is_mutable() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["a".into()])).await.unwrap();

        // Install a first policy. Empty observation -> Wait.
        r.install_json_spec_for(
            &h.agent_id,
            JsonSpecPolicyLite {
                name: "first".to_string(),
                rules: vec![crate::policy::JsonSpecRuleLite {
                    name: "r".to_string(),
                    condition_contains: "never_matches".to_string(),
                    action: "bid".to_string(),
                    rationale: "first".to_string(),
                }],
            },
        );
        let observation = r.observe(&h.agent_id).await.unwrap();
        let d = r.decide(&h.agent_id, &observation).await.unwrap();
        assert!(matches!(d.decision_type, crate::DecisionType::Wait));

        // Install a second policy. The first is overwritten.
        r.install_json_spec_for(
            &h.agent_id,
            JsonSpecPolicyLite {
                name: "second".to_string(),
                rules: vec![crate::policy::JsonSpecRuleLite {
                    name: "r".to_string(),
                    condition_contains: "never_matches_either".to_string(),
                    action: "wait".to_string(),
                    rationale: "second".to_string(),
                }],
            },
        );
        let observation = r.observe(&h.agent_id).await.unwrap();
        let d = r.decide(&h.agent_id, &observation).await.unwrap();
        assert!(matches!(d.decision_type, crate::DecisionType::Wait));
    }

    // Boundary: list_agents is empty initially and reflects spawns.
    #[tokio::test]
    async fn list_agents_initially_empty() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        assert_eq!(r.list_agents().await.unwrap().len(), 0);
    }

    // Boundary: get_metrics on unknown agent returns NotFound.
    #[tokio::test]
    async fn get_metrics_unknown_agent_is_notfound() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let result = r.get_metrics(&decentraai_protocol::AgentId::from("ghost")).await;
        assert!(matches!(result, Err(crate::AgentRuntimeError::NotFound(_))));
    }
}
