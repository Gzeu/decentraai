//! SAES 0.4: Collective Goal Persistence.
//!
//! Provides SQLite-backed storage for `CollectiveGoal` objects.
//! Leverages the same connection management as `SqliteGoalStore`.

use async_trait::async_trait;
use rusqlite::{Connection, params};
use std::sync::Mutex;

use super::collective::{
    AgentId, CollectiveGoal, CollectiveGoalId, CollectiveGoalStore, CollectiveStatus,
    FailurePolicy, SubGoal,
};
use super::goals::GoalPriority;

#[derive(Debug, thiserror::Error)]
pub enum CollectivePersistenceError {
    #[error("sqlite error: {0}")]
    Sql(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<rusqlite::Error> for CollectivePersistenceError {
    fn from(e: rusqlite::Error) -> Self {
        CollectivePersistenceError::Sql(e.to_string())
    }
}

impl From<serde_json::Error> for CollectivePersistenceError {
    fn from(e: serde_json::Error) -> Self {
        CollectivePersistenceError::Serialization(e.to_string())
    }
}

/// SQLite-backed store for collective goals.
pub struct SqliteCollectiveGoalStore {
    conn: std::sync::Arc<Mutex<Connection>>,
}

impl SqliteCollectiveGoalStore {
    /// Create a new store using an existing connection mutex.
    pub fn new(conn: std::sync::Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl CollectiveGoalStore for SqliteCollectiveGoalStore {
    async fn create(&self, goal: CollectiveGoal) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let participants = serde_json::to_string(&goal.participants).map_err(|e| e.to_string())?;
        let sub_goals = serde_json::to_string(&goal.sub_goals).map_err(|e| e.to_string())?;
        let dependencies = serde_json::to_string(&goal.dependencies).map_err(|e| e.to_string())?;
        let metadata = serde_json::to_string(&goal.metadata).map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO collective_goals (
                id, title, description, kind, status, priority,
                proposer_id, participants, sub_goals, progress,
                failure_policy, dependencies, deadline, created_at, updated_at,
                completed_at, failure_reason, metadata, correlation_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                goal.id,
                goal.title,
                goal.description,
                goal.kind,
                goal.status.to_string(),
                goal.priority.0 as i32,
                goal.proposer_id,
                participants,
                sub_goals,
                goal.progress,
                serde_json::to_string(&goal.failure_policy).map_err(|e| e.to_string())?,
                dependencies,
                goal.deadline.map(|d| d as i64),
                goal.created_at as i64,
                goal.updated_at as i64,
                goal.completed_at.map(|t| t as i64),
                goal.failure_reason,
                metadata,
                goal.correlation_id,
            ],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn get(&self, goal_id: &CollectiveGoalId) -> Result<CollectiveGoal, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        conn.query_row(
            "SELECT * FROM collective_goals WHERE id = ?1",
            params![goal_id],
            |row| {
                let status_str: String = row.get(4)?;
                let status = match status_str.as_str() {
                    "proposed" => CollectiveStatus::Proposed,
                    "active" => CollectiveStatus::Active,
                    "completed" => CollectiveStatus::Completed,
                    "failed" => CollectiveStatus::Failed,
                    "cancelled" => CollectiveStatus::Cancelled,
                    _ => CollectiveStatus::Proposed,
                };

                let participants: Vec<String> =
                    serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default();
                let sub_goals_json: String = row.get(8)?;
                let sub_goals: std::collections::HashMap<String, SubGoal> =
                    serde_json::from_str(&sub_goals_json).unwrap_or_default();
                let failure_policy_json: String = row.get(10)?;
                let failure_policy: FailurePolicy =
                    serde_json::from_str(&failure_policy_json).unwrap_or_default();
                let dependencies: Vec<String> =
                    serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
                let metadata_json: String = row.get(17)?;
                let metadata =
                    serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);

                Ok(CollectiveGoal {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    kind: row.get(3)?,
                    status,
                    priority: GoalPriority(row.get::<_, i32>(5)? as u8),
                    proposer_id: row.get(6)?,
                    participants,
                    sub_goals,
                    progress: row.get(9)?,
                    failure_policy,
                    dependencies,
                    deadline: row.get::<_, Option<i64>>(12)?.map(|d| d as u64),
                    created_at: row.get::<_, i64>(13)? as u64,
                    updated_at: row.get::<_, i64>(14)? as u64,
                    completed_at: row.get::<_, Option<i64>>(15)?.map(|t| t as u64),
                    failure_reason: row.get(16)?,
                    metadata,
                    correlation_id: row.get(18)?,
                })
            },
        )
        .map_err(|e| e.to_string())
    }

    async fn update(&self, goal: CollectiveGoal) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        let participants = serde_json::to_string(&goal.participants).map_err(|e| e.to_string())?;
        let sub_goals = serde_json::to_string(&goal.sub_goals).map_err(|e| e.to_string())?;
        let dependencies = serde_json::to_string(&goal.dependencies).map_err(|e| e.to_string())?;
        let metadata = serde_json::to_string(&goal.metadata).map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE collective_goals SET 
                title = ?2, description = ?3, kind = ?4, status = ?5, priority = ?6,
                proposer_id = ?7, participants = ?8, sub_goals = ?9, progress = ?10,
                failure_policy = ?11, dependencies = ?12, deadline = ?13, created_at = ?14, updated_at = ?15,
                completed_at = ?16, failure_reason = ?17, metadata = ?18, correlation_id = ?19
                WHERE id = ?1",
            params![
                goal.id,
                goal.title,
                goal.description,
                goal.kind,
                goal.status.to_string(),
                goal.priority.0 as i32,
                goal.proposer_id,
                participants,
                sub_goals,
                goal.progress,
                serde_json::to_string(&goal.failure_policy).map_err(|e| e.to_string())?,
                dependencies,
                goal.deadline.map(|d| d as i64),
                goal.created_at as i64,
                goal.updated_at as i64,
                goal.completed_at.map(|t| t as i64),
                goal.failure_reason,
                metadata,
                goal.correlation_id,
            ],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }

    async fn list_all(&self) -> Vec<CollectiveGoal> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let mut stmt = match conn.prepare("SELECT * FROM collective_goals") {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        stmt.query_map([], |row| {
            let status_str: String = row.get(4)?;
            let status = match status_str.as_str() {
                "proposed" => CollectiveStatus::Proposed,
                "active" => CollectiveStatus::Active,
                "completed" => CollectiveStatus::Completed,
                "failed" => CollectiveStatus::Failed,
                "cancelled" => CollectiveStatus::Cancelled,
                _ => CollectiveStatus::Proposed,
            };
            let participants: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default();
            let sub_goals_json: String = row.get(8)?;
            let sub_goals: std::collections::HashMap<String, SubGoal> =
                serde_json::from_str(&sub_goals_json).unwrap_or_default();
            let failure_policy_json: String = row.get(10)?;
            let failure_policy: FailurePolicy =
                serde_json::from_str(&failure_policy_json).unwrap_or_default();
            let dependencies: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
            let metadata_json: String = row.get(17)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);

            Ok(CollectiveGoal {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                kind: row.get(3)?,
                status,
                priority: GoalPriority(row.get::<_, i32>(5)? as u8),
                proposer_id: row.get(6)?,
                participants,
                sub_goals,
                progress: row.get(9)?,
                failure_policy,
                dependencies,
                deadline: row.get::<_, Option<i64>>(12)?.map(|d| d as u64),
                created_at: row.get::<_, i64>(13)? as u64,
                updated_at: row.get::<_, i64>(14)? as u64,
                completed_at: row.get::<_, Option<i64>>(15)?.map(|t| t as u64),
                failure_reason: row.get(16)?,
                metadata,
                correlation_id: row.get(18)?,
            })
        })
        .into_iter()
        .flat_map(|r| r.into_iter())
        .filter_map(|r| r.ok())
        .collect()
    }

    async fn list_by_status(&self, status: CollectiveStatus) -> Vec<CollectiveGoal> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let mut stmt = match conn.prepare("SELECT * FROM collective_goals WHERE status = ?1") {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        stmt.query_map(params![status.to_string()], |row| {
            let status_str: String = row.get(4)?;
            let status = match status_str.as_str() {
                "proposed" => CollectiveStatus::Proposed,
                "active" => CollectiveStatus::Active,
                "completed" => CollectiveStatus::Completed,
                "failed" => CollectiveStatus::Failed,
                "cancelled" => CollectiveStatus::Cancelled,
                _ => CollectiveStatus::Proposed,
            };
            let participants: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default();
            let sub_goals_json: String = row.get(8)?;
            let sub_goals: std::collections::HashMap<String, SubGoal> =
                serde_json::from_str(&sub_goals_json).unwrap_or_default();
            let failure_policy_json: String = row.get(10)?;
            let failure_policy: FailurePolicy =
                serde_json::from_str(&failure_policy_json).unwrap_or_default();
            let dependencies: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
            let metadata_json: String = row.get(17)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);

            Ok(CollectiveGoal {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                kind: row.get(3)?,
                status,
                priority: GoalPriority(row.get::<_, i32>(5)? as u8),
                proposer_id: row.get(6)?,
                participants,
                sub_goals,
                progress: row.get(9)?,
                failure_policy,
                dependencies,
                deadline: row.get::<_, Option<i64>>(12)?.map(|d| d as u64),
                created_at: row.get::<_, i64>(13)? as u64,
                updated_at: row.get::<_, i64>(14)? as u64,
                completed_at: row.get::<_, Option<i64>>(15)?.map(|t| t as u64),
                failure_reason: row.get(16)?,
                metadata,
                correlation_id: row.get(18)?,
            })
        })
        .into_iter()
        .flat_map(|r| r.into_iter())
        .filter_map(|r| r.ok())
        .collect()
    }

    async fn list_by_participant(&self, agent_id: &AgentId) -> Vec<CollectiveGoal> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return vec![],
        };

        let mut stmt =
            match conn.prepare("SELECT * FROM collective_goals WHERE participants LIKE ?1") {
                Ok(s) => s,
                Err(_) => return vec![],
            };

        let pattern = format!("%\"{}\"%", agent_id);
        stmt.query_map(params![pattern], |row| {
            let status_str: String = row.get(4)?;
            let status = match status_str.as_str() {
                "proposed" => CollectiveStatus::Proposed,
                "active" => CollectiveStatus::Active,
                "completed" => CollectiveStatus::Completed,
                "failed" => CollectiveStatus::Failed,
                "cancelled" => CollectiveStatus::Cancelled,
                _ => CollectiveStatus::Proposed,
            };
            let participants: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or_default();
            let sub_goals_json: String = row.get(8)?;
            let sub_goals: std::collections::HashMap<String, SubGoal> =
                serde_json::from_str(&sub_goals_json).unwrap_or_default();
            let failure_policy_json: String = row.get(10)?;
            let failure_policy: FailurePolicy =
                serde_json::from_str(&failure_policy_json).unwrap_or_default();
            let dependencies: Vec<String> =
                serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default();
            let metadata_json: String = row.get(17)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or(serde_json::Value::Null);

            Ok(CollectiveGoal {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                kind: row.get(3)?,
                status,
                priority: GoalPriority(row.get::<_, i32>(5)? as u8),
                proposer_id: row.get(6)?,
                participants,
                sub_goals,
                progress: row.get(9)?,
                failure_policy,
                dependencies,
                deadline: row.get::<_, Option<i64>>(12)?.map(|d| d as u64),
                created_at: row.get::<_, i64>(13)? as u64,
                updated_at: row.get::<_, i64>(14)? as u64,
                completed_at: row.get::<_, Option<i64>>(15)?.map(|t| t as u64),
                failure_reason: row.get(16)?,
                metadata,
                correlation_id: row.get(18)?,
            })
        })
        .into_iter()
        .flat_map(|r| r.into_iter())
        .filter_map(|r| r.ok())
        .collect()
    }

    async fn delete(&self, goal_id: &CollectiveGoalId) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM collective_goals WHERE id = ?1",
            params![goal_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
