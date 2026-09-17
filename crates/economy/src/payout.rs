//! Payout redemption — converting compensation earnings into a settled
//! payout record plus matching quota movement.
//!
//! # Flow
//!
//! ```text
//! CompensationLedger (earned − redeemed = redeemable)
//!   → validate(destination erd1, network, amount ≤ redeemable, quota covers)
//!     → CompensationLedger.redeem (earned NEVER decreases; redeemed += amount)
//!       → QuotaLedger moves available → consumed on the destination account
//!         → PayoutRecord { settlement: "ledger", tx_hash: "" }
//! ```
//!
//! # Rules (agreed §7 `payout` spec)
//!
//! - `earned` is monotonic ("total ever credited") and is NEVER zeroed or
//!   decreased. Redemption is tracked in a separate `redeemed` counter;
//!   `redeemable = earned − redeemed`. Old snapshots load `redeemed = 0`.
//! - Payouts settle **in-ledger only**: `settlement: "ledger"`,
//!   `network: "none"`, `tx_hash: ""`. No chain broadcast happens here.
//! - `mainnet` is refused (`invalid_network`): this node never signs for
//!   mainnet. Only `multiversx-testnet` is accepted as a network label,
//!   and even then nothing is broadcast.
//! - Dust floor: amounts `< MIN_PAYOUT_MICRO_CU` are refused
//!   (`payout_below_minimum`), nothing moves.
//! - All amounts are micro-CU (integer-only).

use bech32::primitives::decode::CheckedHrpstring;
use bech32::Bech32;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Dust floor: the smallest redeemable payout (micro-CU).
pub const MIN_PAYOUT_MICRO_CU: u64 = 1;

/// Payout lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutStatus {
    /// Recorded, not yet applied (never emitted by the atomic path, which
    /// commits synchronously — kept so readers can represent it).
    Pending,
    /// Applied to the ledgers, no chain broadcast (the only terminal
    /// status this node produces).
    Sent,
    /// Broadcast + confirmed on-chain. This node never emits it today
    /// (no custody); readers must treat it as "someone else settled".
    Confirmed,
}

/// One payout redemption record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayoutRecord {
    /// Stable id (`po-…`, uuid v4 hex prefix).
    pub payout_id: String,
    /// Amount redeemed (micro-CU).
    pub amount_micro_cu: u64,
    /// Destination wallet (validated `erd1…` bech32, 32 bytes).
    pub destination: String,
    /// Accepted network label (`multiversx-testnet`; `mainnet` refused).
    pub network: String,
    /// Chain tx hash. Always `""` for ledger settlement (nothing broadcast).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tx_hash: String,
    /// Settlement plane. Always `"ledger"` from this node.
    pub settlement: String,
    /// Lifecycle status (`sent` for ledger commits).
    pub status: PayoutStatus,
    /// Compensation account redeemed from (== destination).
    pub redeemed_from: String,
    /// Creation timestamp (unix seconds).
    pub created_at: u64,
}

/// The payout ledger — redemption records keyed by payout id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PayoutLedger {
    /// All payout records, keyed by payout id.
    pub records: BTreeMap<String, PayoutRecord>,
}

/// Payout validation/execution errors. Display strings are the stable
/// machine-readable tokens from the §1.4 error contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PayoutError {
    #[error("payout_below_minimum: amount must be at least {0} micro-CU")]
    BelowMinimum(u64),
    #[error("invalid_destination: {0}")]
    InvalidDestination(String),
    #[error("invalid_network: {0}")]
    InvalidNetwork(String),
    #[error("insufficient_balance: {0}")]
    InsufficientBalance(String),
}

/// Validates a payout destination: strict Bech32 checksum with `erd` HRP
/// and a 32-byte payload (mirrors the wallet-auth address rule).
pub fn validate_destination(addr: &str) -> Result<(), PayoutError> {
    let checked = CheckedHrpstring::new::<Bech32>(addr)
        .map_err(|_| PayoutError::InvalidDestination(addr.to_string()))?;
    if checked.hrp().as_str() != "erd" {
        return Err(PayoutError::InvalidDestination(addr.to_string()));
    }
    let bytes: Vec<u8> = checked.byte_iter().collect();
    if bytes.len() != 32 {
        return Err(PayoutError::InvalidDestination(addr.to_string()));
    }
    Ok(())
}

/// Validates the network label. Only testnet is accepted; mainnet is
/// refused because this node holds no mainnet custody.
pub fn validate_network(network: &str) -> Result<(), PayoutError> {
    if network == "multiversx-testnet" {
        Ok(())
    } else if network == "multiversx-mainnet" {
        Err(PayoutError::InvalidNetwork(
            "mainnet payout refused: no mainnet custody on this node".to_string(),
        ))
    } else {
        Err(PayoutError::InvalidNetwork(network.to_string()))
    }
}

impl PayoutLedger {
    /// Looks up a payout record.
    pub fn get(&self, payout_id: &str) -> Option<&PayoutRecord> {
        self.records.get(payout_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> String {
        // Real erd1 bech32 addresses (shared golden vectors with the
        // wallet-auth tests — synthetic strings fail checksums).
        "erd154tm8nu953lnen33zqxwq3mc9hz8y37mclyvmpdrszv9umcyys7q0p3f90".to_string()
    }

    #[test]
    fn real_address_validates() {
        assert!(validate_destination(&addr()).is_ok());
    }

    #[test]
    fn garbage_destination_rejected() {
        assert!(matches!(
            validate_destination("not-an-address"),
            Err(PayoutError::InvalidDestination(_))
        ));
        assert!(matches!(
            validate_destination(""),
            Err(PayoutError::InvalidDestination(_))
        ));
        // Wrong HRP (mainnet-style "erd1" is fine; "bc1" is not erd).
        assert!(matches!(
            validate_destination("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
            Err(PayoutError::InvalidDestination(_))
        ));
    }

    #[test]
    fn only_testnet_accepted() {
        assert!(validate_network("multiversx-testnet").is_ok());
        assert!(matches!(
            validate_network("multiversx-mainnet"),
            Err(PayoutError::InvalidNetwork(_))
        ));
        assert!(matches!(
            validate_network("solana"),
            Err(PayoutError::InvalidNetwork(_))
        ));
    }
}
