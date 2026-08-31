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

use crate::policy::{DecisionPolicy, DefaultBidPolicy, JsonSpecPolicyLite};
use crate::saes::adaptation::{BehaviorStore, InMemoryBehaviorStore};
use crate::saes::goals::{AgentGoal, GoalPriority, GoalState, GoalStore, InMemoryGoalStore};
use crate::saes::learning::compute_learning_effect;
use crate::saes::outcomes::ActionOutcome;
use crate::{
    ActionResult, ActionType, AgentAction, AgentConfig, AgentDecision, AgentHandle, AgentId,
    AgentMetrics, AgentObservation, AgentRuntime, AgentRuntimeError, AgentState, AgentStatus,
    DecisionType, ResourceUsage,
};
use async_trait::async_trait;
use dashmap::DashMap;
use decentraai_agent_society::AgentId as ProtocolAgentId;
use decentraai_event_bus::{Event, EventBus, EventPriority, Topic};
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
    async fn build(&self, agent_id: &ProtocolAgentId, capabilities: &[String]) -> AgentObservation;
}

/// Local, in-memory implementation of the `AgentRuntime` trait.
///
/// SAES 0.2: the runtime now maintains per-agent `GoalStore` and
/// `BehaviorStore` so the autonomous cycle (goals → observe → decide →
/// act → outcome → learning → adaptation) is real, not just tested.
pub struct LocalAgentRuntime {
    agents: DashMap<ProtocolAgentId, AgentState>,
    event_bus: Arc<EventBus>,
    /// Default policy used when an agent has no per-agent override.
    default_policy: Arc<dyn DecisionPolicy>,
    /// Per-agent policy override.
    agent_policies: DashMap<ProtocolAgentId, Arc<dyn DecisionPolicy>>,
    observation_builder: Arc<dyn ObservationBuilder>,
    /// SAES 0.2: structured goal tracking per agent.
    goal_store: Arc<dyn GoalStore>,
    /// SAES 0.2: behavior profiles for adaptation.
    behavior_store: Arc<dyn BehaviorStore>,
    /// SAES 0.4: collective goal coordination.
    collective_goal_store: Arc<dyn crate::saes::collective::CollectiveGoalStore>,
    /// SAES 0.5: per-agent pressure episode state (hysteresis + cooldown).
    pressure_states: DashMap<ProtocolAgentId, crate::saes::pressure::PressureEpisode>,
}

impl std::fmt::Debug for LocalAgentRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAgentRuntime")
            .field("agent_count", &self.agents.len())
            .field("default_policy", &self.default_policy.name())
            .field("goal_store", &"Arc<dyn GoalStore>")
            .field("behavior_store", &"Arc<dyn BehaviorStore>")
            .finish()
    }
}

impl LocalAgentRuntime {
    pub fn new(event_bus: Arc<EventBus>, observation_builder: Arc<dyn ObservationBuilder>) -> Self {
        Self {
            agents: DashMap::new(),
            event_bus,
            default_policy: Arc::new(DefaultBidPolicy),
            agent_policies: DashMap::new(),
            observation_builder,
            goal_store: Arc::new(InMemoryGoalStore::new()),
            behavior_store: Arc::new(InMemoryBehaviorStore::new()),
            collective_goal_store: Arc::new(
                crate::saes::collective::InMemoryCollectiveGoalStore::new(),
            ),
            pressure_states: DashMap::new(),
        }
    }

    /// Override the default policy (e.g. with a JsonSpecPolicy).
    pub fn with_default_policy(mut self, policy: Arc<dyn DecisionPolicy>) -> Self {
        self.default_policy = policy;
        self
    }

    /// SAES 0.2: inject a custom goal store (e.g. SQLite-backed for production).
    pub fn with_goal_store(mut self, goal_store: Arc<dyn GoalStore>) -> Self {
        self.goal_store = goal_store;
        self
    }

    /// SAES 0.2: inject a custom behavior store.
    pub fn with_behavior_store(mut self, behavior_store: Arc<dyn BehaviorStore>) -> Self {
        self.behavior_store = behavior_store;
        self
    }

    /// SAES 0.4: inject a custom collective goal store.
    pub fn with_collective_goal_store(
        mut self,
        collective_goal_store: Arc<dyn crate::saes::collective::CollectiveGoalStore>,
    ) -> Self {
        self.collective_goal_store = collective_goal_store;
        self
    }

    /// Install a per-agent policy override. The override is used
    /// instead of the default for this agent.
    pub fn install_policy_for(&self, agent_id: &ProtocolAgentId, policy: Arc<dyn DecisionPolicy>) {
        self.agent_policies.insert(agent_id.clone(), policy);
    }

    /// Install a JSON-Spec policy for one specific agent.
    pub fn install_json_spec_for(&self, agent_id: &ProtocolAgentId, spec: JsonSpecPolicyLite) {
        self.agent_policies.insert(agent_id.clone(), Arc::new(spec));
    }

    /// SAES 0.2: access the goal store (for inspection/testing).
    pub fn goal_store(&self) -> &Arc<dyn GoalStore> {
        &self.goal_store
    }

    /// SAES 0.2: access the behavior store (for inspection/testing).
    pub fn behavior_store(&self) -> &Arc<dyn BehaviorStore> {
        &self.behavior_store
    }

    /// SAES 0.4: access the collective goal store (for inspection/testing).
    pub fn collective_goal_store(&self) -> &Arc<dyn crate::saes::collective::CollectiveGoalStore> {
        &self.collective_goal_store
    }

    /// SAES 0.5 Phase 1 — evaluate this agent's pressure signals.
    ///
    /// Integrates the pressure trigger into the `observe → decide` cycle:
    /// when an agent can no longer continue alone (sustained local pressure,
    /// with hysteresis), this produces an explicit [`CollaborationSignal`]
    /// that Phase 2 (Placement Fairness) consumes, and emits a correlated
    /// EventBus event (`agent.pressure.fired` / `agent.pressure.released`).
    ///
    /// Cooldown: even when `should_assist` stays true, the signal is only
    /// re-emitted after `cooldown_ms` since the last fire — the fabric is
    /// never flooded with a RESOURCE_REQUEST on every tick.
    pub async fn evaluate_pressure(
        &self,
        agent_id: &ProtocolAgentId,
        signals: &crate::saes::pressure::PressureSignals,
        thresholds: &crate::saes::pressure::PressureThresholds,
        cooldown_ms: u64,
        capability: &str,
    ) -> Result<Option<crate::saes::pressure::CollaborationSignal>, String> {
        use crate::saes::pressure::{AssistState, CollaborationSignal, evaluate_pressure};

        let now = now_ms();
        let mut episode = self
            .pressure_states
            .get(agent_id)
            .map(|e| e.clone())
            .unwrap_or_default();

        let prev_corr = episode.correlation_id.clone();
        let decision = evaluate_pressure(signals, thresholds, episode.state, prev_corr.as_deref());

        // Persist hysteresis state regardless of emission.
        episode.state = decision.new_state;
        if decision.should_assist {
            episode.correlation_id = Some(decision.correlation_id.clone());
        }

        let agent_str = agent_id.to_string();

        if decision.should_assist {
            if !episode.cooldown_elapsed(now, cooldown_ms) {
                // Still under pressure but inside the cooldown window: no new
                // request, no event. Persist and return None.
                self.pressure_states.insert(agent_id.clone(), episode);
                return Ok(None);
            }
            episode.last_fired_at_ms = now;
            self.pressure_states.insert(agent_id.clone(), episode);

            // Emit a correlated EventBus event.
            let event = Event {
                id: decentraai_event_bus::EventId::new(),
                topic: Topic::agent(&agent_str),
                source: agent_str.clone(),
                timestamp: now,
                event_type: "agent.pressure.fired".to_string(),
                payload: serde_json::json!({
                    "capability": capability,
                    "score": decision.score,
                    "urgency": serde_json::to_string(&decision.urgency).unwrap_or_default(),
                    "reasons": decision.reasons,
                    "correlation_id": decision.correlation_id,
                }),
                metadata: decentraai_event_bus::EventMetadata {
                    correlation_id: Some(decision.correlation_id.clone()),
                    priority: EventPriority::High,
                    tags: vec![
                        "agent-runtime".to_string(),
                        "pressure".to_string(),
                        "saes-0.5".to_string(),
                    ],
                    ..Default::default()
                },
            };
            // Durable publish: appends to the store AND broadcasts, so the
            // pressure event is observable/traceable (correlation_id) rather
            // than fire-and-forget.
            let _ = self.event_bus.publish(event).await;

            return Ok(Some(CollaborationSignal {
                agent_id: agent_str,
                capability: capability.to_string(),
                reasons: decision.reasons.clone(),
                urgency: decision.urgency,
                correlation_id: decision.correlation_id.clone(),
                cpu_cores: 0,
                ram_mb: 0,
                max_lease_seconds: 30,
            }));
        }

        // Not under pressure. If we were previously assisting, emit a release.
        let was_under = matches!(
            self.pressure_states.get(agent_id).map(|e| e.state),
            Some(AssistState::AssistRequested)
        );
        self.pressure_states.insert(agent_id.clone(), episode);
        if was_under {
            let event = Event {
                id: decentraai_event_bus::EventId::new(),
                topic: Topic::agent(&agent_str),
                source: agent_str.clone(),
                timestamp: now,
                event_type: "agent.pressure.released".to_string(),
                payload: serde_json::json!({
                    "score": decision.score,
                    "correlation_id": decision.correlation_id,
                }),
                metadata: decentraai_event_bus::EventMetadata {
                    correlation_id: prev_corr.clone(),
                    priority: EventPriority::Normal,
                    tags: vec![
                        "agent-runtime".to_string(),
                        "pressure".to_string(),
                        "saes-0.5".to_string(),
                    ],
                    ..Default::default()
                },
            };
            // Durable publish so the release is observable/traceable too.
            let _ = self.event_bus.publish(event).await;
        }

        Ok(None)
    }

    /// SAES 0.2: filter observation to prefer tasks aligned with active goals.
    ///
    /// When the agent has active goals, this method reorders tasks in the
    /// observation so that tasks matching the highest-priority goal appear
    /// first. The DefaultBidPolicy picks the first matching task, so this
    /// effectively makes it prefer goal-aligned work.
    ///
    /// Goal matching is done by checking if the goal description contains
    /// a keyword that matches the task's required_capability. This is a
    /// heuristic — production implementations should use explicit capability
    /// mappings.
    fn filter_observation_by_goals(
        &self,
        obs: &AgentObservation,
        active_goals: &[&AgentGoal],
    ) -> AgentObservation {
        let tasks = match obs
            .hub_state_summary
            .get("tasks")
            .and_then(|t| t.as_array())
        {
            Some(arr) => arr,
            None => return obs.clone(),
        };

        if tasks.is_empty() {
            return obs.clone();
        }

        // Sort goals by priority descending.
        let mut sorted_goals: Vec<&AgentGoal> = active_goals.to_vec();
        sorted_goals.sort_by_key(|b| std::cmp::Reverse(b.priority));

        // Build a priority map: task_id → goal priority (higher = more important).
        // A task matches a goal if the goal description contains a keyword
        // that appears in the task's required_capability.
        let mut task_priorities: std::collections::HashMap<String, u8> =
            std::collections::HashMap::new();

        for task in tasks {
            let task_id = task.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let cap = task
                .get("required_capability")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            for goal in &sorted_goals {
                // Check if the goal description or kind matches the capability.
                let desc_lower = goal.description.to_lowercase();
                let kind_lower = goal.kind.to_lowercase();
                let cap_lower = cap.to_lowercase();

                if desc_lower.contains(&cap_lower)
                    || kind_lower.contains(&cap_lower)
                    || cap_lower.contains(desc_lower.split_whitespace().next().unwrap_or(""))
                {
                    let priority = goal.priority.0;
                    // Keep the highest priority for this task.
                    let entry = task_priorities.entry(task_id.to_string()).or_insert(0);
                    if priority > *entry {
                        *entry = priority;
                    }
                }
            }
        }

        // If no tasks matched any goal, return the original observation.
        if task_priorities.is_empty() {
            return obs.clone();
        }

        // Sort tasks by goal priority descending, then by original order.
        let mut indexed_tasks: Vec<(usize, &serde_json::Value)> =
            tasks.iter().enumerate().collect();
        indexed_tasks.sort_by(|a, b| {
            let pa = task_priorities
                .get(a.1.get("id").and_then(|v| v.as_str()).unwrap_or(""))
                .copied()
                .unwrap_or(0);
            let pb = task_priorities
                .get(b.1.get("id").and_then(|v| v.as_str()).unwrap_or(""))
                .copied()
                .unwrap_or(0);
            pb.cmp(&pa).then(a.0.cmp(&b.0))
        });

        // Rebuild the observation with reordered tasks.
        let sorted_tasks: Vec<serde_json::Value> =
            indexed_tasks.into_iter().map(|(_, t)| t.clone()).collect();
        let mut new_obs = obs.clone();
        new_obs.hub_state_summary = serde_json::json!({
            "tasks": sorted_tasks,
            "available_capabilities": obs.available_capabilities,
        });
        new_obs
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

        // SAES 0.2: create structured goals from initial_goals strings.
        let mut goal_ids = Vec::new();
        for goal_desc in &config.initial_goals {
            let kind = if goal_desc.contains("serve") || goal_desc.contains("request") {
                "serve_request".to_string()
            } else if goal_desc.contains("code") || goal_desc.contains("write") {
                "code_generation".to_string()
            } else {
                "general".to_string()
            };
            let mut goal = AgentGoal::new(
                id.clone(),
                goal_desc.clone(),
                kind,
                GoalPriority::NORMAL,
                now,
            );
            // Auto-activate initial goals.
            if let Err(e) = goal.transition_to(GoalState::Active, now) {
                tracing::debug!(agent = %id, error = %e, "saes: failed to activate initial goal");
            }
            goal_ids.push(goal.id.clone());
            if let Err(e) = self.goal_store.add(goal).await {
                tracing::debug!(agent = %id, error = %e, "saes: failed to store initial goal");
            }
        }

        let state = AgentState {
            agent_id: id.clone(),
            status: AgentStatus::Initializing,
            config,
            current_goals: goal_ids,
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

        // SAES 0.2: check deadline monitoring — auto-fail overdue goals.
        let now = now_ms();
        let active_goals = self.goal_store.list_by_agent(agent_id).await;
        for goal in &active_goals {
            if goal.is_overdue(now) {
                tracing::info!(
                    agent = %agent_id,
                    goal = %goal.id,
                    deadline = goal.deadline.unwrap_or(0),
                    "saes: goal overdue, auto-failing"
                );
                if let Ok(mut g) = self.goal_store.get(&goal.id).await {
                    let _ = g.fail("deadline exceeded".to_string(), now);
                    let _ = self.goal_store.update(g).await;
                }
            }
        }

        // SAES 0.2: goal-aware observation filtering.
        // If there are active goals, filter tasks to prefer those aligned
        // with the highest-priority goal. This makes the policy naturally
        // favor goal-aligned tasks without needing goal-awareness.
        let active_goals_filtered: Vec<_> = active_goals
            .iter()
            .filter(|g| g.state == GoalState::Active)
            .collect();

        let effective_obs = if !active_goals_filtered.is_empty() {
            self.filter_observation_by_goals(observation, &active_goals_filtered)
        } else {
            observation.clone()
        };

        let mut decision = policy.decide(&effective_obs);

        // SAES 0.2: adapt decision based on behavior profile.
        let profile = self
            .behavior_store
            .get_or_create(&agent_id.to_string())
            .await;

        if decision.decision_type == DecisionType::Bid {
            // Extract the task's required capability from the decision context.
            if let Some(task_cap) = decision
                .context
                .get("task")
                .and_then(|t| t.get("required_capability"))
                .and_then(|v| v.as_str())
            {
                let (use_strategy, conf, reason) = profile.should_use_strategy(task_cap);
                if !use_strategy {
                    // Override: don't bid on avoided strategies.
                    decision.decision_type = DecisionType::Wait;
                    decision.reasoning =
                        format!("{} [saes: overridden — {}]", decision.reasoning, reason);
                    decision.confidence = conf;
                    tracing::debug!(
                        agent = %agent_id,
                        capability = task_cap,
                        reason = %reason,
                        "saes: bid overridden to wait (avoided strategy)"
                    );
                } else if conf > 0.6 {
                    // Boost confidence for preferred strategies.
                    decision.confidence = (decision.confidence + conf) / 2.0;
                    decision.reasoning = format!(
                        "{} [saes: preferred strategy — {}]",
                        decision.reasoning, reason
                    );
                }
            }
        }

        if let Some(mut s) = self.agents.get_mut(agent_id) {
            s.last_active_at = now;
            s.updated_at = now;
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
            crate::DecisionType::Learn | crate::DecisionType::Reflect => ActionType::MemoryWrite,
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
        let now = now_ms();

        // SAES 0.4: check if this action was linked to a collective goal sub-goal.
        // We can check if the action parameters contain a `sub_goal_id`.
        let sub_goal_id = action
            .parameters
            .get("sub_goal_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // SAES 0.2: build structured outcome and compute learning effect.
        // Use the task's required_capability as the action_kind for behavior tracking,
        // so the profile tracks success/failure per capability (what matters for adaptation).
        let fallback_kind = format!("{:?}", action.action_type);
        let action_kind = action
            .parameters
            .get("context")
            .and_then(|c| c.get("task"))
            .and_then(|t| t.get("required_capability"))
            .and_then(|v| v.as_str())
            .unwrap_or(&fallback_kind);
        let action_outcome = ActionOutcome::from_action_result(
            action.agent_id.clone(), // action_id
            agent_id.clone(),
            None, // goal_id — linked below if we find an active goal
            outcome,
            action_kind,
            now,
        );

        // Find active goals for this agent to link to the outcome.
        let active_goals = self.goal_store.list_by_agent(agent_id).await;
        let mut active_goals_filtered: Vec<_> = active_goals
            .iter()
            .filter(|g| g.state == GoalState::Active)
            .cloned()
            .collect();

        // Link the outcome to the highest-priority active goal (not FIFO).
        // Sort by priority descending so the first element is the most important.
        active_goals_filtered.sort_by_key(|b| std::cmp::Reverse(b.priority));
        let mut outcome_with_goal = action_outcome;
        if let Some(goal) = active_goals_filtered.first() {
            outcome_with_goal.goal_id = Some(goal.id.clone());
        }

        // Compute learning effect (pure function).
        let effect = compute_learning_effect(&outcome_with_goal, &active_goals_filtered, now);

        // Apply goal transitions.
        for (goal_id, new_state, failure_reason) in &effect.goal_transitions {
            if let Ok(mut goal) = self.goal_store.get(goal_id).await {
                // Set failure reason before transitioning if failing.
                if *new_state == GoalState::Failed
                    && let Some(reason) = failure_reason
                {
                    goal.failure_reason = Some(reason.clone());
                }
                if let Err(e) = goal.transition_to(*new_state, now) {
                    tracing::debug!(
                        agent = %agent_id,
                        goal = %goal_id,
                        error = %e,
                        "saes: goal transition failed"
                    );
                } else {
                    if let Err(e) = self.goal_store.update(goal).await {
                        tracing::debug!(
                            agent = %agent_id,
                            goal = %goal_id,
                            error = %e,
                            "saes: goal update failed"
                        );
                    }
                }
            }
        }

        // Apply goal progress updates.
        for (goal_id, new_progress) in &effect.goal_progress_updates {
            if let Ok(mut goal) = self.goal_store.get(goal_id).await {
                goal.set_progress(*new_progress, now);
                if let Err(e) = self.goal_store.update(goal).await {
                    tracing::debug!(
                        agent = %agent_id,
                        goal = %goal_id,
                        error = %e,
                        "saes: goal progress update failed"
                    );
                }
            }
        }

        // Update behavior profile.
        let mut profile = self
            .behavior_store
            .get_or_create(&agent_id.to_string())
            .await;
        profile.incorporate(&effect.entry);

        // SAES 0.4: propagate progress to collective goals.
        if let Some(sid) = sub_goal_id {
            let cg_list = self.collective_goal_store.list_all().await;
            for mut cg in cg_list {
                if let Some(sg) = cg.sub_goals.get_mut(&sid) {
                    // Map AgentGoal state/progress to SubGoal
                    for (gid, progress) in &effect.goal_progress_updates {
                        if gid == &sg.id {
                            sg.set_progress(*progress, now);
                        }
                    }

                    // Check for completion
                    for (gid, state, _) in &effect.goal_transitions {
                        if gid == &sg.id {
                            if *state == GoalState::Completed {
                                sg.complete(now);
                            } else if *state == GoalState::Failed {
                                sg.fail("Learning outcome failure".to_string(), now);
                            }
                        }
                    }

                    cg.recompute_progress(now);
                    let _ = self.collective_goal_store.update(cg).await;
                    break;
                }
            }
        }

        // Update goal stats.
        let completed = self
            .goal_store
            .count_by_state(agent_id, GoalState::Completed)
            .await;
        let failed = self
            .goal_store
            .count_by_state(agent_id, GoalState::Failed)
            .await;
        let abandoned = self
            .goal_store
            .count_by_state(agent_id, GoalState::Abandoned)
            .await;
        profile.update_goal_stats(completed as u64, failed as u64, abandoned as u64, now);
        self.behavior_store.save(profile).await;

        // Update agent metrics from learning effect.
        if let Some(mut s) = self.agents.get_mut(agent_id) {
            let (dc, df, dr) = effect.metrics_delta;
            s.metrics.tasks_completed = s.metrics.tasks_completed.saturating_add(dc);
            s.metrics.tasks_failed = s.metrics.tasks_failed.saturating_add(df);
            s.metrics.total_reward_earned = s.metrics.total_reward_earned.saturating_add(dr);
            if let Some(d) = outcome.reputation_delta {
                s.metrics.reputation_score += d;
            }
            s.last_active_at = now;
            s.updated_at = now;
        }

        // Emit structured learning event.
        let event = Event {
            id: decentraai_event_bus::EventId::new(),
            topic: Topic::agent(&agent_id.to_string()),
            source: agent_id.clone(),
            timestamp: now,
            event_type: "agent.learn".to_string(),
            payload: serde_json::json!({
                "action_type": format!("{:?}", action.action_type),
                "success": outcome.success,
                "reward": outcome.reward,
                "reputation_delta": outcome.reputation_delta,
                "lesson": effect.entry.lesson,
                "goal_id": effect.entry.goal_id,
                "goal_transitions": effect.goal_transitions.iter().map(|(id, s, _)| (id, s.to_string())).collect::<Vec<_>>(),
            }),
            metadata: decentraai_event_bus::EventMetadata {
                priority: EventPriority::Normal,
                tags: vec![
                    "agent-runtime".to_string(),
                    "learn".to_string(),
                    "saes-0.2".to_string(),
                ],
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

    async fn retire(&self, agent_id: &AgentId, reason: String) -> Result<(), AgentRuntimeError> {
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
    async fn build(&self, agent_id: &ProtocolAgentId, capabilities: &[String]) -> AgentObservation {
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
        assert_eq!(
            r.get_state(&h.agent_id).await.unwrap().status,
            AgentStatus::Paused
        );
        r.resume(&h.agent_id).await.unwrap();
        assert_eq!(
            r.get_state(&h.agent_id).await.unwrap().status,
            AgentStatus::Ready
        );
        r.stop(&h.agent_id).await.unwrap();
        assert_eq!(
            r.get_state(&h.agent_id).await.unwrap().status,
            AgentStatus::Stopped
        );
    }

    #[tokio::test]
    async fn retire_sets_status() {
        let (bus,) = runtime();
        let obs = Arc::new(StaticObservationBuilder::empty());
        let r = LocalAgentRuntime::new(bus, obs);
        let h = r.spawn(cfg(vec!["a".into()])).await.unwrap();
        r.retire(&h.agent_id, "test".to_string()).await.unwrap();
        assert_eq!(
            r.get_state(&h.agent_id).await.unwrap().status,
            AgentStatus::Retired
        );
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
        assert!(
            observation
                .available_capabilities
                .contains(&"quantum_simulation_v0".to_string())
        );
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
                    if ev.event_type == "agent.action" && ev.source == h.agent_id {
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
            params
                .get("reasoning")
                .and_then(|v| v.as_str())
                .map(|s| s.contains("task t9")),
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
        let result = r
            .get_metrics(&decentraai_protocol::AgentId::from("ghost"))
            .await;
        assert!(matches!(result, Err(crate::AgentRuntimeError::NotFound(_))));
    }
}
