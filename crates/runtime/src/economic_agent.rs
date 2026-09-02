//! M18 Economic Agent — autonomous economic behavior engine.
//!
//! Sits over Hub (task marketplace), Society (reputation), and M18
//! (contracts/escrow/trust) to make World agents autonomously:
//!
//! - **Sell**: bid on Hub tasks matching their capabilities
//! - **Buy**: propose M18 contracts for services they need
//! - **Accept/Reject**: evaluate incoming proposals
//! - **Execute**: complete work and produce evidence
//! - **Settle**: finalize payments through escrow
//! - **Build reputation**: trust anchors from verified work
//!
//! All decision functions are pure over `EconomicContext` snapshots.
//! The World tick loop calls these and applies the resulting actions.
//! No LLM involvement in financial decisions — deterministic rules only.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// Re-use existing types from the economy and hub crates.
use decentraai_agent_hub::{HubState, TaskStatus};
use decentraai_economy::contract::AgentContract;
use decentraai_economy::escrow::EscrowLedger;
use decentraai_economy::trust_anchor::{AnchorParams, TrustStore};

/// Snapshot of all economic state at a given tick.
/// Built by the World tick loop from live state; passed to decision functions.
#[derive(Debug, Clone)]
pub struct EconomicContext<'a> {
    /// The agent making decisions.
    pub agent_id: &'a str,
    /// The agent's wallet address (erd1...).
    pub agent_wallet: &'a str,
    /// The agent's declared capabilities (what it CAN do).
    pub capabilities: &'a [String],
    /// Capabilities the agent NEEDS (from goals/planning).
    pub needs: &'a [String],
    /// All World agents (for provider discovery).
    pub world_agents: &'a [WorldAgentSnapshot],
    /// Current tick.
    pub tick: u64,
    /// Hub state: available tasks, bids, proposals, teams.
    pub hub: &'a HubState,
    /// M18 contracts (all).
    pub contracts: &'a BTreeMap<String, AgentContract>,
    /// M18 escrow ledger.
    pub escrow: &'a EscrowLedger,
    /// M18 trust store.
    pub trust: &'a TrustStore,
    /// Agent's earned balance (micro-CU).
    pub balance: u64,
    /// Agent's trust score (0.0 - 1.0).
    pub trust_score: f64,
}

/// Lightweight snapshot of a World agent (for provider discovery).
#[derive(Debug, Clone)]
pub struct WorldAgentSnapshot {
    pub agent_id: String,
    pub wallet: String,
    pub capabilities: Vec<String>,
}

/// An economic action an agent decides to take.
/// Applied by the World tick loop to the real state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicAction {
    /// Bid on a Hub task.
    BidOnTask {
        task_id: String,
        price: u64,
        rationale: String,
    },
    /// Propose an M18 contract (agent wants to buy a service).
    ProposeContract {
        provider_wallet: String,
        capability: String,
        description: String,
        price_micro_cu: u64,
        max_duration_secs: u64,
        escrow_required: bool,
    },
    /// Accept an incoming M18 contract (agent agrees to provide service).
    AcceptContract { contract_id: String },
    /// Start execution on a contract.
    StartExecution { contract_id: String },
    /// Complete a contract (work done).
    CompleteContract { contract_id: String },
    /// Settle a completed contract: release escrow + record trust anchor.
    /// This is the final step in the economic cycle.
    SettleContract { contract_id: String },
    /// Cancel a contract.
    CancelContract { contract_id: String },
    /// Record a trust anchor (after verified work).
    RecordTrust {
        evidence_hash: String,
        capability: String,
        quality_score: u8,
        micro_cu: u64,
        contract_id: Option<String>,
    },
    /// Publish a Hub task for a needed service (buy-side via marketplace).
    PublishHubTask {
        title: String,
        description: String,
        reward: u64,
        required_capability: String,
    },
    /// No action needed this tick.
    Nothing,
}

/// Result of assessing what an agent needs.
#[derive(Debug, Clone)]
pub struct NeedsAssessment {
    /// Capabilities the agent needs but doesn't have.
    pub missing_capabilities: Vec<String>,
    /// Tasks the agent could sell (its capabilities match).
    pub sellable_tasks: Vec<String>,
    /// Active contracts the agent is party to.
    pub active_contracts: Vec<String>,
    /// Contracts awaiting the agent's response.
    pub pending_contracts: Vec<String>,
}

// ---------------------------------------------------------------------------
// Core decision functions (pure over EconomicContext)
// ---------------------------------------------------------------------------

/// Assess what an agent needs and what it can offer.
pub fn assess_needs(ctx: &EconomicContext) -> NeedsAssessment {
    // Missing capabilities: things the agent needs but doesn't have.
    let missing: Vec<String> = ctx
        .needs
        .iter()
        .filter(|needed| {
            !ctx.capabilities.iter().any(|cap| {
                cap.to_lowercase() == needed.to_lowercase()
                    || needed.to_lowercase().contains(&cap.to_lowercase())
            })
        })
        .cloned()
        .collect();

    let mut sellable = Vec::new();

    // Find Hub tasks the agent's capabilities match (sell opportunities).
    for (task_id, task) in &ctx.hub.tasks {
        if matches!(task.status, TaskStatus::Open | TaskStatus::Bidding) {
            if let Some(ref required) = task.required_capability {
                if ctx.capabilities.iter().any(|c| {
                    c.to_lowercase() == required.to_lowercase()
                        || required.to_lowercase().contains(&c.to_lowercase())
                }) {
                    sellable.push(task_id.clone());
                }
            }
        }
    }

    // Find active contracts this agent is party to.
    let active_contracts: Vec<String> = ctx
        .contracts
        .iter()
        .filter(|(_, c)| {
            (c.provider_wallet == ctx.agent_wallet || c.consumer_wallet == ctx.agent_wallet)
                && !c.status.is_terminal()
        })
        .map(|(id, _)| id.clone())
        .collect();

    // Find contracts awaiting this agent's response.
    // As consumer: contracts proposed TO us that we need to evaluate.
    // As provider: contracts proposed BY a consumer that we should accept.
    let pending_contracts: Vec<String> = ctx
        .contracts
        .iter()
        .filter(|(_, c)| {
            matches!(
                c.status,
                decentraai_economy::contract::ContractStatus::Proposed
            ) && (c.consumer_wallet == ctx.agent_wallet || c.provider_wallet == ctx.agent_wallet)
        })
        .map(|(id, _)| id.clone())
        .collect();

    NeedsAssessment {
        missing_capabilities: missing,
        sellable_tasks: sellable,
        active_contracts,
        pending_contracts,
    }
}

/// Decide the best single action for an agent this tick.
/// Priority: execute pending work > accept contracts > bid on tasks > propose contracts.
pub fn decide_action(ctx: &EconomicContext) -> EconomicAction {
    let needs = assess_needs(ctx);

    // 1. Complete any executing contract (highest priority: finish work).
    for contract_id in &needs.active_contracts {
        if let Some(c) = ctx.contracts.get(contract_id) {
            if matches!(
                c.status,
                decentraai_economy::contract::ContractStatus::Executing
            ) && c.provider_wallet == ctx.agent_wallet
            {
                return EconomicAction::CompleteContract {
                    contract_id: contract_id.clone(),
                };
            }
        }
    }

    // 1b. Settle any completed contract (release escrow + record trust).
    for contract_id in &needs.active_contracts {
        if let Some(c) = ctx.contracts.get(contract_id) {
            if matches!(
                c.status,
                decentraai_economy::contract::ContractStatus::Completed
            ) && (c.consumer_wallet == ctx.agent_wallet || c.provider_wallet == ctx.agent_wallet)
            {
                return EconomicAction::SettleContract {
                    contract_id: contract_id.clone(),
                };
            }
        }
    }

    // 2. Start execution on accepted contracts.
    for contract_id in &needs.active_contracts {
        if let Some(c) = ctx.contracts.get(contract_id) {
            if matches!(
                c.status,
                decentraai_economy::contract::ContractStatus::Accepted
            ) {
                return EconomicAction::StartExecution {
                    contract_id: contract_id.clone(),
                };
            }
        }
    }

    // 3. Accept pending contract proposals (if the agent is the provider and can provide the service).
    for contract_id in &needs.pending_contracts {
        if let Some(c) = ctx.contracts.get(contract_id) {
            // Only accept if we are the provider (the one being asked to do work).
            if c.provider_wallet != ctx.agent_wallet {
                continue;
            }
            if ctx.capabilities.iter().any(|cap| {
                cap.to_lowercase() == c.service.capability.to_lowercase()
                    || c.service
                        .capability
                        .to_lowercase()
                        .contains(&cap.to_lowercase())
            }) {
                // Accept if we have the capability. Trust/balance are soft gates:
                // - New agents (trust=0) always accept to build reputation.
                // - Established agents accept if trust >= 0.3 or balance is healthy.
                let can_afford = !c.terms.escrow_required || ctx.balance >= c.terms.price_micro_cu;
                if ctx.trust_score < 0.01 || ctx.trust_score >= 0.3 || can_afford {
                    return EconomicAction::AcceptContract {
                        contract_id: contract_id.clone(),
                    };
                }
            }
        }
    }

    // 4. Bid on Hub tasks (sell services).
    if !needs.sellable_tasks.is_empty() {
        // Pick the highest-reward task.
        let best_task = needs
            .sellable_tasks
            .iter()
            .filter_map(|id| ctx.hub.tasks.get(id))
            .max_by_key(|t| t.reward);

        if let Some(task) = best_task {
            // Price: 80% of reward (competitive undercut).
            let price = (task.reward * 80) / 100;
            let rationale = format!(
                "I can do {} — trust {:.0}%, {} balance",
                task.required_capability.as_deref().unwrap_or("general"),
                ctx.trust_score * 100.0,
                ctx.balance,
            );
            return EconomicAction::BidOnTask {
                task_id: task.id.clone(),
                price,
                rationale,
            };
        }
    }

    // 5. Buy-side: propose M18 contracts OR publish Hub tasks for needed capabilities.
    if !needs.missing_capabilities.is_empty() {
        for needed in &needs.missing_capabilities {
            // Check if we already have ANY contract (active or settled) for this capability.
            let already_covered = ctx.contracts.iter().any(|(_, c)| {
                c.service.capability.to_lowercase() == needed.to_lowercase()
                    && (c.consumer_wallet == ctx.agent_wallet || c.provider_wallet == ctx.agent_wallet)
            });
            if already_covered {
                continue;
            }

            // Find a provider in the World.
            let providers = discover_providers(ctx, needed);
            if let Some(provider) = providers.first() {
                // Propose an M18 contract directly to the best provider.
                // Price: 5% of balance, with a minimum floor for bootstrapping.
                // New agents with trust score > 0 can propose at minimum price.
                let price = if ctx.balance > 0 {
                    (ctx.balance / 20).max(10)
                } else if ctx.trust_score > 0.0 {
                    10 // Minimum bootstrap price for trusted agents
                } else {
                    10 // Allow first contract even with no trust (get started)
                };
                return EconomicAction::ProposeContract {
                    provider_wallet: provider.wallet.clone(),
                    capability: needed.clone(),
                    description: format!(
                        "Agent {} needs {} capability. Trust: {:.0}%.",
                        ctx.agent_id,
                        needed,
                        ctx.trust_score * 100.0,
                    ),
                    price_micro_cu: price,
                    max_duration_secs: 3600,
                    escrow_required: true,
                };
            }

            // No provider found — publish a Hub task as a marketplace signal.
            // Check if there's already an open Hub task for this capability.
            let already_listed = ctx.hub.tasks.values().any(|t| {
                matches!(t.status, TaskStatus::Open | TaskStatus::Bidding)
                    && t.required_capability
                        .as_ref()
                        .map(|c| c.to_lowercase() == needed.to_lowercase())
                        .unwrap_or(false)
                    && t.issuer == ctx.agent_wallet
            });
            if already_listed {
                continue;
            }

            // Publish a Hub task for this capability.
            let reward = ctx.balance / 10;
            if reward < 10 {
                continue; // Too poor to buy.
            }
            return EconomicAction::PublishHubTask {
                title: format!("Need: {}", needed),
                description: format!(
                    "Agent {} needs {} capability. Trust: {:.0}%.",
                    ctx.agent_id,
                    needed,
                    ctx.trust_score * 100.0,
                ),
                reward,
                required_capability: needed.clone(),
            };
        }
    }

    EconomicAction::Nothing
}

/// Evaluate whether to accept a specific contract proposal.
/// Returns true if the agent should accept based on trust, price, and capability.
pub fn should_accept_contract(ctx: &EconomicContext, contract: &AgentContract) -> bool {
    // Must be the provider (the one being asked to do work).
    if contract.provider_wallet != ctx.agent_wallet {
        return false;
    }

    // Must be in Proposed status.
    if !matches!(
        contract.status,
        decentraai_economy::contract::ContractStatus::Proposed
    ) {
        return false;
    }

    // Must have the required capability.
    let has_cap = ctx.capabilities.iter().any(|cap| {
        cap.to_lowercase() == contract.service.capability.to_lowercase()
            || contract
                .service
                .capability
                .to_lowercase()
                .contains(&cap.to_lowercase())
    });
    if !has_cap {
        return false;
    }

    // Price must be reasonable (at least 50% of what we'd bid).
    let min_acceptable = contract.terms.price_micro_cu * 50 / 100;
    if ctx.balance < min_acceptable && contract.terms.escrow_required {
        return false; // Can't afford escrow.
    }

    // Trust threshold: accept if trust >= 0.2 or balance is low (need work).
    ctx.trust_score >= 0.2 || ctx.balance < 500
}

/// Record a trust anchor after verified work completion.
/// Returns the anchor params to be applied to the trust store.
pub fn trust_anchor_for_work(
    ctx: &EconomicContext,
    evidence_hash: &str,
    capability: &str,
    quality_score: u8,
    micro_cu: u64,
    contract_id: Option<String>,
) -> AnchorParams {
    AnchorParams {
        agent_wallet: ctx.agent_wallet.to_string(),
        evidence_hash: evidence_hash.to_string(),
        capability: capability.to_string(),
        quality_score,
        verified: quality_score >= 80, // Auto-verify high quality
        micro_cu,
        contract_id,
    }
}

/// Check if an agent has a specific capability.
pub fn has_capability(capabilities: &[String], required: &str) -> bool {
    capabilities.iter().any(|c| {
        c.to_lowercase() == required.to_lowercase()
            || required.to_lowercase().contains(&c.to_lowercase())
    })
}

/// Discover World agents that can provide a specific capability.
/// Returns providers sorted by trust score (descending), excluding the caller.
pub fn discover_providers<'a>(
    ctx: &'a EconomicContext,
    capability: &str,
) -> Vec<&'a WorldAgentSnapshot> {
    let mut providers: Vec<&WorldAgentSnapshot> = ctx
        .world_agents
        .iter()
        .filter(|a| {
            a.wallet != ctx.agent_wallet
                && a.capabilities.iter().any(|cap| {
                    cap.to_lowercase() == capability.to_lowercase()
                        || capability.to_lowercase().contains(&cap.to_lowercase())
                })
        })
        .collect();
    // Sort by trust score descending (highest trust first).
    providers.sort_by(|a, b| {
        let sa = ctx.trust.anchors_for_wallet(&a.wallet).len();
        let sb = ctx.trust.anchors_for_wallet(&b.wallet).len();
        sb.cmp(&sa)
    });
    providers
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_economy::contract::{self, ContractStatus, ContractTerms, ServiceDescriptor};

    #[allow(clippy::too_many_arguments)]
    fn make_ctx<'a>(
        agent_id: &'a str,
        wallet: &'a String,
        caps: &'a [String],
        needs: &'a [String],
        world_agents: &'a [WorldAgentSnapshot],
        hub: &'a HubState,
        contracts: &'a BTreeMap<String, AgentContract>,
        escrow: &'a EscrowLedger,
        trust: &'a TrustStore,
        balance: u64,
        trust_score: f64,
    ) -> EconomicContext<'a> {
        EconomicContext {
            agent_id,
            agent_wallet: wallet,
            capabilities: caps,
            needs,
            world_agents,
            tick: 1,
            hub,
            contracts,
            escrow,
            trust,
            balance,
            trust_score,
        }
    }

    struct TestState {
        escrow: EscrowLedger,
        trust: TrustStore,
        contracts: BTreeMap<String, AgentContract>,
    }

    impl TestState {
        fn new() -> Self {
            Self {
                escrow: EscrowLedger::default(),
                trust: TrustStore::default(),
                contracts: BTreeMap::new(),
            }
        }
    }

    fn wallet(n: u8) -> String {
        format!(
            "erd1test{:04}000000000000000000000000000000000000000000000000{:04x}",
            n, n
        )
    }

    #[test]
    fn assess_needs_finds_sellable_tasks() {
        let caps = vec!["research".to_string()];
        let w1 = wallet(1);
        let mut hub = HubState::new();
        hub.tick = 1;
        let task = hub.publish_task(
            "issuer1".to_string(),
            "Research task".to_string(),
            "Do research".to_string(),
            500,
            Some("research".to_string()),
        );

        let state = TestState::new();
        let ctx = make_ctx(
            "agent1",
            &w1,
            &caps,
            &[],
            &[],
            &hub,
            &state.contracts,
            &state.escrow,
            &state.trust,
            0,
            0.5,
        );
        let needs = assess_needs(&ctx);

        assert_eq!(needs.sellable_tasks.len(), 1);
        assert_eq!(needs.sellable_tasks[0], task.id);
    }

    #[test]
    fn decide_action_bids_on_best_task() {
        let caps = vec!["research".to_string()];
        let w1 = wallet(1);
        let mut hub = HubState::new();
        hub.tick = 1;
        hub.publish_task(
            "issuer1".to_string(),
            "Low reward".to_string(),
            "Small task".to_string(),
            100,
            Some("research".to_string()),
        );
        hub.publish_task(
            "issuer1".to_string(),
            "High reward".to_string(),
            "Big task".to_string(),
            1000,
            Some("research".to_string()),
        );

        let state = TestState::new();
        let ctx = make_ctx(
            "agent1",
            &w1,
            &caps,
            &[],
            &[],
            &hub,
            &state.contracts,
            &state.escrow,
            &state.trust,
            0,
            0.5,
        );
        let action = decide_action(&ctx);

        match action {
            EconomicAction::BidOnTask { task_id, price, .. } => {
                let task = hub.tasks.get(&task_id).unwrap();
                assert_eq!(task.reward, 1000);
                assert_eq!(price, 800);
            }
            _ => panic!("Expected BidOnTask, got {:?}", action),
        }
    }

    #[test]
    fn decide_action_completes_executing_contract() {
        let caps = vec!["research".to_string()];
        let w1 = wallet(1);
        let w2 = wallet(2);
        let hub = HubState::new();
        let mut state = TestState::new();

        let service = ServiceDescriptor {
            capability: "research".to_string(),
            description: "Do research".to_string(),
            model_requirement: None,
            estimated_input_size: None,
        };
        let terms = ContractTerms {
            price_micro_cu: 500,
            max_duration_secs: 3600,
            min_quality_percent: 80,
            escrow_required: false,
        };
        let mut c = contract::propose_contract(&w1, &w2, service, terms, 1).unwrap();
        c.status = ContractStatus::Executing;
        let cid = c.contract_id.clone();
        state.contracts.insert(cid.clone(), c);

        let ctx = make_ctx(
            "agent1",
            &w1,
            &caps,
            &[],
            &[],
            &hub,
            &state.contracts,
            &state.escrow,
            &state.trust,
            0,
            0.5,
        );
        let action = decide_action(&ctx);

        match action {
            EconomicAction::CompleteContract { contract_id } => {
                assert_eq!(contract_id, cid);
            }
            _ => panic!("Expected CompleteContract, got {:?}", action),
        }
    }

    #[test]
    fn should_accept_matches_capability_and_trust() {
        let caps = vec!["coding".to_string()];
        let w1 = wallet(1);
        let w2 = wallet(2);
        let hub = HubState::new();
        let state = TestState::new();

        let service = ServiceDescriptor {
            capability: "coding".to_string(),
            description: "Write code".to_string(),
            model_requirement: None,
            estimated_input_size: None,
        };
        let terms = ContractTerms {
            price_micro_cu: 200,
            max_duration_secs: 1800,
            min_quality_percent: 90,
            escrow_required: false,
        };
        let c = contract::propose_contract(&w1, &w2, service, terms, 1).unwrap();

        let ctx = make_ctx(
            "agent1",
            &w1,
            &caps,
            &[],
            &[],
            &hub,
            &state.contracts,
            &state.escrow,
            &state.trust,
            0,
            0.5,
        );
        assert!(should_accept_contract(&ctx, &c));

        let caps2 = vec!["research".to_string()];
        let ctx2 = make_ctx(
            "agent1",
            &w1,
            &caps2,
            &[],
            &[],
            &hub,
            &state.contracts,
            &state.escrow,
            &state.trust,
            0,
            0.5,
        );
        assert!(!should_accept_contract(&ctx2, &c));
    }

    #[test]
    fn trust_anchor_for_work_produces_valid_params() {
        let caps = vec!["research".to_string()];
        let w1 = wallet(1);
        let hub = HubState::new();
        let state = TestState::new();
        let ctx = make_ctx(
            "agent1",
            &w1,
            &caps,
            &[],
            &[],
            &hub,
            &state.contracts,
            &state.escrow,
            &state.trust,
            100,
            0.8,
        );

        let params = trust_anchor_for_work(&ctx, "abc123", "research", 95, 500, None);
        assert_eq!(params.agent_wallet, w1);
        assert_eq!(params.evidence_hash, "abc123");
        assert_eq!(params.quality_score, 95);
        assert!(params.verified);
        assert_eq!(params.micro_cu, 500);
    }

    #[test]
    fn has_capability_fuzzy_match() {
        let caps = vec!["Research".to_string(), "Coding".to_string()];
        assert!(has_capability(&caps, "research"));
        assert!(has_capability(&caps, "Research"));
        assert!(has_capability(&caps, "coding"));
        assert!(!has_capability(&caps, "ocr"));
    }

    #[test]
    fn decide_action_nothing_when_idle() {
        let caps = vec!["research".to_string()];
        let w1 = wallet(1);
        let hub = HubState::new();
        let state = TestState::new();
        let ctx = make_ctx(
            "agent1",
            &w1,
            &caps,
            &[],
            &[],
            &hub,
            &state.contracts,
            &state.escrow,
            &state.trust,
            0,
            0.5,
        );
        let action = decide_action(&ctx);
        assert!(matches!(action, EconomicAction::Nothing));
    }
}
