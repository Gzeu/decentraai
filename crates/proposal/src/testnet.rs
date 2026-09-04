//! Bounded testnet execution — Policy + Approval → one real effect.
//!
//! [`execute_testnet_experiment`] runs ONLY when it holds BOTH a policy
//! `Allow(Testnet)` AND a [`TestnetApproval`][crate::economic::TestnetApproval]
//! for the same proposal. Value movement itself happens through
//! [`TestnetExecutor`], implemented OUTSIDE the cognitive crate (no wallet,
//! no keys, no network here): tests use a counting mock, production uses
//! the operator-side executor that signs with the injected operator key.
//!
//! Idempotency: completed experiments return their cached report without
//! touching the executor again. Retries are counted against the budget.

use crate::action::ProposedAction;
use crate::budget::{ExperimentBudget, TESTNET_CHAIN_ID, TestnetAsset};
use crate::economic::{EconomicAuthError, TestnetApproval};
use crate::error::ProposalError;
use crate::policy::{ExecutionMode, PolicyDecision};
use crate::protocol::ExperimentProposal;
use crate::store::{AttemptInfo, ExperimentStore};

/// One authorized value movement, fully specified. The executor may not
/// add, split or redirect anything: exactly this asset, destination and
/// amount — or refuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedTransfer {
    /// Experiment this transfer belongs to.
    pub experiment_id: String,
    /// Proposal it was approved for.
    pub proposal_id: String,
    /// Budget backing it.
    pub budget_id: String,
    /// Asset to move.
    pub asset: TestnetAsset,
    /// Destination (budget-allowlisted).
    pub destination: String,
    /// Amount in wei (≤ budget).
    pub amount_wei: u64,
    /// Gas per action (≤ budget).
    pub gas: u64,
}

/// Operator-side value mover. Implementations hold the keys and the
/// network — this trait only describes the handoff. Mock it in tests.
pub trait TestnetExecutor {
    /// Execute exactly `intent`, returning the chain tx hash.
    /// Must be idempotent per `experiment_id` on the caller side
    /// (this function enforces it before calling).
    fn execute_transfer(&self, intent: &AuthorizedTransfer) -> Result<String, ProposalError>;
}

/// What one bounded execution produced (facts for evidence).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TestnetReport {
    /// Experiment id (idempotency key).
    pub experiment_id: String,
    /// Proposal id.
    pub proposal_id: String,
    /// Chain tx hash (external verifiable evidence).
    pub tx_hash: String,
    /// Asset moved.
    pub asset: TestnetAsset,
    /// Amount moved (wei).
    pub amount_wei: u64,
    /// Destination.
    pub destination: String,
    /// Chain (always testnet).
    pub chain_id: String,
    /// Attempts used including this one.
    pub attempts_used: u32,
    /// Execution time (unix ms, caller-provided).
    pub completed_at_ms: u64,
    /// True when this call did NOT touch the executor (cached replay).
    pub replayed: bool,
}

/// Run one bounded testnet experiment end-to-end (pure orchestration).
///
/// Order: idempotency → lane → approval binding → uniformity → budget
/// totals → retry budget → execute → record. Any failure denies without
/// side effects (the executor is called at most once per experiment).
#[allow(clippy::too_many_arguments)]
pub fn execute_testnet_experiment(
    experiment_id: &str,
    proposal: &ExperimentProposal,
    decision: &PolicyDecision,
    approval: &TestnetApproval,
    budget: &ExperimentBudget,
    now_ms: u64,
    store: &mut ExperimentStore,
    executor: &dyn TestnetExecutor,
) -> Result<TestnetReport, ProposalError> {
    // 1. Idempotency: a recorded submission replays its report, no re-execution.
    if let Some(((asset, amount_wei, destination, attempts_used), tx_hash)) =
        store.get(experiment_id).and_then(|rec| {
            rec.tx_hash.clone().map(|tx_hash| {
                (
                    (
                        rec.asset.clone(),
                        rec.amount_wei,
                        rec.destination.clone(),
                        rec.attempts_used,
                    ),
                    tx_hash,
                )
            })
        })
    {
        return Ok(TestnetReport {
            experiment_id: experiment_id.to_string(),
            proposal_id: proposal.id.clone(),
            tx_hash,
            asset,
            amount_wei,
            destination,
            chain_id: TESTNET_CHAIN_ID.to_string(),
            attempts_used,
            completed_at_ms: now_ms,
            replayed: true,
        });
    }
    // 2. Lane must be the testnet Allow.
    if *decision
        != (PolicyDecision::Allow {
            mode: ExecutionMode::Testnet,
        })
    {
        return Err(ProposalError::ExecutionRefused(format!(
            "experiment {experiment_id}: requires Allow(Testnet), got {decision:?}"
        )));
    }
    // 3. Approval must bind THIS proposal + budget.
    if approval.proposal_id != proposal.id || approval.budget_id != budget.id {
        return Err(ProposalError::ExecutionRefused(format!(
            "experiment {experiment_id}: approval binds {}/{} — proposal is {}/{}",
            approval.proposal_id, approval.budget_id, proposal.id, budget.id
        )));
    }
    // 4. Uniformity: every transfer step shares one asset+destination,
    //    totals fit the approval. Mixed targets cannot be one intent.
    let (asset, destination, total_wei) = transfer_totals(proposal)?;
    if asset != approval.asset
        || destination != approval.destination
        || total_wei != approval.amount_wei
    {
        return Err(ProposalError::ExecutionRefused(format!(
            "experiment {experiment_id}: intent drift vs approval"
        )));
    }
    // 5. Retry budget (attempts already used live in the store).
    let attempts_used = store.get(experiment_id).map_or(0, |r| r.attempts_used);
    if attempts_used > budget.max_retries {
        return Err(EconomicAuthError::RetryBudgetExceeded {
            attempts_used,
            max_retries: budget.max_retries,
        }
        .into());
    }
    // 6. Record the attempt BEFORE executing (crash-safe accounting).
    store.record_attempt(
        experiment_id,
        AttemptInfo {
            proposal,
            budget_id: &budget.id,
            asset: &asset,
            destination: &destination,
            amount_wei: total_wei,
            attempts_used: attempts_used.saturating_add(1),
            now_ms,
        },
    );
    let intent = AuthorizedTransfer {
        experiment_id: experiment_id.to_string(),
        proposal_id: proposal.id.clone(),
        budget_id: budget.id.clone(),
        asset,
        destination,
        amount_wei: total_wei,
        gas: approval.gas,
    };
    match executor.execute_transfer(&intent) {
        Ok(tx_hash) => {
            store.mark_submitted(experiment_id, &tx_hash, now_ms);
            Ok(TestnetReport {
                experiment_id: experiment_id.to_string(),
                proposal_id: proposal.id.clone(),
                tx_hash,
                asset: intent.asset,
                amount_wei: intent.amount_wei,
                destination: intent.destination,
                chain_id: TESTNET_CHAIN_ID.to_string(),
                attempts_used: attempts_used.saturating_add(1),
                completed_at_ms: now_ms,
                replayed: false,
            })
        }
        Err(e) => {
            store.mark_failed(experiment_id, &e.to_string(), now_ms);
            Err(e)
        }
    }
}

/// Sum transfer steps: one shared asset+destination required.
/// Public so the operator-side executor builds the identical intent.
pub fn transfer_totals(
    proposal: &ExperimentProposal,
) -> Result<(TestnetAsset, String, u64), ProposalError> {
    let mut asset: Option<TestnetAsset> = None;
    let mut destination: Option<String> = None;
    let mut total: u64 = 0;
    let mut count = 0u32;
    for step in &proposal.steps {
        if let ProposedAction::TestnetTransfer {
            asset: a,
            destination: d,
            amount_wei,
        } = &step.action
        {
            match &asset {
                None => asset = Some(a.clone()),
                Some(first) if first != a => {
                    return Err(ProposalError::ExecutionRefused(format!(
                        "step {}: mixed assets in one experiment",
                        step.id
                    )));
                }
                _ => {}
            }
            match &destination {
                None => destination = Some(d.clone()),
                Some(first) if first != d => {
                    return Err(ProposalError::ExecutionRefused(format!(
                        "step {}: mixed destinations in one experiment",
                        step.id
                    )));
                }
                _ => {}
            }
            total = total.saturating_add(*amount_wei);
            count += 1;
        }
    }
    if count == 0 {
        return Err(ProposalError::ExecutionRefused(
            "testnet lane with no transfer steps".to_string(),
        ));
    }
    Ok((
        asset.unwrap_or(TestnetAsset::Xegld),
        destination.unwrap_or_default(),
        total,
    ))
}
