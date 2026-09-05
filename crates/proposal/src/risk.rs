//! Risk classes and resource commitments.
//!
//! v0.1 allowed only `Sandbox` / `ReadOnly` with `None` commitment.
//! v0.2 opens exactly one more lane: `TestnetEconomic` — real testnet
//! effects (never mainnet) inside an explicit [`crate::budget`] and behind
//! [`crate::economic::TestnetEconomicAuthorization`]. `Economic` stays
//! denied: there is no mainnet lane, by construction.

use serde::{Deserialize, Serialize};

/// What blast radius an experiment claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentRiskClass {
    /// Fully simulated: no world state touched, not even reads that matter.
    Sandbox,
    /// May read live state (observations) but never writes anything.
    ReadOnly,
    /// Touches live/economic state. NO mainnet lane exists: DENIED always.
    Economic,
    /// Real effects on MultiversX TESTNET only, inside an explicit budget
    /// and behind testnet authorization. The only live lane in v0.2.
    TestnetEconomic,
}

impl ExperimentRiskClass {
    /// True for classes that touch live state (`Economic`, `TestnetEconomic`).
    #[must_use]
    pub fn is_live(self) -> bool {
        matches!(self, Self::Economic | Self::TestnetEconomic)
    }

    /// True only for the lane v0.2 can allow with authorization.
    #[must_use]
    pub fn is_testnet(self) -> bool {
        matches!(self, Self::TestnetEconomic)
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
        assert!(ExperimentRiskClass::TestnetEconomic.is_live());
        assert!(!ExperimentRiskClass::Sandbox.is_testnet());
        assert!(ExperimentRiskClass::TestnetEconomic.is_testnet());
        assert!(!ExperimentRiskClass::Economic.is_testnet());
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
            ExperimentRiskClass::TestnetEconomic,
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
