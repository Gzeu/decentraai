//! SAES 0.2 Step 2: Outcome system
//!
//! Structured outcomes that tie action results to goals and produce
//! evidence entries for the learning loop.

use serde::{Deserialize, Serialize};

/// A structured outcome produced after an action executes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionOutcome {
    /// The action that produced this outcome.
    pub action_id: String,
    /// The agent that performed the action.
    pub agent_id: String,
    /// The goal this action was pursuing (if any).
    pub goal_id: Option<String>,
    /// Whether the action succeeded.
    pub success: bool,
    /// Outcome kind (e.g. "bid_won", "task_completed", "task_failed").
    pub kind: String,
    /// Numeric reward earned (if any).
    pub reward: Option<u64>,
    /// Reputation delta (positive = gain, negative = loss).
    pub reputation_delta: Option<f32>,
    /// Duration of the action in milliseconds.
    pub duration_ms: Option<u64>,
    /// Free-form output data.
    pub output: serde_json::Value,
    /// Error message (if success = false).
    pub error: Option<String>,
    /// Evidence ID linking to the evidence store.
    pub evidence_id: Option<String>,
    /// When this outcome was recorded.
    pub recorded_at: u64,
    /// Free-form metadata.
    pub metadata: serde_json::Value,
}

impl ActionOutcome {
    /// Create a new outcome from an action result and optional goal.
    pub fn from_action_result(
        action_id: String,
        agent_id: String,
        goal_id: Option<String>,
        result: &crate::ActionResult,
        action_kind: &str,
        now_ms: u64,
    ) -> Self {
        Self {
            action_id,
            agent_id,
            goal_id,
            success: result.success,
            kind: action_kind.to_string(),
            reward: result.reward,
            reputation_delta: result.reputation_delta,
            duration_ms: None,
            output: result.output.clone().unwrap_or(serde_json::Value::Null),
            error: result.error.clone(),
            evidence_id: result.evidence_id.clone(),
            recorded_at: now_ms,
            metadata: serde_json::Value::Null,
        }
    }

    /// Whether this outcome contributes positively to the agent's learning.
    pub fn is_positive(&self) -> bool {
        self.success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_result(success: bool) -> crate::ActionResult {
        crate::ActionResult {
            success,
            output: Some(serde_json::json!({"key": "value"})),
            error: if success {
                None
            } else {
                Some("boom".to_string())
            },
            evidence_id: Some("ev-001".to_string()),
            reward: if success { Some(100) } else { None },
            reputation_delta: if success { Some(0.1) } else { Some(-0.05) },
        }
    }

    #[test]
    fn outcome_from_successful_result() {
        let r = sample_result(true);
        let o = ActionOutcome::from_action_result(
            "act-1".into(),
            "agent-1".into(),
            Some("goal-1".into()),
            &r,
            "bid_won",
            1000,
        );
        assert!(o.success);
        assert!(o.is_positive());
        assert_eq!(o.reward, Some(100));
        assert_eq!(o.goal_id.as_deref(), Some("goal-1"));
        assert_eq!(o.kind, "bid_won");
    }

    #[test]
    fn outcome_from_failed_result() {
        let r = sample_result(false);
        let o = ActionOutcome::from_action_result(
            "act-2".into(),
            "agent-1".into(),
            None,
            &r,
            "task_failed",
            2000,
        );
        assert!(!o.success);
        assert!(!o.is_positive());
        assert_eq!(o.error.as_deref(), Some("boom"));
        assert!(o.goal_id.is_none());
        assert_eq!(o.kind, "task_failed");
    }

    #[test]
    fn outcome_serialization_roundtrip() {
        let r = sample_result(true);
        let o = ActionOutcome::from_action_result(
            "act-3".into(),
            "agent-1".into(),
            None,
            &r,
            "test_action",
            3000,
        );
        let json = serde_json::to_string(&o).unwrap();
        let back: ActionOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(o.action_id, back.action_id);
        assert_eq!(o.success, back.success);
    }
}
