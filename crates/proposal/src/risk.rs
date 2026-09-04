//! Risk classes and resource commitments.
//!
//! The taxonomy is future-complete on purpose: v0.1 policy allows only
//! `Sandbox` / `ReadOnly` risk with `None` commitment, but the `Economic`
//! risk class and the `Cr` / `DCAI` / `Escrow` commitments already exist as
//! *denied* values — so a future live adapter can be added without
//! redesigning the schema. Denied is a first-class, tested outcome.

use serde::{Deserialize, Serialize};

/// What blast radius an experiment claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentRiskClass {
    /// Fully simulated: no world state touched, not even reads that matter.
    Sandbox,
    /// May read live state (observations) but never writes anything.
    ReadOnly,
    /// Touches live/economic state. Declared for the future;
    /// DENIED deterministically in v0.1.
    Economic,
}

impl ExperimentRiskClass {
    /// True only for the class v0.1 can never allow.
    #[must_use]
    pub fn is_live(self) -> bool {
        matches!(self, Self::Economic)
    }
}

/// What economic resource an experiment asks to commit.
///
/// v0.1 allows `None` only. The other variants exist so the denial is
/// explicit and typed — and so the future adapter needs no schema change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceCommitment {
    /// No economic resource involved. The only v0.1-usable value.
    None,
    /// World currency stake. DENIED in v0.1.
    Cr,
    /// DCAI token involvement. DENIED in v0.1.
    /// (Explicit rename: `snake_case` would render this `d_c_a_i`.)
    #[serde(rename = "dcai")]
    DCAI,
    /// M18 escrow involvement. DENIED in v0.1.
    Escrow,
}

impl ResourceCommitment {
    /// True only for the commitment v0.1 can execute with.
    #[must_use]
    pub fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_classification_is_exact() {
        assert!(!ExperimentRiskClass::Sandbox.is_live());
        assert!(!ExperimentRiskClass::ReadOnly.is_live());
        assert!(ExperimentRiskClass::Economic.is_live());
    }

    #[test]
    fn only_none_commitment_is_free() {
        assert!(ResourceCommitment::None.is_none());
        assert!(!ResourceCommitment::Cr.is_none());
        assert!(!ResourceCommitment::DCAI.is_none());
        assert!(!ResourceCommitment::Escrow.is_none());
    }

    #[test]
    fn taxonomy_survives_json_round_trip() {
        for class in [
            ExperimentRiskClass::Sandbox,
            ExperimentRiskClass::ReadOnly,
            ExperimentRiskClass::Economic,
        ] {
            let s = serde_json::to_string(&class).unwrap();
            let back: ExperimentRiskClass = serde_json::from_str(&s).unwrap();
            assert_eq!(class, back);
        }
        for c in [
            ResourceCommitment::None,
            ResourceCommitment::Cr,
            ResourceCommitment::DCAI,
            ResourceCommitment::Escrow,
        ] {
            let s = serde_json::to_string(&c).unwrap();
            let back: ResourceCommitment = serde_json::from_str(&s).unwrap();
            assert_eq!(c, back);
        }
    }
}
