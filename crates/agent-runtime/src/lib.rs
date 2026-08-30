use async_trait::async_trait;
use decentraai_agent_society::AgentId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentRuntimeError {
    #[error("Agent not found: {0}")]
    NotFound(AgentId),
    #[error("Agent already exists: {0}")]
    AlreadyExists(AgentId),
    #[error("Invalid state transition: {0}")]
    InvalidTransition(String),
    #[error("Policy violation: {0}")]
    PolicyViolation(String),
    #[error("Resource exhausted: {0}")]
    ResourceExhausted(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentStatus {
    Creating,
    Initializing,
    Ready,
    Running,
    Paused,
    Stopping,
    Stopped,
    Error,
    Retired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub agent_id: AgentId,
    pub name: String,
    pub capabilities: Vec<String>,
    pub initial_goals: Vec<String>,
    pub initial_memory: Option<serde_json::Value>,
    pub policy_overrides: Option<serde_json::Value>,
    pub resource_limits: ResourceLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceLimits {
    pub max_memory_mb: Option<u64>,
    pub max_cpu_percent: Option<u8>,
    pub max_concurrent_tasks: Option<u32>,
    pub max_memory_entries: Option<usize>,
    pub max_token_budget: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub agent_id: AgentId,
    pub status: AgentStatus,
    pub config: AgentConfig,
    pub current_goals: Vec<String>,
    pub current_beliefs: serde_json::Value,
    pub resource_usage: ResourceUsage,
    pub metrics: AgentMetrics,
    pub created_at: u64,
    pub updated_at: u64,
    pub last_active_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResourceUsage {
    pub memory_mb: u64,
    pub cpu_percent: f32,
    pub active_tasks: u32,
    pub memory_entries: usize,
    pub token_usage: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentMetrics {
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub bids_placed: u64,
    pub proposals_made: u64,
    pub teams_formed: u64,
    pub total_reward_earned: u64,
    pub reputation_score: f32,
    pub trust_scores_given: u32,
    pub trust_scores_received: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentObservation {
    pub agent_id: AgentId,
    pub timestamp: u64,
    pub hub_state_summary: serde_json::Value,
    pub society_state_summary: serde_json::Value,
    pub personal_memory_summary: serde_json::Value,
    pub arena_state_summary: Option<serde_json::Value>,
    pub queue_depth: usize,
    pub available_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDecision {
    pub agent_id: AgentId,
    pub timestamp: u64,
    pub decision_type: DecisionType,
    pub reasoning: String,
    pub confidence: f32,
    pub expected_outcome: serde_json::Value,
    pub context: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DecisionType {
    Bid,
    Propose,
    FormTeam,
    Execute,
    PublishTask,
    Wait,
    Publish,
    Search,
    Learn,
    Reflect,
    Dream,
    Rest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAction {
    pub agent_id: AgentId,
    pub timestamp: u64,
    pub action_type: ActionType,
    pub parameters: serde_json::Value,
    pub result: Option<ActionResult>,
    pub observation: Option<AgentObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActionType {
    HubState,
    HubBid,
    HubPropose,
    HubFormTeam,
    HubExecute,
    HubPublish,
    SocietyState,
    SocietyTrust,
    SocietyReputation,
    SocietyRelationships,
    MemoryRead,
    MemoryWrite,
    MemorySearch,
    MemorySnapshot,
    ArenaAct,
    ArenaState,
    Discover,
    ExecuteDecision,
    Decide,
    ComputeRequest,
    Embeddings,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    pub success: bool,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub evidence_id: Option<String>,
    pub reward: Option<u64>,
    pub reputation_delta: Option<f32>,
}

#[async_trait]
pub trait AgentRuntime: Send + Sync {
    async fn spawn(&self, config: AgentConfig) -> Result<AgentHandle, AgentRuntimeError>;
    async fn get_state(&self, agent_id: &AgentId) -> Result<AgentState, AgentRuntimeError>;
    async fn observe(&self, agent_id: &AgentId) -> Result<AgentObservation, AgentRuntimeError>;
    async fn decide(
        &self,
        agent_id: &AgentId,
        observation: &AgentObservation,
    ) -> Result<AgentDecision, AgentRuntimeError>;
    async fn act(
        &self,
        agent_id: &AgentId,
        decision: &AgentDecision,
    ) -> Result<AgentAction, AgentRuntimeError>;
    async fn learn(
        &self,
        agent_id: &AgentId,
        action: &AgentAction,
        outcome: &ActionResult,
    ) -> Result<(), AgentRuntimeError>;
    async fn pause(&self, agent_id: &AgentId) -> Result<(), AgentRuntimeError>;
    async fn resume(&self, agent_id: &AgentId) -> Result<(), AgentRuntimeError>;
    async fn stop(&self, agent_id: &AgentId) -> Result<(), AgentRuntimeError>;
    async fn retire(&self, agent_id: &AgentId, reason: String) -> Result<(), AgentRuntimeError>;
    async fn list_agents(&self) -> Result<Vec<AgentId>, AgentRuntimeError>;
    async fn get_metrics(&self, agent_id: &AgentId) -> Result<AgentMetrics, AgentRuntimeError>;
}

pub struct AgentHandle {
    pub agent_id: AgentId,
    pub status: AgentStatus,
}

impl AgentHandle {
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            status: AgentStatus::Creating,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSnapshot {
    pub agent_id: AgentId,
    pub state: AgentState,
    pub memory_snapshot: serde_json::Value,
    pub goals_snapshot: Vec<String>,
    pub beliefs_snapshot: serde_json::Value,
    pub metrics_snapshot: AgentMetrics,
    pub timestamp: u64,
}

#[async_trait]
pub trait AgentPersistence: Send + Sync {
    async fn save(&self, snapshot: &AgentSnapshot) -> Result<(), AgentRuntimeError>;
    async fn load(&self, agent_id: &AgentId) -> Result<Option<AgentSnapshot>, AgentRuntimeError>;
    async fn delete(&self, agent_id: &AgentId) -> Result<(), AgentRuntimeError>;
    async fn list(&self) -> Result<Vec<AgentId>, AgentRuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_config_serialization() {
        let config = AgentConfig {
            agent_id: AgentId::new(),
            name: "test-agent".to_string(),
            capabilities: vec!["analysis".to_string(), "coding".to_string()],
            initial_goals: vec!["maximize_reward".to_string()],
            initial_memory: None,
            policy_overrides: None,
            resource_limits: ResourceLimits::default(),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.agent_id, deserialized.agent_id);
        assert_eq!(config.capabilities, deserialized.capabilities);
    }

    #[test]
    fn test_agent_state_defaults() {
        let config = AgentConfig {
            agent_id: AgentId::new(),
            name: "test".to_string(),
            capabilities: vec![],
            initial_goals: vec![],
            initial_memory: None,
            policy_overrides: None,
            resource_limits: ResourceLimits::default(),
        };

        let state = AgentState {
            agent_id: config.agent_id.clone(),
            status: AgentStatus::Creating,
            config,
            current_goals: vec![],
            current_beliefs: serde_json::json!({}),
            resource_usage: ResourceUsage::default(),
            metrics: AgentMetrics::default(),
            created_at: 0,
            updated_at: 0,
            last_active_at: 0,
        };

        assert_eq!(state.status, AgentStatus::Creating);
        assert_eq!(state.metrics.tasks_completed, 0);
    }
}

// Sprint 0.1: default in-memory implementation of the AgentRuntime trait.
pub mod local;

// Sprint 0.1: lightweight per-agent decision policy (defined here
// to keep `agent-runtime` decoupled from `policy-engine`).
pub mod policy;

// Sprint 0.1: end-to-end genericity proof, isolated from any
// specific registry or policy-engine crate.
#[cfg(test)]
mod capability_proof;

// SAES 0.2: structured agent evolution system (goals, outcomes, learning, adaptation).
pub mod saes;

/// Integration test: full spawn → observe → decide → act → learn pipeline
/// with a realistic daemon-like setup (StaticObservationBuilder + DefaultBidPolicy).
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::local::{LocalAgentRuntime, StaticObservationBuilder};
    use std::sync::Arc;

    #[tokio::test]
    async fn full_lifecycle_pipeline() {
        let event_store = Arc::new(decentraai_event_bus::InMemoryEventStore::new(1000));
        let event_bus = Arc::new(decentraai_event_bus::EventBus::new(event_store));

        // Simulate a node with two agents and available capabilities.
        // Include open tasks so DefaultBidPolicy can find matching bids.
        let obs_builder = Arc::new(StaticObservationBuilder {
            hub_state: serde_json::json!({
                "available_capabilities": ["Coding", "Chat", "Analysis"],
                "node_id": "dca-test",
                "tasks": [
                    {
                        "id": "task-001",
                        "status": "open",
                        "required_capability": "Chat",
                        "reward": 500
                    },
                    {
                        "id": "task-002",
                        "status": "open",
                        "required_capability": "Coding",
                        "reward": 800
                    }
                ]
            }),
            society_state: serde_json::json!({}),
            arena_state: None,
            personal_memory: serde_json::json!({}),
        });

        let runtime = LocalAgentRuntime::new(event_bus, obs_builder);

        // 1. SPAWN two agents
        let config_a = AgentConfig {
            agent_id: "dca-test:generalist".to_string(),
            name: "Generalist".to_string(),
            capabilities: vec!["Chat".to_string(), "Analysis".to_string()],
            initial_goals: vec!["serve requests".to_string()],
            initial_memory: None,
            policy_overrides: None,
            resource_limits: ResourceLimits::default(),
        };
        let config_b = AgentConfig {
            agent_id: "dca-test:coder".to_string(),
            name: "Coder".to_string(),
            capabilities: vec!["Coding".to_string()],
            initial_goals: vec!["write code".to_string()],
            initial_memory: None,
            policy_overrides: None,
            resource_limits: ResourceLimits::default(),
        };

        let handle_a = runtime.spawn(config_a).await.unwrap();
        let handle_b = runtime.spawn(config_b).await.unwrap();
        assert_eq!(handle_a.status, AgentStatus::Ready);
        assert_eq!(handle_b.status, AgentStatus::Ready);

        // 2. OBSERVE both agents
        let obs_a = runtime
            .observe(&"dca-test:generalist".to_string())
            .await
            .unwrap();
        assert_eq!(obs_a.agent_id, "dca-test:generalist");
        assert!(!obs_a.available_capabilities.is_empty());

        let obs_b = runtime
            .observe(&"dca-test:coder".to_string())
            .await
            .unwrap();
        assert_eq!(obs_b.agent_id, "dca-test:coder");

        // 3. DECIDE — DefaultBidPolicy bids when capability matches
        let decision_a = runtime
            .decide(&"dca-test:generalist".to_string(), &obs_a)
            .await
            .unwrap();
        assert_eq!(decision_a.agent_id, "dca-test:generalist");
        // Generalist has Chat+Analysis, hub has Coding+Chat+Analysis → should bid
        assert_eq!(decision_a.decision_type, DecisionType::Bid);

        let decision_b = runtime
            .decide(&"dca-test:coder".to_string(), &obs_b)
            .await
            .unwrap();
        assert_eq!(decision_b.decision_type, DecisionType::Bid);

        // 4. ACT
        let action_a = runtime
            .act(&"dca-test:generalist".to_string(), &decision_a)
            .await
            .unwrap();
        assert_eq!(action_a.action_type, ActionType::HubBid);
        // act() creates the action but doesn't execute it (execution is the
        // daemon's responsibility). Result is None until execute() is called.
        assert!(action_a.result.is_none());

        let action_b = runtime
            .act(&"dca-test:coder".to_string(), &decision_b)
            .await
            .unwrap();
        assert_eq!(action_b.action_type, ActionType::HubBid);

        // 5. LEARN — metrics should increment
        let metrics_before = runtime
            .get_metrics(&"dca-test:generalist".to_string())
            .await
            .unwrap();
        let outcome = ActionResult {
            success: true,
            output: Some(serde_json::json!({"result": "ok"})),
            error: None,
            evidence_id: Some("ev-001".to_string()),
            reward: Some(100),
            reputation_delta: Some(0.1),
        };
        runtime
            .learn(&"dca-test:generalist".to_string(), &action_a, &outcome)
            .await
            .unwrap();
        let metrics_after = runtime
            .get_metrics(&"dca-test:generalist".to_string())
            .await
            .unwrap();
        assert_eq!(
            metrics_after.tasks_completed,
            metrics_before.tasks_completed + 1
        );

        // 6. Verify full agent list
        let agents = runtime.list_agents().await.unwrap();
        assert_eq!(agents.len(), 2);
        assert!(agents.contains(&"dca-test:generalist".to_string()));
        assert!(agents.contains(&"dca-test:coder".to_string()));
    }

    #[tokio::test]
    async fn lifecycle_multiple_cycles() {
        let event_store = Arc::new(decentraai_event_bus::InMemoryEventStore::new(1000));
        let event_bus = Arc::new(decentraai_event_bus::EventBus::new(event_store));
        let obs_builder = Arc::new(StaticObservationBuilder::empty());
        let runtime = LocalAgentRuntime::new(event_bus, obs_builder);

        let config = AgentConfig {
            agent_id: "dca-loop".to_string(),
            name: "Loop Agent".to_string(),
            capabilities: vec!["Chat".to_string()],
            initial_goals: vec!["serve".to_string()],
            initial_memory: None,
            policy_overrides: None,
            resource_limits: ResourceLimits::default(),
        };
        runtime.spawn(config).await.unwrap();

        // Run 5 lifecycle cycles
        for i in 0..5 {
            let obs = runtime.observe(&"dca-loop".to_string()).await.unwrap();
            let decision = runtime.decide(&"dca-loop".to_string(), &obs).await.unwrap();
            let action = runtime
                .act(&"dca-loop".to_string(), &decision)
                .await
                .unwrap();
            let outcome = ActionResult {
                success: true,
                output: None,
                error: None,
                evidence_id: None,
                reward: Some(10 * (i + 1)),
                reputation_delta: None,
            };
            runtime
                .learn(&"dca-loop".to_string(), &action, &outcome)
                .await
                .unwrap();
        }

        let metrics = runtime.get_metrics(&"dca-loop".to_string()).await.unwrap();
        assert_eq!(metrics.tasks_completed, 5);
        assert_eq!(metrics.total_reward_earned, 150); // 10+20+30+40+50
    }
}
