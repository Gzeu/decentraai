use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use decentraai_identity::Identity;
use decentraai_manifest::Manifest;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod infer_protocol;

pub use infer_protocol::{
    InferRequest, InferResponse, InferProgress, InferMessage,
    WorkerStatus, TaskPlacement, WorkerAnnouncement
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
        anyhow::bail!("message exceeds maximum size: {} > {}", data.len(), max_size);
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
        let deserialized: ManifestResponse =
            deserialize_message(&serialized, 1024 * 1024).unwrap();
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
        assert!(result.unwrap_err().to_string().contains("exceeds maximum size"));
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
}
