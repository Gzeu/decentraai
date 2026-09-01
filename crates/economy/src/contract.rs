//! Agent-to-Agent Service Contracts — the first economic primitive for
//! structured agreements between two wallet-backed agents.
//!
//! # Design
//!
//! ```text
//! Contract {
//!   provider (wallet-backed agent)
//!   consumer (wallet-backed agent)
//!   service  (capability + task description)
//!   terms    (price, max_duration, SLA)
//!   status   (lifecycle)
//!   settlement (evidence hash + CU amount + tx ref)
//! }
//! ```
//!
//! # Rules
//!
//! - Both parties MUST be wallet-backed (addresses are `erd1...` bech32).
//! - A contract is created by the consumer (they request the service).
//! - The provider accepts or rejects.
//! - Execution happens off-chain (Fabric/Hub task infrastructure).
//! - Settlement references on-chain evidence (EconomicEvidence BLAKE3 hash).
//! - Status transitions are monotonic: Proposed → Accepted → Executing →
//!   Completed | Disputed | Cancelled.
//! - No LLM determines contract terms: they are explicit data.

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// Contract lifecycle status — monotonic transitions only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    /// Consumer proposed, awaiting provider response.
    Proposed,
    /// Provider accepted the terms.
    Accepted,
    /// Work is in progress (off-chain execution).
    Executing,
    /// Work completed successfully, settlement pending.
    Completed,
    /// Dispute raised by either party.
    Disputed,
    /// Cancelled by consumer before acceptance, or by mutual agreement.
    Cancelled,
    /// Settlement finalized on-chain.
    Settled,
}

impl ContractStatus {
    /// Whether the contract can still be modified.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ContractStatus::Settled | ContractStatus::Cancelled | ContractStatus::Disputed
        )
    }
}

/// Service description — what the consumer is requesting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDescriptor {
    /// Capability kind (from the hub taxonomy, e.g. "chat", "ocr", "embedding").
    pub capability: String,
    /// Free-form task description (bounded).
    pub description: String,
    /// Optional: specific model or tool required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_requirement: Option<String>,
    /// Optional: estimated input size in tokens/chars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_input_size: Option<u64>,
}

/// Financial and temporal terms of the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractTerms {
    /// Price in micro-CU the consumer agrees to pay.
    pub price_micro_cu: u64,
    /// Maximum execution time in seconds.
    pub max_duration_secs: u64,
    /// Minimum quality percent required (0 = no SLA).
    pub min_quality_percent: u8,
    /// Whether the provider must stake escrow before execution.
    pub escrow_required: bool,
}

/// The settlement reference — links off-chain execution to on-chain anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRef {
    /// EconomicEvidence BLAKE3 hash (the on-chain anchor).
    pub evidence_hash: String,
    /// Amount in micro-CU awarded.
    pub amount_micro_cu: u64,
    /// MultiversX transaction hash (once settled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    /// Settlement timestamp.
    pub settled_at: u64,
}

/// An agent-to-agent service contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentContract {
    /// Unique contract identifier.
    pub contract_id: String,
    /// Provider wallet address (erd1...).
    pub provider_wallet: String,
    /// Consumer wallet address (erd1...).
    pub consumer_wallet: String,
    /// What service is being contracted.
    pub service: ServiceDescriptor,
    /// Agreed terms.
    pub terms: ContractTerms,
    /// Current lifecycle status.
    pub status: ContractStatus,
    /// Creation timestamp.
    pub created_at: u64,
    /// Last status change timestamp.
    pub updated_at: u64,
    /// Settlement reference (present after completion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement: Option<SettlementRef>,
    /// Free-form notes from either party.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<ContractNote>,
}

/// A timestamped note on a contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractNote {
    pub author_wallet: String,
    pub message: String,
    pub timestamp: u64,
}

/// Errors from contract operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    #[error("invalid wallet address: {0}")]
    InvalidWallet(String),
    #[error("contract is in terminal state: {0:?}")]
    TerminalStatus(ContractStatus),
    #[error("invalid status transition: {0:?} → {1:?}")]
    InvalidTransition(ContractStatus, ContractStatus),
    #[error("provider and consumer must differ")]
    SameParty,
    #[error("contract not found")]
    NotFound,
    #[error("unauthorized: {0}")]
    Unauthorized(String),
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Generates a deterministic contract ID from the participant addresses and
/// creation timestamp.
pub fn generate_contract_id(
    provider_wallet: &str,
    consumer_wallet: &str,
    created_at: u64,
) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    provider_wallet.hash(&mut hasher);
    consumer_wallet.hash(&mut hasher);
    created_at.hash(&mut hasher);
    let hash = hasher.finish();
    format!("ct-{}", to_hex(&hash.to_be_bytes()))
}

/// Validates a status transition.
pub fn valid_transition(from: ContractStatus, to: ContractStatus) -> bool {
    matches!(
        (from, to),
        (ContractStatus::Proposed, ContractStatus::Accepted)
            | (ContractStatus::Proposed, ContractStatus::Cancelled)
            | (ContractStatus::Accepted, ContractStatus::Executing)
            | (ContractStatus::Accepted, ContractStatus::Cancelled)
            | (ContractStatus::Executing, ContractStatus::Completed)
            | (ContractStatus::Executing, ContractStatus::Disputed)
            | (ContractStatus::Completed, ContractStatus::Settled)
            | (ContractStatus::Completed, ContractStatus::Disputed)
    )
}

/// Creates a new contract (consumer proposes).
pub fn propose_contract(
    provider_wallet: &str,
    consumer_wallet: &str,
    service: ServiceDescriptor,
    terms: ContractTerms,
    now: u64,
) -> Result<AgentContract, ContractError> {
    if provider_wallet == consumer_wallet {
        return Err(ContractError::SameParty);
    }
    if !provider_wallet.starts_with("erd1") || !consumer_wallet.starts_with("erd1") {
        return Err(ContractError::InvalidWallet(
            "both addresses must be erd1... bech32".into(),
        ));
    }
    let contract_id = generate_contract_id(provider_wallet, consumer_wallet, now);
    Ok(AgentContract {
        contract_id,
        provider_wallet: provider_wallet.to_string(),
        consumer_wallet: consumer_wallet.to_string(),
        service,
        terms,
        status: ContractStatus::Proposed,
        created_at: now,
        updated_at: now,
        settlement: None,
        notes: vec![],
    })
}

/// Provider accepts the contract.
pub fn accept_contract(
    contract: &mut AgentContract,
    caller_wallet: &str,
    now: u64,
) -> Result<(), ContractError> {
    if caller_wallet != contract.provider_wallet {
        return Err(ContractError::Unauthorized(
            "only provider can accept".into(),
        ));
    }
    if !valid_transition(contract.status, ContractStatus::Accepted) {
        return Err(ContractError::InvalidTransition(
            contract.status,
            ContractStatus::Accepted,
        ));
    }
    contract.status = ContractStatus::Accepted;
    contract.updated_at = now;
    Ok(())
}

/// Either party marks execution started.
pub fn start_execution(
    contract: &mut AgentContract,
    caller_wallet: &str,
    now: u64,
) -> Result<(), ContractError> {
    if caller_wallet != contract.provider_wallet && caller_wallet != contract.consumer_wallet {
        return Err(ContractError::Unauthorized("not a party".into()));
    }
    if !valid_transition(contract.status, ContractStatus::Executing) {
        return Err(ContractError::InvalidTransition(
            contract.status,
            ContractStatus::Executing,
        ));
    }
    contract.status = ContractStatus::Executing;
    contract.updated_at = now;
    Ok(())
}

/// Provider marks execution completed.
pub fn complete_contract(
    contract: &mut AgentContract,
    caller_wallet: &str,
    now: u64,
) -> Result<(), ContractError> {
    if caller_wallet != contract.provider_wallet {
        return Err(ContractError::Unauthorized(
            "only provider can complete".into(),
        ));
    }
    if !valid_transition(contract.status, ContractStatus::Completed) {
        return Err(ContractError::InvalidTransition(
            contract.status,
            ContractStatus::Completed,
        ));
    }
    contract.status = ContractStatus::Completed;
    contract.updated_at = now;
    Ok(())
}

/// Finalize settlement (after on-chain tx confirmed).
pub fn settle_contract(
    contract: &mut AgentContract,
    settlement: SettlementRef,
    now: u64,
) -> Result<(), ContractError> {
    if !valid_transition(contract.status, ContractStatus::Settled) {
        return Err(ContractError::InvalidTransition(
            contract.status,
            ContractStatus::Settled,
        ));
    }
    contract.status = ContractStatus::Settled;
    contract.settlement = Some(settlement);
    contract.updated_at = now;
    Ok(())
}

/// Cancel (consumer before acceptance, or mutual).
pub fn cancel_contract(
    contract: &mut AgentContract,
    caller_wallet: &str,
    now: u64,
) -> Result<(), ContractError> {
    if caller_wallet != contract.consumer_wallet && caller_wallet != contract.provider_wallet {
        return Err(ContractError::Unauthorized("not a party".into()));
    }
    if !valid_transition(contract.status, ContractStatus::Cancelled) {
        return Err(ContractError::InvalidTransition(
            contract.status,
            ContractStatus::Cancelled,
        ));
    }
    contract.status = ContractStatus::Cancelled;
    contract.updated_at = now;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> String {
        "erd1qykvz2cfamvyhwrc087l8lnsy7f7f0z9exvnjudn3d8fagxn3d8spujzsm".to_string()
    }

    fn consumer() -> String {
        "erd1qqqqqqqqqqqqqpgqzcufga3vm5r44xe3ukzyl4dmhpsvalrkkgjqeyu68x".to_string()
    }

    fn service() -> ServiceDescriptor {
        ServiceDescriptor {
            capability: "chat".into(),
            description: "Summarize a 10k token document".into(),
            model_requirement: None,
            estimated_input_size: Some(10_000),
        }
    }

    fn terms() -> ContractTerms {
        ContractTerms {
            price_micro_cu: 5_000_000,
            max_duration_secs: 120,
            min_quality_percent: 80,
            escrow_required: true,
        }
    }

    #[test]
    fn propose_creates_a_valid_contract() {
        let c = propose_contract(&provider(), &consumer(), service(), terms(), 1000).unwrap();
        assert_eq!(c.status, ContractStatus::Proposed);
        assert!(c.contract_id.starts_with("ct-"));
        assert_eq!(c.provider_wallet, provider());
        assert_eq!(c.consumer_wallet, consumer());
    }

    #[test]
    fn same_party_rejected() {
        let p = provider();
        assert!(matches!(
            propose_contract(&p, &p, service(), terms(), 1),
            Err(ContractError::SameParty)
        ));
    }

    #[test]
    fn full_happy_path() {
        let mut c = propose_contract(&provider(), &consumer(), service(), terms(), 100).unwrap();
        assert_eq!(c.status, ContractStatus::Proposed);

        accept_contract(&mut c, &provider(), 200).unwrap();
        assert_eq!(c.status, ContractStatus::Accepted);

        start_execution(&mut c, &provider(), 300).unwrap();
        assert_eq!(c.status, ContractStatus::Executing);

        complete_contract(&mut c, &provider(), 400).unwrap();
        assert_eq!(c.status, ContractStatus::Completed);

        settle_contract(
            &mut c,
            SettlementRef {
                evidence_hash: "ab".repeat(32),
                amount_micro_cu: 5_000_000,
                tx_hash: Some("tx-abc".into()),
                settled_at: 500,
            },
            500,
        )
        .unwrap();
        assert_eq!(c.status, ContractStatus::Settled);
        assert!(c.settlement.is_some());
    }

    #[test]
    fn invalid_transition_rejected() {
        let mut c = propose_contract(&provider(), &consumer(), service(), terms(), 100).unwrap();
        // Can't go directly to Completed from Proposed
        assert!(matches!(
            complete_contract(&mut c, &provider(), 200),
            Err(ContractError::InvalidTransition(..))
        ));
    }

    #[test]
    fn unauthorized_caller_rejected() {
        let mut c = propose_contract(&provider(), &consumer(), service(), terms(), 100).unwrap();
        let stranger = "erd1qqqqqqqqqqqqqpgq78888888888888888888888888888888ca5h83z".to_string();
        assert!(matches!(
            accept_contract(&mut c, &stranger, 200),
            Err(ContractError::Unauthorized(_))
        ));
    }

    #[test]
    fn cancel_by_consumer_before_acceptance() {
        let mut c = propose_contract(&provider(), &consumer(), service(), terms(), 100).unwrap();
        cancel_contract(&mut c, &consumer(), 200).unwrap();
        assert_eq!(c.status, ContractStatus::Cancelled);
    }

    #[test]
    fn dispute_during_execution() {
        let mut c = propose_contract(&provider(), &consumer(), service(), terms(), 100).unwrap();
        accept_contract(&mut c, &provider(), 200).unwrap();
        start_execution(&mut c, &provider(), 300).unwrap();
        // Simulate dispute
        c.status = ContractStatus::Disputed;
        assert!(c.status.is_terminal());
    }

    #[test]
    fn contract_id_is_deterministic() {
        let c1 = propose_contract(&provider(), &consumer(), service(), terms(), 100).unwrap();
        let c2 = propose_contract(&provider(), &consumer(), service(), terms(), 100).unwrap();
        assert_eq!(c1.contract_id, c2.contract_id);
    }

    #[test]
    fn serialization_round_trip() {
        let c = propose_contract(&provider(), &consumer(), service(), terms(), 100).unwrap();
        let json = serde_json::to_string(&c).unwrap();
        let back: AgentContract = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }
}
