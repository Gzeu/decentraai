//! Collective Orchestration (M17.1) — parallel DAG execution across fabric workers.
//!
//! Extends the existing sequential [`AgentOrchestrator`] with:
//! - Workflow lifecycle states (PLANNED → RUNNING → COMPLETED/FAILED)
//! - Parallel branch execution (fan-out when dependencies permit)
//! - Per-stage retry with bounded attempts
//! - Per-stage evidence recording
//!
//! The invariant holds here too: this module PROPOSES and COORDINATES.
//! Worker selection, trust checks, reservations and credit remain in the
//! deterministic Rust layer.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Lifecycle of a collective workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    Planned,
    Validated,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Per-stage execution status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    Pending,
    Ready,
    Running,
    Completed,
    Failed,
    Skipped,
}

/// One node in the collective DAG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DagStage {
    pub stage_id: String,
    pub capability: String,
    /// Prompt or task description sent to the worker.
    pub prompt: String,
    /// Stage ids this stage depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Max retries on transient failure.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Lease seconds for DFCP negotiation.
    #[serde(default = "default_lease_secs")]
    pub lease_seconds: u64,
}

fn default_max_retries() -> u32 {
    2
}
fn default_lease_secs() -> u64 {
    60
}

/// The full DAG: validated at construction, executed by the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectiveDag {
    pub workflow_id: String,
    pub intent: String,
    pub stages: Vec<DagStage>,
}

/// Errors from DAG construction/validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DagError {
    Empty,
    DuplicateStage(String),
    UnknownDependency {
        stage_id: String,
        depends_on: String,
    },
    CycleDetected(Vec<String>),
}

impl std::fmt::Display for DagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "DAG has no stages"),
            Self::DuplicateStage(id) => write!(f, "duplicate stage id: {id}"),
            Self::UnknownDependency {
                stage_id,
                depends_on,
            } => {
                write!(
                    f,
                    "stage '{stage_id}' depends on unknown stage '{depends_on}'"
                )
            }
            Self::CycleDetected(cycle) => write!(f, "cycle detected: {}", cycle.join(" → ")),
        }
    }
}

impl std::error::Error for DagError {}

/// Validates a set of stages into a well-formed DAG.
/// Checks: non-empty, unique ids, known dependencies, no cycles.
/// Pure function — testable without I/O.
pub fn validate_dag(stages: &[DagStage]) -> Result<(), DagError> {
    if stages.is_empty() {
        return Err(DagError::Empty);
    }
    let mut seen = HashSet::new();
    for s in stages {
        if !seen.insert(s.stage_id.clone()) {
            return Err(DagError::DuplicateStage(s.stage_id.clone()));
        }
    }
    let id_set: HashSet<&str> = stages.iter().map(|s| s.stage_id.as_str()).collect();
    for stage in stages {
        for dep in &stage.depends_on {
            if !id_set.contains(dep.as_str()) {
                return Err(DagError::UnknownDependency {
                    stage_id: stage.stage_id.clone(),
                    depends_on: dep.clone(),
                });
            }
        }
    }

    // Cycle detection via DFS coloring.
    fn dfs(
        id: &str,
        stages: &[DagStage],
        color: &mut HashMap<String, u8>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        color.insert(id.to_string(), 1);
        path.push(id.to_string());
        let stage = stages.iter().find(|s| s.stage_id == id)?;
        for dep in &stage.depends_on {
            match color.get(dep) {
                Some(1) => return Some(path.clone()),
                Some(0) | None => {}
                _ => {}
            }
            if let Some(cycle) = dfs(dep, stages, color, path) {
                return Some(cycle);
            }
        }
        color.insert(id.to_string(), 2);
        path.pop();
        None
    }

    let mut color: HashMap<String, u8> = HashMap::new();
    for s in stages {
        color.entry(s.stage_id.clone()).or_insert(0);
    }
    let mut path = Vec::new();
    for s in stages {
        if color.get(&s.stage_id) == Some(&0) {
            if let Some(cycle) = dfs(&s.stage_id.clone(), stages, &mut color, &mut path) {
                return Err(DagError::CycleDetected(cycle));
            }
        }
    }
    Ok(())
}

/// Returns the stage ids that are ready to execute (all dependencies completed).
/// Pure function — the caller tracks completion state.
pub fn ready_stages(stages: &[DagStage], completed: &HashSet<String>) -> Vec<String> {
    stages
        .iter()
        .filter(|s| {
            !completed.contains(&s.stage_id) && s.depends_on.iter().all(|d| completed.contains(d))
        })
        .map(|s| s.stage_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(id: &str, deps: &[&str]) -> DagStage {
        DagStage {
            stage_id: id.to_string(),
            capability: "chat".to_string(),
            prompt: format!("task for {id}"),
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            max_retries: 2,
            lease_seconds: 60,
        }
    }

    #[test]
    fn valid_dag_passes() {
        let stages = vec![stage("a", &[]), stage("b", &["a"]), stage("c", &["a"])];
        assert!(validate_dag(&stages).is_ok());
    }

    #[test]
    fn empty_dag_is_error() {
        assert_eq!(validate_dag(&[]), Err(DagError::Empty));
    }

    #[test]
    fn duplicate_stage_detected() {
        let stages = vec![stage("a", &[]), stage("a", &[])];
        assert!(matches!(
            validate_dag(&stages),
            Err(DagError::DuplicateStage(_))
        ));
    }

    #[test]
    fn unknown_dependency_detected() {
        let stages = vec![stage("a", &["ghost"])];
        assert!(matches!(
            validate_dag(&stages),
            Err(DagError::UnknownDependency { .. })
        ));
    }

    #[test]
    fn cycle_detected() {
        let mut a = stage("a", &[]);
        a.depends_on.push("c".to_string());
        let stages = vec![a, stage("b", &["a"]), stage("c", &["b"])];
        assert!(matches!(
            validate_dag(&stages),
            Err(DagError::CycleDetected(_))
        ));
    }

    #[test]
    fn ready_stages_respects_dependencies() {
        let stages = vec![
            stage("root", &[]),
            stage("child", &["root"]),
            stage("orphan", &[]),
        ];
        // Initially only root and orphan are ready.
        let done: HashSet<String> = HashSet::new();
        let ready = ready_stages(&stages, &done);
        assert_eq!(ready.len(), 2);
        // After root completes, child becomes ready too.
        let mut done = HashSet::new();
        done.insert("root".to_string());
        let ready = ready_stages(&stages, &done);
        assert!(ready.contains(&"child".to_string()));
    }

    #[test]
    fn fan_in_waits_for_all_parents() {
        let stages = vec![
            stage("a", &[]),
            stage("b", &[]),
            stage("synth", &["a", "b"]),
        ];
        let mut done = HashSet::new();
        done.insert("a".to_string());
        let ready = ready_stages(&stages, &done);
        assert!(
            !ready.contains(&"synth".to_string()),
            "fan-in waits for all parents"
        );
        done.insert("b".to_string());
        let ready = ready_stages(&stages, &done);
        assert!(ready.contains(&"synth".to_string()));
    }
}
