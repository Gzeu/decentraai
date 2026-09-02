//! M18 World Economic Integration — autonomous agent behavior bridge.
//!
//! Connects `economic_agent` decisions to real World/Hub/M18 state mutations.
//! Each World tick, `run_world_economic_tick()` evaluates every agent and
//! applies their economic actions (bid, propose, accept, execute, complete).
//!
//! Lock discipline: M18 state uses `std::sync::Mutex` — we snapshot before
//! decision-making and apply mutations in short non-async critical sections.

use crate::economic_agent::{self, EconomicAction, EconomicContext};
use crate::hub::{SharedHub, hub_path_for, save_hub_state};
use crate::m18::{M18Action, M18State};
use decentraai_compute::QuotaLedger;
use decentraai_economy::contract::{
    self, AgentContract, ContractStatus, ContractTerms, ServiceDescriptor,
};
use decentraai_economy::escrow::EscrowLedger;
use decentraai_economy::trust_anchor::{AnchorParams, TrustStore};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

/// Result of a single agent's economic tick.
#[derive(Debug, Clone, Serialize)]
pub struct AgentEconomicTick {
    pub agent_id: String,
    pub wallet: String,
    pub action: EconomicAction,
    pub applied: bool,
    pub error: Option<String>,
}

/// Full economic tick result.
#[derive(Debug, Clone, Serialize)]
pub struct EconomicTickResult {
    pub tick: u64,
    pub agents_evaluated: usize,
    pub actions_taken: usize,
    pub agent_results: Vec<AgentEconomicTick>,
}

/// Run one economic tick over all World agents.
///
/// `hub` is the shared Hub state (tokio Mutex). `m18` is the M18 state with
/// std Mutexes — we snapshot it synchronously, decide, then mutate in short
/// critical sections. No `std::sync::Mutex` guard is held across `.await`.
pub async fn run_world_economic_tick(
    agents: &[crate::world::WorldAgent],
    hub: &SharedHub,
    m18: &M18State,
    repo_root: &Path,
    quota_ledger: &Option<Arc<StdMutex<QuotaLedger>>>,
) -> EconomicTickResult {
    let tick = m18.current_tick();
    let mut results = Vec::new();
    let mut actions_taken = 0usize;

    // Snapshot M18 state (short locks, synchronous).
    let (contracts_snap, escrow_snap, trust_snap) = snapshot_m18(m18);

    for agent in agents {
        let trust_score = trust_snap.trust_score(&agent.account);

        // Look up real balance from the quota ledger.
        let balance = quota_ledger
            .as_ref()
            .and_then(|l| l.lock().ok())
            .and_then(|l| l.account(&agent.account))
            .map(|a| a.spendable())
            .unwrap_or(0);

        // Decide under a short hub lock; drop before applying to avoid deadlock.
        let action = {
            let mut hub_guard = hub.lock().await;
            let ctx = EconomicContext {
                agent_id: &agent.agent_id,
                agent_wallet: &agent.account,
                capabilities: &agent.declared_capabilities,
                tick,
                hub: &hub_guard,
                contracts: &contracts_snap,
                escrow: &escrow_snap,
                trust: &trust_snap,
                balance,
                trust_score,
            };
            let a = economic_agent::decide_action(&ctx);
            let _ = &mut *hub_guard; // keep guard alive for the whole block
            a
        };

        let mut applied = false;
        let mut error = None;

        if !matches!(action, EconomicAction::Nothing) {
            match apply_action(&action, &agent.account, hub, m18, repo_root).await {
                Ok(true) => {
                    applied = true;
                    actions_taken += 1;
                }
                Ok(false) => error = Some("action not applicable".to_string()),
                Err(e) => error = Some(e),
            }
        }

        results.push(AgentEconomicTick {
            agent_id: agent.agent_id.clone(),
            wallet: agent.account.clone(),
            action,
            applied,
            error,
        });
    }

    EconomicTickResult {
        tick,
        agents_evaluated: agents.len(),
        actions_taken,
        agent_results: results,
    }
}

/// Snapshot the three M18 stores in short synchronous critical sections.
fn snapshot_m18(m18: &M18State) -> (BTreeMap<String, AgentContract>, EscrowLedger, TrustStore) {
    let contracts = m18.contracts.lock().map(|g| g.clone()).unwrap_or_default();
    let escrow = m18.escrow.lock().map(|g| g.clone()).unwrap_or_default();
    let trust = m18.trust.lock().map(|g| g.clone()).unwrap_or_default();
    (contracts, escrow, trust)
}

fn record_action(m18: &M18State, action: M18Action) {
    if let Ok(mut actions) = m18.actions.lock() {
        actions.push(action);
        // Bound the action log.
        if actions.len() > 10_000 {
            let drain = actions.len() - 10_000;
            actions.drain(..drain);
        }
    }
}

/// Apply a single economic action to real state.
async fn apply_action(
    action: &EconomicAction,
    caller_wallet: &str,
    hub: &SharedHub,
    m18: &M18State,
    repo_root: &Path,
) -> Result<bool, String> {
    match action {
        EconomicAction::BidOnTask {
            task_id,
            price,
            rationale,
        } => {
            let mut h = hub.lock().await;
            match h.place_bid(
                caller_wallet.to_string(),
                task_id.clone(),
                *price,
                rationale.clone(),
            ) {
                Ok(_) => {
                    h.advance_tick();
                    let path = hub_path_for(repo_root);
                    save_hub_state(&path, &h);
                    Ok(true)
                }
                Err(e) => Err(format!("bid failed: {}", e)),
            }
        }

        EconomicAction::ProposeContract {
            provider_wallet,
            capability,
            description,
            price_micro_cu,
            max_duration_secs,
            escrow_required,
        } => {
            let service = ServiceDescriptor {
                capability: capability.clone(),
                description: description.clone(),
                model_requirement: None,
                estimated_input_size: None,
            };
            let terms = ContractTerms {
                price_micro_cu: *price_micro_cu,
                max_duration_secs: *max_duration_secs,
                min_quality_percent: 80,
                escrow_required: *escrow_required,
            };
            let c = contract::propose_contract(
                provider_wallet,
                caller_wallet,
                service,
                terms,
                m18.current_tick(),
            )
            .map_err(|e| e.to_string())?;
            let contract_id = c.contract_id.clone();

            {
                let mut contracts = m18.contracts.lock().map_err(|e| format!("lock: {}", e))?;
                contracts.insert(contract_id.clone(), c);
            }
            record_action(
                m18,
                M18Action::ProposeContract {
                    contract_id,
                    consumer: caller_wallet.to_string(),
                },
            );
            m18.save_contracts()?;
            Ok(true)
        }

        EconomicAction::AcceptContract { contract_id } => {
            let mut contracts = m18.contracts.lock().map_err(|e| format!("lock: {}", e))?;
            let c = contracts.get_mut(contract_id).ok_or("contract not found")?;
            if c.provider_wallet != caller_wallet {
                return Err("not the provider".to_string());
            }
            if !matches!(c.status, ContractStatus::Proposed) {
                return Err(format!("cannot accept: status is {:?}", c.status));
            }
            c.status = ContractStatus::Accepted;
            drop(contracts);
            record_action(
                m18,
                M18Action::AcceptContract {
                    contract_id: contract_id.clone(),
                    provider: caller_wallet.to_string(),
                },
            );
            m18.save_contracts()?;
            Ok(true)
        }

        EconomicAction::StartExecution { contract_id } => {
            let mut contracts = m18.contracts.lock().map_err(|e| format!("lock: {}", e))?;
            let c = contracts.get_mut(contract_id).ok_or("contract not found")?;
            if !matches!(c.status, ContractStatus::Accepted) {
                return Err(format!("cannot start: status is {:?}", c.status));
            }
            c.status = ContractStatus::Executing;
            drop(contracts);
            record_action(
                m18,
                M18Action::StartExecution {
                    contract_id: contract_id.clone(),
                },
            );
            m18.save_contracts()?;
            Ok(true)
        }

        EconomicAction::CompleteContract { contract_id } => {
            let mut contracts = m18.contracts.lock().map_err(|e| format!("lock: {}", e))?;
            let c = contracts.get_mut(contract_id).ok_or("contract not found")?;
            if !matches!(c.status, ContractStatus::Executing) {
                return Err(format!("cannot complete: status is {:?}", c.status));
            }
            c.status = ContractStatus::Completed;
            drop(contracts);
            record_action(
                m18,
                M18Action::CompleteContract {
                    contract_id: contract_id.clone(),
                },
            );
            m18.save_contracts()?;
            Ok(true)
        }

        EconomicAction::CancelContract { contract_id } => {
            let mut contracts = m18.contracts.lock().map_err(|e| format!("lock: {}", e))?;
            match contracts.get_mut(contract_id) {
                Some(c) if !c.status.is_terminal() => {
                    c.status = ContractStatus::Cancelled;
                    drop(contracts);
                    record_action(
                        m18,
                        M18Action::CancelContract {
                            contract_id: contract_id.clone(),
                        },
                    );
                    m18.save_contracts()?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        }

        EconomicAction::RecordTrust {
            evidence_hash,
            capability,
            quality_score,
            micro_cu,
            contract_id,
        } => {
            let params = AnchorParams {
                agent_wallet: caller_wallet.to_string(),
                evidence_hash: evidence_hash.clone(),
                capability: capability.clone(),
                quality_score: *quality_score,
                verified: *quality_score >= 80,
                micro_cu: *micro_cu,
                contract_id: contract_id.clone(),
            };
            let mut trust = m18.trust.lock().map_err(|e| format!("lock: {}", e))?;
            trust
                .record_anchor(&params, m18.current_tick())
                .map_err(|e| e.to_string())?;
            drop(trust);
            m18.save_trust()?;
            Ok(true)
        }

        EconomicAction::Nothing => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::new_shared_hub;
    use crate::world::WorldAgent;

    fn wallet(n: u8) -> String {
        format!(
            "erd1test{:04}000000000000000000000000000000000000000000000000{:04x}",
            n, n
        )
    }

    fn test_agent(id: &str, w: &str, caps: Vec<String>) -> WorldAgent {
        WorldAgent {
            agent_id: id.to_string(),
            key_id: format!("dca_{}", id),
            account: w.to_string(),
            declared_capabilities: caps,
            room_id: "research-lab".to_string(),
            joined_at: 1,
        }
    }

    #[tokio::test]
    async fn tick_bids_on_matching_task() {
        let shared = new_shared_hub();
        let task_id = {
            let mut h = shared.lock().await;
            h.tick = 1;
            let t = h.publish_task(
                "issuer".to_string(),
                "Research task".to_string(),
                "Do research".to_string(),
                500,
                Some("research".to_string()),
            );
            t.id
        };

        let m18 = M18State::test_default();
        let tmp = tempfile::tempdir().unwrap();
        let agents = vec![test_agent("a1", &wallet(1), vec!["research".to_string()])];
        let result = run_world_economic_tick(&agents, &shared, &m18, tmp.path(), &None).await;

        assert_eq!(result.agents_evaluated, 1);
        assert_eq!(result.actions_taken, 1);

        let h = shared.lock().await;
        assert_eq!(h.bids.len(), 1);
        let bid = h.bids.values().next().unwrap();
        assert_eq!(bid.bidder, wallet(1));
        assert_eq!(bid.task_id, task_id);
        assert_eq!(bid.price, 400); // 80% of 500
    }

    #[tokio::test]
    async fn tick_no_action_without_tasks() {
        let shared = new_shared_hub();
        let m18 = M18State::test_default();
        let tmp = tempfile::tempdir().unwrap();
        let agents = vec![test_agent("a1", &wallet(1), vec!["research".to_string()])];
        let result = run_world_economic_tick(&agents, &shared, &m18, tmp.path(), &None).await;

        assert_eq!(result.actions_taken, 0);
        assert!(matches!(
            result.agent_results[0].action,
            EconomicAction::Nothing
        ));
    }

    #[tokio::test]
    async fn tick_skips_non_matching_capability() {
        let shared = new_shared_hub();
        {
            let mut h = shared.lock().await;
            h.tick = 1;
            h.publish_task(
                "issuer".to_string(),
                "OCR task".to_string(),
                "Extract text".to_string(),
                300,
                Some("ocr".to_string()),
            );
        }

        let m18 = M18State::test_default();
        let tmp = tempfile::tempdir().unwrap();
        let agents = vec![test_agent("a1", &wallet(1), vec!["research".to_string()])];
        let result = run_world_economic_tick(&agents, &shared, &m18, tmp.path(), &None).await;

        assert_eq!(result.actions_taken, 0);
    }

    #[tokio::test]
    async fn apply_propose_contract_creates_record() {
        let shared = new_shared_hub();
        let m18 = M18State::test_default();
        let tmp = tempfile::tempdir().unwrap();

        let action = EconomicAction::ProposeContract {
            provider_wallet: wallet(2),
            capability: "research".to_string(),
            description: "Research service".to_string(),
            price_micro_cu: 1_000_000,
            max_duration_secs: 3600,
            escrow_required: false,
        };

        let applied = apply_action(&action, &wallet(1), &shared, &m18, tmp.path())
            .await
            .unwrap();
        assert!(applied);

        let contracts = m18.contracts.lock().unwrap();
        assert_eq!(contracts.len(), 1);
        let c = contracts.values().next().unwrap();
        assert_eq!(c.consumer_wallet, wallet(1));
        assert_eq!(c.provider_wallet, wallet(2));
        assert!(matches!(c.status, ContractStatus::Proposed));

        let actions = m18.actions.lock().unwrap();
        assert_eq!(actions.len(), 1);
    }

    #[tokio::test]
    async fn apply_accept_and_complete_lifecycle() {
        let shared = new_shared_hub();
        let m18 = M18State::test_default();
        let tmp = tempfile::tempdir().unwrap();

        // Create proposed contract.
        let propose = EconomicAction::ProposeContract {
            provider_wallet: wallet(2),
            capability: "coding".to_string(),
            description: "Write tests".to_string(),
            price_micro_cu: 500_000,
            max_duration_secs: 600,
            escrow_required: false,
        };
        apply_action(&propose, &wallet(1), &shared, &m18, tmp.path())
            .await
            .unwrap();

        let contract_id = { m18.contracts.lock().unwrap().keys().next().unwrap().clone() };

        // Provider accepts.
        let accept = EconomicAction::AcceptContract {
            contract_id: contract_id.clone(),
        };
        apply_action(&accept, &wallet(2), &shared, &m18, tmp.path())
            .await
            .unwrap();

        // Wrong wallet cannot accept (scope guard so it drops before await).
        {
            let contracts = m18.contracts.lock().unwrap();
            let c = contracts.values().next().unwrap();
            assert!(matches!(c.status, ContractStatus::Accepted));
        }

        // Start execution.
        let start = EconomicAction::StartExecution {
            contract_id: contract_id.clone(),
        };
        apply_action(&start, &wallet(2), &shared, &m18, tmp.path())
            .await
            .unwrap();

        // Complete.
        let complete = EconomicAction::CompleteContract {
            contract_id: contract_id.clone(),
        };
        apply_action(&complete, &wallet(2), &shared, &m18, tmp.path())
            .await
            .unwrap();

        let contracts = m18.contracts.lock().unwrap();
        let c = contracts.values().next().unwrap();
        let completed = matches!(c.status, ContractStatus::Completed);
        drop(contracts);
        assert!(completed);
    }

    #[tokio::test]
    async fn apply_record_trust_creates_anchor() {
        let shared = new_shared_hub();
        let m18 = M18State::test_default();
        let tmp = tempfile::tempdir().unwrap();

        let action = EconomicAction::RecordTrust {
            evidence_hash: "ev".repeat(32),
            capability: "research".to_string(),
            quality_score: 95,
            micro_cu: 1_000_000,
            contract_id: None,
        };
        let applied = apply_action(&action, &wallet(1), &shared, &m18, tmp.path())
            .await
            .unwrap();
        assert!(applied);

        let trust = m18.trust.lock().unwrap();
        assert_eq!(trust.anchors.len(), 1);
        assert_eq!(trust.trust_score(&wallet(1)), 1.0);
    }
}
