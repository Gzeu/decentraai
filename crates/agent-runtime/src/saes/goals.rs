//! SAES 0.2 Step 1: Goal system
//!
//! Structured goals replace the opaque `Vec<String>` in `AgentState`.
//! Each goal has a lifecycle (Pending → Active → Completed/Failed/Abandoned),
//! a priority, optional deadline, and progress tracking.
//!
//! # Design decisions
//!
//! - **Goals are per-agent, not global.** Each agent owns its goals.
//! - **Goals are plain serde types.** No I/O, no runtime coupling.
//! - **GoalStore is a trait.** In-memory for tests, SQLite/Redis for production.
//! - **Backward compatible:** `AgentState.current_goals` stays `Vec<String>` for v1;
//!   the structured goals live alongside and are synced by the runtime.
//! - **Open vocabulary:** goal kinds are free-form strings, not an enum.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Unique goal identifier.
pub type GoalId = String;

/// Agent identifier (re-export from society).
pub type AgentId = decentraai_agent_society::AgentId;

/// Errors in the goal system.
#[derive(Debug, Error)]
pub enum GoalError {
    #[error("goal not found: {0}")]
    NotFound(GoalId),
    #[error("invalid state transition: {from} → {to}")]
    InvalidTransition { from: GoalState, to: GoalState },
    #[error("goal already exists: {0}")]
    AlreadyExists(GoalId),
    #[error("internal: {0}")]
    Internal(String),
}

/// Lifecycle state of a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalState {
    /// Created but not yet active.
    Pending,
    /// Currently being pursued.
    Active,
    /// Successfully completed.
    Completed,
    /// Failed (reason recorded in `failure_reason`).
    Failed,
    /// Abandoned by the agent (not a failure, just dropped).
    Abandoned,
}

impl GoalState {
    /// Valid state transitions. Returns `true` if `self → next` is allowed.
    pub fn can_transition_to(self, next: GoalState) -> bool {
        matches!(
            (self, next),
            (GoalState::Pending, GoalState::Active)
                | (GoalState::Pending, GoalState::Abandoned)
                | (GoalState::Active, GoalState::Completed)
                | (GoalState::Active, GoalState::Failed)
                | (GoalState::Active, GoalState::Abandoned)
        )
    }
}

impl std::fmt::Display for GoalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GoalState::Pending => write!(f, "pending"),
            GoalState::Active => write!(f, "active"),
            GoalState::Completed => write!(f, "completed"),
            GoalState::Failed => write!(f, "failed"),
            GoalState::Abandoned => write!(f, "abandoned"),
        }
    }
}

/// Priority of a goal. Higher number = higher priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GoalPriority(pub u8);

impl GoalPriority {
    pub const LOW: GoalPriority = GoalPriority(1);
    pub const NORMAL: GoalPriority = GoalPriority(5);
    pub const HIGH: GoalPriority = GoalPriority(8);
    pub const CRITICAL: GoalPriority = GoalPriority(10);
}

impl Default for GoalPriority {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// A structured goal owned by an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentGoal {
    /// Unique identifier for this goal.
    pub id: GoalId,
    /// The agent that owns this goal.
    pub agent_id: AgentId,
    /// Human-readable description of what the goal aims to achieve.
    pub description: String,
    /// Free-form kind/category (e.g. "serve_request", "learn_skill", "earn_reward").
    pub kind: String,
    /// Current lifecycle state.
    pub state: GoalState,
    /// Priority (higher = more important).
    pub priority: GoalPriority,
    /// Progress 0.0..=1.0.
    pub progress: f32,
    /// Optional deadline (epoch ms). `None` = no deadline.
    pub deadline: Option<u64>,
    /// Failure reason (set when state = Failed).
    pub failure_reason: Option<String>,
    /// When the goal was created.
    pub created_at: u64,
    /// When the goal was last updated.
    pub updated_at: u64,
    /// When the goal transitioned to Active.
    pub activated_at: Option<u64>,
    /// When the goal reached a terminal state (Completed/Failed/Abandoned).
    pub completed_at: Option<u64>,
    /// Free-form metadata (tags, related evidence, etc.).
    pub metadata: serde_json::Value,
}

impl AgentGoal {
    /// Create a new goal in Pending state.
    pub fn new(
        agent_id: AgentId,
        description: String,
        kind: String,
        priority: GoalPriority,
        now_ms: u64,
    ) -> Self {
        Self {
            id: format!("goal-{}-{}", agent_id, uuid_simple()),
            agent_id,
            description,
            kind,
            state: GoalState::Pending,
            priority,
            progress: 0.0,
            deadline: None,
            failure_reason: None,
            created_at: now_ms,
            updated_at: now_ms,
            activated_at: None,
            completed_at: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Transition to a new state. Returns `Err` if the transition is invalid.
    pub fn transition_to(&mut self, new_state: GoalState, now_ms: u64) -> Result<(), GoalError> {
        if !self.state.can_transition_to(new_state) {
            return Err(GoalError::InvalidTransition {
                from: self.state,
                to: new_state,
            });
        }
        match new_state {
            GoalState::Active => self.activated_at = Some(now_ms),
            GoalState::Completed | GoalState::Failed | GoalState::Abandoned => {
                self.completed_at = Some(now_ms)
            }
            _ => {}
        }
        self.state = new_state;
        self.updated_at = now_ms;
        Ok(())
    }

    /// Update progress (0.0..=1.0). Clamps to valid range.
    pub fn set_progress(&mut self, progress: f32, now_ms: u64) {
        self.progress = progress.clamp(0.0, 1.0);
        self.updated_at = now_ms;
    }

    /// Mark as failed with a reason.
    pub fn fail(&mut self, reason: String, now_ms: u64) -> Result<(), GoalError> {
        self.failure_reason = Some(reason);
        self.transition_to(GoalState::Failed, now_ms)
    }

    /// Check if the goal is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            GoalState::Completed | GoalState::Failed | GoalState::Abandoned
        )
    }

    /// Check if the goal is overdue (has a deadline and current time > deadline).
    pub fn is_overdue(&self, now_ms: u64) -> bool {
        self.deadline
            .is_some_and(|d| now_ms > d && !self.is_terminal())
    }
}

/// Simple UUID-like generator (no external dep needed for goal IDs).
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:x}", t)
}

/// Trait for goal persistence. In-memory for tests, pluggable for production.
#[async_trait::async_trait]
pub trait GoalStore: Send + Sync {
    /// Add a new goal.
    async fn add(&self, goal: AgentGoal) -> Result<(), GoalError>;
    /// Get a goal by ID.
    async fn get(&self, goal_id: &GoalId) -> Result<AgentGoal, GoalError>;
    /// Update a goal (full replace).
    async fn update(&self, goal: AgentGoal) -> Result<(), GoalError>;
    /// List all goals for an agent.
    async fn list_by_agent(&self, agent_id: &AgentId) -> Vec<AgentGoal>;
    /// List goals by state for an agent.
    async fn list_by_state(&self, agent_id: &AgentId, state: GoalState) -> Vec<AgentGoal>;
    /// Delete a goal.
    async fn delete(&self, goal_id: &GoalId) -> Result<(), GoalError>;
    /// Count goals by state for an agent.
    async fn count_by_state(&self, agent_id: &AgentId, state: GoalState) -> usize;
}

/// In-memory goal store for tests and single-node operation.
pub struct InMemoryGoalStore {
    goals: std::sync::RwLock<HashMap<GoalId, AgentGoal>>,
}

impl InMemoryGoalStore {
    pub fn new() -> Self {
        Self {
            goals: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryGoalStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl GoalStore for InMemoryGoalStore {
    async fn add(&self, goal: AgentGoal) -> Result<(), GoalError> {
        let mut goals = self
            .goals
            .write()
            .map_err(|e| GoalError::Internal(e.to_string()))?;
        if goals.contains_key(&goal.id) {
            return Err(GoalError::AlreadyExists(goal.id));
        }
        goals.insert(goal.id.clone(), goal);
        Ok(())
    }

    async fn get(&self, goal_id: &GoalId) -> Result<AgentGoal, GoalError> {
        let goals = self
            .goals
            .read()
            .map_err(|e| GoalError::Internal(e.to_string()))?;
        goals
            .get(goal_id)
            .cloned()
            .ok_or(GoalError::NotFound(goal_id.clone()))
    }

    async fn update(&self, goal: AgentGoal) -> Result<(), GoalError> {
        let mut goals = self
            .goals
            .write()
            .map_err(|e| GoalError::Internal(e.to_string()))?;
        if !goals.contains_key(&goal.id) {
            return Err(GoalError::NotFound(goal.id));
        }
        goals.insert(goal.id.clone(), goal);
        Ok(())
    }

    async fn list_by_agent(&self, agent_id: &AgentId) -> Vec<AgentGoal> {
        let goals = self.goals.read().unwrap_or_else(|e| e.into_inner());
        goals
            .values()
            .filter(|g| g.agent_id == *agent_id)
            .cloned()
            .collect()
    }

    async fn list_by_state(&self, agent_id: &AgentId, state: GoalState) -> Vec<AgentGoal> {
        let goals = self.goals.read().unwrap_or_else(|e| e.into_inner());
        goals
            .values()
            .filter(|g| g.agent_id == *agent_id && g.state == state)
            .cloned()
            .collect()
    }

    async fn delete(&self, goal_id: &GoalId) -> Result<(), GoalError> {
        let mut goals = self
            .goals
            .write()
            .map_err(|e| GoalError::Internal(e.to_string()))?;
        goals
            .remove(goal_id)
            .ok_or(GoalError::NotFound(goal_id.clone()))?;
        Ok(())
    }

    async fn count_by_state(&self, agent_id: &AgentId, state: GoalState) -> usize {
        let goals = self.goals.read().unwrap_or_else(|e| e.into_inner());
        goals
            .values()
            .filter(|g| g.agent_id == *agent_id && g.state == state)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_id() -> AgentId {
        "test-agent:worker".to_string()
    }

    fn make_goal(priority: GoalPriority) -> AgentGoal {
        AgentGoal::new(
            agent_id(),
            "serve 5 requests".to_string(),
            "serve_request".to_string(),
            priority,
            1000,
        )
    }

    #[test]
    fn goal_creation() {
        let g = make_goal(GoalPriority::NORMAL);
        assert_eq!(g.state, GoalState::Pending);
        assert_eq!(g.progress, 0.0);
        assert!(g.failure_reason.is_none());
        assert!(!g.is_terminal());
    }

    #[test]
    fn valid_transitions() {
        let mut g = make_goal(GoalPriority::HIGH);
        g.transition_to(GoalState::Active, 2000).unwrap();
        assert_eq!(g.state, GoalState::Active);
        assert_eq!(g.activated_at, Some(2000));

        g.transition_to(GoalState::Completed, 3000).unwrap();
        assert_eq!(g.state, GoalState::Completed);
        assert_eq!(g.completed_at, Some(3000));
        assert!(g.is_terminal());
    }

    #[test]
    fn invalid_transition_pending_to_completed() {
        let mut g = make_goal(GoalPriority::NORMAL);
        let result = g.transition_to(GoalState::Completed, 2000);
        assert!(result.is_err());
        match result.unwrap_err() {
            GoalError::InvalidTransition { from, to } => {
                assert_eq!(from, GoalState::Pending);
                assert_eq!(to, GoalState::Completed);
            }
            _ => panic!("expected InvalidTransition"),
        }
    }

    #[test]
    fn fail_sets_reason() {
        let mut g = make_goal(GoalPriority::NORMAL);
        g.transition_to(GoalState::Active, 1000).unwrap();
        g.fail("network error".to_string(), 2000).unwrap();
        assert_eq!(g.state, GoalState::Failed);
        assert_eq!(g.failure_reason.as_deref(), Some("network error"));
        assert!(g.is_terminal());
    }

    #[test]
    fn progress_clamping() {
        let mut g = make_goal(GoalPriority::NORMAL);
        g.set_progress(1.5, 1000);
        assert_eq!(g.progress, 1.0);
        g.set_progress(-0.5, 2000);
        assert_eq!(g.progress, 0.0);
        g.set_progress(0.7, 3000);
        assert_eq!(g.progress, 0.7);
    }

    #[test]
    fn deadline_check() {
        let mut g = make_goal(GoalPriority::NORMAL);
        g.deadline = Some(5000);
        assert!(!g.is_overdue(4000));
        assert!(!g.is_overdue(5000));
        assert!(g.is_overdue(6000));
        // Terminal goals are never overdue.
        g.transition_to(GoalState::Active, 1000).unwrap();
        g.transition_to(GoalState::Completed, 2000).unwrap();
        assert!(!g.is_overdue(6000));
    }

    #[test]
    fn abandon_from_pending() {
        let mut g = make_goal(GoalPriority::LOW);
        g.transition_to(GoalState::Abandoned, 1000).unwrap();
        assert!(g.is_terminal());
    }

    #[test]
    fn abandon_from_active() {
        let mut g = make_goal(GoalPriority::NORMAL);
        g.transition_to(GoalState::Active, 1000).unwrap();
        g.transition_to(GoalState::Abandoned, 2000).unwrap();
        assert!(g.is_terminal());
    }

    // GoalStore tests

    #[tokio::test]
    async fn store_add_and_get() {
        let store = InMemoryGoalStore::new();
        let g = make_goal(GoalPriority::NORMAL);
        let id = g.id.clone();
        store.add(g).await.unwrap();
        let fetched = store.get(&id).await.unwrap();
        assert_eq!(fetched.description, "serve 5 requests");
    }

    #[tokio::test]
    async fn store_duplicate_rejected() {
        let store = InMemoryGoalStore::new();
        let g = make_goal(GoalPriority::NORMAL);
        let id = g.id.clone();
        store.add(g).await.unwrap();
        let g2 = AgentGoal::new(
            agent_id(),
            "another".to_string(),
            "serve_request".to_string(),
            GoalPriority::NORMAL,
            1000,
        );
        // Same ID would be duplicate, but IDs are unique by construction.
        // Test with explicit duplicate:
        let mut g3 = g2.clone();
        g3.id = id.clone();
        assert!(store.add(g3).await.is_err());
    }

    #[tokio::test]
    async fn store_update() {
        let store = InMemoryGoalStore::new();
        let mut g = make_goal(GoalPriority::NORMAL);
        store.add(g.clone()).await.unwrap();
        g.transition_to(GoalState::Active, 2000).unwrap();
        store.update(g.clone()).await.unwrap();
        let fetched = store.get(&g.id).await.unwrap();
        assert_eq!(fetched.state, GoalState::Active);
    }

    #[tokio::test]
    async fn store_list_by_agent() {
        let store = InMemoryGoalStore::new();
        let g1 = make_goal(GoalPriority::NORMAL);
        let g2 = make_goal(GoalPriority::HIGH);
        store.add(g1).await.unwrap();
        store.add(g2).await.unwrap();
        let list = store.list_by_agent(&agent_id()).await;
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn store_list_by_state() {
        let store = InMemoryGoalStore::new();
        let mut g1 = make_goal(GoalPriority::NORMAL);
        let g2 = make_goal(GoalPriority::HIGH);
        g1.transition_to(GoalState::Active, 1000).unwrap();
        store.add(g1.clone()).await.unwrap();
        store.add(g2).await.unwrap();
        let active = store.list_by_state(&agent_id(), GoalState::Active).await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, g1.id);
        let pending = store.list_by_state(&agent_id(), GoalState::Pending).await;
        assert_eq!(pending.len(), 1);
    }

    #[tokio::test]
    async fn store_delete() {
        let store = InMemoryGoalStore::new();
        let g = make_goal(GoalPriority::NORMAL);
        let id = g.id.clone();
        store.add(g).await.unwrap();
        store.delete(&id).await.unwrap();
        assert!(store.get(&id).await.is_err());
    }

    #[tokio::test]
    async fn store_count_by_state() {
        let store = InMemoryGoalStore::new();
        let mut g1 = make_goal(GoalPriority::NORMAL);
        let g2 = make_goal(GoalPriority::HIGH);
        let mut g3 = make_goal(GoalPriority::LOW);
        g1.transition_to(GoalState::Active, 1000).unwrap();
        g3.transition_to(GoalState::Active, 1000).unwrap();
        g3.transition_to(GoalState::Completed, 2000).unwrap();
        store.add(g1).await.unwrap();
        store.add(g2).await.unwrap();
        store.add(g3).await.unwrap();
        assert_eq!(
            store.count_by_state(&agent_id(), GoalState::Active).await,
            1
        );
        assert_eq!(
            store.count_by_state(&agent_id(), GoalState::Pending).await,
            1
        );
        assert_eq!(
            store
                .count_by_state(&agent_id(), GoalState::Completed)
                .await,
            1
        );
    }

    #[tokio::test]
    async fn store_get_not_found() {
        let store = InMemoryGoalStore::new();
        assert!(store.get(&"nonexistent".to_string()).await.is_err());
    }

    #[test]
    fn priority_ordering() {
        assert!(GoalPriority::LOW < GoalPriority::NORMAL);
        assert!(GoalPriority::NORMAL < GoalPriority::HIGH);
        assert!(GoalPriority::HIGH < GoalPriority::CRITICAL);
    }
}
