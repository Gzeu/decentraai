//! Cognitive protocol: the untrusted-input boundary and the chain shapes.
//!
//! Chain: `Observation → ResearchQuestion → Hypothesis → AgentIdea →`
//! `ExperimentProposal(steps)`. Everything here is data + validation;
//! decisions live in [`crate::policy`], execution in [`crate::sandbox`].
//!
//! [`parse_proposal`] is the ONLY entry point for AI-generated JSON:
//! closed schema (`deny_unknown_fields` at every level) plus hard bounds,
//! so malformed or hostile proposals become `Err`, never partial state.

use serde::{Deserialize, Serialize};

use crate::action::ProposedAction;
use crate::budget::{ExperimentBudget, MAX_DESTINATION_LEN};
use crate::error::ProposalError;
use crate::risk::{ExperimentRiskClass, ResourceCommitment};

/// Schema version. Bump on any wire-shape change; v0.1 speaks version 1.
pub const PROTOCOL_VERSION: u32 = 1;
/// Max experiment steps per proposal.
pub const MAX_STEPS: usize = 16;
/// Max id length (observation / question / hypothesis / idea / proposal / step).
pub const MAX_ID_LEN: usize = 128;
/// Max free-text length (observation / question / hypothesis / summary / finding).
pub const MAX_TEXT_LEN: usize = 4096;
/// Max source/scenario/rationale length.
pub const MAX_LABEL_LEN: usize = 256;
/// Max simulation steps inside one `Simulate` action.
pub const MAX_SIM_STEPS: u32 = 1_000;

fn check_id(field: &str, v: &str) -> Result<(), ProposalError> {
    if v.is_empty() || v.len() > MAX_ID_LEN {
        return Err(ProposalError::Bound(format!(
            "{field}: id length must be 1..={MAX_ID_LEN}"
        )));
    }
    Ok(())
}

fn check_text(field: &str, v: &str) -> Result<(), ProposalError> {
    if v.is_empty() || v.len() > MAX_TEXT_LEN {
        return Err(ProposalError::Bound(format!(
            "{field}: text length must be 1..={MAX_TEXT_LEN}"
        )));
    }
    Ok(())
}

fn check_label(field: &str, v: &str) -> Result<(), ProposalError> {
    if v.is_empty() || v.len() > MAX_LABEL_LEN {
        return Err(ProposalError::Bound(format!(
            "{field}: label length must be 1..={MAX_LABEL_LEN}"
        )));
    }
    Ok(())
}

/// A recorded fact about the world. Facts only — never prompts or outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    /// Stable id (e.g. `obs:tick-218-supply`).
    pub id: String,
    /// The fact text.
    pub text: String,
    /// Where it was observed (label only in v0.1 — no live fetch here).
    pub source: String,
    /// Observation time (unix ms, caller-provided; pure code takes no clock).
    pub observed_at_ms: u64,
}

/// A question raised by an observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchQuestion {
    /// Stable id.
    pub id: String,
    /// The question text.
    pub text: String,
    /// Observation that raised it.
    pub observation_id: String,
}

/// A testable statement answering a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hypothesis {
    /// Stable id.
    pub id: String,
    /// The statement text.
    pub text: String,
    /// Question it answers.
    pub question_id: String,
}

/// An agent's idea: how to test a hypothesis with an experiment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdea {
    /// Stable id.
    pub id: String,
    /// Hypothesis under test.
    pub hypothesis_id: String,
    /// One-paragraph summary of the planned test.
    pub summary: String,
    /// Proposing agent (label).
    pub proposer: String,
}

/// One experiment step: an action plus its rationale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentStep {
    /// Step id unique within the proposal.
    pub id: String,
    /// What to do.
    pub action: ProposedAction,
    /// Why this step tests the hypothesis.
    pub rationale: String,
}

/// The proposal: the full test plan submitted to policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentProposal {
    /// Schema version (must equal [`PROTOCOL_VERSION`]).
    pub version: u32,
    /// Stable id.
    pub id: String,
    /// Idea under test.
    pub idea_id: String,
    /// Claimed blast radius.
    pub risk: ExperimentRiskClass,
    /// Claimed economic commitment.
    pub commitment: ResourceCommitment,
    /// Explicit budget for the `TestnetEconomic` lane. Absent (`None`) for
    /// sandbox/read-only proposals; REQUIRED for testnet proposals.
    /// `#[serde(default)]` keeps v0.1 JSON parsing unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<ExperimentBudget>,
    /// Ordered steps, 1..=[`MAX_STEPS`].
    pub steps: Vec<ExperimentStep>,
    /// Proposing agent (label).
    pub created_by: String,
}

/// Parse and validate untrusted proposal JSON (the AI-output boundary).
///
/// Closed schema first (serde), then structural bounds. Returns the
/// validated proposal ready for [`crate::policy::decide`].
pub fn parse_proposal(json: &str) -> Result<ExperimentProposal, ProposalError> {
    let p: ExperimentProposal =
        serde_json::from_str(json).map_err(|e| ProposalError::Parse(e.to_string()))?;
    validate_proposal(&p)?;
    Ok(p)
}

/// Re-validate an already-deserialized proposal (defense in depth:
/// policy calls this too, so a constructed value can't skip bounds).
pub fn validate_proposal(p: &ExperimentProposal) -> Result<(), ProposalError> {
    if p.version != PROTOCOL_VERSION {
        return Err(ProposalError::Bound(format!(
            "version: expected {PROTOCOL_VERSION}, got {}",
            p.version
        )));
    }
    check_id("proposal.id", &p.id)?;
    check_id("proposal.idea_id", &p.idea_id)?;
    check_label("proposal.created_by", &p.created_by)?;
    if p.steps.is_empty() || p.steps.len() > MAX_STEPS {
        return Err(ProposalError::Bound(format!(
            "steps: count must be 1..={MAX_STEPS}, got {}",
            p.steps.len()
        )));
    }
    for (i, s) in p.steps.iter().enumerate() {
        check_id(&format!("steps[{i}].id"), &s.id)?;
        check_label(&format!("steps[{i}].rationale"), &s.rationale)?;
        validate_action(&s.action, i)?;
    }
    Ok(())
}

fn validate_action(a: &ProposedAction, i: usize) -> Result<(), ProposalError> {
    let tag = format!("steps[{i}].action");
    match a {
        ProposedAction::Observe { source, query } => {
            check_label(&format!("{tag}.source"), source)?;
            check_text(&format!("{tag}.query"), query)?;
        }
        ProposedAction::Simulate { scenario, steps } => {
            check_label(&format!("{tag}.scenario"), scenario)?;
            if *steps == 0 || *steps > MAX_SIM_STEPS {
                return Err(ProposalError::Bound(format!(
                    "{tag}.steps: must be 1..={MAX_SIM_STEPS}, got {steps}"
                )));
            }
        }
        ProposedAction::RecordFinding { text } => {
            check_text(&format!("{tag}.text"), text)?;
        }
        ProposedAction::EconomicStateMutation { detail }
        | ProposedAction::FundTransfer { detail }
        | ProposedAction::SignerChange { detail }
        | ProposedAction::DCAIMint { detail } => {
            // Economic actions are structurally valid here so the denial is
            // explicit and typed at the policy layer (never a parse error).
            check_text(&format!("{tag}.detail"), detail)?;
        }
        ProposedAction::TestnetTransfer {
            destination,
            amount_wei,
            ..
        } => {
            // Same rule: structurally valid, denied/allows decided by policy
            // against the budget. Bounds still enforced here.
            if destination.is_empty() || destination.len() > MAX_DESTINATION_LEN {
                return Err(ProposalError::Bound(format!(
                    "{tag}.destination: length must be 1..={MAX_DESTINATION_LEN}"
                )));
            }
            if *amount_wei == 0 {
                return Err(ProposalError::Bound(format!(
                    "{tag}.amount_wei: must be >= 1"
                )));
            }
        }
    }
    Ok(())
}

/// Shared sandbox proposal fixture for crate-wide tests.
#[cfg(test)]
pub(crate) fn sandbox_proposal_json() -> String {
    tests::sandbox_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sandbox_json() -> String {
        serde_json::json!({
            "version": 1,
            "id": "prop:supply-drift-1",
            "idea_id": "idea:supply-drift-1",
            "risk": "sandbox",
            "commitment": "none",
            "created_by": "agent:observer-1",
            "steps": [
                {"id": "s1", "rationale": "read last supply",
                 "action": {"kind": "observe", "source": "world", "query": "treasury supply"}},
                {"id": "s2", "rationale": "simulate drift",
                 "action": {"kind": "simulate", "scenario": "supply-drift", "steps": 10}},
                {"id": "s3", "rationale": "record outcome",
                 "action": {"kind": "record_finding", "text": "drift within band"}}
            ]
        })
        .to_string()
    }

    #[test]
    fn valid_sandbox_proposal_parses() {
        let p = parse_proposal(&sandbox_proposal_json()).unwrap();
        assert_eq!(p.steps.len(), 3);
        assert_eq!(p.risk, ExperimentRiskClass::Sandbox);
    }

    #[test]
    fn unknown_field_rejected() {
        let mut v: serde_json::Value = serde_json::from_str(&sandbox_proposal_json()).unwrap();
        v["smuggle"] = serde_json::json!("/drop/table");
        let err = parse_proposal(&v.to_string()).unwrap_err();
        assert!(matches!(err, ProposalError::Parse(_)));
    }

    #[test]
    fn wrong_version_rejected() {
        let mut v: serde_json::Value = serde_json::from_str(&sandbox_proposal_json()).unwrap();
        v["version"] = serde_json::json!(99);
        assert!(matches!(
            parse_proposal(&v.to_string()).unwrap_err(),
            ProposalError::Bound(_)
        ));
    }

    #[test]
    fn empty_steps_rejected() {
        let mut v: serde_json::Value = serde_json::from_str(&sandbox_proposal_json()).unwrap();
        v["steps"] = serde_json::json!([]);
        assert!(matches!(
            parse_proposal(&v.to_string()).unwrap_err(),
            ProposalError::Bound(_)
        ));
    }

    #[test]
    fn oversized_text_rejected() {
        let mut v: serde_json::Value = serde_json::from_str(&sandbox_proposal_json()).unwrap();
        v["steps"][2]["action"]["text"] = serde_json::json!("x".repeat(MAX_TEXT_LEN + 1));
        assert!(matches!(
            parse_proposal(&v.to_string()).unwrap_err(),
            ProposalError::Bound(_)
        ));
    }
}
