use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use decentraai_identity::Identity;
use decentraai_manifest::Manifest;
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use libp2p::PeerId;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod infer_protocol;

pub use infer_protocol::{
    InferMessage, InferProgress, InferRequest, InferResponse, TaskPlacement, WorkerAnnouncement,
    WorkerStatus,
};

pub const CURRENT_PROTOCOL_VERSION: u16 = 1;

// Serde helper modules for base64 encoding/decoding
mod b64 {
    use super::*;

    pub fn serialize<S>(data: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = STANDARD.encode(data);
        encoded.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded: String = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

mod b64_opt {
    use super::*;

    pub fn serialize<S>(data: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match data {
            Some(bytes) => b64::serialize(bytes, serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded: Option<String> = Option::deserialize(deserializer)?;
        match encoded {
            Some(s) => Ok(Some(STANDARD.decode(s).map_err(serde::de::Error::custom)?)),
            None => Ok(None),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestAnnouncement {
    pub protocol_version: u16,
    pub manifest: Manifest,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "b64_opt")]
    pub signature: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRequest {
    pub protocol_version: u16,
    pub manifest_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "b64_opt")]
    pub signature: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Response to a manifest request.
///
/// Intentionally carries no signature: manifest integrity is anchored in the signed
/// manifest's chunk_hashes + Merkle root. Per-chunk BLAKE3 verification at assembly
/// ensures the manifest was not tampered with.
pub struct ManifestResponse {
    pub protocol_version: u16,
    pub manifest: Manifest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkRequest {
    pub protocol_version: u16,
    pub manifest_id: String,
    pub chunk_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Response to a chunk request.
///
/// Intentionally carries no signature: chunk integrity is anchored in the signed
/// manifest's chunk_hashes + Merkle root. Per-chunk BLAKE3 verification at assembly
/// ensures the chunk was not tampered with.
pub struct ChunkResponse {
    pub protocol_version: u16,
    pub chunk_index: u32,
    #[serde(with = "b64")]
    pub chunk_data: Vec<u8>,
}

/// Asks a peer for the full list of manifests it serves (M7a).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogRequest {
    pub protocol_version: u16,
}

/// The peer's served catalog: one manifest per shared model.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogResponse {
    pub protocol_version: u16,
    pub manifests: Vec<Manifest>,
}

/// Deserialize a message with a size cap to prevent memory exhaustion.
pub fn deserialize_message<T: for<'de> Deserialize<'de>>(
    data: &[u8],
    max_size: usize,
) -> Result<T> {
    if data.len() > max_size {
        anyhow::bail!(
            "message exceeds maximum size: {} > {}",
            data.len(),
            max_size
        );
    }
    serde_json::from_slice(data).context("failed to deserialize message")
}

/// Serialize a message to bytes for transmission.
pub fn serialize_message<T: Serialize>(message: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(message).context("failed to serialize message")
}

/// Serialize a manifest response carrying the given manifest.
pub fn manifest_response_bytes(manifest: &Manifest) -> Result<Vec<u8>> {
    serialize_message(&ManifestResponse {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        manifest: manifest.clone(),
    })
}

/// Serialize a manifest announcement carrying the given manifest.
pub fn announcement_bytes(manifest: &Manifest, signature: Option<Vec<u8>>) -> Result<Vec<u8>> {
    serialize_message(&ManifestAnnouncement {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        manifest: manifest.clone(),
        signature,
    })
}

/// Calculate the maximum serialized size for a chunk response message.
///
/// Chunk responses use base64 encoding for binary data, which has a 4/3 overhead.
/// This function calculates the worst-case size including:
/// - Base64 encoding overhead (chunk_size * 4 / 3)
/// - JSON header overhead (4096 bytes headroom)
///
/// Note: Chunk responses use this cap, not the control-plane cap (network.max_message_bytes).
/// The config allows chunk_size_mb 1..=64, so worst-case chunk message is ~89.5 MB,
/// which is acceptable on LAN v1.
pub fn max_chunk_message_bytes(chunk_size: usize) -> usize {
    chunk_size * 4 / 3 + 4096
}

/// Generate canonical bytes for signing a manifest.
///
/// Signers and verifiers MUST use compact serialization with fields in declaration order;
/// never pretty-print; never sign raw wire bytes. This function uses serde_json::to_vec
/// to produce deterministic JSON because Manifest has no map fields.
pub fn canonical_manifest_bytes(manifest: &Manifest) -> Vec<u8> {
    serde_json::to_vec(manifest).expect("manifest must be serializable")
}

/// Sign a manifest using the node's Ed25519 identity.
///
/// The signature is computed over the canonical bytes of the manifest.
pub fn sign_manifest(identity: &Identity, manifest: &Manifest) -> Signature {
    let bytes = canonical_manifest_bytes(manifest);
    identity.sign(&bytes)
}

/// Verify a manifest signature against a public key.
///
/// Verifies that the signature was computed over the canonical bytes of the manifest.
pub fn verify_manifest_signature(
    key: &VerifyingKey,
    manifest: &Manifest,
    sig: &Signature,
) -> Result<()> {
    let bytes = canonical_manifest_bytes(manifest);
    decentraai_identity::verify_signature(key, &bytes, sig)
}

/// Canonical bytes for signing an [`InferRequest`] (P1).
///
/// The `signature` field is stripped before serializing: the signature signs
/// the request fields, not itself. Field order is deterministic (serde in
/// declaration order) and there are no map fields, so canonical JSON matches
/// on both sides. The `nonce` (P4) is included so a captured request cannot be
/// re-minted with a fresh counter without the sender's key.
pub fn canonical_infer_request_bytes(req: &InferRequest) -> Vec<u8> {
    let mut stripped = req.clone();
    stripped.signature = None;
    serde_json::to_vec(&stripped).expect("infer request must be serializable")
}

/// Verifies a signed inference request against the authenticated connected
/// peer (P1/P2). Fails if the request is unsigned, its embedded public key
/// does not map to `connected_peer` (anti-spoof: `sender_peer_id` is not
/// trusted), or the Ed25519 signature does not verify over the canonical bytes.
pub fn verify_infer_request_signature(
    connected_peer: &PeerId,
    req: &InferRequest,
) -> Result<()> {
    let (Some(sig_bytes), Some(pk_bytes)) =
        (req.signature.as_deref(), req.sender_public_key)
    else {
        anyhow::bail!("unsigned inference request");
    };
    let pubkey_kp = libp2p::identity::ed25519::PublicKey::try_from_bytes(&pk_bytes)
        .context("invalid sender public key")?;
    // Anti-spoof: the sender's public key must map to the authenticated
    // connected peer. `sender_peer_id` in the payload is never trusted.
    let expected =
        PeerId::from_public_key(&libp2p::identity::PublicKey::from(pubkey_kp));
    if &expected != connected_peer {
        anyhow::bail!(
            "sender public key maps to {expected}, not the connected peer {connected_peer}"
        );
    }
    let key = VerifyingKey::from_bytes(&pk_bytes).context("invalid ed25519 public key")?;
    let sig = Signature::from_slice(sig_bytes).context("invalid signature")?;
    decentraai_identity::verify_signature(&key, &canonical_infer_request_bytes(req), &sig)
}

/// Signs an [`InferRequest`] with an Ed25519 signing key (32 bytes) — the same
/// material the node identity stores (P1). Sets `sender_public_key` and
/// `signature` (over canonical bytes including the `nonce`). Used by the
/// coordinator to sign outbound requests without sharing a full `Identity`.
pub fn sign_infer_request_with_key(signing_key_bytes: &[u8; 32], req: &mut InferRequest) {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let verifying_key = signing_key.verifying_key();
    req.sender_public_key = Some(verifying_key.to_bytes());
    let bytes = canonical_infer_request_bytes(req);
    req.signature = Some(signing_key.sign(&bytes).to_bytes().to_vec());
}

/// Serialized compute advertisement plus its sender's signature (P3).
///
/// The `advertisement` is the canonical serialized
/// [`decentraai_compute::ComputeAdvertisement`]; `signature` is Ed25519 over
/// those exact bytes with `sender_public_key`. The receiver verifies the
/// signature and that the embedded advertisement's `peer_id` matches the
/// signing public key, so a forged/spoofed advertisement is rejected.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SignedComputeAdvertisement {
    pub protocol_version: u16,
    /// Canonical bytes of the compute advertisement.
    pub advertisement: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_public_key: Option<[u8; 32]>,
    #[serde(skip_serializing_if = "Option::is_none", with = "b64_opt")]
    pub signature: Option<Vec<u8>>,
}

impl Default for SignedComputeAdvertisement {
    fn default() -> Self {
        Self {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            advertisement: Vec::new(),
            sender_public_key: None,
            signature: None,
        }
    }
}

/// Signs serialized advertisement bytes with the node identity (P3). Returns
/// the wire-form `SignedComputeAdvertisement`.
pub fn sign_compute_advertisement(
    signing_key_bytes: &[u8; 32],
    advertisement_bytes: &[u8],
) -> SignedComputeAdvertisement {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(signing_key_bytes);
    let signature = signing_key.sign(advertisement_bytes).to_bytes().to_vec();
    SignedComputeAdvertisement {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        advertisement: advertisement_bytes.to_vec(),
        sender_public_key: Some(signing_key.verifying_key().to_bytes()),
        signature: Some(signature),
    }
}

/// Verifies a signed advertisement. Requires a signature, a sender public key
/// that maps to the embedded advertisement's `peer_id`, and an Ed25519
/// signature valid over the advertisement bytes.
pub fn verify_signed_compute_advertisement(
    signed: &SignedComputeAdvertisement,
    expected_peer: &PeerId,
) -> Result<()> {
    let (Some(sig), Some(pk_bytes)) = (signed.signature.as_deref(), signed.sender_public_key)
    else {
        anyhow::bail!("unsigned compute advertisement");
    };
    let pubkey = libp2p::identity::ed25519::PublicKey::try_from_bytes(&pk_bytes)
        .context("invalid sender public key")?;
    let signer = PeerId::from_public_key(&libp2p::identity::PublicKey::from(pubkey));
    if &signer != expected_peer {
        anyhow::bail!(
            "advertisement signer {signer} does not match the claiming peer {expected_peer}"
        );
    }
    let key = VerifyingKey::from_bytes(&pk_bytes).context("invalid ed25519 public key")?;
    let sig = Signature::from_slice(sig).context("invalid signature")?;
    decentraai_identity::verify_signature(&key, &signed.advertisement, &sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_identity::Identity;

    fn create_test_manifest() -> Manifest {
        Manifest {
            version: 1,
            model_id: "test-model-id".to_string(),
            file_name: "test.gguf".to_string(),
            file_size: 1024,
            chunk_size: 4 * 1024 * 1024,
            chunk_hashes: vec![blake3::hash(b"chunk1").to_hex().to_string()],
            merkle_root: blake3::hash(b"root").to_hex().to_string(),
        }
    }

    #[test]
    fn test_manifest_announcement_roundtrip() {
        let announcement = ManifestAnnouncement {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest: create_test_manifest(),
            signature: Some(vec![1, 2, 3]),
        };
        let serialized = serialize_message(&announcement).unwrap();
        let deserialized: ManifestAnnouncement =
            deserialize_message(&serialized, 1024 * 1024).unwrap();
        assert_eq!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(deserialized.manifest.model_id, "test-model-id");
        assert_eq!(deserialized.signature, Some(vec![1, 2, 3]));
    }

    #[test]
    fn test_manifest_request_roundtrip() {
        let request = ManifestRequest {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest_id: "abc123".to_string(),
            signature: None,
        };
        let serialized = serialize_message(&request).unwrap();
        let deserialized: ManifestRequest = deserialize_message(&serialized, 1024).unwrap();
        assert_eq!(deserialized.manifest_id, "abc123");
        assert_eq!(deserialized.signature, None);
    }

    #[test]
    fn test_manifest_response_roundtrip() {
        let response = ManifestResponse {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest: create_test_manifest(),
        };
        let serialized = serialize_message(&response).unwrap();
        let deserialized: ManifestResponse = deserialize_message(&serialized, 1024 * 1024).unwrap();
        assert_eq!(deserialized.manifest.file_name, "test.gguf");
    }

    #[test]
    fn test_chunk_request_roundtrip() {
        let request = ChunkRequest {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest_id: "model-xyz".to_string(),
            chunk_index: 42,
        };
        let serialized = serialize_message(&request).unwrap();
        let deserialized: ChunkRequest = deserialize_message(&serialized, 1024).unwrap();
        assert_eq!(deserialized.chunk_index, 42);
    }

    #[test]
    fn test_chunk_response_roundtrip() {
        let response = ChunkResponse {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            chunk_index: 7,
            chunk_data: vec![1u8, 2u8, 3u8, 4u8],
        };
        let serialized = serialize_message(&response).unwrap();
        let deserialized: ChunkResponse = deserialize_message(&serialized, 1024).unwrap();
        assert_eq!(deserialized.chunk_index, 7);
        assert_eq!(deserialized.chunk_data, vec![1u8, 2u8, 3u8, 4u8]);
    }

    #[test]
    fn test_4mb_chunk_roundtrip() {
        let chunk_size = 4 * 1024 * 1024; // 4 MiB
        let chunk_data = vec![42u8; chunk_size];
        let response = ChunkResponse {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            chunk_index: 0,
            chunk_data: chunk_data.clone(),
        };

        let serialized = serialize_message(&response).unwrap();
        let max_size = max_chunk_message_bytes(chunk_size);

        // Verify the serialized size is reasonable (< 5.5 MiB for 4 MiB chunk)
        let five_point_five_mib = 5 * 1024 * 1024 + 512 * 1024; // 5.5 MiB
        assert!(
            serialized.len() < five_point_five_mib,
            "serialized size {} exceeds 5.5 MiB",
            serialized.len()
        );

        // Verify it fits within the calculated max size
        assert!(
            serialized.len() <= max_size,
            "serialized size {} exceeds max {}",
            serialized.len(),
            max_size
        );

        let deserialized: ChunkResponse = deserialize_message(&serialized, max_size).unwrap();

        assert_eq!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(deserialized.chunk_index, 0);
        assert_eq!(deserialized.chunk_data.len(), chunk_size);
        assert_eq!(deserialized.chunk_data, chunk_data);
    }

    #[test]
    fn test_unknown_field_rejection() {
        let announcement = ManifestAnnouncement {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest: create_test_manifest(),
            signature: None,
        };
        let mut serialized = serialize_message(&announcement).unwrap();
        // Inject unknown field
        let with_unknown = br#"{"protocol_version":1,"manifest":{},"unknown_field":"bad"}"#;
        serialized.clear();
        serialized.extend_from_slice(with_unknown);
        let result: Result<ManifestAnnouncement> = deserialize_message(&serialized, 1024 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_oversize_rejection() {
        let request = ManifestRequest {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest_id: "x".repeat(10000),
            signature: None,
        };
        let serialized = serialize_message(&request).unwrap();
        let result: Result<ManifestRequest> = deserialize_message(&serialized, 100);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum size")
        );
    }

    #[test]
    fn test_version_mismatch_handling() {
        let mut request = ManifestRequest {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest_id: "test".to_string(),
            signature: None,
        };
        request.protocol_version = 99;
        let serialized = serialize_message(&request).unwrap();
        let deserialized: ManifestRequest = deserialize_message(&serialized, 1024).unwrap();
        // Deserialization succeeds but caller must check version
        assert_eq!(deserialized.protocol_version, 99);
        assert_ne!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
    }

    #[test]
    fn test_optional_signature_field() {
        let announcement = ManifestAnnouncement {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest: create_test_manifest(),
            signature: None,
        };
        let serialized_no_sig = serialize_message(&announcement).unwrap();

        let announcement_with_sig = ManifestAnnouncement {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest: create_test_manifest(),
            signature: Some(vec![0xde, 0xad, 0xbe, 0xef]),
        };
        let serialized_with_sig = serialize_message(&announcement_with_sig).unwrap();

        // None signature should be omitted (skip_serializing_if)
        assert!(!serialized_no_sig.is_empty());
        assert!(serialized_with_sig.len() > serialized_no_sig.len());
    }

    #[test]
    fn test_canonical_manifest_signing_roundtrip() {
        let identity = Identity::generate();
        let manifest = create_test_manifest();

        let signature = sign_manifest(&identity, &manifest);
        let public_key = identity.public_key();

        assert!(verify_manifest_signature(public_key, &manifest, &signature).is_ok());
    }

    #[test]
    fn test_canonical_manifest_tamper_detection() {
        let identity = Identity::generate();
        let mut manifest = create_test_manifest();

        let signature = sign_manifest(&identity, &manifest);
        let public_key = identity.public_key();

        // Tamper with the manifest by changing a chunk hash
        manifest.chunk_hashes[0] = blake3::hash(b"tampered").to_hex().to_string();

        assert!(verify_manifest_signature(public_key, &manifest, &signature).is_err());
    }

    #[test]
    fn test_canonical_manifest_determinism() {
        let manifest = create_test_manifest();

        let bytes1 = canonical_manifest_bytes(&manifest);
        let bytes2 = canonical_manifest_bytes(&manifest);

        assert_eq!(
            bytes1, bytes2,
            "canonical serialization must be deterministic"
        );
    }

    #[test]
    fn test_manifest_helpers() {
        let manifest = create_test_manifest();
        let response = manifest_response_bytes(&manifest).unwrap();
        let parsed: ManifestResponse = deserialize_message(&response, 1024 * 1024).unwrap();
        assert_eq!(parsed.manifest.model_id, "test-model-id");

        let announcement = announcement_bytes(&manifest, None).unwrap();
        let parsed: ManifestAnnouncement = deserialize_message(&announcement, 1024 * 1024).unwrap();
        assert_eq!(parsed.manifest.file_name, "test.gguf");
        assert_eq!(parsed.signature, None);
    }

    #[test]
    fn test_catalog_roundtrip() {
        let request = CatalogRequest {
            protocol_version: CURRENT_PROTOCOL_VERSION,
        };
        let serialized = serialize_message(&request).unwrap();
        let parsed: CatalogRequest = deserialize_message(&serialized, 1024).unwrap();
        assert_eq!(parsed.protocol_version, CURRENT_PROTOCOL_VERSION);

        let response = CatalogResponse {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifests: vec![create_test_manifest()],
        };
        let serialized = serialize_message(&response).unwrap();
        let parsed: CatalogResponse = deserialize_message(&serialized, 1024 * 1024).unwrap();
        assert_eq!(parsed.manifests.len(), 1);
        assert_eq!(parsed.manifests[0].file_name, "test.gguf");
    }

    // ---- P1: inference request signing ----

    fn peer_of(identity: &Identity) -> PeerId {
        let bytes = identity.public_key().to_bytes();
        let pk = libp2p::identity::ed25519::PublicKey::try_from_bytes(&bytes).unwrap();
        PeerId::from_public_key(&libp2p::identity::PublicKey::from(pk))
    }

    fn signed_req(identity: &Identity) -> InferRequest {
        InferRequest::new("m".into(), "hi".into(), 16)
            .with_sender(peer_of(identity))
            .with_nonce(7)
            .sign(identity)
    }

    #[test]
    fn infer_request_signing_roundtrip_verifies_for_connected_peer() {
        let identity = Identity::generate();
        let peer = peer_of(&identity);
        let req = signed_req(&identity);
        assert!(req.is_signed());
        assert!(verify_infer_request_signature(&peer, &req).is_ok());
    }

    #[test]
    fn infer_request_signature_rejects_sender_spoof() {
        // Sign as A but present to a *different* connected peer B: must fail,
        // proving sender_peer_id cannot be spoofed to another trusted identity.
        let identity_a = Identity::generate();
        let identity_b = Identity::generate();
        let peer_b = peer_of(&identity_b);
        let req = signed_req(&identity_a);
        assert!(verify_infer_request_signature(&peer_b, &req).is_err());
    }

    #[test]
    fn infer_request_signature_rejects_tampered_nonce() {
        // The signature covers the nonce: re-minting a fresh counter without
        // the key must fail verification.
        let identity = Identity::generate();
        let peer = peer_of(&identity);
        let mut req = signed_req(&identity);
        req.nonce += 1; // attacker bumps the counter without re-signing
        assert!(verify_infer_request_signature(&peer, &req).is_err());
    }

    #[test]
    fn infer_request_signature_rejects_tampered_prompt() {
        let identity = Identity::generate();
        let peer = peer_of(&identity);
        let mut req = signed_req(&identity);
        req.prompt = "tampered".to_string();
        assert!(verify_infer_request_signature(&peer, &req).is_err());
    }

    #[test]
    fn unsigned_infer_request_is_rejected() {
        let req = InferRequest::new("m".into(), "hi".into(), 16);
        assert!(!req.is_signed());
        assert!(verify_infer_request_signature(&req.sender_peer_id, &req).is_err());
    }

    #[test]
    fn canonical_infer_request_bytes_are_deterministic_and_ignore_signature() {
        let identity = Identity::generate();
        let a = signed_req(&identity);
        let b = a.clone();
        assert_eq!(
            canonical_infer_request_bytes(&a),
            canonical_infer_request_bytes(&b),
            "canonical bytes must be deterministic"
        );
        // Signing twice with different signature bytes must not change the
        // canonical bytes (the signature field is stripped).
        let mut c = a.clone();
        c.signature = Some(vec![1, 2, 3]);
        assert_eq!(
            canonical_infer_request_bytes(&a),
            canonical_infer_request_bytes(&c),
            "canonical bytes must ignore the signature field"
        );
    }

    // ---- P3: signed compute advertisements ----

    #[test]
    fn signed_advertisement_roundtrip_verifies_for_the_claiming_peer() {
        let identity = Identity::generate();
        let peer = peer_of(&identity);
        // Canonical advertisement bytes (arbitrary payload here).
        let adv_bytes = serde_json::to_vec(&["cpu_cores", "gpu"]).unwrap();
        let signed = sign_compute_advertisement(&identity.signing_key_bytes(), &adv_bytes);
        assert!(signed.signature.is_some());
        assert!(verify_signed_compute_advertisement(&signed, &peer).is_ok());
    }

    #[test]
    fn signed_advertisement_rejects_a_spoofed_claiming_peer() {
        let identity = Identity::generate();
        let other = Identity::generate();
        let adv_bytes = serde_json::to_vec(&["gpu"]).unwrap();
        let signed = sign_compute_advertisement(&identity.signing_key_bytes(), &adv_bytes);
        // Present the signed advertisement as claiming to be `other`: the signer
        // key must map to the claiming peer, so this must fail (anti-spoof).
        assert!(verify_signed_compute_advertisement(&signed, &peer_of(&other)).is_err());
    }

    #[test]
    fn signed_advertisement_rejects_tampered_payload() {
        let identity = Identity::generate();
        let peer = peer_of(&identity);
        let adv_bytes = serde_json::to_vec(&["ram"]).unwrap();
        let mut signed = sign_compute_advertisement(&identity.signing_key_bytes(), &adv_bytes);
        // Tamper with the serialized payload after signing.
        signed.advertisement = serde_json::to_vec(&["vram_999"]).unwrap();
        assert!(verify_signed_compute_advertisement(&signed, &peer).is_err());
    }
}
