//! Sandbox execution — Policy → Execution (never the live economy).
//!
//! [`execute`] runs ONLY steps a [`PolicyDecision::Allow`] already cleared,
//! and re-checks every action at the boundary: an economic action here is
//! a hard error ([`ProposalError::EconomicAtBoundary`]), unreachable through
//! policy but tested anyway. There is no code path from this module to Cr,
//! M18, DCAI, wallets, signers or any network.
//!
//! Measurements are deterministic simulations: `BLAKE3(proposal_id ‖
//! step_id ‖ scenario ‖ steps)` truncated to `u64`, forever labeled
//! `simulated: true`. Same proposal in, same numbers out — no randomness,
//! no hidden state.

use crate::action::ProposedAction;
use crate::error::ProposalError;
use crate::policy::{ExecutionMode, PolicyDecision};
use crate::protocol::ExperimentProposal;

/// One executed step's result (facts only, never prompts).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepResult {
    /// Step id from the proposal.
    pub step_id: String,
    /// Action kind name (owned for wire-safe canonical JSON).
    pub action_kind: String,
    /// Deterministic simulated measurement, when the action produces one.
    /// `(value, simulated)` — always `(v, true)` in v0.1.
    pub measurement: Option<(u64, bool)>,
    /// Short factual note (bounded by construction from bounded inputs).
    pub note: String,
}

/// The full execution report for one proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReport {
    /// Proposal that ran.
    pub proposal_id: String,
    /// Lane it ran in.
    pub mode: ExecutionMode,
    /// Per-step results, in proposal order.
    pub results: Vec<StepResult>,
    /// Execution time (unix ms, caller-provided; pure code takes no clock).
    pub completed_at_ms: u64,
}

/// Run an allowed proposal in its lane.
///
/// Refuses anything that is not an `Allow` decision, refuses lanes that
/// don't cover an action (defense in depth behind policy), and hard-stops
/// on economic actions at the boundary.
pub fn execute(
    proposal: &ExperimentProposal,
    decision: &PolicyDecision,
    completed_at_ms: u64,
) -> Result<ExecutionReport, ProposalError> {
    let mode = match decision {
        PolicyDecision::Allow { mode } => *mode,
        PolicyDecision::Deny { reason } => {
            return Err(ProposalError::ExecutionRefused(format!(
                "policy denied proposal {}: {reason:?}",
                proposal.id
            )));
        }
    };
    let mut results = Vec::with_capacity(proposal.steps.len());
    for step in &proposal.steps {
        if step.action.is_economic() {
            return Err(ProposalError::EconomicAtBoundary(format!(
                "step {} carries {}",
                step.id,
                step.action.kind_name()
            )));
        }
        if mode == ExecutionMode::ReadOnly && !matches!(step.action, ProposedAction::Observe { .. })
        {
            return Err(ProposalError::ExecutionRefused(format!(
                "read-only lane cannot run {} at step {}",
                step.action.kind_name(),
                step.id
            )));
        }
        results.push(run_step(&proposal.id, step)?);
    }
    Ok(ExecutionReport {
        proposal_id: proposal.id.clone(),
        mode,
        results,
        completed_at_ms,
    })
}

fn run_step(
    proposal_id: &str,
    step: &crate::protocol::ExperimentStep,
) -> Result<StepResult, ProposalError> {
    let (measurement, note) = match &step.action {
        ProposedAction::Observe { source, query } => {
            (None, format!("observed {source} for {query}"))
        }
        ProposedAction::Simulate { scenario, steps } => {
            let mut h = blake3::Hasher::new();
            h.update(proposal_id.as_bytes());
            h.update(step.id.as_bytes());
            h.update(scenario.as_bytes());
            h.update(&steps.to_le_bytes());
            let digest = h.finalize();
            let mut raw = [0u8; 8];
            raw.copy_from_slice(&digest.as_bytes()[..8]);
            (
                Some((u64::from_le_bytes(raw), true)),
                format!("simulated {scenario} over {steps} steps"),
            )
        }
        ProposedAction::RecordFinding { text } => (None, format!("recorded finding: {text}")),
        // Checked above; kept exhaustive so future kinds can't slip through.
        a => {
            return Err(ProposalError::EconomicAtBoundary(format!(
                "step {} carries {}",
                step.id,
                a.kind_name()
            )));
        }
    };
    Ok(StepResult {
        step_id: step.id.clone(),
        action_kind: step.action.kind_name().to_string(),
        measurement,
        note,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_780_000_000;
    use crate::economic::DenyAllEconomicAuthorization;
    use crate::policy::decide;
    use crate::protocol::parse_proposal;

    fn allowed_report() -> ExecutionReport {
        let p = parse_proposal(&crate::protocol::sandbox_proposal_json()).unwrap();
        let d = decide(&p, &DenyAllEconomicAuthorization, NOW);
        execute(&p, &d, 1_700_000_000_000).unwrap()
    }

    #[test]
    fn sandbox_run_produces_three_results_in_order() {
        let r = allowed_report();
        assert_eq!(r.mode, ExecutionMode::Sandbox);
        assert_eq!(r.results.len(), 3);
        assert_eq!(r.results[0].action_kind, "observe");
        assert_eq!(r.results[1].action_kind, "simulate");
        assert!(r.results[1].measurement.unwrap().1, "labeled simulated");
    }

    #[test]
    fn simulation_is_deterministic() {
        let a = allowed_report();
        let b = allowed_report();
        assert_eq!(a.results[1].measurement, b.results[1].measurement);
    }

    #[test]
    fn denied_decision_cannot_execute() {
        let p = parse_proposal(&crate::protocol::sandbox_proposal_json()).unwrap();
        let denied = PolicyDecision::Deny {
            reason: crate::policy::DenyReason::EconomicRiskClass,
        };
        assert!(matches!(
            execute(&p, &denied, 0),
            Err(ProposalError::ExecutionRefused(_))
        ));
    }

    #[test]
    fn economic_action_hard_stops_at_boundary() {
        let mut p = parse_proposal(&crate::protocol::sandbox_proposal_json()).unwrap();
        p.steps[0].action = ProposedAction::DCAIMint {
            detail: "mint 1".to_string(),
        };
        // Bypass policy on purpose: executor must still refuse.
        let forged = PolicyDecision::Allow {
            mode: ExecutionMode::Sandbox,
        };
        assert!(matches!(
            execute(&p, &forged, 0),
            Err(ProposalError::EconomicAtBoundary(_))
        ));
    }
}
