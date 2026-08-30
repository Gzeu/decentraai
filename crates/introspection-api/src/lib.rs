use async_trait::async_trait;
use dashmap::DashMap;
use decentraai_agent_runtime::{AgentRuntime, AgentState, AgentObservation, AgentDecision, AgentAction, AgentMetrics, AgentConfig, AgentId};
use decentraai_agent_personal_memory::PersonalMemoryStore;
use decentraai_agent_society::SocietyState;
use decentraai_hub::HubState;
use decentraai_arena::ArenaWorld;
use decentraai_compute::QuotaLedger;
use decentraai_tokens::ConsumerKeyStore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum IntrospectionError {
    #[error("Agent not found: {0}")]
    NotFound(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentFullState {
    pub agent_state: serde_json::Value,
    pub metrics: serde_json::Value,
    pub memory: serde_json::Value,
    pub society: serde_json::Value,
    pub arena: serde_json::Value,
    pub quota: serde_json::Value,
    pub goals: Vec<String>,
    pub beliefs: serde_json::Value,
    pub dreams: Vec<String>,
    pub reflections: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentSummary {
    pub agent_id: String,
    pub status: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub metrics_summary: AgentMetricsSummary,
    pub memory_summary: MemorySummary,
    pub society_summary: SocietySummary,
    pub last_active: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentMetricsSummary {
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub success_rate: f32,
    pub total_reward: u64,
    pub reputation: f32,
    pub trust_given: u32,
    pub trust_received: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct MemorySummary {
    pub total_entries: usize,
    pub categories: HashMap<String, usize>,
    pub recent_experiences: usize,
    pub recent_lessons: usize,
    pub relationships: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct SocietySummary {
    pub contributions: u64,
    pub outcomes: u64,
    pub reputation_events: u64,
    pub trust_scores: HashMap<String, f32>,
    pub relationships: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentActivity {
    pub agent_id: String,
    pub timestamp: u64,
    pub action: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentSearchQuery {
    pub agent_id: Option<String>,
    pub capability: Option<String>,
    pub status: Option<String>,
    pub since_timestamp: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct NetworkTopology {
    pub agents: Vec<AgentNode>,
    pub connections: Vec<AgentConnection>,
    pub clusters: Vec<AgentCluster>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentNode {
    pub agent_id: String,
    pub name: String,
    pub status: String,
    pub capabilities: Vec<String>,
    pub reputation: f32,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentConnection {
    pub from: String,
    pub to: String,
    pub relationship_type: String,
    pub trust_score: f32,
    pub interaction_count: u32,
    pub last_interaction: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentCluster {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
    pub cohesion: f32,
    pub shared_goals: Vec<String>,
}

#[derive(Debug, Error)]
pub enum IntrospectionError {
    #[error("Agent not found: {0}")]
    NotFound(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Internal error: {0}")]
    Internal(String),
}

#[async_trait]
pub trait IntrospectionProvider: Send + Sync {
    async fn get_agent_full_state(&self, agent_id: &str) -> Result<AgentFullState, IntrospectionError>;
    async fn get_agent_summary(&self, agent_id: &str) -> Result<AgentSummary, IntrospectionError>;
    async fn list_agents(&self, query: AgentSearchQuery) -> Result<Vec<AgentSummary>, IntrospectionError>;
    async fn get_network_topology(&self) -> Result<NetworkTopology, IntrospectionError>;
    async fn get_agent_activity(&self, agent_id: &str, since: u64, limit: usize) -> Result<Vec<AgentActivity>, IntrospectionError>;
    async fn get_agent_relationships(&self, agent_id: &str) -> Result<Vec<AgentConnection>, IntrospectionError>;
    async fn get_trust_network(&self) -> Result<Vec<AgentConnection>, IntrospectionError>;
    async fn get_collective_memory(&self) -> Result<serde_json::Value, IntrospectionError>;
    async fn search_memory(&self, agent_id: &str, query: &str, limit: usize) -> Result<serde_json::Value, IntrospectionError>;
    async fn get_agent_dreams(&self, agent_id: &str) -> Result<Vec<String>, IntrospectionError>;
    async fn get_agent_beliefs(&self, agent_id: &str) -> Result<serde_json::Value, IntrospectionError>;
    async fn get_agent_lessons(&self, agent_id: &str) -> Result<Vec<String>, IntrospectionError>;
}

pub struct IntrospectionService {
    agent_runtime: Arc<dyn AgentRuntime>,
    personal_memory: Arc<PersonalMemoryStore>,
    society: Arc<RwLock<SocietyState>>,
    hub: Arc<tokio::sync::Mutex<HubState>>,
    arena: Arc<RwLock<ArenaWorld>>,
    quota_ledger: Arc<Mutex<QuotaLedger>>,
    consumer_keys: Arc<ConsumerKeyStore>,
    agent_metadata: Arc<DashMap<String, AgentMetadata>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Clone, Default)]
pub struct AgentMetadata {
    pub agent_id: String,
    pub name: String,
    pub created_at: u64,
    pub last_active: u64,
    pub total_tasks: u64,
    pub total_reward: u64,
    pub tags: Vec<String>,
}

impl IntrospectionService {
    pub fn new(
        agent_runtime: Arc<dyn AgentRuntime>,
        personal_memory: Arc<PersonalMemoryStore>,
        society: Arc<RwLock<SocietyState>>,
        hub: Arc<tokio::sync::Mutex<HubState>>,
        arena: Arc<RwLock<ArenaWorld>>,
        quota_ledger: Arc<Mutex<QuotaLedger>>,
        consumer_keys: Arc<ConsumerKeyStore>,
    ) -> Self {
        Self {
            agent_runtime,
            personal_memory,
            society,
            hub,
            arena: Arc::new(RwLock::new(ArenaWorld::new())),
            quota_ledger,
            consumer_keys: Arc::new(ConsumerKeyStore::new()),
            agent_metadata: Arc::new(DashMap::new()),
        }
    }

    async fn get_agent_state(&self, agent_id: &str) -> Result<AgentState, IntrospectionError> {
        self.agent_runtime.get_state(&AgentId::from(agent_id.to_string()))
            .await
            .map_err(|_| IntrospectionError::NotFound(agent_id.to_string()))
    }

    async fn get_personal_memory(&self, agent_id: &str) -> Result<serde_json::Value, IntrospectionError> {
        let store = self.personal_memory.as_ref();
        let snapshot = store.snapshot(agent_id).await
            .map_err(|_| IntrospectionError::Internal("Failed to get memory snapshot".into()))?;
        Ok(serde_json::to_value(snapshot).unwrap_or(serde_json::json!({})))
    }

    async fn get_society_snapshot(&self, agent_id: &str) -> Result<serde_json::Value, IntrospectionError> {
        let society = self.society.read().await;
        let snapshot = decentraai_agent_society::mcp::build_society_state_response(&society, agent_id);
        Ok(snapshot)
    }

    async fn get_arena_snapshot(&self) -> Result<serde_json::Value, IntrospectionError> {
        let arena = self.arena.read().await;
        Ok(serde_json::json!({
            "tick": arena.tick,
            "width": arena.width,
            "height": arena.height,
            "agents": arena.agents.len(),
            "events": arena.events.len(),
        }))
    }

    async fn get_quota_snapshot(&self, agent_id: &str) -> Result<serde_json::Value, IntrospectionError> {
        let ledger = self.quota_ledger.lock().unwrap();
        if let Some(account) = ledger.account(&agent_id.to_string()) {
            Ok(serde_json::json!({
                "available": account.available,
                "consumed": account.consumed,
                "reserved": account.reserved,
                "earned": account.earned,
                "ceiling": account.ceiling,
            }))
        } else {
            Ok(serde_json::json!({
                "available": 0,
                "consumed": 0,
                "reserved": 0,
                "earned": 0,
                "ceiling": 0,
            }))
        }
    }
}

#[async_trait]
impl IntrospectionProvider for IntrospectionService {
    async fn get_agent_full_state(&self, agent_id: &str) -> Result<AgentFullState, IntrospectionError> {
        let state = self.get_agent_state(agent_id).await?;
        let memory = self.get_personal_memory(agent_id).await?;
        let society = self.get_society_snapshot(agent_id).await?;
        let arena = self.get_arena_snapshot().await?;
        let quota = self.get_quota_snapshot(agent_id).await?;

        let state = self.get_agent_state(agent_id).await?;
        let metrics = serde_json::to_value(&state.metrics).unwrap_or_default();
        let state_val = serde_json::to_value(&state).unwrap_or_default();

        let goals = state.current_goals.clone();
        let beliefs = state.current_beliefs.clone();

        Ok(AgentFullState {
            agent_state: state_val,
            metrics,
            memory: serde_json::json!({}),
            society,
            arena: serde_json::json!({}),
            quota: serde_json::json!({}),
            goals,
            beliefs,
            dreams: vec![],
            reflections: vec![],
        })
    }

    async fn get_agent_summary(&self, agent_id: &str) -> Result<AgentSummary, IntrospectionError> {
        let state = self.get_agent_state(agent_id).await?;
        let memory = self.get_personal_memory(agent_id).await?;
        let society = self.get_society_snapshot(agent_id).await?;

        let metrics = AgentMetricsSummary {
            tasks_completed: state.metrics.tasks_completed,
            tasks_failed: state.metrics.tasks_failed,
            success_rate: if state.metrics.tasks_completed + state.metrics.tasks_failed > 0 {
                state.metrics.tasks_completed as f32 / (state.metrics.tasks_completed + state.metrics.tasks_failed) as f32
            } else { 0.0 },
            total_reward: state.metrics.total_reward,
            reputation: state.metrics.reputation_score,
            trust_given: state.metrics.trust_scores_given,
            trust_received: state.metrics.trust_scores_received,
        };

        let mem_val: serde_json::Value = serde_json::from_str(&memory.to_string()).unwrap_or_default();
        let categories = mem_val.as_object().map(|obj| {
            obj.iter().map(|(k, v)| (k.clone(), v.as_array().map(|a| a.len()).unwrap_or(1))).collect()
        }).unwrap_or_default();

        let memory_summary = MemorySummary {
            total_entries: mem_val.as_object().map(|o| o.len()).unwrap_or(0),
            categories,
            recent_experiences: 0,
            recent_lessons: 0,
            relationships: 0,
        };

        let society_val: serde_json::Value = serde_json::from_str(&serde_json::to_string(&serde_json::json!({})).unwrap()).unwrap();
        let society_summary = SocietySummary {
            contributions: 0,
            outcomes: 0,
            reputation_events: 0,
            trust_scores: HashMap::new(),
            relationships: 0,
        };

        Ok(AgentSummary {
            agent_id: agent_id.to_string(),
            status: format!("{:?}", state.status),
            name: state.config.name.clone(),
            capabilities: state.config.capabilities.clone(),
            metrics_summary: metrics,
            memory_summary,
            society_summary,
            last_active: state.last_active_at,
        })
    }

    async fn list_agents(&self, query: AgentSearchQuery) -> Result<Vec<AgentSummary>, IntrospectionError> {
        let agents = self.agent_runtime.list_agents().await
            .map_err(|_| IntrospectionError::Internal("Failed to list agents".into()))?;

        let mut results = Vec::new();
        for agent_id in agents {
            if let Some(cap) = query.capability.as_ref() {
                let state = self.get_agent_state(&agent_id).await?;
                if !state.config.capabilities.iter().any(|c| c == cap) {
                    continue;
                }
            }
            if let Some(status) = &query.status {
                let state = self.get_agent_state(&agent_id).await?;
                if format!("{:?}", state.status) != *status {
                    continue;
                }
            }

            if let Ok(summary) = self.get_agent_summary(&agent_id).await {
                results.push(summary);
            }
        }

        if let Some(limit) = query.limit {
            results.truncate(limit);
        }

        Ok(results)
    }

    async fn get_network_topology(&self) -> Result<NetworkTopology, IntrospectionError> {
        let agents = self.agent_runtime.list_agents().await
            .map_err(|_| IntrospectionError::Internal("Failed to list agents".into()))?;

        let mut nodes = Vec::new();
        let mut connections = Vec::new();

        for agent_id in &agents {
            if let Ok(state) = self.get_agent_state(&agent_id).await {
                nodes.push(AgentNode {
                    agent_id: agent_id.clone(),
                    name: state.config.name.clone(),
                    status: format!("{:?}", state.status),
                    capabilities: state.config.capabilities.clone(),
                    reputation: 0.0,
                    location: None,
                });
            }
        }

        // Build connections from society relationships
        let society = self.society.read().await;
        for (observer, subjects) in &society.relationships {
            for (subject, relationships) in subjects {
                for rel in relationships {
                    connections.push(AgentConnection {
                        from: observer.clone(),
                        to: subject.clone(),
                        relationship_type: format!("{:?}", rel.kind),
                        trust_score: rel.strength,
                        interaction_count: 1,
                        last_interaction: rel.tick,
                    });
                }
            }
        }

        // Simple clustering by shared capabilities
        let mut clusters = Vec::new();

        Ok(NetworkTopology {
            agents: nodes,
            connections,
            clusters,
        })
    }

    async fn get_agent_activity(&self, agent_id: &str, since: u64, limit: usize) -> Result<Vec<AgentActivity>, IntrospectionError> {
        // This would query event store for agent activities
        Ok(vec![])
    }

    async fn get_agent_relationships(&self, agent_id: &str) -> Result<Vec<AgentConnection>, IntrospectionError> {
        let society = self.society.read().await;
        let mut connections = Vec::new();

        if let Some(subjects) = society.relationships.get(agent_id) {
            for (subject, rels) in subjects {
                connections.extend(rels.iter().map(|rel| AgentConnection {
                    from: agent_id.to_string(),
                    to: subject.clone(),
                    relationship_type: format!("{:?}", rel.kind),
                    trust_score: rel.strength,
                    interaction_count: 1,
                    last_interaction: rel.tick,
                }));
            }
        }

        // Also check reverse relationships
        for (observer, subjects) in &society.relationships {
            if subjects.contains_key(agent_id) {
                for rel in subjects.get(agent_id).unwrap_or(&vec![]) {
                    connections.push(AgentConnection {
                        from: observer.clone(),
                        to: agent_id.to_string(),
                        relationship_type: format!("{:?}", rel.kind),
                        trust_score: rel.strength,
                        interaction_count: 1,
                        last_interaction: rel.tick,
                    });
                }
            }
        }

        Ok(connections)
    }

    async fn get_trust_network(&self) -> Result<Vec<AgentConnection>, IntrospectionError> {
        let society = self.society.read().await;
        let mut connections = Vec::new();

        for (observer, subjects) in &society.trust_scores {
            for (subject, score) in subjects {
                if *score > 0.0 {
                    connections.push(AgentConnection {
                        from: observer.clone(),
                        to: subject.clone(),
                        relationship_type: "trust".to_string(),
                        trust_score: *score,
                        interaction_count: 1,
                        last_interaction: 0,
                    });
                }
            }
        }

        Ok(connections)
    }

    async fn get_collective_memory(&self) -> Result<serde_json::Value, IntrospectionError> {
        Ok(serde_json::json!({}))
    }

    async fn search_memory(&self, agent_id: &str, query: &str, limit: usize) -> Result<serde_json::Value, IntrospectionError> {
        let store = self.personal_memory.as_ref();
        let cached = store.get_or_create(agent_id).await;
        let memory = cached.read().await.memory.clone();
        let results = decentraai_agent_personal_memory::mcp::search_memory(&memory, query, None, limit);
        Ok(serde_json::json!({
            "results": results,
            "count": results.len()
        }))
    }

    async fn get_agent_dreams(&self, agent_id: &str) -> Result<Vec<String>, IntrospectionError> {
        Ok(vec![])
    }

    async fn get_agent_beliefs(&self, agent_id: &str) -> Result<serde_json::Value, IntrospectionError> {
        let state = self.get_agent_state(agent_id).await?;
        Ok(serde_json::to_value(&state.current_beliefs).unwrap_or_default())
    }

    async fn get_agent_lessons(&self, agent_id: &str) -> Result<Vec<String>, IntrospectionError> {
        let store = self.personal_memory.as_ref();
        let snapshot = store.snapshot(agent_id).await
            .map_err(|_| IntrospectionError::Internal("Failed to get memory snapshot".into()))?;
        Ok(snapshot.lessons.lessons.iter().map(|l| l.title.clone()).collect())
    }
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_summary_serialization() {
        let summary = AgentSummary {
            agent_id: "test-agent".to_string(),
            status: "Ready".to_string(),
            name: "Test Agent".to_string(),
            capabilities: vec!["analysis".to_string()],
            metrics_summary: AgentMetricsSummary::default(),
            memory_summary: MemorySummary::default(),
            society_summary: SocietySummary::default(),
            last_active: 0,
        };

        let json = serde_json::to_string(&summary).unwrap();
        let deserialized: AgentSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(summary.agent_id, deserialized.agent_id);
    }
}