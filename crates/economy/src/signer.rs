//! Phase 7 wallet signing — the missing layer between [`UnsignedTxIntent`]
//! and a MultiversX testnet submission.
//!
//! # What this module DOES and does NOT do
//!
//! DOES:
//! - define the [`TransactionSigner`] trait (sign opaque bytes, expose only
//!   the verifying key);
//! - build the deterministic [`canonical_sign_payload`] an operator signs;
//! - load an [`Ed25519Signer`] from operator-held secret injection
//!   (`DECENTRAAI_MX_SIGNER_HEX_FILE` winning over `DECENTRAAI_MX_SIGNER_HEX`).
//!
//! DOES NOT:
//! - hold keys in the repo, in logs, in API responses, or in memory dumps;
//! - complete the on-chain envelope (`nonce`, `gas_limit`, `receiver`
//!   contract address stay with the operator tooling until VERIFIED);
//! - submit anything to any network (see `multiversx_devnet`).
//!
//! # Key separation (never collapsed)
//!
//! node Ed25519 key (receipts + agent auth) ≠ wallet key (funds txs,
//! operator-held) ≠ validator role. Signing here always uses the WALLET
//! identity (`gzeu-wallet`), never the node identity.
//!
//! [`UnsignedTxIntent`]: crate::multiversx_tx::UnsignedTxIntent

use crate::multiversx_tx::UnsignedTxIntent;
use bech32::{ToBase32 as _, Variant, encode};
use ed25519_dalek::{Signer as _, Verifier as _};
use std::fmt;

/// Errors for signer loading and signing. Variants NEVER carry key material —
/// paths and env var NAMES are safe to log, VALUES never are.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignerError {
    #[error("payload must not be empty")]
    EmptyPayload,
    #[error("seed must be 64 hex chars decoding to 32 bytes")]
    InvalidSeedHex,
    #[error("signer seed file unreadable (check DECENTRAAI_MX_SIGNER_HEX_FILE path/permissions)")]
    SeedFileUnreadable,
    #[error("signer seed file is empty")]
    SeedFileEmpty,
    #[error("no signer configured (set DECENTRAAI_MX_SIGNER_HEX_FILE or DECENTRAAI_MX_SIGNER_HEX)")]
    NotConfigured,
    #[error("signature verification failed")]
    VerificationFailed,
}

/// Env var holding the raw 32-byte seed as 64 hex chars (operator-injected).
pub const SIGNER_HEX_ENV: &str = "DECENTRAAI_MX_SIGNER_HEX";
/// Env var holding a PATH to a file with the hex seed (preferred: 0600 file).
pub const SIGNER_HEX_FILE_ENV: &str = "DECENTRAAI_MX_SIGNER_HEX_FILE";

fn opt_field(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("")
}

/// Deterministic preparation payload for an [`UnsignedTxIntent`].
///
/// Fixed field order, `|`-separated, lowercase hex where applicable:
/// `mx-sign/1|network|endpoint|data|value|sender|receiver|chain_id`.
///
/// This is the PREPARATION shape the operator's wallet tooling signs. It is
/// NOT the final on-chain envelope (nonce/gas/receiver stay unset until the
/// operator completes them against VERIFIED contract addresses).
pub fn canonical_sign_payload(intent: &UnsignedTxIntent) -> Vec<u8> {
    format!(
        "mx-sign/1|{}|{}|{}|{}|{}|{}|{}",
        intent.network,
        intent.endpoint,
        intent.data_field(),
        intent.value_denomination,
        opt_field(&intent.sender),
        opt_field(&intent.receiver),
        opt_field(&intent.chain_id),
    )
    .into_bytes()
}

/// Wallet signing capability. Implementations hold the seed in memory only,
/// never log it, never serialize it.
pub trait TransactionSigner {
    /// Sign opaque payload bytes, returning the raw 64-byte signature.
    fn sign_bytes(&self, payload: &[u8]) -> Result<[u8; 64], SignerError>;
    /// The public half — safe to log, store, and compare.
    fn verifying_key_bytes(&self) -> [u8; 32];
    /// Hex signature convenience wrapper.
    fn sign_hex(&self, payload: &[u8]) -> Result<String, SignerError> {
        Ok(hex::encode(self.sign_bytes(payload)?))
    }
}

/// Ed25519 wallet signer. `Debug` is manually redacted: only the public key
/// ever appears in formatted output.
pub struct Ed25519Signer {
    signing_key: ed25519_dalek::SigningKey,
}

impl fmt::Debug for Ed25519Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ed25519Signer")
            .field("verifying_key", &hex::encode(self.verifying_key_bytes()))
            .field("seed", &"<redacted>")
            .finish()
    }
}

impl Ed25519Signer {
    /// Build from a raw 32-byte seed. The caller owns zeroization of `seed`.
    pub fn from_seed_bytes(seed: &[u8; 32]) -> Self {
        Self {
            signing_key: ed25519_dalek::SigningKey::from_bytes(seed),
        }
    }

    /// Build from 64 lowercase/uppercase hex chars (whitespace-trimmed).
    /// The input string is NOT logged on error — only its length class.
    pub fn from_seed_hex(hex_str: &str) -> Result<Self, SignerError> {
        let trimmed = hex_str.trim();
        let raw = hex::decode(trimmed).map_err(|_| SignerError::InvalidSeedHex)?;
        if raw.len() != 32 {
            return Err(SignerError::InvalidSeedHex);
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&raw);
        let signer = Self::from_seed_bytes(&seed);
        // Best-effort: wipe the local copy; the caller's buffer is theirs.
        seed.fill(0);
        Ok(signer)
    }

    /// Verify a signature against this signer's public key.
    pub fn verify(&self, payload: &[u8], signature: &[u8; 64]) -> Result<(), SignerError> {
        let sig = ed25519_dalek::Signature::from_bytes(signature);
        self.signing_key
            .verifying_key()
            .verify(payload, &sig)
            .map_err(|_| SignerError::VerificationFailed)
    }
}

impl TransactionSigner for Ed25519Signer {
    fn sign_bytes(&self, payload: &[u8]) -> Result<[u8; 64], SignerError> {
        if payload.is_empty() {
            return Err(SignerError::EmptyPayload);
        }
        Ok(self.signing_key.sign(payload).to_bytes())
    }

    fn verifying_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }
}

/// Derive the MultiversX bech32 (`erd1…`) address for a verifying key.
/// Pure function of public bytes — safe to log, store, and compare.
pub fn bech32_address(verifying_key: &[u8; 32]) -> String {
    encode("erd", verifying_key.to_base32(), Variant::Bech32)
        .expect("bech32 encoding of 32 bytes never fails")
}

/// Load the operator wallet signer from secret injection.
///
/// Precedence: `DECENTRAAI_MX_SIGNER_HEX_FILE` (0600 file with hex seed)
/// over `DECENTRAAI_MX_SIGNER_HEX` (hex seed directly). Error variants name
/// the VAR/PATH only — values are never read into error strings.
pub fn load_signer_from_env() -> Result<Ed25519Signer, SignerError> {
    if let Ok(path) = std::env::var(SIGNER_HEX_FILE_ENV) {
        let content =
            std::fs::read_to_string(path.trim()).map_err(|_| SignerError::SeedFileUnreadable)?;
        if content.trim().is_empty() {
            return Err(SignerError::SeedFileEmpty);
        }
        return Ed25519Signer::from_seed_hex(&content);
    }
    match std::env::var(SIGNER_HEX_ENV) {
        Ok(v) => Ed25519Signer::from_seed_hex(&v),
        Err(_) => Err(SignerError::NotConfigured),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution::{ContributionFacts, VerificationStatus};
    use crate::evidence::EconomicEvidence;
    use crate::multiversx_tx::Mx8004TxBuilder;

    /// TEST-ONLY seed. No funds, no mainnet, never a real wallet.
    const TEST_SEED_HEX: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";

    fn test_intent() -> UnsignedTxIntent {
        let facts = ContributionFacts {
            worker_id: "w-signer".to_string(),
            verified_units: 1,
            quality_percent: 100,
            reliability_percent: 100,
            latency_ms: 5,
            baseline_latency_ms: 10,
            resource_bytes: 64,
            efficiency_index_x100: 100,
            scarcity_bps: 10_000,
            difficulty_bps: 10_000,
            verification: VerificationStatus::Verified,
            evidence_ref: "ev-signer-1".to_string(),
            verifier_id: "verifier-test".to_string(),
        };
        let ev = EconomicEvidence::from_facts(&facts).unwrap();
        Mx8004TxBuilder::submit_proof("job-signer-1", &ev).unwrap()
    }

    #[test]
    fn canonical_payload_is_deterministic_and_sender_sensitive() {
        let a = test_intent();
        let mut b = a.clone();
        let p1 = canonical_sign_payload(&a);
        let p2 = canonical_sign_payload(&a);
        assert_eq!(p1, p2);
        assert!(p1.starts_with(b"mx-sign/1|multiversx-testnet|submit_proof|"));
        b.sender = Some("erd1sender".to_string());
        assert_ne!(p1, canonical_sign_payload(&b));
    }

    #[test]
    fn sign_verify_roundtrip_with_test_seed() {
        let signer = Ed25519Signer::from_seed_hex(TEST_SEED_HEX).unwrap();
        let payload = canonical_sign_payload(&test_intent());
        let sig = signer.sign_bytes(&payload).unwrap();
        assert!(signer.verify(&payload, &sig).is_ok());
        let mut tampered = payload.clone();
        tampered.push(b'x');
        assert_eq!(
            signer.verify(&tampered, &sig),
            Err(SignerError::VerificationFailed)
        );
    }

    #[test]
    fn sign_hex_matches_raw_bytes() {
        let signer = Ed25519Signer::from_seed_hex(TEST_SEED_HEX).unwrap();
        let payload = b"probe";
        let raw = signer.sign_bytes(payload).unwrap();
        assert_eq!(signer.sign_hex(payload).unwrap(), hex::encode(raw));
    }

    #[test]
    fn empty_payload_rejected() {
        let signer = Ed25519Signer::from_seed_hex(TEST_SEED_HEX).unwrap();
        assert_eq!(signer.sign_bytes(&[]), Err(SignerError::EmptyPayload));
    }

    #[test]
    fn bad_seed_hex_rejected_without_echo() {
        for bad in ["zz", "abcd", &"ab".repeat(31), &"ab".repeat(33)] {
            let err = Ed25519Signer::from_seed_hex(bad).unwrap_err();
            assert_eq!(err, SignerError::InvalidSeedHex);
            assert!(!err.to_string().contains(bad));
        }
        // Empty input rejected too (empty string is trivially "contained",
        // so it gets its own assertion without the echo check).
        assert_eq!(
            Ed25519Signer::from_seed_hex("").unwrap_err(),
            SignerError::InvalidSeedHex
        );
    }

    #[test]
    fn bech32_address_is_stable_erd1_and_key_bound() {
        let a = Ed25519Signer::from_seed_hex(TEST_SEED_HEX).unwrap();
        let addr1 = bech32_address(&a.verifying_key_bytes());
        let addr2 = bech32_address(&a.verifying_key_bytes());
        assert_eq!(addr1, addr2);
        assert!(addr1.starts_with("erd1"), "unexpected: {addr1}");
        assert_eq!(addr1.len(), 62);
        let other = Ed25519Signer::from_seed_hex(&"ab".repeat(32)).unwrap();
        assert_ne!(addr1, bech32_address(&other.verifying_key_bytes()));
    }

    #[test]
    fn debug_never_leaks_seed() {
        let signer = Ed25519Signer::from_seed_hex(TEST_SEED_HEX).unwrap();
        let dbg = format!("{signer:?}");
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains(TEST_SEED_HEX));
        // Public key IS visible (safe by design).
        assert!(dbg.contains(&hex::encode(signer.verifying_key_bytes())));
    }
}
