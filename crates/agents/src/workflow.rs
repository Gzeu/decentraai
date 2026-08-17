//! P9 — collective workflows: named, reusable compositions of the delegation
//! DAG, instantiated and run.
//!
//! # Why templates on top of delegation
//!
//! P3 (`delegation`) decomposes a single `AgentTask` into a DAG of capability
//! stages plus a synthesis stage, binding each to a concrete agent. P9
//! generalizes that *shape* into a reusable `WorkflowTemplate`: the semantic
//! steps and their edges are captured once (a research report, a doc-review
//! pass, …) without naming agents, then instantiated per run against whatever
//! agents are available.
//!
//! A template is deliberately *verification-agnostic*: it describes the DAG
//! (which capabilities, in what order, with what dependencies) but not *how*
//! each step is verified. Verification is supplied at instantiation time by
//! the `AgentTask` the caller passes in, so the same template can be run with
//! `SelfCheck`, `Critic`, or `Consensus` without re-authoring it. This keeps
//! the template a pure, wire-safe document and the trust decision a runtime
//! one.
//!
//! Like the rest of this crate it is PURE (no I/O, no async): execution is
//! injected through `run_workflow`'s executor closure, the exact same type the
//! production coordinator and a unit test drive.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::delegation::{
    DelegationPlan, DelegationResult, DelegationStage, DelegationVerdict, StageAssignment,
    StageResult, execute_plan,
};
use crate::task::AgentTask;

use decentraai_hub::capability::CapabilityKind;
use decentraai_hub::requirements::{CapabilityRequirement, EvidenceLevel};

/// One semantic step in a workflow template.
///
/// A step names a *capability* (not an agent) and its dependencies on other
/// steps' ids. The planner binds each step to a capable agent at run time —
/// the template is independent of the fleet that happens to be online.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Unique step id within the template (e.g. "research").
    pub step_id: String,
    /// The semantic capability this step executes.
    pub capability: CapabilityKind,
    /// Minimum provenance evidence required for the capability.
    pub evidence: EvidenceLevel,
    /// Step ids this step depends on (its inputs), in dependency order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

impl WorkflowStep {
    /// A step that runs `capability` with the given evidence, no dependencies.
    pub fn new(
        step_id: impl Into<String>,
        capability: CapabilityKind,
        evidence: EvidenceLevel,
    ) -> Self {
        Self {
            step_id: step_id.into(),
            capability,
            evidence,
            depends_on: Vec::new(),
        }
    }

    /// Declares that this step depends on other steps by id.
    pub fn depends_on(mut self, deps: &[&str]) -> Self {
        self.depends_on = deps.iter().map(|s| s.to_string()).collect();
        self
    }
}

/// A named, reusable composition of delegation steps.
///
/// Wire-safe so templates can be shared between nodes or persisted; the DAG
/// shape is validated by [`WorkflowTemplate::validate`] before it is ever
/// instantiated into a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowTemplate {
    /// Unique template id (e.g. "research_report").
    pub template_id: String,
    /// Human-friendly name.
    pub name: String,
    /// What the workflow produces / is for.
    pub description: String,
    /// The semantic steps, in declaration order (not necessarily execution
    /// order — that follows the DAG).
    pub steps: Vec<WorkflowStep>,
    /// Whether a final synthesis stage (depending on every step) is appended
    /// at instantiation time.
    pub synthesis: bool,
    /// Creation time (unix ms).
    pub created_at_ms: u64,
}

impl WorkflowTemplate {
    /// A minimal template with the given id and name.
    pub fn new(template_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            template_id: template_id.into(),
            name: name.into(),
            description: String::new(),
            steps: Vec::new(),
            synthesis: false,
            created_at_ms: 0,
        }
    }

    /// Adds a semantic step; chainable.
    ///
    /// Dependencies are declared by step id and checked for existence and
    /// cycles by [`Self::validate`].
    pub fn with_step(
        mut self,
        step_id: impl Into<String>,
        capability: CapabilityKind,
        evidence: EvidenceLevel,
        depends_on: &[&str],
    ) -> Self {
        self.steps.push(
            WorkflowStep::new(step_id, capability, evidence).depends_on(depends_on),
        );
        self
    }

    /// Whether a final synthesis stage (dependent on every step) is appended
    /// at instantiation time.
    pub fn with_synthesis(mut self, synthesis: bool) -> Self {
        self.synthesis = synthesis;
        self
    }

    /// Sets a human description; chainable.
    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Validates the template DAG: non-empty id, unique step ids, known
    /// dependencies, no cycles (Kahn's topological check).
    pub fn validate(&self) -> Result<(), WorkflowError> {
        if self.template_id.is_empty() {
            return Err(WorkflowError::EmptyTemplateId);
        }
        let mut seen = BTreeSet::new();
        for step in &self.steps {
            if step.step_id.is_empty() {
                return Err(WorkflowError::UnknownStep {
                    step_id: step.step_id.clone(),
                });
            }
            if !seen.insert(step.step_id.clone()) {
                return Err(WorkflowError::DuplicateStep {
                    step_id: step.step_id.clone(),
                });
            }
        }
        for step in &self.steps {
            for dep in &step.depends_on {
                if !seen.contains(dep) {
                    return Err(WorkflowError::UnknownDependency {
                        step_id: step.step_id.clone(),
                        depends_on: dep.clone(),
                    });
                }
            }
        }
        // Topological order check (Kahn's algorithm): if we cannot visit all
        // steps, a cycle exists. Same pattern as `DelegationPlan::validate`.
        let mut indegree: BTreeMap<String, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for step in &self.steps {
            indegree.insert(step.step_id.clone(), 0);
            dependents.insert(step.step_id.clone(), Vec::new());
        }
        for step in &self.steps {
            for dep in &step.depends_on {
                *indegree.get_mut(&step.step_id).unwrap() += 1;
                dependents.get_mut(dep).unwrap().push(step.step_id.clone());
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
        if visited != self.steps.len() {
            let unvisited = self
                .steps
                .iter()
                .find(|s| indegree.get(&s.step_id).is_none_or(|d| *d > 0))
                .map(|s| s.step_id.clone())
                .unwrap_or_else(|| self.steps[0].step_id.clone());
            return Err(WorkflowError::CycleDetected { step_id: unvisited });
        }
        Ok(())
    }

    /// Converts this template into a concrete [`DelegationPlan`] for a master
    /// task.
    ///
    /// Each `WorkflowStep` becomes a `DelegationStage` whose task requires
    /// *only* that step's capability; edges follow `depends_on`. When
    /// `synthesis` is true, a final synthesis stage (task id
    /// `{master_task.task_id}.synthesis`) depends on every step and carries
    /// the master task's verification, output schema, and required
    /// capabilities — so a caller wanting a Critic on the synthesis output
    /// supplies a master task `verified_by(TaskVerification::Critic)`.
    ///
    /// The template is validated before building; the produced plan is
    /// returned as-is (its DAG correctness follows from template validation).
    pub fn instantiate(
        &self,
        master_task: &AgentTask,
        plan_id: impl Into<String>,
        created_at_ms: u64,
    ) -> Result<DelegationPlan, WorkflowError> {
        self.validate()?;
        let mut stages = Vec::new();

        for step in &self.steps {
            let mut sub_task = AgentTask::new(format!("{}.{}", master_task.task_id, step.step_id));
            sub_task.required_capabilities =
                vec![CapabilityRequirement {
                    capability: step.capability,
                    evidence: step.evidence,
                }];
            let deps: Vec<String> = step.depends_on.clone();
            stages.push(
                DelegationStage::new(step.step_id.clone(), sub_task)
                    .depends_on(&deps.iter().map(String::as_str).collect::<Vec<_>>()),
            );
        }

        if self.synthesis {
            let deps: Vec<String> = self.steps.iter().map(|s| s.step_id.clone()).collect();
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

        Ok(DelegationPlan {
            plan_id: plan_id.into(),
            master_task_id: master_task.task_id.clone(),
            stages,
            created_at_ms,
        })
    }

    /// The step ids a template will produce at instantiation time: every step
    /// id, plus `"synthesis"` when `synthesis` is enabled. Useful for building
    /// [`StageAssignment`]s before running.
    pub fn delegation_plan_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.steps.iter().map(|s| s.step_id.clone()).collect();
        if self.synthesis {
            ids.push("synthesis".to_string());
        }
        ids
    }
}

/// Template errors — all recoverable and explainable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkflowError {
    /// The template has no id.
    #[error("template id must not be empty")]
    EmptyTemplateId,
    /// Two steps share an id; the DAG would be ambiguous.
    #[error("duplicate step '{step_id}' in workflow template")]
    DuplicateStep { step_id: String },
    /// A step references a dependency that is not a step in the template.
    #[error("step '{step_id}' depends on unknown step '{depends_on}'")]
    UnknownDependency { step_id: String, depends_on: String },
    /// The dependency edges form a cycle; execution order is undefined.
    #[error("step '{step_id}' is part of a dependency cycle")]
    CycleDetected { step_id: String },
    /// A step id could not be resolved (empty id).
    #[error("unknown step '{step_id}'")]
    UnknownStep { step_id: String },
}

/// The canonical `research_report` template from the architecture doc.
///
/// DAG: `research` (Reasoning) → `finance` (Reasoning) and `documents`
/// (DocumentUnderstanding) both depend on `research`; template-level synthesis
/// depends on all three. The finance step is modelled as **Reasoning** rather
/// than Classification for determinism: classification would route to a
/// narrow classifier, while the research report wants the richer, stable
/// reasoning path shared with `research`. A Critic verification on the
/// synthesis output is *not* baked into the template (templates are
/// verification-agnostic); callers supply it via a master task
/// `verified_by(TaskVerification::Critic)`.
pub fn research_report_template() -> WorkflowTemplate {
    WorkflowTemplate::new("research_report", "Research report")
        .describe(
            "Gathers research on a topic, a financial read, and a document
             understanding pass, then synthesizes a report.",
        )
        .with_step("research", CapabilityKind::Reasoning, EvidenceLevel::Any, &[])
        .with_step("finance", CapabilityKind::Reasoning, EvidenceLevel::Any, &["research"])
        .with_step(
            "documents",
            CapabilityKind::DocumentUnderstanding,
            EvidenceLevel::Any,
            &["research"],
        )
        .with_synthesis(true)
}

/// The outcome of running a workflow: the instantiated plan, the raw
/// delegation result, and rolled-up stage counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowOutcome {
    /// The template id the workflow ran from.
    pub template_id: String,
    /// The master task id the workflow instantiated.
    pub task_id: String,
    /// The overall verdict (copied from the delegation result).
    pub verdict: DelegationVerdict,
    /// The concrete plan that was executed.
    pub plan: DelegationPlan,
    /// The raw execution result.
    pub result: DelegationResult,
    /// Number of stages that executed successfully (no error, verified).
    pub completed_stages: usize,
    /// Number of stages that failed (error or unverified).
    pub failed_stages: usize,
}

impl WorkflowOutcome {
    /// Builds an outcome from an instantiated plan and its execution result.
    ///
    /// Stage counts are derived from the result's [`StageResult`]s: a stage is
    /// *completed* when it ran without error and was verified, otherwise it
    /// counts as *failed*.
    pub fn new(
        template_id: impl Into<String>,
        task_id: impl Into<String>,
        plan: DelegationPlan,
        result: DelegationResult,
    ) -> Self {
        let verdict = result.verdict.clone();
        let completed_stages = result
            .stages
            .iter()
            .filter(|s: &&StageResult| s.error.is_none() && s.verified)
            .count();
        let failed_stages = result
            .stages
            .iter()
            .filter(|s: &&StageResult| s.error.is_some() || !s.verified)
            .count();
        Self {
            template_id: template_id.into(),
            task_id: task_id.into(),
            verdict,
            plan,
            result,
            completed_stages,
            failed_stages,
        }
    }
}

/// Runs a workflow template against a master task with an injected executor.
///
/// `assignments` binds each stage (by the ids returned from
/// [`WorkflowTemplate::delegation_plan_ids`]) to an agent; `executor` is the
/// runtime half `(agent_id, stage, inputs_json) -> Result<Value, String>`,
/// identical to `delegation::execute_plan`. The template is validated and
/// instantiated, the plan executed, and the whole thing wrapped in a
/// [`WorkflowOutcome`]. An invalid template surfaces as a `Failed` verdict
/// rather than panicking.
pub fn run_workflow<F>(
    template: &WorkflowTemplate,
    master_task: &AgentTask,
    assignments: &[StageAssignment],
    executor: F,
) -> WorkflowOutcome
where
    F: FnMut(&str, &DelegationStage, &serde_json::Value) -> Result<serde_json::Value, String>,
{
    let plan = match template.instantiate(master_task, template.template_id.clone(), 0) {
        Ok(plan) => plan,
        Err(e) => {
            let empty = DelegationPlan {
                plan_id: template.template_id.clone(),
                master_task_id: master_task.task_id.clone(),
                stages: Vec::new(),
                created_at_ms: 0,
            };
            let result = DelegationResult {
                plan_id: template.template_id.clone(),
                task_id: master_task.task_id.clone(),
                verdict: DelegationVerdict::Failed {
                    reason: e.to_string(),
                },
                stages: Vec::new(),
                final_output: None,
            };
            return WorkflowOutcome::new(&template.template_id, &master_task.task_id, empty, result);
        }
    };
    let result = execute_plan(&plan, assignments, executor);
    WorkflowOutcome::new(&template.template_id, &master_task.task_id, plan, result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskVerification;
    use decentraai_hub::requirements::EvidenceLevel;
    use serde_json::json;

    fn master_task(id: &str) -> AgentTask {
        AgentTask::new(id)
            .require_capability(CapabilityKind::Reasoning, EvidenceLevel::Any)
            .with_schemas(r#"{}"#, r#"{"type":"object"}"#)
            .verified_by(TaskVerification::Critic)
    }

    /// Assigns every stage id of a template to `agent_id`, in declaration
    /// order.
    fn assign_all(template: &WorkflowTemplate, agent_id: &str) -> Vec<StageAssignment> {
        template
            .delegation_plan_ids()
            .into_iter()
            .map(|stage_id| StageAssignment {
                stage_id,
                agent_id: agent_id.to_string(),
            })
            .collect()
    }

    #[test]
    fn validation_rejects_duplicate_step_ids() {
        let template = WorkflowTemplate::new("t", "dup")
            .with_step("a", CapabilityKind::Reasoning, EvidenceLevel::Any, &[])
            .with_step("a", CapabilityKind::Coding, EvidenceLevel::Any, &[]);
        assert!(matches!(
            template.validate(),
            Err(WorkflowError::DuplicateStep { step_id }) if step_id == "a"
        ));
    }

    #[test]
    fn validation_rejects_unknown_dependency() {
        let template = WorkflowTemplate::new("t", "deps")
            .with_step("a", CapabilityKind::Reasoning, EvidenceLevel::Any, &["ghost"]);
        assert!(matches!(
            template.validate(),
            Err(WorkflowError::UnknownDependency { depends_on, .. }) if depends_on == "ghost"
        ));
    }

    #[test]
    fn validation_rejects_cycles() {
        let template = WorkflowTemplate::new("t", "cycle")
            .with_step("a", CapabilityKind::Reasoning, EvidenceLevel::Any, &["b"])
            .with_step("b", CapabilityKind::Reasoning, EvidenceLevel::Any, &["a"]);
        assert!(matches!(template.validate(), Err(WorkflowError::CycleDetected { .. })));
    }

    #[test]
    fn validation_rejects_empty_template_id() {
        let template = WorkflowTemplate::new("", "empty")
            .with_step("a", CapabilityKind::Reasoning, EvidenceLevel::Any, &[]);
        assert_eq!(template.validate(), Err(WorkflowError::EmptyTemplateId));
    }

    #[test]
    fn research_report_template_is_valid_and_shaped_as_expected() {
        let template = research_report_template();
        assert_eq!(template.validate(), Ok(()));
        // research / finance / documents + synthesis
        assert_eq!(template.steps.len(), 3);
        let ids: Vec<&str> = template.steps.iter().map(|s| s.step_id.as_str()).collect();
        assert_eq!(ids, vec!["research", "finance", "documents"]);
        assert!(template.synthesis);
        // finance and documents both depend on research; research has none.
        let finance = template.steps.iter().find(|s| s.step_id == "finance").unwrap();
        assert_eq!(finance.depends_on, vec!["research".to_string()]);
        let documents = template.steps.iter().find(|s| s.step_id == "documents").unwrap();
        assert_eq!(documents.depends_on, vec!["research".to_string()]);
        let research = template.steps.iter().find(|s| s.step_id == "research").unwrap();
        assert!(research.depends_on.is_empty());
    }

    #[test]
    fn instantiate_builds_valid_plan_with_synthesis_on_all_steps() {
        let template = research_report_template();
        let plan = template
            .instantiate(&master_task("m1"), "plan-1", 1_700_000_000_000)
            .unwrap();
        assert_eq!(plan.validate(), Ok(()));
        assert_eq!(plan.stages.len(), 4);
        let synth = plan.stage("synthesis").unwrap();
        // synthesis depends on all three capability steps.
        assert_eq!(
            synth.depends_on,
            vec![
                "research".to_string(),
                "finance".to_string(),
                "documents".to_string()
            ]
        );
        assert_eq!(synth.task.task_id, "m1.synthesis");
        // Each capability stage requires only its own capability.
        let research = plan.stage("research").unwrap();
        assert_eq!(research.task.required_capabilities.len(), 1);
        assert_eq!(
            research.task.required_capabilities[0].capability,
            CapabilityKind::Reasoning
        );
        // Synthesis carries the master task's critic verification.
        assert_eq!(synth.verification, TaskVerification::Critic);
    }

    #[test]
    fn run_workflow_completes_and_feeds_all_inputs_to_synthesis() {
        let template = research_report_template();
        let assignments = assign_all(&template, "a:worker");
        let outcome = run_workflow(&template, &master_task("m1"), &assignments, |_agent, _stage, input| {
            let count = match input {
                serde_json::Value::Object(m) => m.len(),
                _ => 0,
            };
            Ok(json!({ "input_count": count }))
        });
        assert_eq!(outcome.verdict, DelegationVerdict::Completed);
        assert_eq!(outcome.completed_stages, 4);
        assert_eq!(outcome.failed_stages, 0);
        // The synthesis stage received the outputs of all three capability
        // stages.
        let synth = outcome
            .result
            .stages
            .iter()
            .find(|s| s.stage_id == "synthesis")
            .unwrap();
        let received = synth
            .output
            .as_ref()
            .and_then(|v| v.get("input_count"))
            .and_then(|v| v.as_u64())
            .unwrap();
        assert_eq!(received, 3);
        assert!(outcome.result.final_output.is_some());
    }

    #[test]
    fn run_workflow_surfaces_a_failing_capability_stage_as_partial() {
        let template = research_report_template();
        let assignments = assign_all(&template, "a:worker");
        let outcome = run_workflow(&template, &master_task("m1"), &assignments, |_agent, stage, _| {
            if stage.stage_id == "finance" {
                return Err("finance service unavailable".to_string());
            }
            Ok(json!({ "ok": true }))
        });
        // finance failed AND synthesis could not run (its dependency produced
        // no output) — both are honestly reported.
        assert!(matches!(
            outcome.verdict,
            DelegationVerdict::Partial { ref failed_stages }
                if failed_stages == &vec!["finance".to_string(), "synthesis".to_string()]
        ));
        assert_eq!(outcome.failed_stages, 2);
        // research and documents still completed.
        assert_eq!(outcome.completed_stages, 2);
    }

    #[test]
    fn run_workflow_surfaces_invalid_template_as_failed() {
        let bad = WorkflowTemplate::new("t", "bad")
            .with_step("a", CapabilityKind::Reasoning, EvidenceLevel::Any, &[])
            .with_step("a", CapabilityKind::Coding, EvidenceLevel::Any, &[]);
        let outcome = run_workflow(&bad, &master_task("m1"), &[], |_, _, _| Ok(json!({})));
        assert!(matches!(outcome.verdict, DelegationVerdict::Failed { .. }));
        assert_eq!(outcome.completed_stages, 0);
    }

    #[test]
    fn template_and_outcome_round_trip_over_json() {
        let template = research_report_template();
        let assignments = assign_all(&template, "a:worker");
        let outcome = run_workflow(&template, &master_task("m1"), &assignments, |_agent, _stage, input| {
            let count = match input {
                serde_json::Value::Object(m) => m.len(),
                _ => 0,
            };
            Ok(json!({ "input_count": count }))
        });

        let t_json = serde_json::to_string(&template).unwrap();
        let t_back: WorkflowTemplate = serde_json::from_str(&t_json).unwrap();
        assert_eq!(template, t_back);

        let o_json = serde_json::to_string(&outcome).unwrap();
        let o_back: WorkflowOutcome = serde_json::from_str(&o_json).unwrap();
        assert_eq!(outcome, o_back);
    }
}
