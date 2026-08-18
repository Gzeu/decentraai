//! Generic agent task — the task contract a later phase routes and executes.
//!
//! Today the fabric only routes `InferRequest` (prompt → tokens). A
//! collective fabric needs tasks whose input/output are arbitrary (schemas),
//! which require *capabilities* not just a model, carry budgets, and declare
//! how their result must be verified. This type defines that contract now
//! (P0) so the delegation phase (P3) can build on a settled shape.
//!
//! It is deliberately NOT wired into execution yet: defining the shape with
//! tests is the P0 deliverable; routing it is the P3 milestone.

use decentraai_compute::WorkloadRequirements;
use decentraai_hub::capability::CapabilityKind;
use decentraai_hub::requirements::{CapabilityRequirement, EvidenceLevel};
use serde::{Deserialize, Serialize};

/// How a task's result must be verified before it is consumed (§11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskVerification {
    /// No explicit verification (best-effort output).
    None,
    /// The executing agent self-checks the output against its schema.
    SelfCheck,
    /// A dedicated critic agent reviews the result.
    Critic,
    /// Multiple agents produce results and the fabric resolves disagreement.
    Consensus,
}

/// Wire-safe physical workload requirement for an agent task.
///
/// The fabric's `decentraai_compute::WorkloadRequirements` is deliberately
/// protocol-agnostic (no serde derives); tasks must travel over the P2P
/// channel, so this is the serializable mirror with a lossless conversion
/// both ways. Keep the fields in sync with the compute type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorkloadRequirement {
    /// Model the task needs to execute.
    pub model_hash: String,
    /// Estimated host RAM footprint (MiB).
    pub est_ram_mb: u64,
    /// Estimated VRAM footprint (MiB); `0` = CPU-only.
    pub est_vram_mb: u64,
    /// Max tokens the task may emit.
    pub max_tokens: u32,
    /// Whether streaming is preferred.
    pub stream: bool,
    /// Task priority (higher wins).
    pub priority: u8,
    /// Optional semantic capability name (snake_case), carried for honest
    /// verdicts — same convention as the compute type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_capability: Option<String>,
}

impl From<WorkloadRequirements> for AgentWorkloadRequirement {
    fn from(wl: WorkloadRequirements) -> Self {
        Self {
            model_hash: wl.model_hash,
            est_ram_mb: wl.est_ram_mb,
            est_vram_mb: wl.est_vram_mb,
            max_tokens: wl.max_tokens,
            stream: wl.stream,
            priority: wl.priority,
            required_capability: wl.required_capability,
        }
    }
}

impl AgentWorkloadRequirement {
    /// Back to the compute type for the fabric's scheduler/matcher.
    pub fn to_compute(&self) -> WorkloadRequirements {
        let mut wl =
            WorkloadRequirements::new(self.model_hash.clone(), self.est_ram_mb, self.est_vram_mb);
        wl.max_tokens = self.max_tokens;
        wl.stream = self.stream;
        wl.priority = self.priority;
        wl.required_capability = self.required_capability.clone();
        wl
    }
}

/// A generic task in the collective fabric.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTask {
    /// Unique task id.
    pub task_id: String,
    /// Parent task id when this is a sub-task of a decomposed task (DAG).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Semantic capabilities the executor must claim.
    pub required_capabilities: Vec<CapabilityRequirement>,
    /// Physical workload requirements (model + resources), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_workload: Option<AgentWorkloadRequirement>,
    /// Optional JSON-schema hint for the input payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<String>,
    /// Optional JSON-schema hint for the expected output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<String>,
    /// Hard token budget for the task (0 = node default).
    pub budget_max_tokens: u32,
    /// Verification requirement before the result is consumed.
    pub verification: TaskVerification,
    /// Task priority (higher wins; mirrors the fabric's priority band).
    pub priority: u8,
}

impl AgentTask {
    /// A minimal task with the given id.
    pub fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            parent_id: None,
            required_capabilities: Vec::new(),
            required_workload: None,
            input_schema: None,
            output_schema: None,
            budget_max_tokens: 0,
            verification: TaskVerification::None,
            priority: 128,
        }
    }

    /// Marks the task as a sub-task of another task.
    pub fn child_of(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_id = Some(parent_id.into());
        self
    }

    /// Adds a semantic capability requirement.
    pub fn require_capability(
        mut self,
        capability: CapabilityKind,
        evidence: EvidenceLevel,
    ) -> Self {
        self.required_capabilities.push(CapabilityRequirement {
            capability,
            evidence,
        });
        self
    }

    /// Sets the physical workload requirements (losslessly converted from
    /// the compute type).
    pub fn with_workload(mut self, workload: WorkloadRequirements) -> Self {
        self.required_workload = Some(AgentWorkloadRequirement::from(workload));
        self
    }

    /// Sets input and output schema hints.
    pub fn with_schemas(mut self, input: impl Into<String>, output: impl Into<String>) -> Self {
        self.input_schema = Some(input.into());
        self.output_schema = Some(output.into());
        self
    }

    /// Sets the verification requirement.
    pub fn verified_by(mut self, verification: TaskVerification) -> Self {
        self.verification = verification;
        self
    }

    /// Sets the token budget.
    pub fn with_budget(mut self, max_tokens: u32) -> Self {
        self.budget_max_tokens = max_tokens;
        self
    }

    /// Whether the task requires a model (physical execution).
    pub fn needs_execution(&self) -> bool {
        self.required_workload.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_hub::capability::CapabilityKind;

    #[test]
    fn task_round_trips_over_wire() {
        let task = AgentTask::new("t-1")
            .child_of("t-0")
            .require_capability(CapabilityKind::Ocr, EvidenceLevel::Verified)
            .with_schemas(r#"{"type":"string"}"#, r#"{"type":"string"}"#)
            .verified_by(TaskVerification::Critic)
            .with_budget(4096);
        let json = serde_json::to_string(&task).unwrap();
        let back: AgentTask = serde_json::from_str(&json).unwrap();
        assert_eq!(task, back);
    }

    #[test]
    fn default_task_is_minimal_and_honest() {
        let task = AgentTask::new("t-2");
        assert!(task.parent_id.is_none());
        assert!(task.required_capabilities.is_empty());
        assert!(!task.needs_execution());
        assert_eq!(task.verification, TaskVerification::None);
        assert_eq!(task.priority, 128);
    }

    #[test]
    fn workload_marks_execution_task_and_round_trips() {
        let mut wl = WorkloadRequirements::new("m".into(), 256, 0);
        wl.required_capability = Some("ocr".into());
        let task = AgentTask::new("t-3").with_workload(wl);
        assert!(task.needs_execution());
        // Conversion is lossless both ways.
        let json = serde_json::to_string(&task).unwrap();
        let back: AgentTask = serde_json::from_str(&json).unwrap();
        let wl2 = back.required_workload.unwrap().to_compute();
        assert_eq!(wl2.model_hash, "m");
        assert_eq!(wl2.est_ram_mb, 256);
        assert_eq!(wl2.required_capability.as_deref(), Some("ocr"));
    }
}
