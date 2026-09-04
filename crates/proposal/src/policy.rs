//! Sandbox policy — the deterministic gate (AI → Proposal → Policy).
//!
//! [`decide`] is pure over the proposal: same input, same decision, every
//! time (no clock, no I/O, no randomness). v0.1 allow-list:
//!
//! - risk `Sandbox` with `None` commitment and sandbox actions → `Allow(Sandbox)`
//! - risk `ReadOnly` with `None` commitment and observe-only steps → `Allow(ReadOnly)`
//!
//! Everything else is a typed `Deny`: live risk class, any real
//! commitment (via the [`crate::economic`] seam), any economic action
//! (checked step-by-step even when the header claims sandbox), or an
//! action that exceeds the claimed risk class.

use crate::action::ProposedAction;
use crate::economic::EconomicAuthorization;
use crate::protocol::{ExperimentProposal, validate_proposal};
use crate::risk::{ExperimentRiskClass, ResourceCommitment};

use serde::{Deserialize, Serialize};

/// Execution lane granted by an Allow decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Full sandbox: observe + simulate + record.
    Sandbox,
    /// Observation only: observe steps, nothing else.
    ReadOnly,
}

/// Why a proposal was denied. Every reason is explicit and testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    /// Claimed risk class is live (`Economic`).
    EconomicRiskClass,
    /// A real commitment was requested; the seam refused it.
    EconomicCommitment {
        /// Which commitment was requested.
        commitment: ResourceCommitment,
        /// Seam refusal detail.
        detail: String,
    },
    /// Step `index` carries an economic action (never executable in v0.1).
    EconomicAction {
        /// Step index.
        index: usize,
        /// Action kind name.
        kind: &'static str,
    },
    /// A non-economic action exceeds the claimed risk class
    /// (e.g. `simulate` inside a `ReadOnly` proposal).
    ActionExceedsRiskClass {
        /// Step index.
        index: usize,
        /// Action kind name.
        kind: &'static str,
    },
    /// Structural problem (re-checked defensively; parse catches these first).
    InvalidStructure(String),
}

/// The policy verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Execute in the granted lane.
    Allow {
        /// Which lane.
        mode: ExecutionMode,
    },
    /// Do not execute. Reason is always explicit.
    Deny {
        /// Why.
        reason: DenyReason,
    },
}

impl PolicyDecision {
    /// True for Allow decisions.
    #[must_use]
    pub fn is_allow(self) -> bool {
        matches!(self, Self::Allow { .. })
    }
}

/// Deterministic gate: proposal + economic seam → verdict.
///
/// Order is load-bearing (cheapest, most structural checks first):
/// structure → risk class → commitment (via seam) → per-action scan.
pub fn decide(
    proposal: &ExperimentProposal,
    economic: &dyn EconomicAuthorization,
) -> PolicyDecision {
    if let Err(e) = validate_proposal(proposal) {
        return PolicyDecision::Deny {
            reason: DenyReason::InvalidStructure(e.to_string()),
        };
    }
    if proposal.risk.is_live() {
        return PolicyDecision::Deny {
            reason: DenyReason::EconomicRiskClass,
        };
    }
    if !proposal.commitment.is_none() {
        let detail = match economic.authorize_commitment(proposal.commitment, &proposal.id) {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        };
        return PolicyDecision::Deny {
            reason: DenyReason::EconomicCommitment {
                commitment: proposal.commitment,
                detail,
            },
        };
    }
    for (index, step) in proposal.steps.iter().enumerate() {
        if step.action.is_economic() {
            return PolicyDecision::Deny {
                reason: DenyReason::EconomicAction {
                    index,
                    kind: step.action.kind_name(),
                },
            };
        }
        if !risk_covers(proposal.risk, &step.action) {
            return PolicyDecision::Deny {
                reason: DenyReason::ActionExceedsRiskClass {
                    index,
                    kind: step.action.kind_name(),
                },
            };
        }
    }
    PolicyDecision::Allow {
        mode: match proposal.risk {
            ExperimentRiskClass::Sandbox => ExecutionMode::Sandbox,
            ExperimentRiskClass::ReadOnly => ExecutionMode::ReadOnly,
            // Live risk returned above; unreachable here.
            ExperimentRiskClass::Economic => {
                return PolicyDecision::Deny {
                    reason: DenyReason::EconomicRiskClass,
                };
            }
        },
    }
}

/// Sandbox lane covers every non-economic action; ReadOnly covers observe.
fn risk_covers(risk: ExperimentRiskClass, action: &ProposedAction) -> bool {
    match risk {
        ExperimentRiskClass::Sandbox => true,
        ExperimentRiskClass::ReadOnly => {
            matches!(action, ProposedAction::Observe { .. })
        }
        ExperimentRiskClass::Economic => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economic::DenyAllEconomicAuthorization;
    use crate::protocol::parse_proposal;

    fn sandbox() -> ExperimentProposal {
        parse_proposal(&crate::protocol::sandbox_proposal_json()).unwrap()
    }

    fn decision(p: &ExperimentProposal) -> PolicyDecision {
        decide(p, &DenyAllEconomicAuthorization)
    }

    #[test]
    fn sandbox_proposal_allowed_in_sandbox_lane() {
        assert_eq!(
            decision(&sandbox()),
            PolicyDecision::Allow {
                mode: ExecutionMode::Sandbox
            }
        );
    }

    #[test]
    fn economic_risk_denied() {
        let mut p = sandbox();
        p.risk = ExperimentRiskClass::Economic;
        assert_eq!(
            decision(&p),
            PolicyDecision::Deny {
                reason: DenyReason::EconomicRiskClass
            }
        );
    }

    #[test]
    fn every_real_commitment_denied_through_seam() {
        for c in [
            ResourceCommitment::Cr,
            ResourceCommitment::DCAI,
            ResourceCommitment::Escrow,
        ] {
            let mut p = sandbox();
            p.commitment = c;
            match decision(&p) {
                PolicyDecision::Deny {
                    reason: DenyReason::EconomicCommitment { commitment, .. },
                } => assert_eq!(commitment, c),
                other => panic!("expected EconomicCommitment deny, got {other:?}"),
            }
        }
    }

    #[test]
    fn economic_action_denied_even_under_sandbox_header() {
        let mut p = sandbox();
        p.steps[0].action = ProposedAction::FundTransfer {
            detail: "1 Cr to X".to_string(),
        };
        assert_eq!(
            decision(&p),
            PolicyDecision::Deny {
                reason: DenyReason::EconomicAction {
                    index: 0,
                    kind: "fund_transfer"
                }
            }
        );
    }

    #[test]
    fn simulate_exceeds_readonly_lane() {
        let mut p = sandbox();
        p.risk = ExperimentRiskClass::ReadOnly;
        assert_eq!(
            decision(&p),
            PolicyDecision::Deny {
                reason: DenyReason::ActionExceedsRiskClass {
                    index: 1,
                    kind: "simulate"
                }
            }
        );
    }

    #[test]
    fn readonly_observe_only_allowed_in_readonly_lane() {
        let mut p = sandbox();
        p.risk = ExperimentRiskClass::ReadOnly;
        p.steps.truncate(1);
        assert_eq!(
            decision(&p),
            PolicyDecision::Allow {
                mode: ExecutionMode::ReadOnly
            }
        );
    }
}
