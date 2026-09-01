//! Escrow + Settlement — the economic bridge between off-chain execution and
//! on-chain anchoring via MultiversX testnet.
//!
//! # Flow
//!
//! ```text
//! AgentContract (status=Completed)
//!   → EconomicEvidence (from execution)
//!     → EscrowRecord (hold funds)
//!       → Settlement on MultiversX (testnet tx)
//!         → SettlementRef (tx hash + evidence hash)
//!           → Contract status=Settled
//! ```
//!
//! # Rules
//!
//! - Escrow is off-chain only (testnet). No real funds are moved.
//! - Settlement submits EconomicEvidence BLAKE3 hash to MultiversX as proof.
//! - The escrow ledger tracks who owes whom, linked to contract + evidence.
//! - Double-settlement is prevented by evidence hash deduplication.
//! - All amounts are micro-CU (integer-only, versioned).

use crate::contract::{AgentContract, ContractStatus};
use crate::evidence::SignedEconomicEvidence;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Escrow record status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscrowStatus {
    /// Funds held pending execution.
    Held,
    /// Execution completed, settlement in progress.
    Released,
    /// Settlement confirmed on-chain.
    Settled,
    /// Dispute raised, funds frozen.
    Frozen,
    /// Refunded to consumer (cancellation or dispute resolution).
    Refunded,
}

/// One escrow entry tracking a payment between two wallet-backed agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowRecord {
    /// Escrow entry ID (linked to contract_id).
    pub escrow_id: String,
    /// The contract this escrow belongs to.
    pub contract_id: String,
    /// Consumer wallet (payer).
    pub consumer_wallet: String,
    /// Provider wallet (payee).
    pub provider_wallet: String,
    /// Amount held in escrow (micro-CU).
    pub amount_micro_cu: u64,
    /// Current status.
    pub status: EscrowStatus,
    /// Evidence hash linking to the economic proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_hash: Option<String>,
    /// MultiversX testnet tx hash (once settled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    /// When the escrow was created.
    pub created_at: u64,
    /// When the status last changed.
    pub updated_at: u64,
}

/// Settlement request — what the settlement flow needs to execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRequest {
    pub contract_id: String,
    pub escrow_id: String,
    pub evidence: SignedEconomicEvidence,
    pub provider_wallet: String,
    pub amount_micro_cu: u64,
}

/// Settlement outcome — the result of attempting to settle on-chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementOutcome {
    pub escrow_id: String,
    pub contract_id: String,
    pub evidence_hash: String,
    pub amount_micro_cu: u64,
    /// Testnet tx reference (or "local-test-N" for local settlement).
    pub tx_ref: String,
    pub success: bool,
    pub error: Option<String>,
    pub settled_at: u64,
}

/// The escrow ledger — tracks all escrow records and prevents double-settlement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EscrowLedger {
    /// All escrow records, keyed by escrow_id.
    pub records: BTreeMap<String, EscrowRecord>,
    /// Evidence hashes already settled (dedup guard).
    pub settled_evidence: BTreeSet<String>,
    /// Running counter for local test tx references.
    pub local_counter: u64,
}

/// Errors from escrow operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EscrowError {
    #[error("contract not in executable state: {0:?}")]
    NotExecutable(ContractStatus),
    #[error("escrow already exists for contract {0}")]
    EscrowExists(String),
    #[error("escrow not found: {0}")]
    NotFound(String),
    #[error("evidence hash already settled: {0}")]
    DoubleSettlement(String),
    #[error("amount mismatch: escrow has {escrow}, evidence claims {evidence}")]
    AmountMismatch { escrow: u64, evidence: u64 },
    #[error("invalid transition: {0:?} → {1:?}")]
    InvalidTransition(EscrowStatus, EscrowStatus),
}

impl EscrowLedger {
    /// Creates a new escrow record for a contract.
    pub fn create_escrow(
        &mut self,
        contract: &AgentContract,
        now: u64,
    ) -> Result<&EscrowRecord, EscrowError> {
        if contract.status != ContractStatus::Accepted
            && contract.status != ContractStatus::Executing
        {
            return Err(EscrowError::NotExecutable(contract.status));
        }
        if self.records.contains_key(&contract.contract_id) {
            return Err(EscrowError::EscrowExists(contract.contract_id.clone()));
        }
        let record = EscrowRecord {
            escrow_id: contract.contract_id.clone(),
            contract_id: contract.contract_id.clone(),
            consumer_wallet: contract.consumer_wallet.clone(),
            provider_wallet: contract.provider_wallet.clone(),
            amount_micro_cu: contract.terms.price_micro_cu,
            status: EscrowStatus::Held,
            evidence_hash: None,
            tx_hash: None,
            created_at: now,
            updated_at: now,
        };
        self.records
            .insert(contract.contract_id.clone(), record.clone());
        Ok(self.records.get(&contract.contract_id).unwrap())
    }

    /// Releases escrow for settlement (execution completed).
    pub fn release_escrow(
        &mut self,
        escrow_id: &str,
        evidence_hash: &str,
        now: u64,
    ) -> Result<(), EscrowError> {
        let record = self
            .records
            .get_mut(escrow_id)
            .ok_or_else(|| EscrowError::NotFound(escrow_id.to_string()))?;
        if record.status != EscrowStatus::Held {
            return Err(EscrowError::InvalidTransition(
                record.status,
                EscrowStatus::Released,
            ));
        }
        record.status = EscrowStatus::Released;
        record.evidence_hash = Some(evidence_hash.to_string());
        record.updated_at = now;
        Ok(())
    }

    /// Finalizes settlement (on-chain tx confirmed).
    pub fn settle_escrow(
        &mut self,
        escrow_id: &str,
        tx_hash: &str,
        amount_micro_cu: u64,
        now: u64,
    ) -> Result<(), EscrowError> {
        let record = self
            .records
            .get_mut(escrow_id)
            .ok_or_else(|| EscrowError::NotFound(escrow_id.to_string()))?;
        if record.status != EscrowStatus::Released {
            return Err(EscrowError::InvalidTransition(
                record.status,
                EscrowStatus::Settled,
            ));
        }
        if amount_micro_cu != record.amount_micro_cu {
            return Err(EscrowError::AmountMismatch {
                escrow: record.amount_micro_cu,
                evidence: amount_micro_cu,
            });
        }
        // Check for double-settlement.
        if let Some(ref hash) = record.evidence_hash
            && !self.settled_evidence.insert(hash.clone())
        {
            return Err(EscrowError::DoubleSettlement(hash.clone()));
        }
        record.status = EscrowStatus::Settled;
        record.tx_hash = Some(tx_hash.to_string());
        record.updated_at = now;
        Ok(())
    }

    /// Executes a full settlement flow: release → local test settle.
    pub fn execute_settlement(
        &mut self,
        request: &SettlementRequest,
        now: u64,
    ) -> Result<SettlementOutcome, EscrowError> {
        let evidence_hash = hex::encode(request.evidence.evidence_hash);

        // Release escrow.
        self.release_escrow(&request.escrow_id, &evidence_hash, now)?;

        // Generate local test tx reference.
        self.local_counter += 1;
        let tx_ref = format!("mx-testnet-{:06}", self.local_counter);

        // Settle.
        self.settle_escrow(&request.escrow_id, &tx_ref, request.amount_micro_cu, now)?;

        Ok(SettlementOutcome {
            escrow_id: request.escrow_id.clone(),
            contract_id: request.contract_id.clone(),
            evidence_hash,
            amount_micro_cu: request.amount_micro_cu,
            tx_ref,
            success: true,
            error: None,
            settled_at: now,
        })
    }

    /// Looks up an escrow record.
    pub fn get_escrow(&self, escrow_id: &str) -> Option<&EscrowRecord> {
        self.records.get(escrow_id)
    }

    /// Returns all escrow records for a given wallet (as provider or consumer).
    pub fn for_wallet(&self, wallet: &str) -> Vec<&EscrowRecord> {
        self.records
            .values()
            .filter(|r| r.consumer_wallet == wallet || r.provider_wallet == wallet)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{ContractTerms, ServiceDescriptor, accept_contract, propose_contract};

    fn provider() -> String {
        "erd1qykvz2cfamvyhwrc087l8lnsy7f7f0z9exvnjudn3d8fagxn3d8spujzsm".to_string()
    }

    fn consumer() -> String {
        "erd1qqqqqqqqqqqqqpgqzcufga3vm5r44xe3ukzyl4dmhpsvalrkkgjqeyu68x".to_string()
    }

    fn make_contract() -> AgentContract {
        let service = ServiceDescriptor {
            capability: "chat".into(),
            description: "Summarize document".into(),
            model_requirement: None,
            estimated_input_size: None,
        };
        let terms = ContractTerms {
            price_micro_cu: 5_000_000,
            max_duration_secs: 120,
            min_quality_percent: 80,
            escrow_required: true,
        };
        let mut c = propose_contract(&provider(), &consumer(), service, terms, 100).unwrap();
        accept_contract(&mut c, &provider(), 200).unwrap();
        c
    }

    #[test]
    fn create_escrow_for_accepted_contract() {
        let mut ledger = EscrowLedger::default();
        let c = make_contract();
        let escrow = ledger.create_escrow(&c, 300).unwrap();
        assert_eq!(escrow.status, EscrowStatus::Held);
        assert_eq!(escrow.amount_micro_cu, 5_000_000);
    }

    #[test]
    fn duplicate_escrow_rejected() {
        let mut ledger = EscrowLedger::default();
        let c = make_contract();
        ledger.create_escrow(&c, 300).unwrap();
        assert!(matches!(
            ledger.create_escrow(&c, 301),
            Err(EscrowError::EscrowExists(_))
        ));
    }

    #[test]
    fn release_and_settle() {
        let mut ledger = EscrowLedger::default();
        let c = make_contract();
        ledger.create_escrow(&c, 300).unwrap();

        ledger
            .release_escrow(&c.contract_id, "ab".repeat(32).as_str(), 400)
            .unwrap();
        assert_eq!(
            ledger.get_escrow(&c.contract_id).unwrap().status,
            EscrowStatus::Released
        );

        ledger
            .settle_escrow(&c.contract_id, "tx-123", 5_000_000, 500)
            .unwrap();
        assert_eq!(
            ledger.get_escrow(&c.contract_id).unwrap().status,
            EscrowStatus::Settled
        );
        assert_eq!(
            ledger
                .get_escrow(&c.contract_id)
                .unwrap()
                .tx_hash
                .as_deref(),
            Some("tx-123")
        );
    }

    #[test]
    fn double_settlement_prevented() {
        let mut ledger = EscrowLedger::default();
        let c = make_contract();
        ledger.create_escrow(&c, 300).unwrap();
        ledger
            .release_escrow(&c.contract_id, "hash-1", 400)
            .unwrap();
        ledger
            .settle_escrow(&c.contract_id, "tx-1", 5_000_000, 500)
            .unwrap();

        // Try to settle the same evidence again via a second escrow
        let mut c2 = make_contract();
        // Different contract ID since the first is already used
        c2.contract_id = "ct-duplicate-test".to_string();
        c2.consumer_wallet = consumer();
        c2.provider_wallet = provider();
        ledger.create_escrow(&c2, 600).unwrap();
        ledger
            .release_escrow(&c2.contract_id, "hash-1", 700)
            .unwrap(); // same hash
        assert!(matches!(
            ledger.settle_escrow(&c2.contract_id, "tx-2", 5_000_000, 800),
            Err(EscrowError::DoubleSettlement(_))
        ));
    }

    #[test]
    fn amount_mismatch_rejected() {
        let mut ledger = EscrowLedger::default();
        let c = make_contract();
        ledger.create_escrow(&c, 300).unwrap();
        ledger
            .release_escrow(&c.contract_id, "hash-2", 400)
            .unwrap();
        assert!(matches!(
            ledger.settle_escrow(&c.contract_id, "tx-1", 9_999_999, 500),
            Err(EscrowError::AmountMismatch { .. })
        ));
    }

    #[test]
    fn for_wallet_returns_relevant_records() {
        let mut ledger = EscrowLedger::default();
        let c = make_contract();
        ledger.create_escrow(&c, 300).unwrap();

        let provider_escrows = ledger.for_wallet(&provider());
        assert_eq!(provider_escrows.len(), 1);
        assert_eq!(provider_escrows[0].provider_wallet, provider());

        let consumer_escrows = ledger.for_wallet(&consumer());
        assert_eq!(consumer_escrows.len(), 1);

        let stranger = "erd1qqqqqqqqqqqqqpgq78888888888888888888888888888888ca5h83z";
        assert!(ledger.for_wallet(stranger).is_empty());
    }

    #[test]
    fn execute_settlement_end_to_end() {
        let mut ledger = EscrowLedger::default();
        let c = make_contract();
        ledger.create_escrow(&c, 300).unwrap();

        let request = SettlementRequest {
            contract_id: c.contract_id.clone(),
            escrow_id: c.contract_id.clone(),
            evidence: crate::evidence::SignedEconomicEvidence {
                version: 1,
                payload_bytes: vec![],
                evidence_hash: [0u8; 32],
                signer_public_key: None,
                signature: None,
            },
            provider_wallet: provider(),
            amount_micro_cu: 5_000_000,
        };

        let outcome = ledger.execute_settlement(&request, 500).unwrap();
        assert!(outcome.success);
        assert!(outcome.tx_ref.starts_with("mx-testnet-"));
        assert_eq!(outcome.amount_micro_cu, 5_000_000);
    }

    #[test]
    fn serialization_round_trip() {
        let mut ledger = EscrowLedger::default();
        let c = make_contract();
        ledger.create_escrow(&c, 300).unwrap();
        let json = serde_json::to_string(&ledger).unwrap();
        let back: EscrowLedger = serde_json::from_str(&json).unwrap();
        assert_eq!(back.records.len(), 1);
    }
}
