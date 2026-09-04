//! Policy — the deterministic gate (AI → Proposal → Policy).
//!
//! [`decide`] is pure over (proposal, clock): same inputs, same verdict,
//! every time (no I/O, no randomness; `now_unix` is a parameter).
//!
//! v0.1 lanes (unchanged): `Sandbox` / `ReadOnly` with `None` commitment.
//! v0.2 adds exactly one live lane: `TestnetEconomic` with a valid
//! [`ExperimentBudget`][crate::budget::ExperimentBudget], a matching
//! declared commitment, and testnet-only actions. `Economic` (the
//! mainnet-class lane) is denied always — there is no code path that
//! allows it.
//!
//! Order is load-bearing (cheapest, most structural checks first):
//! structure → risk class → commitment/budget → per-action scan.

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
    /// Bounded testnet execution. The Allow decision alone does NOT
    /// authorize value movement: the operator-side executor must also hold
    /// a [`crate::economic::TestnetApproval`] for the same proposal.
    Testnet,
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
    /// Testnet lane without a usable budget (absent or invalid).
    MissingBudget(String),
    /// Testnet lane without the matching declared commitment
    /// (xEGLD actions need `Cr`, DCAI actions need `DCAI`).
    CommitmentMismatch {
        /// Declared commitment.
        commitment: ResourceCommitment,
        /// Required commitment kind name.
        required: &'static str,
    },
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

/// Deterministic gate: proposal + economic seam + clock → verdict.
pub fn decide(
    proposal: &ExperimentProposal,
    economic: &dyn EconomicAuthorization,
    now_unix: u64,
) -> PolicyDecision {
    if let Err(e) = validate_proposal(proposal) {
        return PolicyDecision::Deny {
            reason: DenyReason::InvalidStructure(e.to_string()),
        };
    }
    match proposal.risk {
        ExperimentRiskClass::Economic => PolicyDecision::Deny {
            reason: DenyReason::EconomicRiskClass,
        },
        ExperimentRiskClass::TestnetEconomic => decide_testnet(proposal, now_unix),
        ExperimentRiskClass::Sandbox | ExperimentRiskClass::ReadOnly => {
            decide_offline(proposal, economic)
        }
    }
}

/// v0.1 lanes, unchanged: `None` commitment, no economic actions,
/// actions covered by the claimed lane.
fn decide_offline(
    proposal: &ExperimentProposal,
    economic: &dyn EconomicAuthorization,
) -> PolicyDecision {
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
    scan_actions(proposal)
}

/// v0.2 testnet lane: valid budget + matching commitment + testnet-only
/// actions within budget caps. Value authorization itself happens later
/// in [`crate::economic`] (per-request approval); policy grants the lane.
fn decide_testnet(proposal: &ExperimentProposal, now_unix: u64) -> PolicyDecision {
    let Some(budget) = &proposal.budget else {
        return PolicyDecision::Deny {
            reason: DenyReason::MissingBudget(
                "testnet lane requires an explicit budget".to_string(),
            ),
        };
    };
    if let Err(e) = budget.validate(now_unix) {
        return PolicyDecision::Deny {
            reason: DenyReason::MissingBudget(e.to_string()),
        };
    }
    if proposal.steps.len() > budget.max_actions as usize {
        return PolicyDecision::Deny {
            reason: DenyReason::ActionExceedsRiskClass {
                index: budget.max_actions as usize,
                kind: "budget_action_cap",
            },
        };
    }
    for (index, step) in proposal.steps.iter().enumerate() {
        match &step.action {
            ProposedAction::TestnetTransfer { asset, .. } => {
                let required = step.action.required_commitment();
                if proposal.commitment != required {
                    let need = match asset {
                        crate::budget::TestnetAsset::Xegld => "Cr",
                        crate::budget::TestnetAsset::Dcai => "DCAI",
                    };
                    return PolicyDecision::Deny {
                        reason: DenyReason::CommitmentMismatch {
                            commitment: proposal.commitment,
                            required: need,
                        },
                    };
                }
            }
            a if a.is_economic() => {
                return PolicyDecision::Deny {
                    reason: DenyReason::EconomicAction {
                        index,
                        kind: a.kind_name(),
                    },
                };
            }
            _ => {}
        }
    }
    PolicyDecision::Allow {
        mode: ExecutionMode::Testnet,
    }
}

/// Shared per-action scan for the offline lanes.
fn scan_actions(proposal: &ExperimentProposal) -> PolicyDecision {
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
            // Live risks never reach the offline scan.
            ExperimentRiskClass::Economic | ExperimentRiskClass::TestnetEconomic => {
                return PolicyDecision::Deny {
                    reason: DenyReason::EconomicRiskClass,
                };
            }
        },
    }
}

/// Sandbox lane covers every non-economic action; ReadOnly covers observe.
/// Live lanes never reach the offline scan (decide routes them first).
fn risk_covers(risk: ExperimentRiskClass, action: &ProposedAction) -> bool {
    match risk {
        ExperimentRiskClass::Sandbox => true,
        ExperimentRiskClass::ReadOnly => {
            matches!(action, ProposedAction::Observe { .. })
        }
        ExperimentRiskClass::Economic | ExperimentRiskClass::TestnetEconomic => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economic::DenyAllEconomicAuthorization;

    const NOW: u64 = 1_780_000_000;
    use crate::protocol::parse_proposal;

    fn sandbox() -> ExperimentProposal {
        parse_proposal(&crate::protocol::sandbox_proposal_json()).unwrap()
    }

    fn decision(p: &ExperimentProposal) -> PolicyDecision {
        decide(p, &DenyAllEconomicAuthorization, NOW)
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
