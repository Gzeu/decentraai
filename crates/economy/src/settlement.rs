//! Chain-agnostic settlement (Phases 6–7): an adapter INTERFACE only.
//!
//! ```text
//! VerifiedContribution (EconomicEvidence, signed)
//!         ↓
//! SettlementRecord      (what a ledger needs to know — nothing more)
//!         ↓
//! BlockchainAdapter     (replaceable; today: LocalTestAdapter)
//!         ↓
//! transaction reference
//! ```
//!
//! # Non-negotiables
//!
//! - The core fabric operates with NO blockchain at all: the default
//!   adapter is a deterministic local test sink.
//! - No mainnet, no wallets created automatically, no private keys in this
//!   repository. Future implementations hold keys in external secret
//!   stores; the traits here deliberately never expose key material.
//! - Adapters are REPLACEABLE: everything downstream sees only
//!   [`SettlementReceipt`].
//!
//! # Phase 7 future interfaces
//!
//! [`WalletIdentity`], [`TransactionSigner`], [`BalanceQuery`] and the
//! settlement/balance/fee concepts below are declared for forward
//! compatibility. They have NO production implementation yet by design —
//! only deterministic local/test code exists.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// One settled economic fact: who earned how much, anchored to which proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementRecord {
    /// Worker identity (attribution).
    pub worker_id: String,
    /// Awarded amount in micro-CU — matches the signed evidence exactly.
    pub amount_micro_cu: u64,
    /// BLAKE3 anchor of the canonical economic evidence payload.
    pub evidence_hash: [u8; 32],
    /// Formula version that produced the amount.
    pub cu_version: u32,
    /// Economic epoch this settlement belongs to (simulator/time bucketing).
    pub epoch: u64,
}

/// The adapter's answer. `tx_ref` is opaque downstream — its format belongs
/// to the adapter implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementReceipt {
    pub adapter: String,
    pub tx_ref: String,
    pub accepted: bool,
}

/// Errors an adapter may return. Deliberately coarse: adapters wrap foreign
/// systems whose error types we must not leak into core economics.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettlementError {
    #[error("adapter '{0}' rejected the settlement")]
    Rejected(String),
}

/// THE extension point for any future chain/L2/database sink.
///
/// Implementations must be deterministic in their acceptance decision and
/// must never require network access to be constructible (test adapters
/// exist so the economy can always run fully offline).
pub trait BlockchainAdapter: Send + Sync {
    /// Stable adapter name (appears inside every receipt).
    fn name(&self) -> &'static str;

    /// Submits one settlement record. Implementations decide acceptance;
    /// they MUST NOT mutate economics — this is bookkeeping transport only.
    fn submit_settlement(
        &self,
        record: &SettlementRecord,
    ) -> Result<SettlementReceipt, SettlementError>;
}

/// Deterministic local/test sink: accepts every well-formed record and
/// issues sequential references (`local-test-000001`, …). Proves the whole
/// economic pipeline runs with zero blockchain present.
#[derive(Default)]
pub struct LocalTestAdapter {
    counter: Mutex<u64>,
}

impl BlockchainAdapter for LocalTestAdapter {
    fn name(&self) -> &'static str {
        "local-test"
    }

    fn submit_settlement(
        &self,
        record: &SettlementRecord,
    ) -> Result<SettlementReceipt, SettlementError> {
        if record.amount_micro_cu == 0 && record.evidence_hash == [0u8; 32] {
            // A zero-amount settlement with a null anchor is malformed noise.
            return Err(SettlementError::Rejected(self.name().to_string()));
        }
        let mut n = self.counter.lock().expect("local test adapter lock");
        *n += 1;
        Ok(SettlementReceipt {
            adapter: self.name().to_string(),
            tx_ref: format!("local-test-{n:06}"),
            accepted: true,
        })
    }
}

// ---------------------------------------------------------------------------
// Phase 7 — FUTURE interfaces. Declared, documented, NOT implemented for
// production. Any real implementation must keep keys OUT of this repository.
// ---------------------------------------------------------------------------

/// Wallet-bound identity for future chains: an address derived from (or
/// linked to) a node identity. No key material lives behind this trait.
pub trait WalletIdentity: Send + Sync {
    fn address(&self) -> &str;
}

/// Signs arbitrary payloads for a future chain. IMPLEMENTATION NOTE: real
/// signers load keys from a secret manager at call time; nothing here ever
/// stores or transmits private keys.
pub trait TransactionSigner: Send + Sync {
    fn sign_payload(&self, payload: &[u8]) -> Vec<u8>;
}

/// Read-only balance view for a future token layer. Returns `None` when the
/// address is unknown — callers must treat unknown as zero, never as error.
pub trait BalanceQuery: Send + Sync {
    fn balance_micro_cu(&self, address: &str) -> Option<u64>;
}

/// Network fee concept for a future chain: quoted BEFORE submission so the
/// economy can account costs deterministically.
pub trait NetworkFeeQuote: Send + Sync {
    fn quote_fee_micro_cu(&self, record: &SettlementRecord) -> u64;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(amount: u64) -> SettlementRecord {
        SettlementRecord {
            worker_id: "w".into(),
            amount_micro_cu: amount,
            evidence_hash: [9u8; 32],
            cu_version: 2,
            epoch: 1,
        }
    }

    #[test]
    fn local_adapter_is_deterministic_and_replaceable() {
        let adapter = LocalTestAdapter::default();
        let r1 = adapter.submit_settlement(&record(100)).unwrap();
        let r2 = adapter.submit_settlement(&record(200)).unwrap();
        assert!(r1.accepted && r2.accepted);
        assert_eq!(r1.tx_ref, "local-test-000001");
        assert_eq!(r2.tx_ref, "local-test-000002");
        assert_eq!(r1.adapter, "local-test");

        // Replaceability: another impl answers through the same interface —
        // downstream code only ever sees SettlementReceipt.
        struct SecondAdapter;
        impl BlockchainAdapter for SecondAdapter {
            fn name(&self) -> &'static str {
                "second"
            }
            fn submit_settlement(
                &self,
                _r: &SettlementRecord,
            ) -> Result<SettlementReceipt, SettlementError> {
                Ok(SettlementReceipt {
                    adapter: "second".into(),
                    tx_ref: "x-1".into(),
                    accepted: true,
                })
            }
        }
        let r = SecondAdapter.submit_settlement(&record(1)).unwrap();
        assert_eq!(r.adapter, "second");
    }

    #[test]
    fn malformed_zero_records_are_rejected() {
        let adapter = LocalTestAdapter::default();
        let mut bad = record(0);
        bad.evidence_hash = [0u8; 32];
        assert!(adapter.submit_settlement(&bad).is_err());
        // A legitimate zero-amount correction WITH a real anchor is fine.
        bad.evidence_hash = [1u8; 32];
        assert!(adapter.submit_settlement(&bad).is_ok());
    }
}
