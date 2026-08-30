//! SAES 0.2 Step 4: Behavior adaptation
//!
//! The adaptation module translates learning outcomes into changed behavior.
//! It maintains per-agent `BehaviorProfile` that tracks:
//! - Success/failure rates per action kind
//! - Preferred strategies (what works)
//! - Avoided strategies (what doesn't work)
//! - Goal completion patterns
//!
//! The profile is consulted by the decision policy to bias future decisions.
//! This is the "changed behaviour" step in the autonomous cycle.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::learning::LearningEntry;

/// Per-agent behavior profile that accumulates learning.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BehaviorProfile {
    /// Agent this profile belongs to.
    pub agent_id: String,
    /// Success count per action kind.
    pub success_counts: HashMap<String, u64>,
    /// Failure count per action kind.
    pub failure_counts: HashMap<String, u64>,
    /// Total reward earned per action kind.
    pub reward_by_kind: HashMap<String, u64>,
    /// Goal completion rate (completed / total terminal).
    pub goal_completion_rate: f32,
    /// Total goals completed.
    pub goals_completed: u64,
    /// Total goals failed.
    pub goals_failed: u64,
    /// Total goals abandoned.
    pub goals_abandoned: u64,
    /// Preferred strategies: action kinds with >60% success rate and >2 samples.
    pub preferred_strategies: Vec<String>,
    /// Avoided strategies: action kinds with <30% success rate and >2 samples.
    pub avoided_strategies: Vec<String>,
    /// Number of learning entries incorporated.
    pub entries_processed: u64,
    /// When the profile was last updated.
    pub last_updated: u64,
}

impl BehaviorProfile {
    /// Create a new empty profile for an agent.
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            ..Default::default()
        }
    }

    /// Incorporate a learning entry into the profile.
    pub fn incorporate(&mut self, entry: &LearningEntry) {
        let kind = entry.outcome_kind.clone();
        if entry.positive {
            *self.success_counts.entry(kind.clone()).or_insert(0) += 1;
        } else {
            *self.failure_counts.entry(kind.clone()).or_insert(0) += 1;
        }
        self.entries_processed += 1;
        self.last_updated = entry.recorded_at;
        self.recompute_strategies();
    }

    /// Update goal completion stats.
    pub fn update_goal_stats(&mut self, completed: u64, failed: u64, abandoned: u64, now_ms: u64) {
        self.goals_completed = completed;
        self.goals_failed = failed;
        self.goals_abandoned = abandoned;
        let total = completed + failed + abandoned;
        self.goal_completion_rate = if total > 0 {
            completed as f32 / total as f32
        } else {
            0.0
        };
        self.last_updated = now_ms;
    }

    /// Recompute preferred/avoided strategies from current counts.
    fn recompute_strategies(&mut self) {
        self.preferred_strategies.clear();
        self.avoided_strategies.clear();

        let all_kinds: std::collections::HashSet<String> = self
            .success_counts
            .keys()
            .chain(self.failure_counts.keys())
            .cloned()
            .collect();

        for kind in all_kinds {
            let successes = self.success_counts.get(&kind).copied().unwrap_or(0);
            let failures = self.failure_counts.get(&kind).copied().unwrap_or(0);
            let total = successes + failures;

            if total < 3 {
                continue; // Not enough data
            }

            let rate = successes as f32 / total as f32;
            if rate > 0.6 {
                self.preferred_strategies.push(kind);
            } else if rate < 0.3 {
                self.avoided_strategies.push(kind);
            }
        }

        self.preferred_strategies.sort();
        self.avoided_strategies.sort();
    }

    /// Get the success rate for an action kind.
    pub fn success_rate(&self, kind: &str) -> f32 {
        let successes = self.success_counts.get(kind).copied().unwrap_or(0);
        let failures = self.failure_counts.get(kind).copied().unwrap_or(0);
        let total = successes + failures;
        if total == 0 {
            return 0.5; // Unknown = neutral
        }
        successes as f32 / total as f32
    }

    /// Get the confidence in the success rate (based on sample count).
    pub fn confidence(&self, kind: &str) -> f32 {
        let total = self.success_counts.get(kind).copied().unwrap_or(0)
            + self.failure_counts.get(kind).copied().unwrap_or(0);
        // Wilson score lower bound approximation
        if total == 0 {
            return 0.0;
        }
        let n = total as f32;
        let z = 1.96; // 95% confidence
        let p = self.success_rate(kind);
        let denominator = 1.0 + z * z / n;
        let center = p + z * z / (2.0 * n);
        let spread = z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt();
        ((center - spread) / denominator).max(0.0)
    }

    /// Decide whether to use a strategy based on the profile.
    /// Returns (use_strategy, confidence, reason).
    pub fn should_use_strategy(&self, kind: &str) -> (bool, f32, String) {
        if self.avoided_strategies.contains(&kind.to_string()) {
            let rate = self.success_rate(kind);
            return (
                false,
                1.0 - rate,
                format!(
                    "avoided strategy '{}' (success rate: {:.0}%)",
                    kind,
                    rate * 100.0
                ),
            );
        }

        if self.preferred_strategies.contains(&kind.to_string()) {
            let rate = self.success_rate(kind);
            return (
                true,
                rate,
                format!(
                    "preferred strategy '{}' (success rate: {:.0}%)",
                    kind,
                    rate * 100.0
                ),
            );
        }

        // Unknown strategy — neutral, use if no better option
        (
            true,
            0.5,
            format!("unknown strategy '{}', proceeding with caution", kind),
        )
    }
}

/// Store for behavior profiles. Trait for pluggable persistence.
#[async_trait::async_trait]
pub trait BehaviorStore: Send + Sync {
    /// Get or create a profile for an agent.
    async fn get_or_create(&self, agent_id: &str) -> BehaviorProfile;
    /// Save a profile.
    async fn save(&self, profile: BehaviorProfile);
}

/// In-memory behavior store for tests.
pub struct InMemoryBehaviorStore {
    profiles: std::sync::RwLock<HashMap<String, BehaviorProfile>>,
}

impl InMemoryBehaviorStore {
    pub fn new() -> Self {
        Self {
            profiles: std::sync::RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryBehaviorStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl BehaviorStore for InMemoryBehaviorStore {
    async fn get_or_create(&self, agent_id: &str) -> BehaviorProfile {
        let mut profiles = self.profiles.write().unwrap();
        profiles
            .entry(agent_id.to_string())
            .or_insert_with(|| BehaviorProfile::new(agent_id.to_string()))
            .clone()
    }

    async fn save(&self, profile: BehaviorProfile) {
        let mut profiles = self.profiles.write().unwrap();
        profiles.insert(profile.agent_id.clone(), profile);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(kind: &str, positive: bool) -> LearningEntry {
        LearningEntry {
            id: format!("learn-{}", kind),
            agent_id: "test-agent".to_string(),
            goal_id: None,
            outcome_kind: kind.to_string(),
            positive,
            lesson: if positive {
                "good".into()
            } else {
                "bad".into()
            },
            confidence: 0.8,
            recorded_at: 1000,
        }
    }

    #[test]
    fn profile_starts_empty() {
        let p = BehaviorProfile::new("agent-1".into());
        assert_eq!(p.entries_processed, 0);
        assert!(p.preferred_strategies.is_empty());
        assert!(p.avoided_strategies.is_empty());
    }

    #[test]
    fn incorporate_success() {
        let mut p = BehaviorProfile::new("agent-1".into());
        p.incorporate(&make_entry("bid_won", true));
        assert_eq!(p.success_counts.get("bid_won"), Some(&1));
        assert_eq!(p.failure_counts.get("bid_won"), None);
        assert_eq!(p.entries_processed, 1);
    }

    #[test]
    fn incorporate_failure() {
        let mut p = BehaviorProfile::new("agent-1".into());
        p.incorporate(&make_entry("task_failed", false));
        assert_eq!(p.failure_counts.get("task_failed"), Some(&1));
        assert_eq!(p.success_counts.get("task_failed"), None);
    }

    #[test]
    fn preferred_strategy_after_enough_samples() {
        let mut p = BehaviorProfile::new("agent-1".into());
        // 4 successes, 1 failure = 80% success rate > 60% threshold
        for _ in 0..4 {
            p.incorporate(&make_entry("bid_won", true));
        }
        p.incorporate(&make_entry("bid_won", false));
        assert!(p.preferred_strategies.contains(&"bid_won".to_string()));
        assert!(!p.avoided_strategies.contains(&"bid_won".to_string()));
    }

    #[test]
    fn avoided_strategy_after_enough_samples() {
        let mut p = BehaviorProfile::new("agent-1".into());
        // 1 success, 4 failures = 20% success rate < 30% threshold
        p.incorporate(&make_entry("risky_task", true));
        for _ in 0..4 {
            p.incorporate(&make_entry("risky_task", false));
        }
        assert!(p.avoided_strategies.contains(&"risky_task".to_string()));
        assert!(!p.preferred_strategies.contains(&"risky_task".to_string()));
    }

    #[test]
    fn not_enough_data_for_classification() {
        let mut p = BehaviorProfile::new("agent-1".into());
        // 2 samples total, below threshold of 3
        p.incorporate(&make_entry("new_thing", true));
        p.incorporate(&make_entry("new_thing", true));
        assert!(!p.preferred_strategies.contains(&"new_thing".to_string()));
        assert!(!p.avoided_strategies.contains(&"new_thing".to_string()));
    }

    #[test]
    fn success_rate_calculation() {
        let mut p = BehaviorProfile::new("agent-1".into());
        for _ in 0..3 {
            p.incorporate(&make_entry("x", true));
        }
        for _ in 0..2 {
            p.incorporate(&make_entry("x", false));
        }
        assert!((p.success_rate("x") - 0.6).abs() < 1e-6);
        // Unknown kind returns neutral
        assert!((p.success_rate("unknown") - 0.5).abs() < 1e-6);
    }

    #[test]
    fn should_use_preferred() {
        let mut p = BehaviorProfile::new("agent-1".into());
        for _ in 0..5 {
            p.incorporate(&make_entry("safe_task", true));
        }
        p.incorporate(&make_entry("safe_task", false));
        let (use_it, conf, reason) = p.should_use_strategy("safe_task");
        assert!(use_it);
        assert!(conf > 0.6);
        assert!(reason.contains("preferred"));
    }

    #[test]
    fn should_avoid() {
        let mut p = BehaviorProfile::new("agent-1".into());
        p.incorporate(&make_entry("bad_task", true));
        for _ in 0..5 {
            p.incorporate(&make_entry("bad_task", false));
        }
        let (use_it, conf, reason) = p.should_use_strategy("bad_task");
        assert!(!use_it);
        assert!(conf > 0.5);
        assert!(reason.contains("avoided"));
    }

    #[test]
    fn goal_stats() {
        let mut p = BehaviorProfile::new("agent-1".into());
        p.update_goal_stats(7, 2, 1, 5000);
        assert_eq!(p.goals_completed, 7);
        assert_eq!(p.goals_failed, 2);
        assert_eq!(p.goals_abandoned, 1);
        assert!((p.goal_completion_rate - 0.7).abs() < 1e-6);
    }

    #[test]
    fn confidence_increases_with_samples() {
        let mut p = BehaviorProfile::new("agent-1".into());
        let c0 = p.confidence("x");
        p.incorporate(&make_entry("x", true));
        let c1 = p.confidence("x");
        p.incorporate(&make_entry("x", true));
        p.incorporate(&make_entry("x", false));
        let c3 = p.confidence("x");
        assert!(c0 < c1);
        assert!(c1 < c3);
    }

    #[tokio::test]
    async fn store_get_or_create() {
        let store = InMemoryBehaviorStore::new();
        let p = store.get_or_create("agent-1").await;
        assert_eq!(p.agent_id, "agent-1");
        assert_eq!(p.entries_processed, 0);
    }

    #[tokio::test]
    async fn store_save_and_get() {
        let store = InMemoryBehaviorStore::new();
        let mut p = BehaviorProfile::new("agent-1".into());
        p.incorporate(&make_entry("bid_won", true));
        store.save(p).await;
        let loaded = store.get_or_create("agent-1").await;
        assert_eq!(loaded.entries_processed, 1);
    }

    #[test]
    fn profile_serialization_roundtrip() {
        let mut p = BehaviorProfile::new("agent-1".into());
        p.incorporate(&make_entry("test", true));
        let json = serde_json::to_string(&p).unwrap();
        let back: BehaviorProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p.agent_id, back.agent_id);
        assert_eq!(p.entries_processed, back.entries_processed);
    }
}
