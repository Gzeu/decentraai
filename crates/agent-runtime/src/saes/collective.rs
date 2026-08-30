//! SAES 0.4: Collective Goal Coordination
//!
//! Enables multiple agents to coordinate on shared objectives. Each
//! collective goal can have multiple participants, each with their own
//! sub-goal. Progress propagates from sub-goals to the collective goal.
//!
//! # Design principles
//!
//! - **Generic**: No hardcoding for specific agents, models, or capabilities.
//!   All identifiers are free-form strings; no closed enums.
//! - **Open vocabulary**: Goal kinds, agent IDs, and capability strings are
//!   free-form, not enums.
//! - **Composable**: Collective goals build on top of existing `GoalStore`
//!   and `BehaviorStore` without replacing them.
//! - **Event-driven**: State changes publish to the existing event-bus with
//!   `correlation_id` for traceability. Every operation carries a
//!   `correlation_id` that links the full chain from proposal → join →
//!   progress → completion.
//! - **External-agent ready**: The design allows future external agents to
//!   join collective goals without core modifications. Agents are
//!   identified solely by their `agent_id` string.
//! - **Failure policy**: Configurable policy determines how sub-goal failures
//!   affect the collective goal. Policies are data-driven, not hardcoded.
//!
//! # Lifecycle
//!
//! ```text
//! propose (agent A creates collective goal, correlation_id = goal_id)
//!   → join (agent B joins, gets sub-goal, correlation_id propagates)
//!     → sub-goal progress (A and B work independently, report via learn!)
//!       → collective progress updates (aggregated from sub-goal progress)
//!         → collective completed (all sub-goals completed)
//!           OR collective failed (failure policy applied,
//!             correlation_id traces which sub-goal failed)
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::goals::{GoalPriority, GoalState};
use decentraai_event_bus::{Event, EventId, EventMetadata, EventPriority, Topic};

/// Unique identifier for a collective goal.
pub type CollectiveGoalId = String;

/// Unique identifier for a sub-goal.
pub type SubGoalId = String;

/// Agent identifier (re-export).
pub type AgentId = String;

/// Failure policy for how sub-goal failures affect the collective goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    /// Any sub-goal failure fails the entire collective goal.
    FailFast,
    /// Sub-goal failures are tolerated; collective completes if enough
    /// progress is made (threshold-based).
    #[default]
    Tolerant,
    /// Sub-goal failures are ignored; collective completes when all
    /// remaining sub-goals finish.
    Ignore,
}

/// Status of a collective goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectiveStatus {
    /// Proposed, waiting for participants.
    Proposed,
    /// Active, participants working.
    Active,
    /// All sub-goals completed.
    Completed,
    /// Failed (reason recorded).
    Failed,
    /// Cancelled by proposer or operator.
    Cancelled,
}

impl CollectiveStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for CollectiveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Proposed => write!(f, "proposed"),
            Self::Active => write!(f, "active"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// A sub-goal assigned to a specific participant in a collective goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubGoal {
    /// Unique sub-goal identifier.
    pub id: SubGoalId,
    /// The collective goal this belongs to.
    pub collective_goal_id: CollectiveGoalId,
    /// The agent responsible for this sub-goal.
    pub agent_id: AgentId,
    /// Description of what this agent needs to do.
    pub description: String,
    /// Current state.
    pub state: GoalState,
    /// Progress 0.0..=1.0.
    pub progress: f32,
    /// Failure reason (if state = Failed).
    pub failure_reason: Option<String>,
    /// When the sub-goal was created.
    pub created_at: u64,
    /// When the sub-goal was last updated.
    pub updated_at: u64,
    /// When the sub-goal reached a terminal state.
    pub completed_at: Option<u64>,
    /// Free-form metadata.
    pub metadata: serde_json::Value,
}

impl SubGoal {
    pub fn new(
        collective_goal_id: CollectiveGoalId,
        agent_id: AgentId,
        description: String,
        now_ms: u64,
    ) -> Self {
        Self {
            id: format!("sub-{}-{}", collective_goal_id, uuid_simple()),
            collective_goal_id,
            agent_id,
            description,
            state: GoalState::Pending,
            progress: 0.0,
            failure_reason: None,
            created_at: now_ms,
            updated_at: now_ms,
            completed_at: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn activate(&mut self, now_ms: u64) {
        if self.state == GoalState::Pending {
            self.state = GoalState::Active;
            self.updated_at = now_ms;
        }
    }

    pub fn set_progress(&mut self, progress: f32, now_ms: u64) {
        self.progress = progress.clamp(0.0, 1.0);
        self.updated_at = now_ms;
    }

    pub fn complete(&mut self, now_ms: u64) {
        self.state = GoalState::Completed;
        self.progress = 1.0;
        self.completed_at = Some(now_ms);
        self.updated_at = now_ms;
    }

    pub fn fail(&mut self, reason: String, now_ms: u64) {
        self.state = GoalState::Failed;
        self.failure_reason = Some(reason);
        self.completed_at = Some(now_ms);
        self.updated_at = now_ms;
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            GoalState::Completed | GoalState::Failed | GoalState::Abandoned
        )
    }
}

/// A collective goal that multiple agents can participate in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectiveGoal {
    /// Unique identifier.
    pub id: CollectiveGoalId,
    /// Human-readable title.
    pub title: String,
    /// Detailed description.
    pub description: String,
    /// Goal kind/category (free-form).
    pub kind: String,
    /// Current status.
    pub status: CollectiveStatus,
    /// Priority.
    pub priority: GoalPriority,
    /// Agent that proposed this goal.
    pub proposer_id: AgentId,
    /// Participating agent IDs.
    pub participants: Vec<AgentId>,
    /// Sub-goals keyed by sub-goal ID.
    pub sub_goals: HashMap<SubGoalId, SubGoal>,
    /// Overall progress 0.0..=1.0 (aggregated from sub-goals).
    pub progress: f32,
    /// Failure policy.
    pub failure_policy: FailurePolicy,
    /// Dependencies (other collective goal IDs that must complete first).
    pub dependencies: Vec<CollectiveGoalId>,
    /// Optional deadline (epoch ms).
    pub deadline: Option<u64>,
    /// When the goal was created.
    pub created_at: u64,
    /// When the goal was last updated.
    pub updated_at: u64,
    /// When the goal reached a terminal state.
    pub completed_at: Option<u64>,
    /// Failure reason.
    pub failure_reason: Option<String>,
    /// Free-form metadata.
    pub metadata: serde_json::Value,
    /// Correlation ID for event traceability.
    pub correlation_id: String,
}

impl CollectiveGoal {
    pub fn new(
        title: String,
        description: String,
        kind: String,
        proposer_id: AgentId,
        priority: GoalPriority,
        failure_policy: FailurePolicy,
        now_ms: u64,
    ) -> Self {
        let id = format!("cg-{}", uuid_simple());
        Self {
            id: id.clone(),
            title,
            description,
            kind,
            status: CollectiveStatus::Proposed,
            priority,
            proposer_id,
            participants: vec![],
            sub_goals: HashMap::new(),
            progress: 0.0,
            failure_policy,
            dependencies: vec![],
            deadline: None,
            created_at: now_ms,
            updated_at: now_ms,
            completed_at: None,
            failure_reason: None,
            metadata: serde_json::Value::Null,
            correlation_id: id,
        }
    }

    /// Add a participant and create their sub-goal.
    /// Publishes `goal.proposed` and `agent.joined` events via the provided publisher.
    pub fn add_participant(
        &mut self,
        agent_id: AgentId,
        sub_goal_description: String,
        now_ms: u64,
        publisher: Option<&dyn Fn(Event) -> Result<(), String>>,
    ) -> Result<SubGoalId, String> {
        if self.status.is_terminal() {
            return Err("goal is in terminal state".to_string());
        }
        if self.participants.contains(&agent_id) {
            return Err(format!("agent {} already participating", agent_id));
        }

        self.participants.push(agent_id.clone());
        let sub_goal = SubGoal::new(
            self.id.clone(),
            agent_id.clone(),
            sub_goal_description.clone(),
            now_ms,
        );
        let sub_goal_id = sub_goal.id.clone();
        self.sub_goals.insert(sub_goal_id.clone(), sub_goal);

        // Publish goal.proposed event if this is the first participant
        if self.status == CollectiveStatus::Proposed {
            let event = Event {
                id: EventId::new(),
                topic: Topic::system(),
                source: self.proposer_id.clone(),
                timestamp: now_ms,
                event_type: "goal.proposed".to_string(),
                payload: serde_json::json!({
                    "goal_id": self.id,
                    "title": self.title,
                    "proposer_id": self.proposer_id,
                    "correlation_id": self.correlation_id,
                }),
                metadata: EventMetadata {
                    correlation_id: Some(self.correlation_id.clone()),
                    priority: EventPriority::Normal,
                    tags: vec!["saes-0.4".to_string(), "collective".to_string()],
                    ..Default::default()
                },
            };
            if let Some(pub_fn) = publisher {
                let _ = pub_fn(event);
            }
            self.status = CollectiveStatus::Active;
        }

        // Publish agent.joined event
        let join_event = Event {
            id: EventId::new(),
            topic: Topic::system(),
            source: agent_id.clone(),
            timestamp: now_ms,
            event_type: "agent.joined".to_string(),
            payload: serde_json::json!({
                "goal_id": self.id,
                "agent_id": agent_id,
                "sub_goal_description": sub_goal_description,
                "correlation_id": self.correlation_id,
            }),
            metadata: EventMetadata {
                correlation_id: Some(self.correlation_id.clone()),
                priority: EventPriority::Normal,
                tags: vec![
                    "saes-0.4".to_string(),
                    "collective".to_string(),
                    "agent-joined".to_string(),
                ],
                ..Default::default()
            },
        };
        if let Some(pub_fn) = publisher {
            let _ = pub_fn(join_event);
        }

        self.updated_at = now_ms;

        Ok(sub_goal_id)
    }

    /// Update sub-goal progress and recompute collective progress.
    /// Publishes `progress.updated` event via the provided publisher.
    pub fn update_sub_goal_progress(
        &mut self,
        sub_goal_id: &SubGoalId,
        progress: f32,
        now_ms: u64,
        publisher: Option<&dyn Fn(Event) -> Result<(), String>>,
    ) -> Result<(), String> {
        let sub_goal = self
            .sub_goals
            .get_mut(sub_goal_id)
            .ok_or_else(|| format!("sub-goal not found: {}", sub_goal_id))?;

        sub_goal.set_progress(progress, now_ms);
        let agent_id = sub_goal.agent_id.clone();
        let _ = sub_goal;

        self.recompute_progress(now_ms);

        // Publish progress.updated event
        let progress_event = Event {
            id: EventId::new(),
            topic: Topic::system(),
            source: agent_id,
            timestamp: now_ms,
            event_type: "progress.updated".to_string(),
            payload: serde_json::json!({
                "goal_id": self.id,
                "sub_goal_id": sub_goal_id,
                "agent_id": self.sub_goals.get(sub_goal_id).map(|sg| &sg.agent_id).unwrap_or(&"".to_string()),
                "progress": progress,
                "correlation_id": self.correlation_id,
            }),
            metadata: EventMetadata {
                correlation_id: Some(self.correlation_id.clone()),
                priority: EventPriority::Normal,
                tags: vec![
                    "saes-0.4".to_string(),
                    "collective".to_string(),
                    "progress-updated".to_string(),
                ],
                ..Default::default()
            },
        };
        if let Some(pub_fn) = publisher {
            let _ = pub_fn(progress_event);
        }

        Ok(())
    }

    /// Complete a sub-goal and recompute collective state.
    /// Publishes `goal.completed` event via the provided publisher.
    pub fn complete_sub_goal(
        &mut self,
        sub_goal_id: &SubGoalId,
        now_ms: u64,
        publisher: Option<&dyn Fn(Event) -> Result<(), String>>,
    ) -> Result<(), String> {
        // Use a block to limit the scope of the mutable borrow of self.sub_goals
        let agent_id = {
            let sub_goal_mut = self
                .sub_goals
                .get_mut(sub_goal_id)
                .ok_or_else(|| format!("sub-goal not found: {}", sub_goal_id))?;
            sub_goal_mut.complete(now_ms);
            sub_goal_mut.agent_id.clone()
        };

        self.recompute_progress(now_ms);

        // Now get the completed sub-goal data for the event.
        let sub_goal_for_event = self.sub_goals.get(sub_goal_id);

        // Publish goal.completed event
        let completed_event = Event {
            id: EventId::new(),
            topic: Topic::system(),
            source: agent_id,
            timestamp: now_ms,
            event_type: "goal.completed".to_string(),
            payload: serde_json::json!({
                "goal_id": self.id,
                "sub_goal_id": sub_goal_id,
                "agent_id": sub_goal_for_event
                    .map(|sg| sg.agent_id.clone())
                    .unwrap_or_default(),
                "correlation_id": self.correlation_id,
            }),
            metadata: EventMetadata {
                correlation_id: Some(self.correlation_id.clone()),
                priority: EventPriority::Normal,
                tags: vec![
                    "saes-0.4".to_string(),
                    "collective".to_string(),
                    "goal-completed".to_string(),
                ],
                ..Default::default()
            },
        };
        if let Some(pub_fn) = publisher {
            let _ = pub_fn(completed_event);
        }

        Ok(())
    }

    /// Fail a sub-goal and apply failure policy.
    /// Publishes `goal.failed` event via the provided publisher.
    pub fn fail_sub_goal(
        &mut self,
        sub_goal_id: &SubGoalId,
        reason: String,
        now_ms: u64,
        publisher: Option<&dyn Fn(Event) -> Result<(), String>>,
    ) -> Result<(), String> {
        let agent_id = {
            let sub_goal = self
                .sub_goals
                .get_mut(sub_goal_id)
                .ok_or_else(|| format!("sub-goal not found: {}", sub_goal_id))?;

            sub_goal.fail(reason.clone(), now_ms);
            sub_goal.agent_id.clone()
        };

        self.recompute_progress(now_ms);

        // Publish goal.failed event
        let failed_event = Event {
            id: EventId::new(),
            topic: Topic::system(),
            source: agent_id.clone(),
            timestamp: now_ms,
            event_type: "goal.failed".to_string(),
            payload: serde_json::json!({
                "goal_id": self.id,
                "sub_goal_id": sub_goal_id,
                "agent_id": agent_id,
                "reason": reason,
                "correlation_id": self.correlation_id,
            }),
            metadata: EventMetadata {
                correlation_id: Some(self.correlation_id.clone()),
                priority: EventPriority::Normal,
                tags: vec![
                    "saes-0.4".to_string(),
                    "collective".to_string(),
                    "goal-failed".to_string(),
                ],
                ..Default::default()
            },
        };
        if let Some(pub_fn) = publisher {
            let _ = pub_fn(failed_event);
        }

        // Apply failure policy.
        match self.failure_policy {
            FailurePolicy::FailFast => {
                self.status = CollectiveStatus::Failed;
                self.failure_reason = Some(format!("sub-goal {} failed: {}", sub_goal_id, reason));
                self.completed_at = Some(now_ms);
            }
            FailurePolicy::Tolerant => {
                // Check if enough sub-goals are still viable.
                let total = self.sub_goals.len();
                let failed = self
                    .sub_goals
                    .values()
                    .filter(|sg| sg.state == GoalState::Failed)
                    .count();
                let remaining = total - failed;
                // If remaining is less than or equal to half, fail the collective.
                if remaining <= total / 2 && total > 0 {
                    self.status = CollectiveStatus::Failed;
                    self.failure_reason = Some(format!("{} of {} sub-goals failed", failed, total));
                    self.completed_at = Some(now_ms);
                }
            }
            FailurePolicy::Ignore => {
                // Only fail if ALL sub-goals failed.
                let all_failed = self
                    .sub_goals
                    .values()
                    .all(|sg| sg.state == GoalState::Failed);
                if all_failed && !self.sub_goals.is_empty() {
                    self.status = CollectiveStatus::Failed;
                    self.failure_reason = Some("all sub-goals failed".to_string());
                    self.completed_at = Some(now_ms);
                }
            }
        }

        self.updated_at = now_ms;
        Ok(())
    }

    /// Recompute collective progress from sub-goals.
    pub fn recompute_progress(&mut self, now_ms: u64) {
        if self.sub_goals.is_empty() {
            self.progress = 0.0;
            return;
        }

        let total: f32 = self.sub_goals.len() as f32;
        let sum: f32 = self.sub_goals.values().map(|sg| sg.progress).sum();
        self.progress = sum / total;

        // Check if all sub-goals are completed.
        let all_completed = self
            .sub_goals
            .values()
            .all(|sg| sg.state == GoalState::Completed);

        if all_completed && self.status == CollectiveStatus::Active {
            self.status = CollectiveStatus::Completed;
            self.progress = 1.0;
            self.completed_at = Some(now_ms);
        }

        self.updated_at = now_ms;
    }

    pub fn is_overdue(&self, now_ms: u64) -> bool {
        self.deadline
            .is_some_and(|d| now_ms > d && !self.status.is_terminal())
    }
}

/// Trait for collective goal persistence.
#[async_trait::async_trait]
pub trait CollectiveGoalStore: Send + Sync {
    /// Create a new collective goal.
    async fn create(&self, goal: CollectiveGoal) -> Result<(), String>;
    /// Get a collective goal by ID.
    async fn get(&self, goal_id: &CollectiveGoalId) -> Result<CollectiveGoal, String>;
    /// Update a collective goal.
    async fn update(&self, goal: CollectiveGoal) -> Result<(), String>;
    /// List all collective goals.
    async fn list_all(&self) -> Vec<CollectiveGoal>;
    /// List collective goals by status.
    async fn list_by_status(&self, status: CollectiveStatus) -> Vec<CollectiveGoal>;
    /// List collective goals where an agent is a participant.
    async fn list_by_participant(&self, agent_id: &AgentId) -> Vec<CollectiveGoal>;
    /// Delete a collective goal.
    async fn delete(&self, goal_id: &CollectiveGoalId) -> Result<(), String>;
}

/// In-memory collective goal store for tests.
pub struct InMemoryCollectiveGoalStore {
    goals: std::sync::RwLock<HashMap<CollectiveGoalId, CollectiveGoal>>,
}

impl InMemoryCollectiveGoalStore {
    pub fn new() -> Self {
        Self {
            goals: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryCollectiveGoalStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl CollectiveGoalStore for InMemoryCollectiveGoalStore {
    async fn create(&self, goal: CollectiveGoal) -> Result<(), String> {
        let mut goals = self.goals.write().map_err(|e| e.to_string())?;
        if goals.contains_key(&goal.id) {
            return Err(format!("goal already exists: {}", goal.id));
        }
        goals.insert(goal.id.clone(), goal);
        Ok(())
    }

    async fn get(&self, goal_id: &CollectiveGoalId) -> Result<CollectiveGoal, String> {
        let goals = self.goals.read().map_err(|e| e.to_string())?;
        goals
            .get(goal_id)
            .cloned()
            .ok_or_else(|| format!("goal not found: {}", goal_id))
    }

    async fn update(&self, goal: CollectiveGoal) -> Result<(), String> {
        let mut goals = self.goals.write().map_err(|e| e.to_string())?;
        if !goals.contains_key(&goal.id) {
            return Err(format!("goal not found: {}", goal.id));
        }
        goals.insert(goal.id.clone(), goal);
        Ok(())
    }

    async fn list_all(&self) -> Vec<CollectiveGoal> {
        let goals = self.goals.read().unwrap_or_else(|e| e.into_inner());
        goals.values().cloned().collect()
    }

    async fn list_by_status(&self, status: CollectiveStatus) -> Vec<CollectiveGoal> {
        let goals = self.goals.read().unwrap_or_else(|e| e.into_inner());
        goals
            .values()
            .filter(|g| g.status == status)
            .cloned()
            .collect()
    }

    async fn list_by_participant(&self, agent_id: &AgentId) -> Vec<CollectiveGoal> {
        let goals = self.goals.read().unwrap_or_else(|e| e.into_inner());
        goals
            .values()
            .filter(|g| g.participants.contains(agent_id))
            .cloned()
            .collect()
    }

    async fn delete(&self, goal_id: &CollectiveGoalId) -> Result<(), String> {
        let mut goals = self.goals.write().map_err(|e| e.to_string())?;
        goals
            .remove(goal_id)
            .ok_or_else(|| format!("goal not found: {}", goal_id))?;
        Ok(())
    }
}

/// Simple UUID-like generator.
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:x}", t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_collective_goal(proposer: &str) -> CollectiveGoal {
        CollectiveGoal::new(
            "Test Goal".to_string(),
            "A test collective goal".to_string(),
            "research".to_string(),
            proposer.to_string(),
            GoalPriority::HIGH,
            FailurePolicy::Tolerant,
            1000,
        )
    }

    #[test]
    fn collective_goal_propose() {
        let goal = test_collective_goal("agent-a");
        assert_eq!(goal.status, CollectiveStatus::Proposed);
        assert_eq!(goal.proposer_id, "agent-a");
        assert!(goal.participants.is_empty());
        assert!(goal.sub_goals.is_empty());
    }

    #[test]
    fn collective_goal_join() {
        let mut goal = test_collective_goal("agent-a");
        let sub_id = goal
            .add_participant("agent-b".to_string(), "do analysis".to_string(), 1001, None)
            .unwrap();

        assert_eq!(goal.status, CollectiveStatus::Active);
        assert_eq!(goal.participants.len(), 1);
        assert!(goal.participants.contains(&"agent-b".to_string()));
        assert!(goal.sub_goals.contains_key(&sub_id));
    }

    #[test]
    fn collective_goal_duplicate_join_rejected() {
        let mut goal = test_collective_goal("agent-a");
        goal.add_participant("agent-b".to_string(), "task 1".to_string(), 1001, None)
            .unwrap();
        let result = goal.add_participant("agent-b".to_string(), "task 2".to_string(), 1002, None);
        assert!(result.is_err());
    }

    #[test]
    fn collective_goal_progress_aggregation() {
        let mut goal = test_collective_goal("agent-a");
        let sub1 = goal
            .add_participant("agent-a".to_string(), "task 1".to_string(), 1001, None)
            .unwrap();
        let sub2 = goal
            .add_participant("agent-b".to_string(), "task 2".to_string(), 1002, None)
            .unwrap();

        goal.update_sub_goal_progress(&sub1, 0.6, 1010, None)
            .unwrap();
        assert!((goal.progress - 0.3).abs() < 0.01); // 0.6/2 = 0.3

        goal.update_sub_goal_progress(&sub2, 0.8, 1011, None)
            .unwrap();
        assert!((goal.progress - 0.7).abs() < 0.01); // (0.6+0.8)/2 = 0.7
    }

    #[test]
    fn collective_goal_completion() {
        let mut goal = test_collective_goal("agent-a");
        let sub1 = goal
            .add_participant("agent-a".to_string(), "task 1".to_string(), 1001, None)
            .unwrap();
        let sub2 = goal
            .add_participant("agent-b".to_string(), "task 2".to_string(), 1002, None)
            .unwrap();

        goal.complete_sub_goal(&sub1, 1010, None).unwrap();
        assert_eq!(goal.status, CollectiveStatus::Active);
        assert!((goal.progress - 0.5).abs() < 0.01);

        goal.complete_sub_goal(&sub2, 1011, None).unwrap();
        assert_eq!(goal.status, CollectiveStatus::Completed);
        assert_eq!(goal.progress, 1.0);
    }

    #[test]
    fn collective_goal_fail_fast() {
        let mut goal = CollectiveGoal::new(
            "Critical Task".to_string(),
            "Must not fail".to_string(),
            "deployment".to_string(),
            "agent-a".to_string(),
            GoalPriority::CRITICAL,
            FailurePolicy::FailFast,
            1000,
        );
        let sub1 = goal
            .add_participant("agent-a".to_string(), "task 1".to_string(), 1001, None)
            .unwrap();
        goal.add_participant("agent-b".to_string(), "task 2".to_string(), 1002, None)
            .unwrap();

        goal.fail_sub_goal(&sub1, "crashed".to_string(), 1010, None)
            .unwrap();
        assert_eq!(goal.status, CollectiveStatus::Failed);
        assert!(goal.failure_reason.is_some());
    }

    #[test]
    fn collective_goal_fail_tolerant() {
        let mut goal = CollectiveGoal::new(
            "Research Task".to_string(),
            "Can tolerate some failures".to_string(),
            "research".to_string(),
            "agent-a".to_string(),
            GoalPriority::NORMAL,
            FailurePolicy::Tolerant,
            1000,
        );
        let sub1 = goal
            .add_participant("agent-a".to_string(), "task 1".to_string(), 1001, None)
            .unwrap();
        let sub2 = goal
            .add_participant("agent-b".to_string(), "task 2".to_string(), 1002, None)
            .unwrap();
        goal.add_participant("agent-c".to_string(), "task 3".to_string(), 1003, None)
            .unwrap();

        // One failure out of 3 — should still be active.
        goal.fail_sub_goal(&sub1, "timeout".to_string(), 1010, None)
            .unwrap();
        assert_eq!(goal.status, CollectiveStatus::Active);

        // Two failures out of 3 — less than half remaining, should fail.
        goal.fail_sub_goal(&sub2, "error".to_string(), 1011, None)
            .unwrap();
        assert_eq!(goal.status, CollectiveStatus::Failed);
    }

    #[test]
    fn collective_goal_fail_ignore() {
        let mut goal = CollectiveGoal::new(
            "Best-effort Task".to_string(),
            "Failures ignored".to_string(),
            "exploration".to_string(),
            "agent-a".to_string(),
            GoalPriority::LOW,
            FailurePolicy::Ignore,
            1000,
        );
        let sub1 = goal
            .add_participant("agent-a".to_string(), "task 1".to_string(), 1001, None)
            .unwrap();
        goal.add_participant("agent-b".to_string(), "task 2".to_string(), 1002, None)
            .unwrap();

        // One failure — ignored.
        goal.fail_sub_goal(&sub1, "failed".to_string(), 1010, None)
            .unwrap();
        assert_eq!(goal.status, CollectiveStatus::Active);

        // Complete the other one.
        let sub2_id = goal
            .sub_goals
            .keys()
            .find(|k| k != &&sub1)
            .cloned()
            .unwrap();
        goal.complete_sub_goal(&sub2_id, 1011, None).unwrap();
        // Still active because not all completed (one failed, one completed).
        assert_eq!(goal.status, CollectiveStatus::Active);
    }

    #[test]
    fn collective_goal_terminal_blocks_join() {
        let mut goal = test_collective_goal("agent-a");
        let sub1 = goal
            .add_participant("agent-a".to_string(), "task 1".to_string(), 1001, None)
            .unwrap();
        let sub2 = goal
            .add_participant("agent-b".to_string(), "task 2".to_string(), 1002, None)
            .unwrap();
        goal.complete_sub_goal(&sub1, 1010, None).unwrap();
        goal.complete_sub_goal(&sub2, 1011, None).unwrap();

        assert_eq!(goal.status, CollectiveStatus::Completed);
        let result =
            goal.add_participant("agent-c".to_string(), "too late".to_string(), 1020, None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn in_memory_collective_store_crud() {
        let store = InMemoryCollectiveGoalStore::new();
        let goal = test_collective_goal("agent-a");

        store.create(goal.clone()).await.unwrap();
        let loaded = store.get(&goal.id).await.unwrap();
        assert_eq!(loaded.id, goal.id);

        let all = store.list_all().await;
        assert_eq!(all.len(), 1);

        let active = store.list_by_status(CollectiveStatus::Proposed).await;
        assert_eq!(active.len(), 1);

        store.delete(&goal.id).await.unwrap();
        assert!(store.get(&goal.id).await.is_err());
    }

    #[tokio::test]
    async fn in_memory_collective_store_participant_query() {
        let store = InMemoryCollectiveGoalStore::new();
        let mut g1 = test_collective_goal("agent-a");
        g1.add_participant("agent-b".to_string(), "task".to_string(), 1001, None)
            .unwrap();
        store.create(g1).await.unwrap();

        let mut g2 = test_collective_goal("agent-c");
        g2.add_participant("agent-d".to_string(), "task".to_string(), 1002, None)
            .unwrap();
        store.create(g2).await.unwrap();

        let b_goals = store.list_by_participant(&"agent-b".to_string()).await;
        assert_eq!(b_goals.len(), 1);
        assert!(b_goals[0].participants.contains(&"agent-b".to_string()));

        // agent-a is proposer but NOT a participant (no sub-goal assigned).
        let a_goals = store.list_by_participant(&"agent-a".to_string()).await;
        assert_eq!(a_goals.len(), 0);
    }
}
