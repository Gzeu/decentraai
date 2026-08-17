//! Unified agent capability — the seam between the two capability languages.
//!
//! DecentraAI historically had two parallel capability models that were NOT
//! cross-wired:
//!
//! - **semantic**: `decentraai-hub` taxonomy (`CapabilityKind`, 26 kinds:
//!   OCR, coding, vision…) with provenance (`Verified`/`Inferred`);
//! - **execution**: `decentraai-compute` physical capability (`ComputeCapability`,
//!   served models, engine) plus — new with the agent model — tools.
//!
//! [`AgentCapability`] is the single view that combines both, so a
//! coordinator can ask "which agents can do OCR on a model they may use"
//! instead of answering with two unrelated matchers.

use decentraai_hub::capability::{CapabilityClaim, ModelCapabilities};
use serde::{Deserialize, Serialize};

use crate::agent::AgentRecord;
use crate::tool::ToolDescriptor;

/// The full capability view of one agent: semantic claims + models + tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCapability {
    /// Semantic claims (hub taxonomy) with provenance.
    pub semantic: Vec<CapabilityClaim>,
    /// Model hashes the agent may use.
    pub models: Vec<String>,
    /// Tools the agent exposes.
    pub tools: Vec<ToolDescriptor>,
}

impl AgentCapability {
    /// The empty capability (no claims, no models, no tools).
    pub fn empty() -> Self {
        Self {
            semantic: Vec::new(),
            models: Vec::new(),
            tools: Vec::new(),
        }
    }

    /// Builds the capability view of an [`AgentRecord`].
    pub fn from_record(record: &AgentRecord) -> Self {
        Self {
            semantic: record.semantic_capabilities.clone(),
            models: record.allowed_models.clone(),
            tools: record.tools.clone(),
        }
    }

    /// Whether the agent claims a semantic capability (any provenance).
    pub fn has_semantic(&self, capability: decentraai_hub::capability::CapabilityKind) -> bool {
        self.semantic.iter().any(|c| c.capability == capability)
    }

    /// Whether the agent may use the given model hash.
    pub fn has_model(&self, model_hash: &str) -> bool {
        self.models.iter().any(|m| m == model_hash)
    }

    /// Whether the agent exposes a tool with the given name.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name == name)
    }
}

/// Lifts a raw claim list into the hub [`ModelCapabilities`] view so the
/// existing provenance-aware `hub::requirements::match_requirements` can be
/// reused verbatim on agent claims (the seam between the two systems).
pub fn model_capabilities_from_claims(claims: &[CapabilityClaim]) -> ModelCapabilities {
    ModelCapabilities {
        claims: claims.to_vec(),
        tasks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentRecord, ROLE_GENERALIST};
    use crate::tool::ToolDescriptor;
    use decentraai_hub::capability::{CapabilityKind, Provenance};

    #[test]
    fn capability_view_mirrors_record() {
        let record = AgentRecord::new("dca-a:g", "G", ROLE_GENERALIST)
            .with_capability(CapabilityKind::Chat, Provenance::Inferred)
            .with_model("m1")
            .with_tool(ToolDescriptor::new("t", crate::TOOL_KIND_BUILTIN));
        let cap = AgentCapability::from_record(&record);
        assert!(cap.has_semantic(CapabilityKind::Chat));
        assert!(!cap.has_semantic(CapabilityKind::Ocr));
        assert!(cap.has_model("m1"));
        assert!(cap.has_tool("t"));
    }

    #[test]
    fn claim_lift_produces_hub_view() {
        let claims = vec![CapabilityClaim {
            capability: CapabilityKind::Ocr,
            provenance: Provenance::Verified,
        }];
        let view = model_capabilities_from_claims(&claims);
        assert_eq!(view.claims.len(), 1);
        assert!(view.tasks.is_empty());
    }

    #[test]
    fn capability_round_trips_over_wire() {
        let cap = AgentCapability {
            semantic: vec![CapabilityClaim {
                capability: CapabilityKind::Coding,
                provenance: Provenance::Inferred,
            }],
            models: vec!["m1".into()],
            tools: vec![ToolDescriptor::new("t", crate::TOOL_KIND_MCP)],
        };
        let json = serde_json::to_string(&cap).unwrap();
        let back: AgentCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(cap, back);
    }
}