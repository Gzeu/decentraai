//! Unified agent matcher — ONE compositional verdict for an agent task.
//!
//! The fabric previously had two unrelated matchers: the hub's semantic
//! `match_requirements` (provenance-aware capability checklist) and the
//! compute `CapabilityMatcher` (physical eligibility: trust, model served,
//! RAM/VRAM floors, health, reservation headroom). A caller had to run both
//! and reconcile two verdicts by hand.
//!
//! [`match_agent`] composes them: it first checks the *agent's* semantic
//! claims against the semantic requirements, then the *hosting node's*
//! physical advertisement against the workload requirements — with an
//! agent-level gate in between (the agent must be allowed to use the
//! required model). The result is a single [`AgentMatchOutcome`] with an
//! explainable reason.
//!
//! Honesty invariants (unchanged from the fabric):
//! - A `Verified` semantic requirement is only satisfied by a `Verified`
//!   claim — an `Inferred` claim is surfaced as
//!   `SemanticInsufficientProvenance`, never flattened to success.
//! - Physical eligibility runs through the existing compute matcher so
//!   trust, reservations and capacity rules are never bypassed.

use decentraai_compute::{CapabilityMatcher, MatchReason, ReservationLedger, WorkloadRequirements};
use decentraai_hub::capability::CapabilityKind;
use decentraai_hub::requirements::{
    CapabilityMatch, CapabilityRequirement, EvidenceLevel, match_requirements,
};
use libp2p::PeerId;

use crate::agent::AgentRecord;
use crate::capability::model_capabilities_from_claims;

/// What an agent task requires — the union of the two capability languages.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentRequirement {
    /// Semantic requirements (hub taxonomy, provenance-aware).
    pub semantic: Vec<CapabilityRequirement>,
    /// Physical workload requirements (model + resources). `None` = the task
    /// has no execution requirement (e.g. a pure tool/planning task).
    pub workload: Option<WorkloadRequirements>,
}

impl AgentRequirement {
    /// A requirement with only semantic demands.
    pub fn semantic_only(requirements: Vec<CapabilityRequirement>) -> Self {
        Self {
            semantic: requirements,
            workload: None,
        }
    }

    /// A requirement with both semantic and physical demands.
    pub fn new(
        semantic: Vec<CapabilityRequirement>,
        workload: Option<WorkloadRequirements>,
    ) -> Self {
        Self { semantic, workload }
    }

    /// Adds one semantic requirement.
    pub fn require_capability(
        mut self,
        capability: CapabilityKind,
        evidence: EvidenceLevel,
    ) -> Self {
        self.semantic.push(CapabilityRequirement {
            capability,
            evidence,
        });
        self
    }
}

/// Why an agent was rejected. Composed of the semantic and physical layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMatchReason {
    /// A required semantic capability has no claim at all.
    SemanticMissing {
        capability: CapabilityKind,
        reason: String,
    },
    /// A required semantic capability exists but with weaker provenance than
    /// demanded (e.g. requirement says `Verified`, agent only has `Inferred`).
    SemanticInsufficientProvenance {
        capability: CapabilityKind,
        reason: String,
    },
    /// The agent is not allowed to use the required model.
    AgentNotAllowedModel { model_hash: String },
    /// The physical/execution gate failed (delegated to the compute matcher).
    Physical(MatchReason),
}

/// The single verdict of the unified matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMatchOutcome {
    Eligible,
    Rejected(AgentMatchReason),
}

impl AgentMatchOutcome {
    pub fn is_eligible(&self) -> bool {
        matches!(self, AgentMatchOutcome::Eligible)
    }
}

/// Semantic-only check: match an agent's claims against a set of semantic
/// requirements, reusing the hub's provenance-aware matcher verbatim.
///
/// The returned [`CapabilityMatch`] is the hub result — every check carries a
/// human reason, and `is_satisfied()` is true iff every requirement is met
/// with sufficient provenance.
pub fn match_agent_semantic(
    agent: &AgentRecord,
    requirements: &[CapabilityRequirement],
) -> CapabilityMatch {
    let view = model_capabilities_from_claims(&agent.semantic_capabilities);
    match_requirements(&view, requirements)
}

/// The unified agent matcher: semantic claims (agent) + execution capacity
/// (hosting node) + agent-level model gate, returning one verdict.
///
/// `node_adv` is the hosting node's [`decentraai_compute::ComputeAdvertisement`],
/// `matcher`/`ledger`/`trusted`/`local_peer` are the compute-side inputs the
/// existing scheduler would apply to this workload.
pub fn match_agent(
    agent: &AgentRecord,
    node_adv: &decentraai_compute::ComputeAdvertisement,
    requirement: &AgentRequirement,
    matcher: &CapabilityMatcher,
    ledger: &ReservationLedger,
    trusted: bool,
    local_peer: Option<&PeerId>,
) -> AgentMatchOutcome {
    // 1. Semantic gate: the agent must satisfy every semantic requirement.
    let semantic = match_agent_semantic(agent, &requirement.semantic);
    if !semantic.is_satisfied() {
        for check in &semantic.checks {
            use decentraai_hub::requirements::RequirementStatus;
            match &check.status {
                RequirementStatus::Missing => {
                    return AgentMatchOutcome::Rejected(AgentMatchReason::SemanticMissing {
                        capability: check.capability,
                        reason: check.reason.clone(),
                    });
                }
                RequirementStatus::InsufficientProvenance { .. } => {
                    return AgentMatchOutcome::Rejected(
                        AgentMatchReason::SemanticInsufficientProvenance {
                            capability: check.capability,
                            reason: check.reason.clone(),
                        },
                    );
                }
                RequirementStatus::Satisfied { .. } => {}
            }
        }
        // Defensive: never return Eligible with an unsatisfied semantic match.
        return AgentMatchOutcome::Rejected(AgentMatchReason::SemanticMissing {
            capability: CapabilityKind::Chat,
            reason: "semantic match failed with no detailed check".to_string(),
        });
    }

    // 2. Agent-level model gate: the agent must be allowed to use the model.
    if let Some(wl) = &requirement.workload {
        if !agent.has_model(&wl.model_hash) {
            return AgentMatchOutcome::Rejected(AgentMatchReason::AgentNotAllowedModel {
                model_hash: wl.model_hash.clone(),
            });
        }
    }

    // 3. Physical gate: delegate to the existing compute matcher, which
    //    enforces trust, model presence on the node, health, remote opt-in,
    //    RAM/VRAM floors minus reservations, load and reservation cap.
    if let Some(wl) = &requirement.workload {
        match matcher.matches(node_adv, wl, ledger, trusted, local_peer) {
            decentraai_compute::MatchOutcome::Eligible => AgentMatchOutcome::Eligible,
            decentraai_compute::MatchOutcome::Rejected(reason) => {
                AgentMatchOutcome::Rejected(AgentMatchReason::Physical(reason))
            }
        }
    } else {
        // No execution requirement — semantic gate was the whole check.
        AgentMatchOutcome::Eligible
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentRecord, ROLE_GENERALIST, ROLE_PLANNER, ROLE_SPECIALIST};
    use decentraai_compute::{ComputeAdvertisement, GpuSpec, ServedModel, WorkerHealth};
    use decentraai_hub::capability::Provenance;
    use libp2p::identity::Keypair;
    use std::time::Duration;

    fn test_peer() -> PeerId {
        PeerId::from(Keypair::generate_ed25519().public())
    }

    /// A node that serves model "abc" (est_ram 256, est_vram 3072) with 16 GiB
    /// free RAM and 18 GiB free VRAM, healthy, remote-enabled.
    fn node_advertisement(peer: PeerId) -> ComputeAdvertisement {
        ComputeAdvertisement {
            peer_id: peer,
            node_name: "node".into(),
            capability: decentraai_compute::ComputeCapability {
                cpu_cores: 8,
                ram_mb: 32 * 1024,
                gpu: Some(GpuSpec::simple("gpu", 24 * 1024, "d")),
                engine: "llama_server".into(),
                served_models: vec![ServedModel {
                    model_hash: "abc".into(),
                    file_name: "m.gguf".into(),
                    size_mb: 1024,
                    est_ram_mb: 256,
                    est_vram_mb: 3072,
                    context_tokens: 0,
                }],
                can_provision: false,
                available_models: vec![],
            },
            availability: decentraai_compute::ComputeAvailability {
                available_ram_mb: 16 * 1024,
                available_vram_mb: Some(18 * 1024),
                load_percent: 5,
                queue_depth: 0,
                tokens_per_second: 100,
                current_latency_ms: 50,
                status: WorkerHealth::Ready,
                gpu_temperature_celsius: None,
                gpu_utilization_percent: None,
                battery_percent: None,
            },
            announced_at_ms: 1_700_000_000_000,
            accepts_remote_inference: true,
            node_id: "dca-node".into(),
            node_version: "1.0.0".into(),
        }
    }

    fn workload() -> WorkloadRequirements {
        let mut wl = WorkloadRequirements::new("abc".into(), 256, 3072);
        wl.required_capability = Some("ocr".into());
        wl
    }

    fn ledger() -> ReservationLedger {
        ReservationLedger::new(Duration::from_secs(60), 4)
    }

    #[test]
    fn eligible_agent_passes_both_gates() {
        let peer = test_peer();
        let mut agent = AgentRecord::new("a:ocr", "OCR", ROLE_SPECIALIST)
            .with_capability(CapabilityKind::Ocr, Provenance::Verified)
            .with_model("abc");
        agent.set_state(crate::agent::AgentState::Ready);
        let req = AgentRequirement::new(
            vec![CapabilityRequirement {
                capability: CapabilityKind::Ocr,
                evidence: EvidenceLevel::Verified,
            }],
            Some(workload()),
        );
        let outcome = match_agent(
            &agent,
            &node_advertisement(peer),
            &req,
            &CapabilityMatcher::default(),
            &ledger(),
            true,
            None,
        );
        assert_eq!(outcome, AgentMatchOutcome::Eligible);
    }

    #[test]
    fn inferred_claim_rejected_against_verified_requirement() {
        // Honesty: an INFERRED OCR claim must NOT satisfy a VERIFIED
        // requirement — the mismatch is reported explicitly.
        let peer = test_peer();
        let agent = AgentRecord::new("a:ocr", "OCR", ROLE_SPECIALIST)
            .with_capability(CapabilityKind::Ocr, Provenance::Inferred)
            .with_model("abc");
        let req = AgentRequirement::new(
            vec![CapabilityRequirement {
                capability: CapabilityKind::Ocr,
                evidence: EvidenceLevel::Verified,
            }],
            Some(workload()),
        );
        let outcome = match_agent(
            &agent,
            &node_advertisement(peer),
            &req,
            &CapabilityMatcher::default(),
            &ledger(),
            true,
            None,
        );
        assert!(matches!(
            outcome,
            AgentMatchOutcome::Rejected(AgentMatchReason::SemanticInsufficientProvenance {
                capability: CapabilityKind::Ocr,
                ..
            })
        ));
    }

    #[test]
    fn missing_capability_rejected() {
        let peer = test_peer();
        let agent = AgentRecord::new("a:chat", "Chat", ROLE_GENERALIST)
            .with_capability(CapabilityKind::Chat, Provenance::Inferred)
            .with_model("abc");
        let req = AgentRequirement::new(
            vec![CapabilityRequirement {
                capability: CapabilityKind::Ocr,
                evidence: EvidenceLevel::Any,
            }],
            Some(workload()),
        );
        let outcome = match_agent(
            &agent,
            &node_advertisement(peer),
            &req,
            &CapabilityMatcher::default(),
            &ledger(),
            true,
            None,
        );
        assert!(matches!(
            outcome,
            AgentMatchOutcome::Rejected(AgentMatchReason::SemanticMissing {
                capability: CapabilityKind::Ocr,
                ..
            })
        ));
    }

    #[test]
    fn agent_model_gate_blocks_unallowed_model() {
        let peer = test_peer();
        let agent = AgentRecord::new("a:ocr", "OCR", ROLE_SPECIALIST)
            .with_capability(CapabilityKind::Ocr, Provenance::Verified)
            .with_model("other");
        let req = AgentRequirement::new(
            vec![CapabilityRequirement {
                capability: CapabilityKind::Ocr,
                evidence: EvidenceLevel::Verified,
            }],
            Some(workload()),
        );
        let outcome = match_agent(
            &agent,
            &node_advertisement(peer),
            &req,
            &CapabilityMatcher::default(),
            &ledger(),
            true,
            None,
        );
        assert!(matches!(
            outcome,
            AgentMatchOutcome::Rejected(AgentMatchReason::AgentNotAllowedModel {
                model_hash
            }) if model_hash == "abc"
        ));
    }

    #[test]
    fn physical_gate_delegates_to_compute_matcher() {
        // The node does not serve "xyz" — the unified verdict must surface
        // the physical rejection (compute matcher's ModelNotServed).
        let peer = test_peer();
        let agent = AgentRecord::new("a:ocr", "OCR", ROLE_SPECIALIST)
            .with_capability(CapabilityKind::Ocr, Provenance::Verified)
            .with_model("xyz");
        let mut wl = workload();
        wl.model_hash = "xyz".into();
        let req = AgentRequirement::new(
            vec![CapabilityRequirement {
                capability: CapabilityKind::Ocr,
                evidence: EvidenceLevel::Verified,
            }],
            Some(wl),
        );
        let outcome = match_agent(
            &agent,
            &node_advertisement(peer),
            &req,
            &CapabilityMatcher::default(),
            &ledger(),
            true,
            None,
        );
        assert!(matches!(
            outcome,
            AgentMatchOutcome::Rejected(AgentMatchReason::Physical(MatchReason::ModelNotServed))
        ));
    }

    #[test]
    fn untrusted_node_rejected_at_physical_gate() {
        let peer = test_peer();
        let agent = AgentRecord::new("a:ocr", "OCR", ROLE_SPECIALIST)
            .with_capability(CapabilityKind::Ocr, Provenance::Verified)
            .with_model("abc");
        let req = AgentRequirement::new(
            vec![CapabilityRequirement {
                capability: CapabilityKind::Ocr,
                evidence: EvidenceLevel::Verified,
            }],
            Some(workload()),
        );
        let outcome = match_agent(
            &agent,
            &node_advertisement(peer),
            &req,
            &CapabilityMatcher::default(),
            &ledger(),
            false,
            None,
        );
        assert!(matches!(
            outcome,
            AgentMatchOutcome::Rejected(AgentMatchReason::Physical(MatchReason::NotTrusted))
        ));
    }

    #[test]
    fn semantic_only_requirement_skips_physical_gate() {
        let peer = test_peer();
        let agent = AgentRecord::new("a:planner", "Planner", ROLE_PLANNER)
            .with_capability(CapabilityKind::Reasoning, Provenance::Verified);
        let req = AgentRequirement::semantic_only(vec![CapabilityRequirement {
            capability: CapabilityKind::Reasoning,
            evidence: EvidenceLevel::Verified,
        }]);
        let outcome = match_agent(
            &agent,
            &node_advertisement(peer),
            &req,
            &CapabilityMatcher::default(),
            &ledger(),
            true,
            None,
        );
        assert_eq!(outcome, AgentMatchOutcome::Eligible);
    }

    #[test]
    fn empty_requirements_are_vacuously_satisfied() {
        let peer = test_peer();
        let agent = AgentRecord::new("a:empty", "Empty", ROLE_GENERALIST);
        let req = AgentRequirement::semantic_only(vec![]);
        let outcome = match_agent(
            &agent,
            &node_advertisement(peer),
            &req,
            &CapabilityMatcher::default(),
            &ledger(),
            true,
            None,
        );
        assert_eq!(outcome, AgentMatchOutcome::Eligible);
    }
}
