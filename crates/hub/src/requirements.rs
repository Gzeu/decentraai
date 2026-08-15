//! Capability requirements and provenance-aware matching (Next-Gen Phase 1).
//!
//! The Hub is a capability marketplace: an operator or agent should be able
//! to ask "which models can do X?" without naming a model. This module lets a
//! caller express a set of *required capabilities* (e.g. a tool that needs
//! OCR and summarization) and match them against a model's [`ModelCapabilities`]
//! as an explainable checklist, not an opaque score.
//!
//! Honesty rules (same provenance discipline as `crate::capability`):
//! - A required capability is only considered satisfied by a claim of the
//!   requested (or stronger) provenance. `Verified` never comes from an
//!   `Inferred` claim.
//! - A capability with no evidence stays `Missing` (UNKNOWN is not satisfied).
//! - The result exposes per-requirement reasons so the operator sees exactly
//!   which capability is missing and why.
//!
//! The module is pure: no I/O, no async. It reuses `CapabilityKind`,
//! `Provenance`, and `ModelCapabilities` from `crate::capability` — it does not
//! create a second capability system.

use crate::capability::{CapabilityKind, ModelCapabilities, Provenance};
use serde::Serialize;

/// The minimum evidence required to consider a capability satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceLevel {
    /// A VERIFIED claim is required; an INFERRED claim is not enough.
    Verified,
    /// Either VERIFIED or INFERRED evidence is acceptable.
    Any,
}

/// One capability a tool/workload requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CapabilityRequirement {
    pub capability: CapabilityKind,
    /// How strong the evidence must be.
    pub evidence: EvidenceLevel,
}

/// The verdict for a single required capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    /// Satisfied with the strongest available provenance for this capability.
    /// `provenance` is the strongest claim that satisfies the requirement.
    Satisfied { provenance: Provenance },
    /// The capability is claimed, but only with weaker provenance than the
    /// requirement demands (e.g. requirement wants VERIFIED, model only has
    /// INFERRED). Reported as a distinct, visible state — never flattened.
    InsufficientProvenance { found: Provenance, required: EvidenceLevel },
    /// No claim exists for this capability.
    Missing,
}

/// A single requirement check with its reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequirementCheck {
    pub capability: CapabilityKind,
    pub requirement: EvidenceLevel,
    pub status: RequirementStatus,
    /// Human explanation of the verdict (why it passed/failed).
    pub reason: String,
}

/// The full matching result: satisfied iff every required capability is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityMatch {
    pub satisfied: bool,
    pub checks: Vec<RequirementCheck>,
    /// The capabilities the model satisfies that were NOT required (context).
    pub extra: Vec<CapabilityKind>,
}

impl CapabilityMatch {
    /// Whether the model satisfies every required capability.
    pub fn is_satisfied(&self) -> bool {
        self.satisfied
    }
}

/// Match a model's capabilities against a set of required capabilities.
///
/// Provenance-aware: a requirement with `EvidenceLevel::Verified` is only
/// satisfied by a VERIFIED claim. `EvidenceLevel::Any` accepts VERIFIED or
/// INFERRED. A capability with no claim is `Missing`, never assumed.
pub fn match_requirements(
    model: &ModelCapabilities,
    requirements: &[CapabilityRequirement],
) -> CapabilityMatch {
    let mut checks = Vec::new();
    let mut satisfied = true;

    for req in requirements {
        // Strongest provenance the model actually claims for this capability.
        let has_verified = model
            .claims
            .iter()
            .any(|c| c.capability == req.capability && c.provenance == Provenance::Verified);
        let has_inferred = model
            .claims
            .iter()
            .any(|c| c.capability == req.capability && c.provenance == Provenance::Inferred);

        let status = match (has_verified, has_inferred) {
            (true, _) => RequirementStatus::Satisfied {
                provenance: Provenance::Verified,
            },
            (false, true) => match req.evidence {
                EvidenceLevel::Verified => RequirementStatus::InsufficientProvenance {
                    found: Provenance::Inferred,
                    required: EvidenceLevel::Verified,
                },
                EvidenceLevel::Any => RequirementStatus::Satisfied {
                    provenance: Provenance::Inferred,
                },
            },
            (false, false) => RequirementStatus::Missing,
        };

        if !matches!(status, RequirementStatus::Satisfied { .. }) {
            satisfied = false;
        }

        let reason = match &status {
            RequirementStatus::Satisfied { provenance } => format!(
                "{} — {} evidence",
                req.capability.label(),
                match provenance {
                    Provenance::Verified => "VERIFIED",
                    Provenance::Inferred => "INFERRED",
                }
            ),
            RequirementStatus::InsufficientProvenance { .. } => format!(
                "{} — only INFERRED evidence, but VERIFIED required",
                req.capability.label()
            ),
            RequirementStatus::Missing => format!(
                "{} — no evidence (UNKNOWN)",
                req.capability.label()
            ),
        };

        checks.push(RequirementCheck {
            capability: req.capability,
            requirement: req.evidence,
            status,
            reason,
        });
    }

    // Extra capabilities the model has that were not requested (context).
    let extra = model
        .claims
        .iter()
        .map(|c| c.capability)
        .filter(|cap| !requirements.iter().any(|r| r.capability == *cap))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    CapabilityMatch {
        satisfied,
        checks,
        extra,
    }
}

/// Convenience: does the model satisfy all required capabilities (any evidence)?
pub fn satisfies_any(
    model: &ModelCapabilities,
    requirements: &[CapabilityRequirement],
) -> bool {
    match_requirements(model, requirements).is_satisfied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{CapabilityClaim, ModelCapabilities, Provenance};

    fn model_with(claims: &[(CapabilityKind, Provenance)]) -> ModelCapabilities {
        ModelCapabilities {
            claims: claims
                .iter()
                .map(|(c, p)| CapabilityClaim {
                    capability: *c,
                    provenance: *p,
                })
                .collect(),
            tasks: vec![],
        }
    }

    fn req(capability: CapabilityKind, evidence: EvidenceLevel) -> CapabilityRequirement {
        CapabilityRequirement { capability, evidence }
    }

    #[test]
    fn verified_claim_satisfies_verified_requirement() {
        let model = model_with(&[(CapabilityKind::Ocr, Provenance::Verified)]);
        let m = match_requirements(&model, &[req(CapabilityKind::Ocr, EvidenceLevel::Verified)]);
        assert!(m.is_satisfied());
        assert_eq!(
            m.checks[0].status,
            RequirementStatus::Satisfied {
                provenance: Provenance::Verified
            }
        );
    }

    #[test]
    fn inferred_claim_does_not_satisfy_verified_requirement() {
        // Honesty: an INFERRED OCR claim must NOT satisfy a VERIFIED
        // requirement — the mismatch is reported explicitly.
        let model = model_with(&[(CapabilityKind::Ocr, Provenance::Inferred)]);
        let m = match_requirements(&model, &[req(CapabilityKind::Ocr, EvidenceLevel::Verified)]);
        assert!(!m.is_satisfied());
        assert_eq!(
            m.checks[0].status,
            RequirementStatus::InsufficientProvenance {
                found: Provenance::Inferred,
                required: EvidenceLevel::Verified
            }
        );
        assert!(m.checks[0].reason.contains("INFERRED"));
    }

    #[test]
    fn inferred_claim_satisfies_any_requirement() {
        let model = model_with(&[(CapabilityKind::Summarization, Provenance::Inferred)]);
        let m = match_requirements(&model, &[req(CapabilityKind::Summarization, EvidenceLevel::Any)]);
        assert!(m.is_satisfied());
        assert_eq!(
            m.checks[0].status,
            RequirementStatus::Satisfied {
                provenance: Provenance::Inferred
            }
        );
    }

    #[test]
    fn missing_capability_is_never_satisfied() {
        let model = model_with(&[]);
        let m = match_requirements(&model, &[req(CapabilityKind::Vision, EvidenceLevel::Any)]);
        assert!(!m.is_satisfied());
        assert_eq!(m.checks[0].status, RequirementStatus::Missing);
        assert!(m.checks[0].reason.contains("UNKNOWN"));
    }

    #[test]
    fn all_requirements_must_be_met() {
        let model = model_with(&[
            (CapabilityKind::Ocr, Provenance::Verified),
            (CapabilityKind::Summarization, Provenance::Verified),
        ]);
        let m = match_requirements(
            &model,
            &[
                req(CapabilityKind::Ocr, EvidenceLevel::Verified),
                req(CapabilityKind::Vision, EvidenceLevel::Verified),
            ],
        );
        assert!(!m.is_satisfied(), "missing Vision must fail the whole match");
        assert_eq!(m.checks[1].status, RequirementStatus::Missing);
    }

    #[test]
    fn extra_capabilities_are_reported_separately() {
        let model = model_with(&[
            (CapabilityKind::Ocr, Provenance::Verified),
            (CapabilityKind::Coding, Provenance::Inferred),
        ]);
        let m = match_requirements(&model, &[req(CapabilityKind::Ocr, EvidenceLevel::Verified)]);
        assert!(m.is_satisfied());
        assert_eq!(m.extra, vec![CapabilityKind::Coding]);
    }

    #[test]
    fn satisfies_any_helper() {
        let model = model_with(&[(CapabilityKind::Ocr, Provenance::Verified)]);
        assert!(satisfies_any(&model, &[req(CapabilityKind::Ocr, EvidenceLevel::Any)]));
        assert!(!satisfies_any(&model, &[req(CapabilityKind::Vision, EvidenceLevel::Any)]));
    }

    #[test]
    fn empty_requirements_are_vacuously_satisfied() {
        let model = model_with(&[]);
        let m = match_requirements(&model, &[]);
        assert!(m.is_satisfied());
        assert!(m.checks.is_empty());
        assert!(m.extra.is_empty());
    }

    #[test]
    fn strongest_provenance_wins_for_any_requirement() {
        // A model with both VERIFIED and INFERRED claims satisfies an `Any`
        // requirement at VERIFIED strength.
        let model = model_with(&[
            (CapabilityKind::Coding, Provenance::Inferred),
            (CapabilityKind::Coding, Provenance::Verified),
        ]);
        let m = match_requirements(&model, &[req(CapabilityKind::Coding, EvidenceLevel::Any)]);
        assert!(m.is_satisfied());
        assert_eq!(
            m.checks[0].status,
            RequirementStatus::Satisfied {
                provenance: Provenance::Verified
            }
        );
    }

    #[test]
    fn serializes_for_api_consumption() {
        let model = model_with(&[(CapabilityKind::Ocr, Provenance::Verified)]);
        let m = match_requirements(&model, &[req(CapabilityKind::Ocr, EvidenceLevel::Verified)]);
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"satisfied\":true"));
        assert!(json.contains("ocr"));
    }
}
