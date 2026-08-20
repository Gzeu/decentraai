//! Signed compute receipts — cryptographic verification for P13 ("Verified
//! Compute").
//!
//! # The problem
//!
//! [`VerifiedComputeReceipt`](crate::receipt::VerifiedComputeReceipt) is honest
//! trust/ledger data: it records that the runtime associated an execution with
//! a worker, capability, duration and verdict. But until now it carried *no
//! proof of origin* — nothing cryptographically bound the worker node to the
//! claim. That is the difference between "the ledger says so" and
//! "node X provably signed it".
//!
//! # What this module adds
//!
//! A [`SignedComputeReceipt`] wraps the plain receipt with an Ed25519 signature
//! over the receipt's **canonical bytes**, made by the worker's node identity
//! (the same Ed25519 key that derives its libp2p `PeerId`). Any other node can
//! independently verify the claim given the signer's public key. Tampering with
//! any field breaks the signature — that is the P13 "bit-flip must fail"
//! acceptance criterion.
//!
//! The construction mirrors the fabric's existing [`sign_agent_advertisement`]:
//! canonicalize the payload, sign the bytes, carry the signature + public key
//! alongside the payload. We deliberately reuse that pattern rather than invent
//! a new scheme.
//!
//! Everything here is pure (no I/O): sign/verify are functions over bytes.

use ed25519_dalek::{Signer, Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Protocol version guard so a signature from an older schema never silently
/// validates against a newer one.
pub const SIGNED_RECEIPT_VERSION: u16 = 1;

/// A compute receipt signed by the worker's node identity (Ed25519).
///
/// - `receipt_bytes`: the **canonical** serialization of the receipt payload —
///   the exact bytes that were signed. Verifying signs/compares against this, so
///   any field change invalidates the signature.
/// - `signer_public_key`: the Ed25519 public key of the node that executed + 
///   signed (derives its libp2p `PeerId`).
/// - `signature`: Ed25519 over `receipt_bytes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedComputeReceipt {
    pub version: u16,
    pub receipt_bytes: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_public_key: Option<[u8; 32]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

impl Default for SignedComputeReceipt {
    fn default() -> Self {
        Self {
            version: SIGNED_RECEIPT_VERSION,
            receipt_bytes: Vec::new(),
            signer_public_key: None,
            signature: None,
        }
    }
}

/// Serialize a receipt into the canonical bytes that must be signed. Uses a
/// single consistent serialization so sign and verify agree byte-for-byte.
pub fn canonicalize_receipt<R: Serialize>(receipt: &R) -> Vec<u8> {
    // A deterministic, compact JSON serialization. `serde_json::to_vec` with
    // no pretty-printing is deterministic for these structs (no maps), so the
    // bytes are stable across the fabric.
    serde_json::to_vec(receipt).unwrap_or_default()
}

/// Signs a canonical receipt payload with the node's Ed25519 signing key.
/// Returns the signed envelope carrying the bytes + signature + public key.
pub fn sign_receipt(
    signing_key_bytes: &[u8; 32],
    receipt_canonical_bytes: &[u8],
) -> SignedComputeReceipt {
    let signing_key = SigningKey::from_bytes(signing_key_bytes);
    let signature: Signature = signing_key.sign(receipt_canonical_bytes);
    SignedComputeReceipt {
        version: SIGNED_RECEIPT_VERSION,
        receipt_bytes: receipt_canonical_bytes.to_vec(),
        signer_public_key: Some(signing_key.verifying_key().to_bytes()),
        signature: Some(signature.to_bytes().to_vec()),
    }
}

/// Verifies a signed receipt: the version matches, a signature + public key are
/// present, and the Ed25519 signature validates against `receipt_bytes`.
///
/// Returns `Ok(())` on a valid signature, `Err(msg)` otherwise (tampered
/// payload, wrong key, missing signature, mismatched version).
pub fn verify_receipt_signature(signed: &SignedComputeReceipt) -> Result<(), String> {
    if signed.version != SIGNED_RECEIPT_VERSION {
        return Err(format!(
            "signed receipt version {} != supported {}",
            signed.version, SIGNED_RECEIPT_VERSION
        ));
    }
    let Some(pub_bytes) = signed.signer_public_key else {
        return Err("signed receipt missing public key".into());
    };
    let sig_bytes: [u8; 64] = signed
        .signature
        .as_deref()
        .and_then(|b| b.try_into().ok())
        .unwrap_or([0u8; 64]);
    let Ok(vk) = VerifyingKey::from_bytes(&pub_bytes) else {
        return Err("invalid signer public key".into());
    };
    // `Signature::from_bytes` returns a `Signature` directly in ed25519-dalek 3.x.
    let sig = Signature::from_bytes(&sig_bytes);
    vk.verify_strict(signed.receipt_bytes.as_slice(), &sig)
        .map_err(|_| "signature does not verify (tampered payload or wrong key)".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand_core::{OsRng, RngCore};

    fn key() -> ([u8; 32], [u8; 32]) {
        // Photon-free: a real Ed25519 keypair. `signing_key_bytes` is what's
        // exposed by the node identity; `public` is the verifying half.
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);
        (sk.to_bytes(), sk.verifying_key().to_bytes())
    }

    #[test]
    fn sign_then_verify_roundtrip() {
        let (sk, _pk) = key();
        let payload = b"{\"execution_id\":\"e1\"}".to_vec();
        let signed = sign_receipt(&sk, &payload);
        assert_eq!(signed.receipt_bytes, payload);
        assert_eq!(signed.version, SIGNED_RECEIPT_VERSION);
        assert!(signed.signature.is_some());
        assert!(signed.signer_public_key.is_some());
        assert_eq!(verify_receipt_signature(&signed), Ok(()));
    }

    #[test]
    fn tampered_payload_fails_verification() {
        let (sk, _pk) = key();
        let payload = b"{\"execution_id\":\"e1\"}".to_vec();
        let mut signed = sign_receipt(&sk, &payload);
        // Bit-flip: mutate the receipt payload after signing.
        signed.receipt_bytes = b"{\"execution_id\":\"e2\"}".to_vec();
        assert!(
            verify_receipt_signature(&signed).is_err(),
            "tampered payload must fail verification"
        );
    }

    #[test]
    fn wrong_signing_key_fails() {
        let (sk_a, _) = key();
        let (sk_b, _) = key();
        let payload = b"payload".to_vec();
        let mut signed = sign_receipt(&sk_a, &payload);
        // Re-sign the same bytes with a different key: the signature won't match
        // the original key it claims to be from.
        let re_signed = sign_receipt(&sk_b, &payload);
        signed.signature = re_signed.signature;
        // signer_public_key still points at key_a; signature is from key_b.
        assert!(
            verify_receipt_signature(&signed).is_err(),
            "a signature from a different key must fail"
        );
    }

    #[test]
    fn missing_signature_fails() {
        let (sk, _pk) = key();
        let payload = b"payload".to_vec();
        let mut signed = sign_receipt(&sk, &payload);
        signed.signature = None;
        assert!(verify_receipt_signature(&signed).is_err());
    }

    #[test]
    fn wrong_version_fails() {
        let (sk, _pk) = key();
        let payload = b"payload".to_vec();
        let mut signed = sign_receipt(&sk, &payload);
        signed.version = 999;
        assert!(verify_receipt_signature(&signed).is_err());
    }

    #[test]
    fn canonicalization_is_deterministic() {
        let a = canonicalize_receipt(&serde_json::json!({"x": 1, "y": [2, 3]}));
        let b = canonicalize_receipt(&serde_json::json!({"x": 1, "y": [2, 3]}));
        assert_eq!(a, b, "canonical bytes must be stable");
    }

    // Integration (P13): a real receipt with a signed output hash is
    // cryptographically verifiable AND, once verified, feeds the compensation
    // ledger exactly once (idempotent per execution_id). This is the "signed
    // verified compute" acceptance path end-to-end.
    #[test]
    fn signed_verified_receipt_applies_once_from_same_execution() {
        use super::super::receipt::{ReceiptVerdict, VerifiedComputeReceipt};
        use decentraai_compute::{CompensationLedger, ContributionProfile};
        let (sk, _pk) = key();
        let mut ledger = CompensationLedger::default();

        // A real verified execution with a BLAKE3 output hash — the field that
        // the P13 acceptance criterion requires to be signed.
        let receipt = VerifiedComputeReceipt::new(
            "exec-whole",
            "peer-worker",
            "a:worker",
            "inference",
            120,
            ReceiptVerdict::Verified,
            2000,
        )
        .with_output_hash("blake3:realhash");

        // Canonicalize + sign (output_hash is inside the signed bytes).
        let canonical = canonicalize_receipt(&receipt);
        let signed = sign_receipt(&sk, &canonical);
        // Any node can verify independently of the signer.
        assert_eq!(verify_receipt_signature(&signed), Ok(()));

        // A tampered signature/bytes on the same receipt must fail.
        let mut tampered = signed.clone();
        tampered.signature = Some(vec![0u8; 64]);
        assert!(verify_receipt_signature(&tampered).is_err());

        // The verified receipt (not the envelope) drives the ledger, exactly once.
        let profile = ContributionProfile {
            cpu_cores: 4,
            ram_mb: 8192,
            vram_mb: 0,
            online_seconds: 3600,
            verified_requests: 10,
            failed_requests: 1,
        };
        let first = receipt.apply_compensation(&mut ledger, &profile);
        assert!(first > 0, "verified+signed work credits once");
        let again = receipt.apply_compensation(&mut ledger, &profile);
        assert_eq!(again, 0, "same execution_id never double-credits");
    }
}