//! SAES 0.2 Step 3: Learning loop
//!
//! Connects outcomes to goals and reputation. When an action completes,
//! the learning module:
//! 1. Records the outcome as a structured `LearningEntry`
//! 2. Updates the goal's progress/state based on the outcome
//! 3. Emits a learning event on the EventBus
//!
//! This module is pure decision logic. It does NOT perform I/O directly;
//! it produces `LearningEffect` structs that the runtime applies.

use serde::{Deserialize, Serialize};

use super::goals::{AgentGoal, GoalId, GoalState};
use super::outcomes::ActionOutcome;

/// A single learning entry: what happened, what we learned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEntry {
    /// Unique ID for this learning entry.
    pub id: String,
    /// The agent that learned this.
    pub agent_id: String,
    /// The goal that was being pursued (if any).
    pub goal_id: Option<String>,
    /// The outcome that triggered this learning.
    pub outcome_kind: String,
    /// Whether the outcome was positive.
    pub positive: bool,
    /// What the agent should do differently next time (free-form).
    pub lesson: String,
    /// Confidence in the lesson (0.0..=1.0).
    pub confidence: f32,
    /// When this was recorded.
    pub recorded_at: u64,
}

/// The effects of learning that the runtime must apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEffect {
    /// Goal state transitions to apply.
    pub goal_transitions: Vec<(GoalId, GoalState)>,
    /// Goal progress updates: (goal_id, new_progress).
    pub goal_progress_updates: Vec<(GoalId, f32)>,
    /// The learning entry to record.
    pub entry: LearningEntry,
    /// Metrics to update: (tasks_completed_delta, tasks_failed_delta, reward_delta).
    pub metrics_delta: (u64, u64, u64),
}

/// Compute the learning effect from an outcome and the agent's current goals.
///
/// This is a pure function: given the same inputs, it always produces the same output.
pub fn compute_learning_effect(
    outcome: &ActionOutcome,
    active_goals: &[AgentGoal],
    now_ms: u64,
) -> LearningEffect {
    let lesson = derive_lesson(outcome, active_goals);
    let confidence = if outcome.success { 0.8 } else { 0.9 }; // failures teach more
    let goal_id = outcome.goal_id.clone();

    // Determine goal transitions and progress updates
    let mut goal_transitions = Vec::new();
    let mut goal_progress_updates = Vec::new();

    #[allow(clippy::collapsible_if)]
    if let Some((goal, gid)) = goal_id
        .as_ref()
        .and_then(|gid| active_goals.iter().find(|g| g.id == *gid).map(|g| (g, gid)))
    {
        if goal.state == GoalState::Active {
            if outcome.success {
                // Progress toward completion
                let new_progress = (goal.progress + 0.2).min(1.0);
                goal_progress_updates.push((gid.clone(), new_progress));
                if new_progress >= 1.0 {
                    goal_transitions.push((gid.clone(), GoalState::Completed));
                }
            } else {
                // Failed outcome → fail the goal
                goal_transitions.push((gid.clone(), GoalState::Failed));
            }
        }
    }

    // Metrics deltas
    let tasks_completed = if outcome.success { 1 } else { 0 };
    let tasks_failed = if outcome.success { 0 } else { 1 };
    let reward = outcome.reward.unwrap_or(0);

    let entry = LearningEntry {
        id: format!("learn-{}-{}", outcome.agent_id, now_ms),
        agent_id: outcome.agent_id.clone(),
        goal_id,
        outcome_kind: outcome.kind.clone(),
        positive: outcome.is_positive(),
        lesson,
        confidence,
        recorded_at: now_ms,
    };

    LearningEffect {
        goal_transitions,
        goal_progress_updates,
        entry,
        metrics_delta: (tasks_completed, tasks_failed, reward),
    }
}

/// Derive a lesson from an outcome and active goals.
fn derive_lesson(outcome: &ActionOutcome, active_goals: &[AgentGoal]) -> String {
    let goal_context = outcome
        .goal_id
        .as_ref()
        .and_then(|gid| active_goals.iter().find(|g| g.id == *gid))
        .map(|g| format!(" while pursuing goal '{}'", g.description))
        .unwrap_or_default();

    if outcome.success {
        format!(
            "Action '{}' succeeded{}. Reward: {:?}.",
            outcome.kind, goal_context, outcome.reward
        )
    } else {
        format!(
            "Action '{}' failed{}: {}. Adjust strategy.",
            outcome.kind,
            goal_context,
            outcome.error.as_deref().unwrap_or("unknown error")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::goals::{AgentGoal, GoalPriority};
    use super::*;

    fn agent_id() -> String {
        "test-agent:worker".to_string()
    }

    fn make_goal(id: &str, state: GoalState, progress: f32) -> AgentGoal {
        let mut g = AgentGoal::new(
            agent_id(),
            format!("goal {}", id),
            "test".to_string(),
            GoalPriority::NORMAL,
            1000,
        );
        g.id = id.to_string();
        g.state = state;
        g.progress = progress;
        g
    }

    fn success_outcome(goal_id: Option<String>) -> ActionOutcome {
        ActionOutcome {
            action_id: "act-1".into(),
            agent_id: agent_id(),
            goal_id,
            success: true,
            kind: "bid_won".into(),
            reward: Some(50),
            reputation_delta: Some(0.1),
            duration_ms: Some(100),
            output: serde_json::json!({}),
            error: None,
            evidence_id: Some("ev-1".into()),
            recorded_at: 2000,
            metadata: serde_json::Value::Null,
        }
    }

    fn failure_outcome(goal_id: Option<String>) -> ActionOutcome {
        ActionOutcome {
            action_id: "act-2".into(),
            agent_id: agent_id(),
            goal_id,
            success: false,
            kind: "task_failed".into(),
            reward: None,
            reputation_delta: Some(-0.05),
            duration_ms: None,
            output: serde_json::Value::Null,
            error: Some("timeout".into()),
            evidence_id: None,
            recorded_at: 3000,
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn learning_effect_no_goal() {
        let o = success_outcome(None);
        let effect = compute_learning_effect(&o, &[], 2000);
        assert!(effect.entry.positive);
        assert!(effect.goal_transitions.is_empty());
        assert_eq!(effect.metrics_delta, (1, 0, 50));
    }

    #[test]
    fn learning_effect_goal_progress() {
        let goal = make_goal("g1", GoalState::Active, 0.5);
        let o = success_outcome(Some("g1".into()));
        let effect = compute_learning_effect(&o, &[goal], 2000);
        assert_eq!(effect.goal_progress_updates.len(), 1);
        assert_eq!(effect.goal_progress_updates[0], ("g1".into(), 0.7));
        assert!(effect.goal_transitions.is_empty());
    }

    #[test]
    fn learning_effect_goal_completion() {
        let goal = make_goal("g1", GoalState::Active, 0.9);
        let o = success_outcome(Some("g1".into()));
        let effect = compute_learning_effect(&o, &[goal], 2000);
        assert_eq!(effect.goal_progress_updates[0], ("g1".into(), 1.0));
        assert_eq!(
            effect.goal_transitions[0],
            ("g1".into(), GoalState::Completed)
        );
    }

    #[test]
    fn learning_effect_goal_failure() {
        let goal = make_goal("g1", GoalState::Active, 0.3);
        let o = failure_outcome(Some("g1".into()));
        let effect = compute_learning_effect(&o, &[goal], 3000);
        assert_eq!(effect.goal_transitions[0], ("g1".into(), GoalState::Failed));
        assert!(effect.goal_progress_updates.is_empty());
        assert_eq!(effect.metrics_delta, (0, 1, 0));
    }

    #[test]
    fn lesson_includes_goal_context() {
        let goal = make_goal("g1", GoalState::Active, 0.5);
        let o = success_outcome(Some("g1".into()));
        let effect = compute_learning_effect(&o, &[goal], 2000);
        assert!(effect.entry.lesson.contains("goal 'goal g1'"));
    }

    #[test]
    fn lesson_failure_has_higher_confidence() {
        let o_fail = failure_outcome(None);
        let o_ok = success_outcome(None);
        let effect_fail = compute_learning_effect(&o_fail, &[], 2000);
        let effect_ok = compute_learning_effect(&o_ok, &[], 2000);
        assert!(effect_fail.entry.confidence > effect_ok.entry.confidence);
    }

    #[test]
    fn learning_entry_serialization() {
        let o = success_outcome(None);
        let effect = compute_learning_effect(&o, &[], 2000);
        let json = serde_json::to_string(&effect).unwrap();
        let back: LearningEffect = serde_json::from_str(&json).unwrap();
        assert_eq!(effect.entry.id, back.entry.id);
        assert_eq!(effect.metrics_delta, back.metrics_delta);
    }
}
