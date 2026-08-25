//! Fabric Intelligence TaskPlan → Collective DAG bridge.
//!
//! Converts an intelligence-proposed workflow (list of capability steps with
//! dependencies) into a validated [`CollectiveDag`] that the orchestrator
//! can execute across the fabric.

use crate::collective::{CollectiveDag, DagError, DagStage, validate_dag};
use serde::{Deserialize, Serialize};

/// A proposed stage from Fabric Intelligence's TaskPlan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedStage {
    /// Explicit stage id; falls back to `capability` when absent so existing
    /// callers (and tests) keep working.
    #[serde(default)]
    pub stage_id: Option<String>,
    pub capability: String,
    pub prompt: String,
    pub depends_on: Vec<String>,
}

/// Builds a `CollectiveDag` from a TaskPlan's workflow proposal.
///
/// Pure function: validates structure and returns either a ready-to-execute
/// DAG or a descriptive error. The caller decides what to do with it.
pub fn task_plan_to_dag(
    workflow_id: &str,
    intent: &str,
    stages: &[ProposedStage],
) -> Result<CollectiveDag, DagError> {
    let dag_stages: Vec<DagStage> = stages
        .iter()
        .map(|s| DagStage {
            stage_id: s
                .stage_id
                .clone()
                .unwrap_or_else(|| s.capability.clone()),
            capability: s.capability.clone(),
            prompt: s.prompt.clone(),
            depends_on: s.depends_on.clone(),
            max_retries: 2,
            lease_seconds: 60,
        })
        .collect();
    validate_dag(&dag_stages)?;
    Ok(CollectiveDag {
        workflow_id: workflow_id.to_string(),
        intent: intent.to_string(),
        stages: dag_stages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_plan_builds_valid_dag() {
        let stages = vec![
            ProposedStage {
                stage_id: None,
                capability: "research".into(),
                prompt: "search".into(),
                depends_on: vec![],
            },
            ProposedStage {
                stage_id: None,
                capability: "summarization".into(),
                prompt: "summarize".into(),
                depends_on: vec!["research".into()],
            },
        ];
        let dag = task_plan_to_dag("wf1", "test", &stages).unwrap();
        assert_eq!(dag.stages.len(), 2);
    }

    #[test]
    fn parallel_plan_builds_fan_out() {
        let stages = vec![
            ProposedStage {
                stage_id: None,
                capability: "a".into(),
                prompt: "p".into(),
                depends_on: vec![],
            },
            ProposedStage {
                stage_id: None,
                capability: "b".into(),
                prompt: "p".into(),
                depends_on: vec![],
            },
            ProposedStage {
                stage_id: None,
                capability: "synth".into(),
                prompt: "s".into(),
                depends_on: vec!["a".into(), "b".into()],
            },
        ];
        let dag = task_plan_to_dag("wf2", "fan-out", &stages).unwrap();
        assert_eq!(dag.stages.len(), 3);
        // Fan-in stage has two deps.
        assert_eq!(dag.stages[2].depends_on.len(), 2);
    }
}
