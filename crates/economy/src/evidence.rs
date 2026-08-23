//! Cryptographic evidence for economic claims (Phase 5).
//!
//! ```text
//! execution facts → EconomicEvidence (canonical bytes)
//!                 → BLAKE3 hash          (independent verification anchor)
//!                 → Ed25519 signature     (existing audited primitive)
//!                 → SignedEconomicEvidence
//! ```
//!
//! # Separation of concerns (never conflated)
//!
//! - **identity** — the Ed25519 verifying key inside the envelope;
//! - **authorization** — WHO may submit settlements is the caller's policy
//!   layer (RBAC/governance), not this module's business;
//! - **signature** — the `signature` field over canonical bytes;
//! - **evidence** — the [`EconomicEvidence`] payload itself;
//! - **economic accounting** — [`crate::engine::RewardEngine`].
//!
//! # No invented cryptography
//!
//! Signing/verification delegate to the existing, already-audited helpers
//! in `decentraai_agents::signed_receipt` (Ed25519 via `ed25519-dalek`) and
//! hashing uses BLAKE3 like every other integrity anchor in the fabric.
//! The extra check here is ECONOMIC: after signature verification the
//! payload is deserialized and the CU value is RECOMPUTED with the formula
//! version it claims — a validly-signed receipt carrying a wrong amount is
//! rejected anyway.

use crate::contribution::{ContributionFacts, VerificationStatus, compute_award};
use decentraai_agents::signed_receipt::{
    canonicalize_receipt, sign_receipt, verify_receipt_signature,
};
use serde::{Deserialize, Serialize};

/// Envelope version for economic evidence.
pub const ECONOMIC_EVIDENCE_VERSION: u16 = 1;

/// The canonical economic claim: facts + the award they produce under the
/// formula version they were computed with. This is what gets hashed and
/// signed — later verifiers can independently re-derive everything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicEvidence {
    /// Formula version used to compute [`Self::micro_cu`].
    pub cu_version: u32,
    /// The recorded facts (attribution, units, bands, verifier…).
    pub facts: ContributionFacts,
    /// The deterministic award implied by those facts under `cu_version`.
    pub micro_cu: u64,
}

impl EconomicEvidence {
    /// Builds evidence from facts. Rejects anything that would not pay:
    /// evidence exists to prove VALUE TRANSFERRED, so unverified work has
    /// no economic evidence to carry.
    pub fn from_facts(facts: &ContributionFacts) -> Result<Self, EvidenceError> {
        if facts.verification != VerificationStatus::Verified {
            return Err(EvidenceError::NotVerified);
        }
        if facts.evidence_ref.trim().is_empty() {
            return Err(EvidenceError::MissingEvidence);
        }
        let award = compute_award(facts);
        Ok(Self {
            cu_version: award.version,
            facts: facts.clone(),
            micro_cu: award.micro_cu,
        })
    }

    /// Canonical bytes (deterministic compact JSON — same helper the fabric
    /// receipts use, so sign and verify agree byte-for-byte).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonicalize_receipt(self)
    }

    /// BLAKE3 digest of the canonical bytes: the anchor future settlement
    /// records reference instead of the whole payload.
    pub fn evidence_hash(&self) -> [u8; 32] {
        *blake3::hash(&self.canonical_bytes()).as_bytes()
    }
}

/// Errors from building or verifying economic evidence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvidenceError {
    #[error("economic evidence requires a verified contribution")]
    NotVerified,
    #[error("economic evidence requires an evidence reference")]
    MissingEvidence,
    #[error("signature invalid: {0}")]
    BadSignature(String),
    #[error("evidence hash mismatch — payload tampered")]
    HashMismatch,
    #[error("payload is not a valid EconomicEvidence")]
    MalformedPayload,
    #[error(
        "signed amount ({signed}) does not match recomputed award ({recomputed}) — formula mismatch"
    )]
    AmountMismatch { signed: u64, recomputed: u64 },
}

/// The signed envelope: canonical payload + its BLAKE3 anchor + Ed25519
/// identity/signature. Wire-safe by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedEconomicEvidence {
    pub version: u16,
    /// Canonical payload bytes (exactly what was signed).
    pub payload_bytes: Vec<u8>,
    /// BLAKE3 over `payload_bytes` — the settlement anchor.
    pub evidence_hash: [u8; 32],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_public_key: Option<[u8; 32]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

/// Signs economic evidence with an Ed25519 signing key (32 bytes). The key
/// comes FROM THE CALLER — this crate never holds, generates or stores keys.
pub fn sign_economic_evidence(
    signing_key_bytes: &[u8; 32],
    evidence: &EconomicEvidence,
) -> SignedEconomicEvidence {
    let bytes = evidence.canonical_bytes();
    let mut signed = sign_receipt(signing_key_bytes, &bytes);
    SignedEconomicEvidence {
        version: ECONOMIC_EVIDENCE_VERSION,
        evidence_hash: evidence.evidence_hash(),
        payload_bytes: std::mem::take(&mut signed.receipt_bytes),
        signer_public_key: signed.signer_public_key,
        signature: signed.signature,
    }
}

/// Full independent verification of a signed envelope:
///
/// 1. envelope version matches;
/// 2. Ed25519 signature validates over the exact payload bytes;
/// 3. BLAKE3(payload) equals the carried `evidence_hash`;
/// 4. payload deserializes into an `EconomicEvidence`;
/// 5. the recomputed award under the claimed formula version matches the
///    signed amount — a correctly-signed but economically-wrong claim is
///    still rejected.
pub fn verify_economic_evidence(
    signed: &SignedEconomicEvidence,
) -> Result<EconomicEvidence, EvidenceError> {
    if signed.version != ECONOMIC_EVIDENCE_VERSION {
        return Err(EvidenceError::BadSignature(format!(
            "envelope version {} != supported {}",
            signed.version, ECONOMIC_EVIDENCE_VERSION
        )));
    }
    let legacy = decentraai_agents::signed_receipt::SignedComputeReceipt {
        version: decentraai_agents::signed_receipt::SIGNED_RECEIPT_VERSION,
        receipt_bytes: signed.payload_bytes.clone(),
        signer_public_key: signed.signer_public_key,
        signature: signed.signature.clone(),
    };
    verify_receipt_signature(&legacy).map_err(EvidenceError::BadSignature)?;

    let digest = *blake3::hash(&signed.payload_bytes).as_bytes();
    if digest != signed.evidence_hash {
        return Err(EvidenceError::HashMismatch);
    }

    let evidence: EconomicEvidence = serde_json::from_slice(&signed.payload_bytes)
        .map_err(|_| EvidenceError::MalformedPayload)?;

    let recomputed = compute_award(&evidence.facts);
    if recomputed.micro_cu != evidence.micro_cu || recomputed.version != evidence.cu_version {
        return Err(EvidenceError::AmountMismatch {
            signed: evidence.micro_cu,
            recomputed: recomputed.micro_cu,
        });
    }
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(worker: &str, evidence: &str) -> ContributionFacts {
        ContributionFacts {
            worker_id: worker.into(),
            verified_units: 7,
            quality_percent: 95,
            reliability_percent: 100,
            latency_ms: 900,
            baseline_latency_ms: 1000,
            resource_bytes: 4096,
            efficiency_index_x100: 100,
            scarcity_bps: 15_000,
            difficulty_bps: 10_000,
            verification: VerificationStatus::Verified,
            evidence_ref: evidence.into(),
            verifier_id: "verifier-x".into(),
        }
    }

    fn key() -> [u8; 32] {
        let mut k = [7u8; 32];
        k[0] = 42;
        k
    }

    #[test]
    fn sign_verify_round_trip_recomputes_the_amount() {
        let ev = EconomicEvidence::from_facts(&facts("w", "ev-9")).unwrap();
        assert!(ev.micro_cu > 0);
        let signed = sign_economic_evidence(&key(), &ev);
        assert_eq!(signed.evidence_hash, ev.evidence_hash());

        let back = verify_economic_evidence(&signed).unwrap();
        assert_eq!(back, ev, "round trip preserves the claim");
        assert_eq!(back.facts.worker_id, "w");
    }

    #[test]
    fn tampering_is_caught_by_hash_or_signature_or_amount() {
        let ev = EconomicEvidence::from_facts(&facts("w", "ev-tamper")).unwrap();
        let mut signed = sign_economic_evidence(&key(), &ev);

        // 1. Payload byte flipped → signature fails first.
        signed.payload_bytes[10] ^= 0x01;
        assert!(matches!(
            verify_economic_evidence(&signed),
            Err(EvidenceError::BadSignature(_))
        ));

        // 2. Valid signature but stale hash → hash gate.
        let mut signed = sign_economic_evidence(&key(), &ev);
        signed.evidence_hash[31] ^= 0x01;
        assert!(matches!(
            verify_economic_evidence(&signed),
            Err(EvidenceError::HashMismatch)
        ));

        // 3. Correctly-signed WRONG amount → the economic recheck rejects
        //    even though cryptography passes.
        let mut inflated = ev.clone();
        inflated.micro_cu *= 1_000_000;
        let forged = sign_economic_evidence(&key(), &inflated);
        assert!(matches!(
            verify_economic_evidence(&forged),
            Err(EvidenceError::AmountMismatch { .. })
        ));
    }

    #[test]
    fn unverified_work_has_no_economic_evidence() {
        let mut f = facts("w", "ev-pending");
        f.verification = VerificationStatus::Pending;
        assert!(matches!(
            EconomicEvidence::from_facts(&f),
            Err(EvidenceError::NotVerified)
        ));
        let mut no_ev = facts("w", " ");
        no_ev.evidence_ref = " ".into();
        assert!(matches!(
            EconomicEvidence::from_facts(&no_ev),
            Err(EvidenceError::MissingEvidence)
        ));
    }

    #[test]
    fn evidence_hash_is_stable_and_distinct_per_claim() {
        let a = EconomicEvidence::from_facts(&facts("w", "ev-a")).unwrap();
        let b = EconomicEvidence::from_facts(&facts("w", "ev-b")).unwrap();
        assert_eq!(a.evidence_hash(), a.evidence_hash(), "stable");
        assert_ne!(a.evidence_hash(), b.evidence_hash(), "distinct claims");
    }
}
