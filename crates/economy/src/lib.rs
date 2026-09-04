//! DecentraAI economic foundation (Model Colony era) — deterministic,
//! versioned, evidence-gated economics.
//!
//! # The chain this crate implements
//!
//! ```text
//! VERIFIED COMPUTE
//!   → VERIFIED CONTRIBUTION   (facts, only with an evidence reference)
//!   → DETERMINISTIC ECONOMIC VALUE  (Contribution Units, integer math)
//!   → CRYPTOGRAPHIC PROOF     (bridges the existing Ed25519 receipts)
//!   → TRUST ANCHOR            (wallet-backed, verifiable, chained)
//!   → AGENT CONTRACT          (provider ↔ consumer, terms, settlement)
//!   → ESCROW                  (hold → release → settle)
//!   → SETTLEMENT              (MultiversX testnet anchoring)
//! ```
//!
//! # Hard rules (enforced by construction, tested per rule)
//!
//! - No LLM ever determines a reward: every function here is pure over
//!   recorded facts.
//! - The formula is VERSIONED ([`ECONOMICS_VERSION`]): changing weights is a
//!   new version, never a silent mutation of history.
//! - Integer-only math (basis points / micro-CU) — bit-exact reproduction.
//! - Rewards require verification status `verified`; anything else pays 0.
//! - This crate never holds private keys, never talks to a network, never
//!   launches a token. Settlement is an adapter trait with local test impls.

pub mod contract;
pub mod contribution;
pub mod dcai_esdt;
pub mod engine;
pub mod escrow;
pub mod evidence;
#[cfg(test)]
pub mod governance_invariants;
pub mod multiversx_devnet;
pub mod multiversx_identity;
pub mod multiversx_tx;
pub mod settlement;
pub mod signer;
pub mod tokenomics;
pub mod trust_anchor;

use thiserror::Error;

/// Economic-layer errors — all recoverable, all explainable.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EconomyError {
    #[error("verification gate: only verified contributions earn")]
    NotVerified,
    #[error("an award requires an evidence reference")]
    MissingEvidence,
    #[error("unknown worker '{worker_id}'")]
    UnknownWorker { worker_id: String },
    #[error("self-verification rejected: worker '{worker_id}' cannot verify its own work")]
    SelfVerification { worker_id: String },
    #[error("duplicate evidence '{evidence_ref}' for worker '{worker_id}' — replay rejected")]
    DuplicateEvidence {
        worker_id: String,
        evidence_ref: String,
    },
}

/// Version of the Contribution Unit formula. Bump on ANY weight/rule change;
/// historical awards stay explainable under the version that produced them.
pub const ECONOMICS_VERSION: u32 = 2;

/// Basis points denominator (100 % = 10_000 bps).
pub const BPS: u64 = 10_000;
