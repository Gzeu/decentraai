//! Deterministic validation of a parsed plan against what the fabric
//! actually has. This is the "the fabric validates" half of the invariant:
//! it never executes anything, it only classifies the plan's capabilities as
//! satisfied or missing given a snapshot of available fabric capabilities.

use decentraai_hub::capability::CapabilityKind;
use serde::Serialize;

use crate::plan::TaskPlan;

/// Result of checking one [`TaskPlan`] against the mesh's capability set.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanValidation {
    /// Required capabilities the fabric can satisfy right now.
    pub satisfied: Vec<String>,
    /// Required capabilities with no known provider in the mesh.
    pub missing: Vec<String>,
    /// Optional capabilities that were found (informational).
    pub optional_found: Vec<String>,
}

impl PlanValidation {
    /// A plan is executable only when EVERY required capability is present.
    pub fn executable(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Checks `plan` against `available` — the capability set assembled by the
/// deterministic fabric from worker advertisements and skill registries.
///
/// Pure function: the caller decides where `available` came from; this code
/// cannot be fooled into trusting a peer, because it performs NO discovery
/// of its own.
pub fn validate_against_fabric(plan: &TaskPlan, available: &[CapabilityKind]) -> PlanValidation {
    let mut satisfied = Vec::new();
    let mut missing = Vec::new();
    let mut optional_found = Vec::new();
    for cap in &plan.capabilities {
        let known = cap.name.parse::<CapabilityKind>().is_ok_and(|k| {
            // The plan parser already guarantees taxonomy validity; here we
            // check PRESENCE in the mesh snapshot.
            available.contains(&k)
        });
        match (cap.required, known) {
            (true, true) => satisfied.push(cap.name.clone()),
            (true, false) => missing.push(cap.name.clone()),
            (false, true) => optional_found.push(cap.name.clone()),
            // An unsatisfied optional step is simply skipped by the planner.
            (false, false) => {}
        }
    }
    PlanValidation {
        satisfied,
        missing,
        optional_found,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::TaskPlan;

    fn parse(s: &str) -> TaskPlan {
        TaskPlan::parse(s).expect("test plan must parse")
    }

    #[test]
    fn required_missing_blocks_execution() {
        let plan = parse(
            r#"{"intent":"x","capabilities":[{"name":"ocr","required":true},{"name":"reasoning","required":true}],"workflow":["ocr","reasoning"],"confidence":1}"#,
        );
        let v = validate_against_fabric(&plan, &[CapabilityKind::Ocr]);
        assert_eq!(v.satisfied, vec!["ocr"]);
        assert_eq!(v.missing, vec!["reasoning"]);
        assert!(!v.executable(), "a missing required capability blocks execution");
    }

    #[test]
    fn all_required_satisfied_is_executable() {
        let plan = parse(
            r#"{"intent":"x","capabilities":[{"name":"embeddings","required":true},{"name":"reranking","required":false}],"workflow":["embeddings"],"confidence":0.8}"#,
        );
        let v = validate_against_fabric(
            &plan,
            &[CapabilityKind::Embeddings, CapabilityKind::Reranking],
        );
        assert!(v.executable());
        assert!(v.optional_found.contains(&"reranking".to_string()));
    }

    #[test]
    fn empty_mesh_rejects_everything_required() {
        let plan = parse(
            r#"{"intent":"x","capabilities":[{"name":"coding","required":true}],"workflow":["coding"],"confidence":1}"#,
        );
        let v = validate_against_fabric(&plan, &[]);
        assert!(!v.executable());
    }
}
