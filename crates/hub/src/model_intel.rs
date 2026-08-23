//! Model Intelligence registry (Model Colony) — deterministic facts about
//! the local models that can serve the fabric.
//!
//! # Two orthogonal axes, never conflated
//!
//! - [`AvailabilityState`] is a RUNTIME fact: can this model execute right
//!   now on this node? (loaded, degraded under pressure, unloaded)
//! - [`GovernanceStage`] is a GOVERNANCE fact: what may this model be used
//!   for, given the evidence about it? (experimental → shadow → candidate →
//!   approved; rejected at any point)
//!
//! Mixing them (a single `status` field, as first imagined) would let a
//! governance conclusion be implied by a runtime blip. They stay separate;
//! routing consumes both through hard gates.
//!
//! # No winner is hard-coded
//!
//! The seeded colony (Qwen3 1.7B / Gemma 3 1B / Phi-4-mini, all Q4) starts
//! every member at [`GovernanceStage::Experimental`] with
//! [`Provenance::Inferred`] capability claims. Benchmarks and verified
//! execution observations — never opinions — move models through the
//! lifecycle via gated transitions.

use crate::capability::{CapabilityKind, Provenance};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Runtime availability of a model on THIS node right now.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityState {
    /// Loaded and serving.
    #[default]
    Available,
    /// Serving but under resource pressure or degraded health.
    Degraded,
    /// Not loaded / not executable here at the moment.
    Unavailable,
}

/// Governance lifecycle of a model candidate.
///
/// Flow: experimental → shadow → candidate → approved; any pre-approved
/// stage (and approved itself, as revocation) → rejected. No transitions
/// out of rejected — register a new record instead.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceStage {
    /// Registered with inferred claims; benchmark-only traffic.
    #[default]
    Experimental,
    /// Receives shadow copies of production tasks alongside the approved
    /// production model; results accumulate as evidence.
    Shadow,
    /// Evidence collected; eligible for operator review to become approved.
    Candidate,
    /// Verified by benchmark evidence + operator approval; serves production.
    Approved,
    /// Rejected by evidence or operator decision; terminal.
    Rejected,
}

impl GovernanceStage {
    /// May serve PRODUCTION traffic: only approved models.
    pub fn serves_production(self) -> bool {
        self == GovernanceStage::Approved
    }

    /// May receive SHADOW copies of production tasks.
    pub fn receives_shadow(self) -> bool {
        matches!(self, GovernanceStage::Shadow | GovernanceStage::Candidate)
    }

    /// May run benchmark/evaluation tasks (the evidence-gathering path).
    pub fn may_benchmark(self) -> bool {
        self != GovernanceStage::Rejected
    }
}

/// Whether one governance transition is allowed.
pub fn can_transition_governance(from: GovernanceStage, to: GovernanceStage) -> bool {
    use GovernanceStage::*;
    matches!(
        (from, to),
        (Experimental, Shadow)
            | (Shadow, Candidate)
            | (Candidate, Approved)
            | (Experimental, Rejected)
            | (Shadow, Rejected)
            | (Candidate, Rejected)
            | (Approved, Rejected)
    )
}

/// Hardware footprint of a model, used by fit checks BEFORE any load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareRequirements {
    /// RAM the runtime needs once the model is resident (weights + KV for
    /// a nominal context). Absolute bytes — no unit ambiguity.
    pub ram_needed_bytes: u64,
    /// Free RAM the node must still hold AFTER residency (safety floor).
    pub min_free_ram_bytes: u64,
}

impl HardwareRequirements {
    /// Whether a node with these memory numbers fits this model.
    /// Pure function — same inputs, same verdict, always.
    pub fn fits(&self, total_ram_bytes: u64, available_ram_bytes: u64) -> bool {
        available_ram_bytes
            >= self
                .ram_needed_bytes
                .saturating_add(self.min_free_ram_bytes)
            && total_ram_bytes >= self.ram_needed_bytes
    }
}

/// One capability claim with an observed/inferred strength (0..=100) and
/// the provenance of the claim. Inferred claims NEVER outrank verified ones
/// in routing (verified gets a fixed bonus instead — see fabric routing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityClaim {
    pub kind: CapabilityKind,
    /// Strength percent 0..=100.
    pub strength: u8,
    pub provenance: Provenance,
}

impl CapabilityClaim {
    pub fn inferred(kind: CapabilityKind, strength: u8) -> Self {
        Self {
            kind,
            strength: strength.min(100),
            provenance: Provenance::Inferred,
        }
    }

    pub fn verified(kind: CapabilityKind, strength: u8) -> Self {
        Self {
            kind,
            strength: strength.min(100),
            provenance: Provenance::Verified,
        }
    }

    /// Effective routing strength: verified claims carry a flat bonus over
    /// equally-strong inferred claims — evidence outranks marketing.
    pub fn effective_strength(&self) -> u32 {
        let base = u32::from(self.strength);
        match self.provenance {
            Provenance::Verified => base * 2,
            Provenance::Inferred => base,
        }
    }
}

/// One registered model of the colony.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelIntelRecord {
    /// Stable id, e.g. `"qwen3-1.7b-q4"`.
    pub model_id: String,
    /// Who provides/serves it (`"local"` for colony members).
    pub provider: String,
    /// Inference runtime (`"llama.cpp"`).
    pub runtime: String,
    /// Quantization tag (`"q4_k_m"`).
    pub quantization: String,
    /// Maximum context length in tokens.
    pub context_length: u32,
    /// Capability claims (closed taxonomy — [`CapabilityKind`]).
    pub capabilities: Vec<CapabilityClaim>,
    /// Romanian language strength 0..=100 (language ≠ capability).
    pub romanian_strength: u8,
    /// Version tag of the artifact (GGUF revision).
    pub version: String,
    /// Hardware footprint.
    pub hardware: HardwareRequirements,
    /// Governance stage.
    pub governance: GovernanceStage,
}

impl ModelIntelRecord {
    /// Claimed strength for one capability, 0 when unclaimed.
    pub fn claim_strength(&self, kind: CapabilityKind) -> Option<&CapabilityClaim> {
        self.capabilities.iter().find(|c| c.kind == kind)
    }

    /// Summarized view for wire/dashboard (no secrets by construction).
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "model_id": self.model_id,
            "provider": self.provider,
            "runtime": self.runtime,
            "quantization": self.quantization,
            "context_length": self.context_length,
            "capabilities": self.capabilities.iter().map(|c| serde_json::json!({
                "kind": c.kind, "strength": c.strength, "provenance": c.provenance,
            })).collect::<Vec<_>>(),
            "romanian_strength": self.romanian_strength,
            "version": self.version,
            "hardware": {
                "ram_needed_bytes": self.hardware.ram_needed_bytes,
                "min_free_ram_bytes": self.hardware.min_free_ram_bytes,
            },
            "governance": self.governance,
        })
    }
}

/// Registry errors — all recoverable and explainable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelIntelError {
    #[error("model '{model_id}' is already registered")]
    DuplicateModel { model_id: String },
    #[error("model '{model_id}' is not registered")]
    UnknownModel { model_id: String },
    #[error("invalid governance transition for '{model_id}': {from:?} → {to:?}")]
    InvalidTransition {
        model_id: String,
        from: GovernanceStage,
        to: GovernanceStage,
    },
}

/// Deterministic model registry (BTreeMap → iteration is always sorted by
/// model_id, matching the fabric's tie-break discipline).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelIntelRegistry {
    models: BTreeMap<String, ModelIntelRecord>,
}

impl ModelIntelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a model. Fails on duplicate ids so collisions are loud.
    pub fn register(&mut self, record: ModelIntelRecord) -> Result<(), ModelIntelError> {
        if self.models.contains_key(&record.model_id) {
            return Err(ModelIntelError::DuplicateModel {
                model_id: record.model_id,
            });
        }
        self.models.insert(record.model_id.clone(), record);
        Ok(())
    }

    pub fn get(&self, model_id: &str) -> Option<&ModelIntelRecord> {
        self.models.get(model_id)
    }

    /// All records, model_id ascending (deterministic).
    pub fn all(&self) -> Vec<&ModelIntelRecord> {
        self.models.values().collect()
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Applies a governance transition after the state machine validates it.
    pub fn transition_governance(
        &mut self,
        model_id: &str,
        to: GovernanceStage,
    ) -> Result<GovernanceStage, ModelIntelError> {
        let record =
            self.models
                .get_mut(model_id)
                .ok_or_else(|| ModelIntelError::UnknownModel {
                    model_id: model_id.to_string(),
                })?;
        let from = record.governance;
        if !can_transition_governance(from, to) {
            return Err(ModelIntelError::InvalidTransition {
                model_id: model_id.to_string(),
                from,
                to,
            });
        }
        record.governance = to;
        Ok(to)
    }
}

/// GiB helper for readable seeds.
const fn gib(n: u64) -> u64 {
    n * 1024 * 1024 * 1024
}

/// Seeds the initial MODEL COLONY: three Q4 candidates, all EXPERIMENTAL,
/// all claims INFERRED until benchmarks say otherwise. Deliberately NO
/// ordering between them — the whole point of the colony is that evidence,
/// not intuition, decides.
pub fn seed_model_colony() -> ModelIntelRegistry {
    let mut reg = ModelIntelRegistry::new();

    // Generalist with strong tool/function calling; solid reasoning baseline.
    let _ = reg.register(ModelIntelRecord {
        model_id: "qwen3-1.7b-q4".into(),
        provider: "local".into(),
        runtime: "llama.cpp".into(),
        quantization: "q4_k_m".into(),
        context_length: 32_768,
        capabilities: vec![
            CapabilityClaim::inferred(CapabilityKind::Reasoning, 75),
            CapabilityClaim::inferred(CapabilityKind::Coding, 70),
            CapabilityClaim::inferred(CapabilityKind::ToolCalling, 80),
            CapabilityClaim::inferred(CapabilityKind::StructuredOutput, 75),
            CapabilityClaim::inferred(CapabilityKind::Agents, 65),
        ],
        romanian_strength: 60,
        version: "gguf-v1".into(),
        hardware: HardwareRequirements {
            ram_needed_bytes: gib(3),
            min_free_ram_bytes: gib(2),
        },
        governance: GovernanceStage::Experimental,
    });

    // Multilingual summarizer/classifier; compact and fast.
    let _ = reg.register(ModelIntelRecord {
        model_id: "gemma-3-1b-q4".into(),
        provider: "local".into(),
        runtime: "llama.cpp".into(),
        quantization: "q4_k_m".into(),
        context_length: 32_768,
        capabilities: vec![
            CapabilityClaim::inferred(CapabilityKind::Summarization, 75),
            CapabilityClaim::inferred(CapabilityKind::Classification, 70),
            CapabilityClaim::inferred(CapabilityKind::Chat, 70),
            CapabilityClaim::inferred(CapabilityKind::TextGeneration, 65),
        ],
        romanian_strength: 70,
        version: "gguf-v1".into(),
        hardware: HardwareRequirements {
            ram_needed_bytes: gib(2),
            min_free_ram_bytes: gib(2),
        },
        governance: GovernanceStage::Experimental,
    });

    // Long-context structured-output specialist.
    let _ = reg.register(ModelIntelRecord {
        model_id: "phi-4-mini-q4".into(),
        provider: "local".into(),
        runtime: "llama.cpp".into(),
        quantization: "q4_k_m".into(),
        context_length: 16_384,
        capabilities: vec![
            CapabilityClaim::inferred(CapabilityKind::Reasoning, 70),
            CapabilityClaim::inferred(CapabilityKind::StructuredOutput, 80),
            CapabilityClaim::inferred(CapabilityKind::FunctionCalling, 75),
            CapabilityClaim::inferred(CapabilityKind::Coding, 65),
        ],
        romanian_strength: 50,
        version: "gguf-v1".into(),
        hardware: HardwareRequirements {
            ram_needed_bytes: gib(3),
            min_free_ram_bytes: gib(2),
        },
        governance: GovernanceStage::Experimental,
    });

    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn governance_transitions_are_gated_and_terminal_rejection_holds() {
        use GovernanceStage::*;
        // The happy path.
        assert!(can_transition_governance(Experimental, Shadow));
        assert!(can_transition_governance(Shadow, Candidate));
        assert!(can_transition_governance(Candidate, Approved));
        // Revocation from anywhere pre-approved + from approved itself.
        assert!(can_transition_governance(Approved, Rejected));
        // Illegal jumps.
        assert!(
            !can_transition_governance(Experimental, Approved),
            "no skipping evidence stages"
        );
        assert!(!can_transition_governance(Experimental, Candidate));
        assert!(!can_transition_governance(Shadow, Approved));
        assert!(
            !can_transition_governance(Candidate, Shadow),
            "no demotion churn"
        );
        assert!(
            !can_transition_governance(Rejected, Experimental),
            "rejected is terminal"
        );
        assert!(!can_transition_governance(Approved, Candidate));

        // Traffic-class helpers.
        assert!(GovernanceStage::Approved.serves_production());
        assert!(!GovernanceStage::Shadow.serves_production());
        assert!(GovernanceStage::Candidate.receives_shadow());
        assert!(
            !GovernanceStage::Approved.receives_shadow(),
            "approved IS production, not shadow"
        );
        assert!(!GovernanceStage::Rejected.may_benchmark());
        assert!(GovernanceStage::Experimental.may_benchmark());
    }

    #[test]
    fn hardware_fit_is_exact_and_never_borrows_the_safety_floor() {
        let hw = HardwareRequirements {
            ram_needed_bytes: gib(3),
            min_free_ram_bytes: gib(2),
        };
        // Exactly enough: needs 3GiB + must KEEP 2GiB free → 5GiB available works.
        assert!(hw.fits(gib(8), gib(5)));
        // One byte short of the floor: no fit.
        assert!(!hw.fits(gib(8), gib(5) - 1));
        // Total RAM below the requirement: no fit even if momentarily free.
        assert!(!hw.fits(gib(2), gib(20)));
    }

    #[test]
    fn verified_claims_outrank_equally_strong_inferred_claims() {
        let inf = CapabilityClaim::inferred(CapabilityKind::Reasoning, 80);
        let ver = CapabilityClaim::verified(CapabilityKind::Reasoning, 80);
        assert!(ver.effective_strength() > inf.effective_strength());
        assert_eq!(inf.effective_strength(), 80);
        assert_eq!(ver.effective_strength(), 160);
    }

    #[test]
    fn seed_registers_exactly_three_experimental_candidates_no_winner() {
        let reg = seed_model_colony();
        assert_eq!(reg.len(), 3);
        let all = reg.all();
        // Sorted by id — deterministic.
        let ids: Vec<&str> = all.iter().map(|m| m.model_id.as_str()).collect();
        assert_eq!(ids, vec!["gemma-3-1b-q4", "phi-4-mini-q4", "qwen3-1.7b-q4"]);
        for m in &all {
            assert_eq!(m.governance, GovernanceStage::Experimental);
            assert!(
                m.capabilities
                    .iter()
                    .all(|c| c.provenance == Provenance::Inferred),
                "{}: seeds start INFERRED",
                m.model_id
            );
            assert!(m.context_length > 0);
            assert!(m.hardware.ram_needed_bytes > 0);
        }
        // Duplicate registration is loud.
        let dup = ModelIntelRecord {
            model_id: "gemma-3-1b-q4".into(),
            ..all[0].clone()
        };
        assert!(matches!(
            reg.clone().register(dup),
            Err(ModelIntelError::DuplicateModel { .. })
        ));
    }

    #[test]
    fn registry_transition_updates_record_and_validates() {
        let mut reg = seed_model_colony();
        reg.transition_governance("gemma-3-1b-q4", GovernanceStage::Shadow)
            .unwrap();
        assert_eq!(
            reg.get("gemma-3-1b-q4").unwrap().governance,
            GovernanceStage::Shadow
        );
        // Illegal jump errors and leaves state untouched.
        let err = reg.transition_governance("gemma-3-1b-q4", GovernanceStage::Approved);
        assert!(matches!(
            err,
            Err(ModelIntelError::InvalidTransition { .. })
        ));
        assert_eq!(
            reg.get("gemma-3-1b-q4").unwrap().governance,
            GovernanceStage::Shadow
        );
        assert!(matches!(
            reg.transition_governance("ghost", GovernanceStage::Shadow),
            Err(ModelIntelError::UnknownModel { .. })
        ));
    }
}
