//! P3 — delegation DAG: decompose a complex task into a graph of sub-tasks
//! routed to capable agents, with per-hop verification.
//!
//! The fabric's existing planner (`decentraai-fabric`) routes *inference*
//! requests (prompt → tokens) as single/sequential/fan-out stages. This
//! module generalizes that shape to *agent tasks*: an `AgentTask` that
//! requires several capabilities is decomposed into a DAG of `DelegationStage`s
//! (one per required capability, then a synthesis stage), each bound to an
//! agent chosen deterministically from the agent registry, and each verified
//! before its output feeds the next stage.
//!
//! It is PURE (no I/O, no async): execution is injected through a closure so
//! the production coordinator and a unit test drive the exact same code.
//! Honesty rules:
//! - A stage is only routed to an agent that satisfies its capability
//!   requirements (reusing the unified matcher's provenance rules).
//! - If a capability cannot be routed to any agent, the plan is rejected —
//!   the fabric never invents an executor.
//! - Verification runs per hop when the stage demands it (schema check,
//!   critic, consensus — see `crate::verification`); an unverified stage
//!   result is surfaced, never silently trusted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::AgentRecord;
use crate::task::{AgentTask, TaskVerification};
use crate::verification::VerificationCheck;

/// Per-hop schema check on an actual JSON value (not its serialization).
///
/// The fabric's `verification::check_output_schema` validates a *string*; a
/// stage output here is already a JSON `Value`, so its serialization would
/// always parse. This check instead asks the honest structural question:
/// when the stage promised a JSON object, is the output actually an object?
/// Non-JSON hints are skipped (never claimed as validation), and a missing
/// hint requires nothing.
fn check_value_schema(output: &Value, schema_hint: Option<&str>) -> VerificationCheck {
    match schema_hint {
        None => VerificationCheck {
            check_kind: crate::verification::CheckKind::Schema,
            passed: true,
            detail: "no schema required".to_string(),
        },
        Some(hint) => {
            // The hint itself must parse as JSON for the check to mean
            // anything; otherwise the check is honestly skipped.
            let hint_value: Result<Value, _> = serde_json::from_str(hint);
            match hint_value {
                Ok(Value::Object(_)) => {
                    if matches!(output, Value::Object(_)) {
                        VerificationCheck {
                            check_kind: crate::verification::CheckKind::Schema,
                            passed: true,
                            detail: "output is a JSON object per schema hint".to_string(),
                        }
                    } else {
                        VerificationCheck {
                            check_kind: crate::verification::CheckKind::Schema,
                            passed: false,
                            detail: "output is not a JSON object, but the schema hint requires one"
                                .to_string(),
                        }
                    }
                }
                Ok(_) => VerificationCheck {
                    check_kind: crate::verification::CheckKind::Schema,
                    passed: true,
                    detail: "schema hint is JSON but not an object — structural check only"
                        .to_string(),
                },
                Err(_) => VerificationCheck {
                    check_kind: crate::verification::CheckKind::Schema,
                    passed: true,
                    detail: "schema hint is not JSON — structural check skipped (honest)"
                        .to_string(),
                },
            }
        }
    }
}

/// One node in the delegation DAG: a sub-task bound to a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationStage {
    /// Unique stage id within the plan (e.g. "s1", "synth").
    pub stage_id: String,
    /// The sub-task this stage executes.
    pub task: AgentTask,
    /// Stage ids this stage depends on (its inputs).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Verification demanded before the output is consumed by dependents.
    pub verification: TaskVerification,
}

impl DelegationStage {
    /// A stage that runs `task` with no dependencies.
    pub fn new(stage_id: impl Into<String>, task: AgentTask) -> Self {
        Self {
            stage_id: stage_id.into(),
            task,
            depends_on: Vec::new(),
            verification: TaskVerification::None,
        }
    }

    /// Declares that this stage depends on other stages.
    pub fn depends_on(mut self, deps: &[&str]) -> Self {
        self.depends_on = deps.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Sets the verification requirement for this stage's output.
    pub fn verified_by(mut self, verification: TaskVerification) -> Self {
        self.verification = verification;
        self
    }
}

/// The full delegation plan: a DAG of stages for one master task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegationPlan {
    /// Unique plan id.
    pub plan_id: String,
    /// The master task id this plan decomposes.
    pub master_task_id: String,
    /// All stages of the plan.
    pub stages: Vec<DelegationStage>,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
}

/// Which agent executes which stage (planner output, deterministic).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageAssignment {
    pub stage_id: String,
    pub agent_id: String,
}

/// Delegation planning errors — all recoverable and explainable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationError {
    EmptyPlanId,
    EmptyMasterTaskId,
    DuplicateStage {
        stage_id: String,
    },
    UnknownDependency {
        stage_id: String,
        depends_on: String,
    },
    CycleDetected {
        stage_id: String,
    },
    /// No agent in the registry can satisfy a required capability.
    UnroutableCapability {
        capability: String,
    },
}

impl fmt::Display for DelegationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DelegationError::EmptyPlanId => write!(f, "plan id must not be empty"),
            DelegationError::EmptyMasterTaskId => {
                write!(f, "master task id must not be empty")
            }
            DelegationError::DuplicateStage { stage_id } => {
                write!(f, "duplicate stage '{stage_id}' in plan")
            }
            DelegationError::UnknownDependency {
                stage_id,
                depends_on,
            } => {
                write!(
                    f,
                    "stage '{stage_id}' depends on unknown stage '{depends_on}'"
                )
            }
            DelegationError::CycleDetected { stage_id } => {
                write!(f, "stage '{stage_id}' is part of a dependency cycle")
            }
            DelegationError::UnroutableCapability { capability } => {
                write!(f, "no agent can satisfy required capability '{capability}'")
            }
        }
    }
}

impl std::error::Error for DelegationError {}

impl DelegationPlan {
    /// Validates the DAG: unique stage ids, known dependencies, no cycles.
    pub fn validate(&self) -> Result<(), DelegationError> {
        if self.plan_id.is_empty() {
            return Err(DelegationError::EmptyPlanId);
        }
        if self.master_task_id.is_empty() {
            return Err(DelegationError::EmptyMasterTaskId);
        }
        let mut seen = BTreeSet::new();
        for stage in &self.stages {
            if !seen.insert(stage.stage_id.clone()) {
                return Err(DelegationError::DuplicateStage {
                    stage_id: stage.stage_id.clone(),
                });
            }
        }
        for stage in &self.stages {
            for dep in &stage.depends_on {
                if !seen.contains(dep) {
                    return Err(DelegationError::UnknownDependency {
                        stage_id: stage.stage_id.clone(),
                        depends_on: dep.clone(),
                    });
                }
            }
        }
        // Topological order check (Kahn's algorithm): if we cannot visit all
        // stages, a cycle exists.
        let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for stage in &self.stages {
            indegree.insert(stage.stage_id.clone(), 0);
            dependents.insert(stage.stage_id.clone(), Vec::new());
        }
        for stage in &self.stages {
            for dep in &stage.depends_on {
                *indegree.get_mut(&stage.stage_id).unwrap() += 1;
                dependents
                    .get_mut(dep)
                    .unwrap()
                    .push(stage.stage_id.clone());
            }
        }
        let mut queue: VecDeque<String> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut visited = 0usize;
        while let Some(id) = queue.pop_front() {
            visited += 1;
            if let Some(deps) = dependents.get(&id) {
                let mut ready: Vec<String> = deps
                    .iter()
                    .filter(|dep| {
                        let d = indegree.get_mut(*dep).unwrap();
                        *d -= 1;
                        *d == 0
                    })
                    .cloned()
                    .collect();
                ready.sort();
                queue.extend(ready);
            }
        }
        if visited != self.stages.len() {
            // Find a stage that was never visited for a precise error.
            let visited_set: BTreeSet<String> = indegree.keys().cloned().collect();
            let unvisited = self
                .stages
                .iter()
                .find(|s| !visited_set.contains(&s.stage_id))
                .map(|s| s.stage_id.clone())
                .unwrap_or_else(|| self.stages[0].stage_id.clone());
            return Err(DelegationError::CycleDetected {
                stage_id: unvisited,
            });
        }
        Ok(())
    }

    /// Stages in deterministic topological order (dependencies first,
    /// ties broken by stage_id asc).
    pub fn stages_in_order(&self) -> Vec<&DelegationStage> {
        if self.validate().is_err() {
            return Vec::new();
        }
        let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for stage in &self.stages {
            indegree.insert(stage.stage_id.clone(), 0);
            dependents.insert(stage.stage_id.clone(), Vec::new());
        }
        for stage in &self.stages {
            for dep in &stage.depends_on {
                *indegree.get_mut(&stage.stage_id).unwrap() += 1;
                dependents
                    .get_mut(dep)
                    .unwrap()
                    .push(stage.stage_id.clone());
            }
        }
        let mut queue: VecDeque<String> = indegree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut order = Vec::new();
        while let Some(id) = queue.pop_front() {
            order.push(self.stage(&id).unwrap());
            if let Some(deps) = dependents.get(&id) {
                let mut ready: Vec<String> = deps
                    .iter()
                    .filter(|dep| {
                        let d = indegree.get_mut(*dep).unwrap();
                        *d -= 1;
                        *d == 0
                    })
                    .cloned()
                    .collect();
                ready.sort();
                queue.extend(ready);
            }
        }
        order
    }

    /// Looks up a stage by id.
    pub fn stage(&self, stage_id: &str) -> Option<&DelegationStage> {
        self.stages.iter().find(|s| s.stage_id == stage_id)
    }

    /// The stages that depend directly on the given stage.
    pub fn dependents(&self, stage_id: &str) -> Vec<&DelegationStage> {
        self.stages
            .iter()
            .filter(|s| s.depends_on.iter().any(|d| d == stage_id))
            .collect()
    }
}

/// The planner: decomposes a master task into a routable DAG.
///
/// Strategy (deterministic, greedy): one stage per required semantic
/// capability, each assigned to the first capable agent in the registry
/// (sorted by agent_id for determinism), then one synthesis stage that
/// depends on all capability stages. Capability provenance is honored
/// through the unified matcher's `match_agent_semantic`.
#[derive(Debug, Clone, Default)]
pub struct DelegationPlanner;

impl DelegationPlanner {
    /// Plans a task against a registry of agents.
    ///
    /// Returns `UnroutableCapability` when no agent can satisfy a required
    /// capability — the plan is never built with a phantom executor.
    pub fn plan_task(
        &self,
        master_task: &AgentTask,
        agents: &[AgentRecord],
        plan_id: impl Into<String>,
        created_at_ms: u64,
    ) -> Result<DelegationPlan, DelegationError> {
        use crate::matcher::match_agent_semantic;

        let plan_id = plan_id.into();
        if plan_id.is_empty() {
            return Err(DelegationError::EmptyPlanId);
        }
        let mut stages = Vec::new();

        // One stage per required capability; dedupe capabilities to avoid
        // double-routing the same requirement (deterministic order).
        let mut seen_caps = BTreeSet::new();
        for req in &master_task.required_capabilities {
            let cap = req.capability;
            // Snake_case wire name (e.g. "ocr") — stable across versions.
            let cap_name = serde_json::to_string(&cap).unwrap_or_else(|_| cap.label().to_string());
            let cap_name = cap_name.trim_matches('"').to_string();
            if !seen_caps.insert(cap_name.clone()) {
                continue;
            }
            // First capable agent, sorted deterministically.
            let mut candidates: Vec<&AgentRecord> = agents
                .iter()
                .filter(|a| {
                    match_agent_semantic(
                        a,
                        &[decentraai_hub::requirements::CapabilityRequirement {
                            capability: cap,
                            evidence: req.evidence,
                        }],
                    )
                    .is_satisfied()
                })
                .collect();
            candidates.sort_by_key(|a| a.agent_id.clone());
            if candidates.is_empty() {
                return Err(DelegationError::UnroutableCapability {
                    capability: cap_name,
                });
            }
            let mut sub_task = AgentTask::new(format!("{}.{}", master_task.task_id, cap_name));
            sub_task.required_capabilities =
                vec![decentraai_hub::requirements::CapabilityRequirement {
                    capability: cap,
                    evidence: req.evidence,
                }];
            sub_task.verification = TaskVerification::SelfCheck;
            stages.push(
                DelegationStage::new(format!("cap:{cap_name}"), sub_task)
                    .verified_by(TaskVerification::SelfCheck),
            );
        }

        // Synthesis stage: depends on every capability stage; carries the
        // master task's verification requirement.
        if !stages.is_empty() {
            let deps: Vec<String> = stages.iter().map(|s| s.stage_id.clone()).collect();
            let mut synthesis = AgentTask::new(format!("{}.synthesis", master_task.task_id));
            synthesis.required_capabilities = master_task.required_capabilities.clone();
            synthesis.output_schema = master_task.output_schema.clone();
            synthesis.verification = master_task.verification;
            stages.push(
                DelegationStage::new("synthesis".to_string(), synthesis)
                    .depends_on(&deps.iter().map(String::as_str).collect::<Vec<_>>())
                    .verified_by(master_task.verification),
            );
        }

        let plan = DelegationPlan {
            plan_id,
            master_task_id: master_task.task_id.clone(),
            stages,
            created_at_ms,
        };
        plan.validate()?;
        Ok(plan)
    }
}

/// Result of one executed stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StageResult {
    pub stage_id: String,
    pub agent_id: String,
    pub output: Option<Value>,
    pub verified: bool,
    /// The schema verification check performed (empty when verification was
    /// not demanded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<VerificationCheck>,
    pub error: Option<String>,
}

/// Final verdict of a delegation run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationVerdict {
    /// Every stage executed and its output was verified.
    Completed,
    /// Some stages failed; `failed_stages` names them.
    Partial { failed_stages: Vec<String> },
    /// The run could not start (plan invalid) or the whole chain failed.
    Failed { reason: String },
}

/// The full outcome of executing a plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DelegationResult {
    pub plan_id: String,
    pub task_id: String,
    pub verdict: DelegationVerdict,
    pub stages: Vec<StageResult>,
    /// The synthesis stage's output, when the plan completed.
    pub final_output: Option<Value>,
}

/// Executes a plan with an injected executor.
///
/// `assignments` maps each stage to the agent that runs it; `executor` is the
/// runtime half: `(agent_id, stage, inputs_json) -> Result<Value, String>`.
/// Stages run in topological order; a stage's inputs are the outputs of its
/// dependencies (merged as `{"<dep_stage_id>": <output>}`). Per-hop schema
/// verification runs when the stage demands `SelfCheck` or stronger.
///
/// This is deliberately sync and pure: the caller bridges to async I/O
/// (P2P messaging / engine calls) inside `executor`.
pub fn execute_plan<F>(
    plan: &DelegationPlan,
    assignments: &[StageAssignment],
    mut executor: F,
) -> DelegationResult
where
    F: FnMut(&str, &DelegationStage, &Value) -> Result<Value, String>,
{
    if let Err(e) = plan.validate() {
        return DelegationResult {
            plan_id: plan.plan_id.clone(),
            task_id: plan.master_task_id.clone(),
            verdict: DelegationVerdict::Failed {
                reason: e.to_string(),
            },
            stages: Vec::new(),
            final_output: None,
        };
    }
    let assigned: BTreeMap<String, String> = assignments
        .iter()
        .map(|a| (a.stage_id.clone(), a.agent_id.clone()))
        .collect();

    let mut outputs: BTreeMap<String, Value> = BTreeMap::new();
    let mut results: Vec<StageResult> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut final_output: Option<Value> = None;

    for stage in plan.stages_in_order() {
        let agent_id = match assigned.get(&stage.stage_id) {
            Some(a) => a.clone(),
            None => {
                failed.push(stage.stage_id.clone());
                results.push(StageResult {
                    stage_id: stage.stage_id.clone(),
                    agent_id: String::new(),
                    output: None,
                    verified: false,
                    checks: Vec::new(),
                    error: Some("no agent assigned to stage".to_string()),
                });
                continue;
            }
        };
        // Build the merged input from dependency outputs.
        let mut inputs = serde_json::Map::new();
        let mut missing_dep = false;
        for dep in &stage.depends_on {
            if let Some(out) = outputs.get(dep) {
                inputs.insert(dep.clone(), out.clone());
            } else {
                missing_dep = true;
                break;
            }
        }
        if missing_dep {
            failed.push(stage.stage_id.clone());
            results.push(StageResult {
                stage_id: stage.stage_id.clone(),
                agent_id,
                output: None,
                verified: false,
                checks: Vec::new(),
                error: Some("dependency did not produce an output".to_string()),
            });
            continue;
        }
        let input_value = Value::Object(inputs);
        match executor(&agent_id, stage, &input_value) {
            Ok(output) => {
                // Per-hop verification: schema check when the stage demands
                // verification beyond None. The check runs on the VALUE
                // itself (not its JSON serialization — that would always be
                // valid JSON), so a stage that promised a JSON object but
                // returned a string is caught.
                let mut checks = Vec::new();
                let mut verified = true;
                if stage.verification != TaskVerification::None {
                    let check = check_value_schema(&output, stage.task.output_schema.as_deref());
                    verified = check.passed;
                    checks.push(check);
                }
                if verified {
                    outputs.insert(stage.stage_id.clone(), output.clone());
                } else {
                    failed.push(stage.stage_id.clone());
                }
                results.push(StageResult {
                    stage_id: stage.stage_id.clone(),
                    agent_id,
                    output: Some(output),
                    verified,
                    checks,
                    error: if verified {
                        None
                    } else {
                        Some("output failed verification".into())
                    },
                });
            }
            Err(e) => {
                failed.push(stage.stage_id.clone());
                results.push(StageResult {
                    stage_id: stage.stage_id.clone(),
                    agent_id,
                    output: None,
                    verified: false,
                    checks: Vec::new(),
                    error: Some(e),
                });
            }
        }
    }

    if let Some(synth) = plan.stage("synthesis") {
        if let Some(out) = outputs.get("synthesis") {
            final_output = Some(out.clone());
        } else if results
            .iter()
            .any(|r| r.stage_id == "synthesis" && r.error.is_some())
        {
            // synthesis explicitly failed
        } else if synth.depends_on.is_empty() && plan.stages.len() == 1 {
            final_output = outputs.get("synthesis").cloned();
        }
    }

    let verdict = if failed.is_empty() {
        DelegationVerdict::Completed
    } else if results.is_empty() {
        DelegationVerdict::Failed {
            reason: "plan did not execute".to_string(),
        }
    } else {
        DelegationVerdict::Partial {
            failed_stages: failed,
        }
    };

    DelegationResult {
        plan_id: plan.plan_id.clone(),
        task_id: plan.master_task_id.clone(),
        verdict,
        stages: results,
        final_output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentRecord, ROLE_GENERALIST, ROLE_SPECIALIST};
    use crate::task::TaskVerification;
    use decentraai_hub::capability::{CapabilityKind, Provenance};
    use decentraai_hub::requirements::EvidenceLevel;

    fn ocr_agent(id: &str) -> AgentRecord {
        AgentRecord::new(id, "OCR", ROLE_SPECIALIST)
            .with_capability(CapabilityKind::Ocr, Provenance::Verified)
    }

    fn coding_agent(id: &str) -> AgentRecord {
        AgentRecord::new(id, "Coder", ROLE_SPECIALIST)
            .with_capability(CapabilityKind::Coding, Provenance::Verified)
    }

    fn generalist_agent(id: &str) -> AgentRecord {
        AgentRecord::new(id, "Generalist", ROLE_GENERALIST)
            .with_capability(CapabilityKind::Chat, Provenance::Inferred)
            .with_capability(CapabilityKind::Reasoning, Provenance::Inferred)
    }

    fn master_task() -> AgentTask {
        AgentTask::new("master-1")
            .require_capability(CapabilityKind::Ocr, EvidenceLevel::Verified)
            .require_capability(CapabilityKind::Coding, EvidenceLevel::Verified)
            .with_schemas(r#"{}"#, r#"{"type":"object"}"#)
            .verified_by(TaskVerification::SelfCheck)
    }

    #[test]
    fn planner_builds_capability_stages_plus_synthesis() {
        let agents = vec![
            ocr_agent("a:ocr"),
            coding_agent("a:code"),
            generalist_agent("a:gen"),
        ];
        let plan = DelegationPlanner
            .plan_task(&master_task(), &agents, "p1", 1_700_000_000_000)
            .unwrap();
        assert_eq!(plan.validate(), Ok(()));
        // cap:ocr, cap:coding, synthesis
        assert_eq!(plan.stages.len(), 3);
        let order = plan.stages_in_order();
        assert_eq!(order[0].stage_id, "cap:coding");
        assert_eq!(order[1].stage_id, "cap:ocr");
        assert_eq!(order[2].stage_id, "synthesis");
        assert_eq!(order[2].depends_on.len(), 2);
    }

    #[test]
    fn planner_rejects_unroutable_capability() {
        let agents = vec![coding_agent("a:code")];
        let err = DelegationPlanner
            .plan_task(&master_task(), &agents, "p2", 0)
            .unwrap_err();
        assert!(matches!(
            err,
            DelegationError::UnroutableCapability { capability } if capability == "ocr"
        ));
    }

    #[test]
    fn plan_validation_catches_cycles() {
        let mut a = DelegationStage::new("a", AgentTask::new("t1"));
        a.depends_on = vec!["b".into()];
        let mut b = DelegationStage::new("b", AgentTask::new("t2"));
        b.depends_on = vec!["a".into()];
        let plan = DelegationPlan {
            plan_id: "p".into(),
            master_task_id: "m".into(),
            stages: vec![a, b],
            created_at_ms: 0,
        };
        assert!(matches!(
            plan.validate(),
            Err(DelegationError::CycleDetected { .. })
        ));
        assert!(plan.stages_in_order().is_empty());
    }

    #[test]
    fn plan_validation_catches_unknown_dependency() {
        let stage = DelegationStage::new("a", AgentTask::new("t1")).depends_on(&["ghost"]);
        let plan = DelegationPlan {
            plan_id: "p".into(),
            master_task_id: "m".into(),
            stages: vec![stage],
            created_at_ms: 0,
        };
        assert!(matches!(
            plan.validate(),
            Err(DelegationError::UnknownDependency { .. })
        ));
    }

    #[test]
    fn execute_plan_runs_stages_in_order_and_verifies() {
        let agents = vec![ocr_agent("a:ocr"), coding_agent("a:code")];
        let plan = DelegationPlanner
            .plan_task(&master_task(), &agents, "p1", 0)
            .unwrap();
        let assignments = vec![
            StageAssignment {
                stage_id: "cap:coding".into(),
                agent_id: "a:code".into(),
            },
            StageAssignment {
                stage_id: "cap:ocr".into(),
                agent_id: "a:ocr".into(),
            },
            StageAssignment {
                stage_id: "synthesis".into(),
                agent_id: "a:gen".into(),
            },
        ];
        let result = execute_plan(&plan, &assignments, |agent_id, _stage, input| {
            assert!(
                agent_id.starts_with("a:"),
                "executor receives the assigned agent"
            );
            let stage_inputs = match input {
                Value::Object(m) => m.len(),
                _ => 0,
            };
            let mut out = serde_json::Map::new();
            out.insert(
                "from".into(),
                Value::String(format!("{agent_id}::{stage_inputs}")),
            );
            Ok(Value::Object(out))
        });
        assert_eq!(result.verdict, DelegationVerdict::Completed);
        assert_eq!(result.stages.len(), 3);
        // Synthesis received the outputs of both capability stages.
        let synth = result
            .stages
            .iter()
            .find(|s| s.stage_id == "synthesis")
            .unwrap();
        let synth_inputs = synth
            .output
            .as_ref()
            .and_then(|v| v.get("from"))
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        assert!(
            synth_inputs.contains("::2"),
            "synthesis got two inputs: {synth_inputs}"
        );
        assert!(result.final_output.is_some());
    }

    #[test]
    fn execute_plan_surfaces_failed_stage_as_partial() {
        let agents = vec![ocr_agent("a:ocr"), coding_agent("a:code")];
        let plan = DelegationPlanner
            .plan_task(&master_task(), &agents, "p1", 0)
            .unwrap();
        let assignments = vec![
            StageAssignment {
                stage_id: "cap:coding".into(),
                agent_id: "a:code".into(),
            },
            StageAssignment {
                stage_id: "cap:ocr".into(),
                agent_id: "a:ocr".into(),
            },
            StageAssignment {
                stage_id: "synthesis".into(),
                agent_id: "a:gen".into(),
            },
        ];
        let result = execute_plan(&plan, &assignments, |agent_id, stage, _| {
            if stage.stage_id == "cap:ocr" {
                return Err("OCR service down".to_string());
            }
            Ok(Value::String(agent_id.to_string()))
        });
        // cap:ocr failed AND synthesis could not run (its dependency produced
        // no output) — both are honestly reported as failed.
        assert!(matches!(
            result.verdict,
            DelegationVerdict::Partial { ref failed_stages }
                if failed_stages == &vec!["cap:ocr".to_string(), "synthesis".to_string()]
        ));
        let synth = result
            .stages
            .iter()
            .find(|s| s.stage_id == "synthesis")
            .unwrap();
        assert!(synth.error.is_some());
    }

    #[test]
    fn execute_plan_verifies_output_schema_per_hop() {
        // The synthesis stage demands output schema {"type":"object"}; a
        // non-JSON output must fail verification and the run must be Partial.
        let agents = vec![ocr_agent("a:ocr")];
        let mut task = AgentTask::new("m")
            .require_capability(CapabilityKind::Ocr, EvidenceLevel::Verified)
            .verified_by(TaskVerification::SelfCheck);
        task.output_schema = Some(r#"{"type":"object"}"#.into());
        let plan = DelegationPlanner
            .plan_task(&task, &agents, "p1", 0)
            .unwrap();
        let assignments = vec![
            StageAssignment {
                stage_id: "cap:ocr".into(),
                agent_id: "a:ocr".into(),
            },
            StageAssignment {
                stage_id: "synthesis".into(),
                agent_id: "a:gen".into(),
            },
        ];
        let result = execute_plan(&plan, &assignments, |_agent, _stage, _| {
            Ok(Value::String("not valid json object".to_string()))
        });
        assert!(matches!(result.verdict, DelegationVerdict::Partial { .. }));
        let synth = result
            .stages
            .iter()
            .find(|s| s.stage_id == "synthesis")
            .unwrap();
        assert!(!synth.verified);
    }

    #[test]
    fn plan_and_result_round_trip_over_wire() {
        let agents = vec![ocr_agent("a:ocr"), coding_agent("a:code")];
        let plan = DelegationPlanner
            .plan_task(&master_task(), &agents, "p1", 0)
            .unwrap();
        let json = serde_json::to_string(&plan).unwrap();
        let back: DelegationPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(plan, back);
    }
}
