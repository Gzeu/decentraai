//! The experiment action vocabulary (closed set).
//!
//! Three action kinds are executable in the v0.1 sandbox; four economic
//! kinds exist ONLY to be denied — naming them explicitly is what makes
//! `EconomicStateMutation → DENIED`, `FundTransfer → DENIED`,
//! `SignerChange → DENIED`, `DCAIMint → DENIED` a typed guarantee rather
//! than a missing-feature accident.

use serde::{Deserialize, Serialize};

use crate::risk::{ExperimentRiskClass, ResourceCommitment};

/// One step's action. Closed schema: unknown `kind` values fail parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProposedAction {
    /// Read a live observation source without side effects.
    Observe {
        /// Where to read from (dataset, endpoint label — never executed).
        source: String,
        /// What to look at.
        query: String,
    },
    /// Run a bounded simulation inside the sandbox.
    Simulate {
        /// Scenario label.
        scenario: String,
        /// Simulation steps (bounded by [`crate::protocol::MAX_SIM_STEPS`]).
        steps: u32,
    },
    /// Record a finding into the sandbox-local report.
    RecordFinding {
        /// Finding text (bounded).
        text: String,
    },
    /// Mutate live/economic state. DENIED in v0.1.
    EconomicStateMutation {
        /// Declared target (recorded in the denial, never touched).
        detail: String,
    },
    /// Move funds. DENIED in v0.1.
    FundTransfer {
        /// Declared transfer (recorded in the denial, never executed).
        detail: String,
    },
    /// Change an authority signer. DENIED in v0.1.
    SignerChange {
        /// Declared change (recorded in the denial, never applied).
        detail: String,
    },
    /// Mint DCAI. DENIED in v0.1.
    /// (Explicit rename: `snake_case` would render this `d_c_a_i_mint`.)
    #[serde(rename = "dcai_mint")]
    DCAIMint {
        /// Declared mint (recorded in the denial, never executed).
        detail: String,
    },
}

impl ProposedAction {
    /// Machine-readable kind name (stable strings for evidence/denials).
    #[must_use]
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Observe { .. } => "observe",
            Self::Simulate { .. } => "simulate",
            Self::RecordFinding { .. } => "record_finding",
            Self::EconomicStateMutation { .. } => "economic_state_mutation",
            Self::FundTransfer { .. } => "fund_transfer",
            Self::SignerChange { .. } => "signer_change",
            Self::DCAIMint { .. } => "dcai_mint",
        }
    }

    /// True for the four economic kinds. The policy AND the executor
    /// boundary both check this — two independent gates, same predicate.
    #[must_use]
    pub fn is_economic(&self) -> bool {
        matches!(
            self,
            Self::EconomicStateMutation { .. }
                | Self::FundTransfer { .. }
                | Self::SignerChange { .. }
                | Self::DCAIMint { .. }
        )
    }

    /// Minimum risk class that may carry this action.
    #[must_use]
    pub fn required_risk(&self) -> ExperimentRiskClass {
        if self.is_economic() {
            ExperimentRiskClass::Economic
        } else {
            match self {
                Self::Observe { .. } => ExperimentRiskClass::ReadOnly,
                Self::Simulate { .. } | Self::RecordFinding { .. } => ExperimentRiskClass::Sandbox,
                // Economic arms covered above; unreachable here.
                _ => ExperimentRiskClass::Economic,
            }
        }
    }

    /// Minimum commitment this action needs. Sandbox actions need none;
    /// economic actions need a real commitment (which v0.1 never grants).
    #[must_use]
    pub fn required_commitment(&self) -> ResourceCommitment {
        if self.is_economic() {
            // Economic actions cannot run on `None`; the exact commitment
            // kind is the proposer's declaration, checked by policy.
            ResourceCommitment::Cr
        } else {
            ResourceCommitment::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observe() -> ProposedAction {
        ProposedAction::Observe {
            source: "world".to_string(),
            query: "tick".to_string(),
        }
    }

    #[test]
    fn economic_kinds_are_exactly_four() {
        let economics = [
            ProposedAction::EconomicStateMutation {
                detail: "x".to_string(),
            },
            ProposedAction::FundTransfer {
                detail: "x".to_string(),
            },
            ProposedAction::SignerChange {
                detail: "x".to_string(),
            },
            ProposedAction::DCAIMint {
                detail: "x".to_string(),
            },
        ];
        for a in &economics {
            assert!(a.is_economic(), "{}", a.kind_name());
            assert_eq!(a.required_risk(), ExperimentRiskClass::Economic);
        }
        assert!(!observe().is_economic());
        assert_eq!(observe().required_risk(), ExperimentRiskClass::ReadOnly);
        assert_eq!(
            ProposedAction::Simulate {
                scenario: "s".to_string(),
                steps: 1
            }
            .required_risk(),
            ExperimentRiskClass::Sandbox
        );
    }

    #[test]
    fn unknown_kind_fails_closed_schema() {
        let bad = r#"{"kind":"launch_missiles","detail":"x"}"#;
        let err = serde_json::from_str::<ProposedAction>(bad).unwrap_err();
        assert!(err.to_string().contains("unknown variant"));
    }
}
