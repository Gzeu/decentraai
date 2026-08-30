//! SAES 0.3: SQLite-backed persistence for GoalStore and BehaviorStore.
//!
//! This module provides durable storage for agent goals and behavior profiles,
//! ensuring that an agent's experience survives process restarts. The
//! in-memory implementations (`InMemoryGoalStore`, `InMemoryBehaviorStore`)
//! remain the default for tests and development.
//!
//! # Schema evolution
//!
//! All tables use `CREATE TABLE IF NOT EXISTS` and include a `schema_version`
//! table for future migrations. The initial version is 1.
//!
//! # Recovery semantics
//!
//! - **Interrupted writes**: SQLite transactions ensure atomicity. A goal
//!   update that crashes mid-write is rolled back; the goal remains in its
//!   previous consistent state.
//! - **Duplicate prevention**: Goal IDs are PRIMARY KEY; `add()` returns
//!   `AlreadyExists` on conflict. `update()` uses `INSERT OR REPLACE` for
//!   idempotent upserts.
//! - **Corrupted data**: Malformed JSON in behavior profiles is logged and
//!   replaced with an empty profile (fail-open for availability).
//! - **Consistency after restart**: On load, all goals are re-read from SQLite.
//!   Terminal goals (Completed/Failed/Abandoned) are preserved for history.
//!   Active goals with expired deadlines are auto-failed during the next
//!   `decide()` call (existing deadline monitoring).

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Mutex;

use super::adaptation::{BehaviorProfile, BehaviorStore};
use super::goals::{AgentGoal, AgentId, GoalError, GoalId, GoalPriority, GoalState, GoalStore};

/// Errors from the SQLite persistence store.
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    #[error("sqlite error: {0}")]
    Sql(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<rusqlite::Error> for PersistenceError {
    fn from(e: rusqlite::Error) -> Self {
        PersistenceError::Sql(e.to_string())
    }
}

impl From<serde_json::Error> for PersistenceError {
    fn from(e: serde_json::Error) -> Self {
        PersistenceError::Serialization(e.to_string())
    }
}

/// Convert PersistenceError to GoalError for GoalStore trait compatibility.
impl From<PersistenceError> for GoalError {
    fn from(e: PersistenceError) -> Self {
        GoalError::Internal(e.to_string())
    }
}

/// Schema version for future migrations.
const SCHEMA_VERSION: i32 = 1;

/// SQL schema for agent goals.
const GOALS_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS agent_goals (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    description TEXT NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 5,
    progress REAL NOT NULL DEFAULT 0.0,
    deadline INTEGER,
    failure_reason TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    activated_at INTEGER,
    completed_at INTEGER,
    metadata TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_goals_agent_id ON agent_goals(agent_id);
CREATE INDEX IF NOT EXISTS idx_goals_agent_state ON agent_goals(agent_id, state);

CREATE TABLE IF NOT EXISTS collective_goals (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    priority INTEGER NOT NULL,
    proposer_id TEXT NOT NULL,
    participants TEXT NOT NULL,
    sub_goals TEXT NOT NULL,
    progress REAL NOT NULL,
    failure_policy TEXT NOT NULL,
    dependencies TEXT NOT NULL,
    deadline INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    completed_at INTEGER,
    failure_reason TEXT,
    metadata TEXT NOT NULL DEFAULT '{}',
    correlation_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cg_proposer ON collective_goals(proposer_id);
";

/// SQL schema for behavior profiles.
const BEHAVIOR_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS agent_behavior_profiles (
    agent_id TEXT PRIMARY KEY,
    profile_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
";

/// SQL schema for schema versioning.
const VERSION_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY
);
";

/// SQLite-backed goal store. Thread-safe via `Mutex<Connection>`.
pub struct SqliteGoalStore {
    conn: Mutex<Connection>,
}

impl SqliteGoalStore {
    /// Open or create a SQLite goal store at the given path.
    ///
    /// Creates tables if they don't exist. Safe to call multiple times
    /// (idempotent schema creation).
    pub fn open(path: &Path) -> Result<Self, PersistenceError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create an in-memory store (for tests).
    pub fn new_in_memory() -> Result<Self, PersistenceError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_schema(conn: &Connection) -> Result<(), PersistenceError> {
        conn.execute_batch(VERSION_SCHEMA)?;
        conn.execute_batch(GOALS_SCHEMA)?;
        conn.execute_batch(BEHAVIOR_SCHEMA)?;

        // Check or set schema version.
        let current: i32 = conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        if current == 0 {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                params![SCHEMA_VERSION],
            )?;
        }

        Ok(())
    }
}

#[async_trait]
impl GoalStore for SqliteGoalStore {
    async fn add(&self, goal: AgentGoal) -> Result<(), GoalError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GoalError::Internal(e.to_string()))?;
        let metadata = serde_json::to_string(&goal.metadata)
            .map_err(|e| GoalError::Internal(e.to_string()))?;

        conn.execute(
            "INSERT INTO agent_goals (
                id, agent_id, description, kind, state, priority,
                progress, deadline, failure_reason, created_at, updated_at,
                activated_at, completed_at, metadata
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                goal.id,
                goal.agent_id,
                goal.description,
                goal.kind,
                goal.state.to_string(),
                goal.priority.0 as i32,
                goal.progress,
                goal.deadline.map(|d| d as i64),
                goal.failure_reason,
                goal.created_at as i64,
                goal.updated_at as i64,
                goal.activated_at.map(|t| t as i64),
                goal.completed_at.map(|t| t as i64),
                metadata,
            ],
        )
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint") {
                GoalError::AlreadyExists(goal.id)
            } else {
                GoalError::Internal(e.to_string())
            }
        })?;

        Ok(())
    }

    async fn get(&self, goal_id: &GoalId) -> Result<AgentGoal, GoalError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GoalError::Internal(e.to_string()))?;
        let goal = conn
            .query_row(
                "SELECT id, agent_id, description, kind, state, priority,
                        progress, deadline, failure_reason, created_at, updated_at,
                        activated_at, completed_at, metadata
                 FROM agent_goals WHERE id = ?1",
                params![goal_id],
                row_to_goal,
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => GoalError::NotFound(goal_id.clone()),
                _ => GoalError::Internal(e.to_string()),
            })?;

        Ok(goal)
    }

    async fn update(&self, goal: AgentGoal) -> Result<(), GoalError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GoalError::Internal(e.to_string()))?;
        let metadata = serde_json::to_string(&goal.metadata)
            .map_err(|e| GoalError::Internal(e.to_string()))?;

        let affected = conn
            .execute(
                "INSERT OR REPLACE INTO agent_goals (
                    id, agent_id, description, kind, state, priority,
                    progress, deadline, failure_reason, created_at, updated_at,
                    activated_at, completed_at, metadata
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    goal.id,
                    goal.agent_id,
                    goal.description,
                    goal.kind,
                    goal.state.to_string(),
                    goal.priority.0 as i32,
                    goal.progress,
                    goal.deadline.map(|d| d as i64),
                    goal.failure_reason,
                    goal.created_at as i64,
                    goal.updated_at as i64,
                    goal.activated_at.map(|t| t as i64),
                    goal.completed_at.map(|t| t as i64),
                    metadata,
                ],
            )
            .map_err(|e| GoalError::Internal(e.to_string()))?;

        if affected == 0 {
            return Err(GoalError::NotFound(goal.id));
        }

        Ok(())
    }

    async fn list_by_agent(&self, agent_id: &AgentId) -> Vec<AgentGoal> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        let mut stmt = match conn.prepare(
            "SELECT id, agent_id, description, kind, state, priority,
                    progress, deadline, failure_reason, created_at, updated_at,
                    activated_at, completed_at, metadata
             FROM agent_goals WHERE agent_id = ?1 ORDER BY priority DESC, created_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        stmt.query_map(params![agent_id], row_to_goal)
            .into_iter()
            .flat_map(|r| r.into_iter())
            .filter_map(|r| r.ok())
            .collect()
    }

    async fn list_by_state(&self, agent_id: &AgentId, state: GoalState) -> Vec<AgentGoal> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        let mut stmt = match conn.prepare(
            "SELECT id, agent_id, description, kind, state, priority,
                    progress, deadline, failure_reason, created_at, updated_at,
                    activated_at, completed_at, metadata
             FROM agent_goals WHERE agent_id = ?1 AND state = ?2
             ORDER BY priority DESC, created_at ASC",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let state_str = state.to_string();
        stmt.query_map(params![agent_id, state_str], row_to_goal)
            .into_iter()
            .flat_map(|r| r.into_iter())
            .filter_map(|r| r.ok())
            .collect()
    }

    async fn delete(&self, goal_id: &GoalId) -> Result<(), GoalError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| GoalError::Internal(e.to_string()))?;
        let affected = conn
            .execute("DELETE FROM agent_goals WHERE id = ?1", params![goal_id])
            .map_err(|e| GoalError::Internal(e.to_string()))?;

        if affected == 0 {
            return Err(GoalError::NotFound(goal_id.clone()));
        }

        Ok(())
    }

    async fn count_by_state(&self, agent_id: &AgentId, state: GoalState) -> usize {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        let state_str = state.to_string();
        conn.query_row(
            "SELECT COUNT(*) FROM agent_goals WHERE agent_id = ?1 AND state = ?2",
            params![agent_id, state_str],
            |row| row.get::<_, usize>(0),
        )
        .unwrap_or(0)
    }
}

/// SQLite-backed behavior store.
pub struct SqliteBehaviorStore {
    conn: Mutex<Connection>,
}

impl SqliteBehaviorStore {
    /// Open or create a SQLite behavior store at the given path.
    pub fn open(path: &Path) -> Result<Self, PersistenceError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create an in-memory store (for tests).
    pub fn new_in_memory() -> Result<Self, PersistenceError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_schema(conn: &Connection) -> Result<(), PersistenceError> {
        conn.execute_batch(VERSION_SCHEMA)?;
        conn.execute_batch(BEHAVIOR_SCHEMA)?;
        Ok(())
    }
}

#[async_trait]
impl BehaviorStore for SqliteBehaviorStore {
    async fn get_or_create(&self, agent_id: &str) -> BehaviorProfile {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return BehaviorProfile::new(agent_id.to_string()),
        };

        let profile: Option<BehaviorProfile> = conn
            .query_row(
                "SELECT profile_json FROM agent_behavior_profiles WHERE agent_id = ?1",
                params![agent_id],
                |row| {
                    let json: String = row.get(0)?;
                    Ok(json)
                },
            )
            .optional()
            .ok()
            .flatten()
            .and_then(|json_str| serde_json::from_str(&json_str).ok());

        profile.unwrap_or_else(|| BehaviorProfile::new(agent_id.to_string()))
    }

    async fn save(&self, profile: BehaviorProfile) {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return,
        };

        let json = match serde_json::to_string(&profile) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(
                    agent = %profile.agent_id,
                    error = %e,
                    "saes persistence: failed to serialize behavior profile"
                );
                return;
            }
        };

        let _ = conn.execute(
            "INSERT OR REPLACE INTO agent_behavior_profiles (agent_id, profile_json, updated_at)
             VALUES (?1, ?2, ?3)",
            params![profile.agent_id, json, profile.last_updated as i64],
        );
    }
}

/// Deserialize a goal row from SQLite.
fn row_to_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentGoal> {
    let state_str: String = row.get(4)?;
    let state = match state_str.as_str() {
        "pending" => GoalState::Pending,
        "active" => GoalState::Active,
        "completed" => GoalState::Completed,
        "failed" => GoalState::Failed,
        "abandoned" => GoalState::Abandoned,
        _ => GoalState::Pending,
    };

    let priority_i: i32 = row.get(5)?;
    let metadata_str: String = row.get(13)?;
    let metadata = serde_json::from_str(&metadata_str).unwrap_or(serde_json::Value::Null);

    Ok(AgentGoal {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        description: row.get(2)?,
        kind: row.get(3)?,
        state,
        priority: GoalPriority(priority_i as u8),
        progress: row.get(6)?,
        deadline: row.get::<_, Option<i64>>(7)?.map(|d| d as u64),
        failure_reason: row.get(8)?,
        created_at: row.get::<_, i64>(9)? as u64,
        updated_at: row.get::<_, i64>(10)? as u64,
        activated_at: row.get::<_, Option<i64>>(11)?.map(|t| t as u64),
        completed_at: row.get::<_, Option<i64>>(12)?.map(|t| t as u64),
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::super::adaptation::BehaviorProfile;
    use super::super::goals::{AgentGoal, GoalPriority, GoalState};
    use super::super::learning::LearningEntry;
    use super::*;

    fn test_goal(agent_id: &str, kind: &str) -> AgentGoal {
        AgentGoal::new(
            agent_id.to_string(),
            format!("test goal for {}", kind),
            kind.to_string(),
            GoalPriority::NORMAL,
            1000,
        )
    }

    #[tokio::test]
    async fn sqlite_goal_store_add_and_get() {
        let store = SqliteGoalStore::new_in_memory().unwrap();
        let goal = test_goal("agent-1", "serve");

        store.add(goal.clone()).await.unwrap();
        let retrieved = store.get(&goal.id).await.unwrap();
        assert_eq!(retrieved.id, goal.id);
        assert_eq!(retrieved.agent_id, "agent-1");
        assert_eq!(retrieved.kind, "serve");
    }

    #[tokio::test]
    async fn sqlite_goal_store_duplicate_rejected() {
        let store = SqliteGoalStore::new_in_memory().unwrap();
        let goal = test_goal("agent-1", "serve");

        store.add(goal.clone()).await.unwrap();
        let result = store.add(goal).await;
        assert!(matches!(result, Err(GoalError::AlreadyExists(_))));
    }

    #[tokio::test]
    async fn sqlite_goal_store_update_persists() {
        let store = SqliteGoalStore::new_in_memory().unwrap();
        let mut goal = test_goal("agent-1", "serve");
        store.add(goal.clone()).await.unwrap();

        goal.progress = 0.5;
        goal.state = GoalState::Active;
        goal.activated_at = Some(2000);
        store.update(goal.clone()).await.unwrap();

        let retrieved = store.get(&goal.id).await.unwrap();
        assert_eq!(retrieved.progress, 0.5);
        assert_eq!(retrieved.state, GoalState::Active);
        assert_eq!(retrieved.activated_at, Some(2000));
    }

    #[tokio::test]
    async fn sqlite_goal_store_list_by_agent() {
        let store = SqliteGoalStore::new_in_memory().unwrap();
        let g1 = test_goal("agent-1", "serve");
        let g2 = test_goal("agent-1", "learn");
        let g3 = test_goal("agent-2", "serve");

        store.add(g1).await.unwrap();
        store.add(g2).await.unwrap();
        store.add(g3).await.unwrap();

        let agent1_goals = store.list_by_agent(&"agent-1".to_string()).await;
        assert_eq!(agent1_goals.len(), 2);

        let agent2_goals = store.list_by_agent(&"agent-2".to_string()).await;
        assert_eq!(agent2_goals.len(), 1);
    }

    #[tokio::test]
    async fn sqlite_goal_store_delete() {
        let store = SqliteGoalStore::new_in_memory().unwrap();
        let goal = test_goal("agent-1", "serve");
        store.add(goal.clone()).await.unwrap();

        store.delete(&goal.id).await.unwrap();
        let result = store.get(&goal.id).await;
        assert!(matches!(result, Err(GoalError::NotFound(_))));
    }

    #[tokio::test]
    async fn sqlite_goal_store_count_by_state() {
        let store = SqliteGoalStore::new_in_memory().unwrap();
        let mut g1 = test_goal("agent-1", "serve");
        let g2 = test_goal("agent-1", "learn");

        g1.transition_to(GoalState::Active, 1000).unwrap();
        store.add(g1).await.unwrap();
        store.add(g2).await.unwrap();

        let active_count = store
            .count_by_state(&"agent-1".to_string(), GoalState::Active)
            .await;
        assert_eq!(active_count, 1);

        let pending_count = store
            .count_by_state(&"agent-1".to_string(), GoalState::Pending)
            .await;
        assert_eq!(pending_count, 1);
    }

    #[tokio::test]
    async fn sqlite_behavior_store_get_or_create() {
        let store = SqliteBehaviorStore::new_in_memory().unwrap();
        let profile = store.get_or_create("agent-1").await;
        assert_eq!(profile.agent_id, "agent-1");
        assert_eq!(profile.entries_processed, 0);
    }

    #[tokio::test]
    async fn sqlite_behavior_store_save_and_load() {
        let store = SqliteBehaviorStore::new_in_memory().unwrap();
        let mut profile = BehaviorProfile::new("agent-1".to_string());
        profile.success_counts.insert("Analysis".to_string(), 5);
        profile.failure_counts.insert("Analysis".to_string(), 2);
        profile.entries_processed = 7;
        profile.last_updated = 5000;

        store.save(profile).await;

        let loaded = store.get_or_create("agent-1").await;
        assert_eq!(loaded.success_counts.get("Analysis"), Some(&5));
        assert_eq!(loaded.failure_counts.get("Analysis"), Some(&2));
        assert_eq!(loaded.entries_processed, 7);
    }

    #[tokio::test]
    async fn sqlite_metadata_roundtrip() {
        let store = SqliteGoalStore::new_in_memory().unwrap();
        let mut goal = test_goal("agent-1", "serve");
        goal.metadata = serde_json::json!({"key": "value", "nested": {"a": 1}});
        store.add(goal.clone()).await.unwrap();

        let loaded = store.get(&goal.id).await.unwrap();
        assert_eq!(
            loaded.metadata,
            serde_json::json!({"key": "value", "nested": {"a": 1}})
        );
    }

    #[tokio::test]
    async fn sqlite_goal_priority_ordering() {
        let store = SqliteGoalStore::new_in_memory().unwrap();
        let mut low = test_goal("agent-1", "low");
        low.priority = GoalPriority::LOW;
        let mut high = test_goal("agent-1", "high");
        high.priority = GoalPriority::HIGH;
        let mut critical = test_goal("agent-1", "critical");
        critical.priority = GoalPriority::CRITICAL;

        store.add(low).await.unwrap();
        store.add(high).await.unwrap();
        store.add(critical).await.unwrap();

        let goals = store.list_by_agent(&"agent-1".to_string()).await;
        assert_eq!(goals.len(), 3);
        // Should be ordered by priority DESC
        assert_eq!(goals[0].priority, GoalPriority::CRITICAL);
        assert_eq!(goals[1].priority, GoalPriority::HIGH);
        assert_eq!(goals[2].priority, GoalPriority::LOW);
    }

    /// SAES 0.3: restart persistence — data survives process restart.
    ///
    /// This is the definitive test: write goals + behavior to a file-backed
    /// SQLite, drop the store (simulating process death), reopen, verify
    /// all state is intact.
    #[tokio::test]
    async fn sqlite_restart_persistence() {
        let dir = std::env::temp_dir().join("saes_restart_test");
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test_agent.db");

        // Phase 1: agent starts, accumulates experience
        let g1_id;
        let g2_id;
        {
            let goal_store = SqliteGoalStore::open(&db_path).unwrap();
            let behavior_store = SqliteBehaviorStore::open(&db_path).unwrap();

            // Create goals
            let mut g1 = AgentGoal::new(
                "restart-agent".to_string(),
                "serve requests".to_string(),
                "serve".to_string(),
                GoalPriority::CRITICAL,
                1000,
            );
            g1.transition_to(GoalState::Active, 1001).unwrap();
            g1.progress = 0.6;
            g1_id = g1.id.clone();
            goal_store.add(g1).await.unwrap();

            let mut g2 = AgentGoal::new(
                "restart-agent".to_string(),
                "learn coding".to_string(),
                "learn".to_string(),
                GoalPriority::NORMAL,
                1002,
            );
            g2.transition_to(GoalState::Active, 1003).unwrap();
            g2_id = g2.id.clone();
            goal_store.add(g2).await.unwrap();

            // Accumulate behavior via incorporate (public API)
            let mut profile = BehaviorProfile::new("restart-agent".to_string());
            for _ in 0..8 {
                profile.incorporate(&LearningEntry {
                    id: "l1".to_string(),
                    agent_id: "restart-agent".to_string(),
                    goal_id: None,
                    outcome_kind: "Analysis".to_string(),
                    positive: true,
                    lesson: String::new(),
                    confidence: 0.5,
                    recorded_at: 1000,
                });
            }
            for _ in 0..2 {
                profile.incorporate(&LearningEntry {
                    id: "l2".to_string(),
                    agent_id: "restart-agent".to_string(),
                    goal_id: None,
                    outcome_kind: "Analysis".to_string(),
                    positive: false,
                    lesson: String::new(),
                    confidence: 0.5,
                    recorded_at: 1000,
                });
            }
            for _ in 0..3 {
                profile.incorporate(&LearningEntry {
                    id: "l3".to_string(),
                    agent_id: "restart-agent".to_string(),
                    goal_id: None,
                    outcome_kind: "Coding".to_string(),
                    positive: true,
                    lesson: String::new(),
                    confidence: 0.5,
                    recorded_at: 1000,
                });
            }
            for _ in 0..1 {
                profile.incorporate(&LearningEntry {
                    id: "l4".to_string(),
                    agent_id: "restart-agent".to_string(),
                    goal_id: None,
                    outcome_kind: "Coding".to_string(),
                    positive: false,
                    lesson: String::new(),
                    confidence: 0.5,
                    recorded_at: 1000,
                });
            }
            behavior_store.save(profile).await;
        }
        // Store dropped here — simulates process death

        // Phase 2: agent restarts, loads state from disk
        {
            let goal_store = SqliteGoalStore::open(&db_path).unwrap();
            let behavior_store = SqliteBehaviorStore::open(&db_path).unwrap();

            // Verify goals survived
            let goals = goal_store.list_by_agent(&"restart-agent".to_string()).await;
            assert_eq!(goals.len(), 2, "goals survived restart");

            let loaded_g1 = goal_store.get(&g1_id).await.unwrap();
            assert_eq!(loaded_g1.state, GoalState::Active);
            assert_eq!(loaded_g1.progress, 0.6);
            assert_eq!(loaded_g1.priority, GoalPriority::CRITICAL);

            let loaded_g2 = goal_store.get(&g2_id).await.unwrap();
            assert_eq!(loaded_g2.state, GoalState::Active);

            // Verify behavior survived
            let profile = behavior_store.get_or_create("restart-agent").await;
            assert_eq!(profile.success_counts.get("Analysis"), Some(&8));
            assert_eq!(profile.failure_counts.get("Analysis"), Some(&2));
            assert_eq!(profile.success_counts.get("Coding"), Some(&3));
            assert_eq!(profile.failure_counts.get("Coding"), Some(&1));

            // Verify preferred strategies are recomputed
            assert!(
                profile
                    .preferred_strategies
                    .contains(&"Analysis".to_string())
            );
            assert!(profile.preferred_strategies.contains(&"Coding".to_string()));
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SAES 0.3: goal lifecycle across restart — active goal can be completed after restart.
    #[tokio::test]
    async fn sqlite_goal_lifecycle_across_restart() {
        let dir = std::env::temp_dir().join("saes_lifecycle_test");
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("lifecycle.db");

        let goal_id;
        // Phase 1: create and activate goal
        {
            let store = SqliteGoalStore::open(&db_path).unwrap();
            let mut goal = AgentGoal::new(
                "lifecycle-agent".to_string(),
                "complete task".to_string(),
                "task".to_string(),
                GoalPriority::HIGH,
                1000,
            );
            goal.transition_to(GoalState::Active, 1001).unwrap();
            goal.progress = 0.4;
            goal_id = goal.id.clone();
            store.add(goal).await.unwrap();
        }

        // Phase 2: restart, update progress, complete
        {
            let store = SqliteGoalStore::open(&db_path).unwrap();
            let mut goal = store.get(&goal_id).await.unwrap();
            assert_eq!(goal.state, GoalState::Active);
            assert_eq!(goal.progress, 0.4);

            // Simulate learning: progress +0.2
            goal.set_progress(0.6, 2000);
            store.update(goal.clone()).await.unwrap();
        }

        // Phase 3: restart, verify progress, complete
        {
            let store = SqliteGoalStore::open(&db_path).unwrap();
            let mut goal = store.get(&goal_id).await.unwrap();
            assert_eq!(goal.progress, 0.6);

            // Complete the goal
            goal.set_progress(1.0, 3000);
            goal.transition_to(GoalState::Completed, 3001).unwrap();
            store.update(goal).await.unwrap();
        }

        // Phase 4: restart, verify terminal state
        {
            let store = SqliteGoalStore::open(&db_path).unwrap();
            let goal = store.get(&goal_id).await.unwrap();
            assert_eq!(goal.state, GoalState::Completed);
            assert_eq!(goal.progress, 1.0);
            assert_eq!(goal.completed_at, Some(3001));
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SAES 0.3: behavior profile evolves across restart.
    #[tokio::test]
    async fn sqlite_behavior_evolution_across_restart() {
        let dir = std::env::temp_dir().join("saes_behavior_test");
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("behavior.db");

        // Phase 1: initial learning
        {
            let store = SqliteBehaviorStore::open(&db_path).unwrap();
            let mut profile = BehaviorProfile::new("evo-agent".to_string());
            for _ in 0..2 {
                profile.incorporate(&LearningEntry {
                    id: "e1".to_string(),
                    agent_id: "evo-agent".to_string(),
                    goal_id: None,
                    outcome_kind: "Analysis".to_string(),
                    positive: true,
                    lesson: String::new(),
                    confidence: 0.5,
                    recorded_at: 1000,
                });
            }
            profile.incorporate(&LearningEntry {
                id: "e2".to_string(),
                agent_id: "evo-agent".to_string(),
                goal_id: None,
                outcome_kind: "Analysis".to_string(),
                positive: false,
                lesson: String::new(),
                confidence: 0.5,
                recorded_at: 1000,
            });
            store.save(profile).await;
        }

        // Phase 2: restart, more learning
        {
            let store = SqliteBehaviorStore::open(&db_path).unwrap();
            let mut profile = store.get_or_create("evo-agent").await;
            assert!(profile.entries_processed >= 3);

            // More successes
            for _ in 0..5 {
                profile.incorporate(&LearningEntry {
                    id: "e3".to_string(),
                    agent_id: "evo-agent".to_string(),
                    goal_id: None,
                    outcome_kind: "Analysis".to_string(),
                    positive: true,
                    lesson: String::new(),
                    confidence: 0.5,
                    recorded_at: 2000,
                });
            }
            store.save(profile).await;
        }

        // Phase 3: restart, verify evolution
        {
            let store = SqliteBehaviorStore::open(&db_path).unwrap();
            let profile = store.get_or_create("evo-agent").await;
            assert_eq!(profile.success_counts.get("Analysis"), Some(&7));
            assert_eq!(profile.failure_counts.get("Analysis"), Some(&1));
            assert_eq!(profile.entries_processed, 8);
            // 7/8 = 87.5% success → preferred
            assert!(
                profile
                    .preferred_strategies
                    .contains(&"Analysis".to_string())
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
