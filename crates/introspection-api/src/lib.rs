use async_trait::async_trait;
use dashmap::DashMap;
use decentraai_agent_runtime::{AgentRuntime, AgentState};
use decentraai_agent_society::SocietyState;
use decentraai_arena::ArenaWorld;
use decentraai_compute::QuotaLedger;
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentMetricsSummary {
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub success_rate: f32,
    pub total_reward: u64,
    pub reputation: f32,
    pub trust_given: u32,
    pub trust_received: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemorySummary {
    pub total_entries: usize,
    pub categories: HashMap<String, usize>,
    pub recent_experiences: usize,
    pub recent_lessons: usize,
    pub relationships: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SocietySummary {
    pub contributions: u64,
    pub outcomes: u64,
    pub reputation_events: u64,
    pub trust_scores: HashMap<String, f32>,
    pub relationships: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentActivity {
    pub agent_id: String,
    pub timestamp: u64,
    pub action: String,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentSearchQuery {
    pub agent_id: Option<String>,
    pub capability: Option<String>,
    pub status: Option<String>,
    pub since_timestamp: Option<u64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkTopology {
    pub agents: Vec<AgentNode>,
    pub connections: Vec<AgentConnection>,
    pub clusters: Vec<AgentCluster>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentNode {
    pub agent_id: String,
    pub name: String,
    pub status: String,
    pub capabilities: Vec<String>,
    pub reputation: f32,
    pub location: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConnection {
    pub from: String,
    pub to: String,
    pub relationship_type: String,
    pub trust_score: f32,
    pub interaction_count: u32,
    pub last_interaction: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentCluster {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
    pub cohesion: f32,
    pub shared_goals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentMetadata {
    pub agent_id: String,
    pub name: String,
    pub created_at: u64,
    pub last_active: u64,
    pub total_tasks: u64,
    pub total_reward: u64,
    pub tags: Vec<String>,
}

#[async_trait]
pub trait IntrospectionProvider: Send + Sync {
    async fn get_agent_full_state(
        &self,
        agent_id: &str,
    ) -> Result<AgentFullState, IntrospectionError>;
    async fn get_agent_summary(&self, agent_id: &str) -> Result<AgentSummary, IntrospectionError>;
    async fn list_agents(
        &self,
        query: AgentSearchQuery,
    ) -> Result<Vec<AgentSummary>, IntrospectionError>;
    async fn get_network_topology(&self) -> Result<NetworkTopology, IntrospectionError>;
    async fn get_agent_activity(
        &self,
        agent_id: &str,
        since: u64,
        limit: usize,
    ) -> Result<Vec<AgentActivity>, IntrospectionError>;
    async fn get_agent_relationships(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentConnection>, IntrospectionError>;
    async fn get_trust_network(&self) -> Result<Vec<AgentConnection>, IntrospectionError>;
    async fn get_collective_memory(&self) -> Result<serde_json::Value, IntrospectionError>;
    async fn search_memory(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<serde_json::Value, IntrospectionError>;
    async fn get_agent_dreams(&self, agent_id: &str) -> Result<Vec<String>, IntrospectionError>;
    async fn get_agent_beliefs(
        &self,
        agent_id: &str,
    ) -> Result<serde_json::Value, IntrospectionError>;
    async fn get_agent_lessons(&self, agent_id: &str) -> Result<Vec<String>, IntrospectionError>;
}

pub struct IntrospectionService {
    agent_runtime: Arc<dyn AgentRuntime>,
    personal_memory: Arc<decentraai_agent_personal_memory::PersonalMemoryStore>,
    society: Arc<RwLock<SocietyState>>,
    arena: Arc<RwLock<ArenaWorld>>,
    quota_ledger: Arc<Mutex<QuotaLedger>>,
    #[allow(dead_code)]
    agent_metadata: Arc<DashMap<String, AgentMetadata>>,
}

impl IntrospectionService {
    pub fn new(
        agent_runtime: Arc<dyn AgentRuntime>,
        personal_memory: Arc<decentraai_agent_personal_memory::PersonalMemoryStore>,
        society: Arc<RwLock<SocietyState>>,
        arena: Arc<RwLock<ArenaWorld>>,
        quota_ledger: Arc<Mutex<QuotaLedger>>,
    ) -> Self {
        Self {
            agent_runtime,
            personal_memory,
            society,
            arena,
            quota_ledger,
            agent_metadata: Arc::new(DashMap::new()),
        }
    }

    async fn get_agent_state(&self, agent_id: &str) -> Result<AgentState, IntrospectionError> {
        self.agent_runtime
            .get_state(&agent_id.to_string())
            .await
            .map_err(|_| IntrospectionError::NotFound(agent_id.to_string()))
    }

    async fn get_personal_memory(
        &self,
        agent_id: &str,
    ) -> Result<serde_json::Value, IntrospectionError> {
        let snapshot = self
            .personal_memory
            .snapshot(&agent_id.to_string())
            .await
            .map_err(|e| {
                IntrospectionError::Internal(format!("Failed to get memory snapshot: {e}"))
            })?;
        Ok(serde_json::to_value(snapshot).unwrap_or(serde_json::json!({})))
    }

    async fn get_society_snapshot(
        &self,
        agent_id: &str,
    ) -> Result<serde_json::Value, IntrospectionError> {
        let society = self.society.read().await;
        let snapshot = decentraai_agent_society::mcp::build_society_state_response(
            &society,
            &agent_id.to_string(),
        );
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

    async fn get_quota_snapshot(
        &self,
        agent_id: &str,
    ) -> Result<serde_json::Value, IntrospectionError> {
        let ledger = self.quota_ledger.lock().unwrap();
        if let Some(account) = ledger.account(&agent_id.to_string()) {
            Ok(serde_json::json!({
                "available": account.available,
                "consumed": account.consumed,
                "reserved": account.reserved,
                "earned": account.earned,
            }))
        } else {
            Ok(serde_json::json!({
                "available": 0,
                "consumed": 0,
                "reserved": 0,
                "earned": 0,
            }))
        }
    }
}

#[async_trait]
impl IntrospectionProvider for IntrospectionService {
    async fn get_agent_full_state(
        &self,
        agent_id: &str,
    ) -> Result<AgentFullState, IntrospectionError> {
        let state = self.get_agent_state(agent_id).await?;
        let memory = self.get_personal_memory(agent_id).await?;
        let society = self.get_society_snapshot(agent_id).await?;
        let arena = self.get_arena_snapshot().await?;
        let quota = self.get_quota_snapshot(agent_id).await?;

        let metrics = serde_json::to_value(&state.metrics).unwrap_or_default();
        let state_val = serde_json::to_value(&state).unwrap_or_default();

        let goals = state.current_goals.clone();
        let beliefs = state.current_beliefs.clone();

        Ok(AgentFullState {
            agent_state: state_val,
            metrics,
            memory,
            society,
            arena,
            quota,
            goals,
            beliefs,
            dreams: vec![],
            reflections: vec![],
        })
    }

    async fn get_agent_summary(&self, agent_id: &str) -> Result<AgentSummary, IntrospectionError> {
        let state = self.get_agent_state(agent_id).await?;

        let metrics = AgentMetricsSummary {
            tasks_completed: state.metrics.tasks_completed,
            tasks_failed: state.metrics.tasks_failed,
            success_rate: if state.metrics.tasks_completed + state.metrics.tasks_failed > 0 {
                state.metrics.tasks_completed as f32
                    / (state.metrics.tasks_completed + state.metrics.tasks_failed) as f32
            } else {
                0.0
            },
            total_reward: state.metrics.total_reward_earned,
            reputation: state.metrics.reputation_score,
            trust_given: state.metrics.trust_scores_given,
            trust_received: state.metrics.trust_scores_received,
        };

        let memory_summary = MemorySummary::default();
        let society_summary = SocietySummary::default();

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

    async fn list_agents(
        &self,
        query: AgentSearchQuery,
    ) -> Result<Vec<AgentSummary>, IntrospectionError> {
        let agents = self
            .agent_runtime
            .list_agents()
            .await
            .map_err(|e| IntrospectionError::Internal(format!("Failed to list agents: {e}")))?;

        let mut results = Vec::new();
        for agent_id in agents {
            if let Some(cap) = query.capability.as_ref() {
                if let Ok(state) = self.get_agent_state(&agent_id).await {
                    if !state.config.capabilities.iter().any(|c| c == cap) {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            if let Some(status) = &query.status {
                if let Ok(state) = self.get_agent_state(&agent_id).await {
                    if format!("{:?}", state.status) != *status {
                        continue;
                    }
                } else {
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
        let agents = self
            .agent_runtime
            .list_agents()
            .await
            .map_err(|e| IntrospectionError::Internal(format!("Failed to list agents: {e}")))?;

        let mut nodes = Vec::new();

        for agent_id in &agents {
            if let Ok(state) = self.get_agent_state(agent_id).await {
                nodes.push(AgentNode {
                    agent_id: agent_id.clone(),
                    name: state.config.name.clone(),
                    status: format!("{:?}", state.status),
                    capabilities: state.config.capabilities.clone(),
                    reputation: state.metrics.reputation_score,
                    location: None,
                });
            }
        }

        // Build connections from society relationships
        let mut connections = Vec::new();
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

        Ok(NetworkTopology {
            agents: nodes,
            connections,
            clusters: vec![],
        })
    }

    async fn get_agent_activity(
        &self,
        _agent_id: &str,
        _since: u64,
        _limit: usize,
    ) -> Result<Vec<AgentActivity>, IntrospectionError> {
        // Event store not wired yet
        Ok(vec![])
    }

    async fn get_agent_relationships(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentConnection>, IntrospectionError> {
        let society = self.society.read().await;
        let mut connections = Vec::new();

        if let Some(subjects) = society.relationships.get(agent_id) {
            for (subject, rels) in subjects {
                for rel in rels {
                    connections.push(AgentConnection {
                        from: agent_id.to_string(),
                        to: subject.clone(),
                        relationship_type: format!("{:?}", rel.kind),
                        trust_score: rel.strength,
                        interaction_count: 1,
                        last_interaction: rel.tick,
                    });
                }
            }
        }

        // Also check reverse relationships
        for (observer, subjects) in &society.relationships {
            if let Some(rels) = subjects.get(agent_id) {
                for rel in rels {
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

        // Build trust network from relationships with positive strength
        for (observer, subjects) in &society.relationships {
            for (subject, rels) in subjects {
                for rel in rels {
                    if rel.strength > 0.0 {
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
        }

        Ok(connections)
    }

    async fn get_collective_memory(&self) -> Result<serde_json::Value, IntrospectionError> {
        Ok(serde_json::json!({}))
    }

    async fn search_memory(
        &self,
        agent_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<serde_json::Value, IntrospectionError> {
        let cached = self
            .personal_memory
            .get_or_create(&agent_id.to_string())
            .await;
        let mem = cached.read().await;
        let results =
            decentraai_agent_personal_memory::mcp::search_memory(&mem.memory, query, None, limit);
        Ok(serde_json::json!({
            "results": results,
            "count": results.len()
        }))
    }

    async fn get_agent_dreams(&self, _agent_id: &str) -> Result<Vec<String>, IntrospectionError> {
        Ok(vec![])
    }

    async fn get_agent_beliefs(
        &self,
        agent_id: &str,
    ) -> Result<serde_json::Value, IntrospectionError> {
        let state = self.get_agent_state(agent_id).await?;
        Ok(serde_json::to_value(&state.current_beliefs).unwrap_or_default())
    }

    async fn get_agent_lessons(&self, agent_id: &str) -> Result<Vec<String>, IntrospectionError> {
        let snapshot = self
            .personal_memory
            .snapshot(&agent_id.to_string())
            .await
            .map_err(|e| {
                IntrospectionError::Internal(format!("Failed to get memory snapshot: {e}"))
            })?;
        Ok(snapshot
            .recent_lessons
            .iter()
            .map(|l| l.title.clone())
            .collect())
    }
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
