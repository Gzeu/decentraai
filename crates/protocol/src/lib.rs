use anyhow::{Context, Result};
use decentraai_manifest::Manifest;
use serde::{Deserialize, Serialize};

pub const CURRENT_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestAnnouncement {
    pub protocol_version: u16,
    pub manifest: Manifest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestRequest {
    pub protocol_version: u16,
    pub manifest_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestResponse {
    pub protocol_version: u16,
    pub manifest: Manifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkRequest {
    pub protocol_version: u16,
    pub manifest_id: String,
    pub chunk_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkResponse {
    pub protocol_version: u16,
    pub chunk_index: u32,
    pub chunk_data: Vec<u8>,
}

pub fn deserialize_message<T: for<'de> Deserialize<'de>>(
    data: &[u8],
    max_size: usize,
) -> Result<T> {
    if data.len() > max_size {
        anyhow::bail!("message size {} exceeds maximum {}", data.len(), max_size);
    }

    let mut de = serde_json::Deserializer::from_slice(data);
    T::deserialize(&mut de).context("failed to deserialize message")
}

pub fn serialize_message<T: Serialize>(message: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(message).context("failed to serialize message")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_manifest() -> Manifest {
        Manifest {
            version: 1,
            model_id: "test-model-id".to_string(),
            file_name: "test.gguf".to_string(),
            file_size: 1024,
            chunk_size: 256,
            chunk_hashes: vec![
                blake3::hash(b"chunk0").to_hex().to_string(),
                blake3::hash(b"chunk1").to_hex().to_string(),
            ],
            merkle_root: blake3::hash(b"test").to_hex().to_string(),
        }
    }

    #[test]
    fn test_manifest_announcement_roundtrip() {
        let manifest = create_test_manifest();
        let announcement = ManifestAnnouncement {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest,
            signature: None,
        };

        let serialized = serialize_message(&announcement).unwrap();
        let deserialized: ManifestAnnouncement =
            deserialize_message(&serialized, 1024 * 1024).unwrap();

        assert_eq!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(deserialized.manifest.model_id, "test-model-id");
        assert!(deserialized.signature.is_none());
    }

    #[test]
    fn test_manifest_request_roundtrip() {
        let request = ManifestRequest {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest_id: "test-manifest-id".to_string(),
            signature: None,
        };

        let serialized = serialize_message(&request).unwrap();
        let deserialized: ManifestRequest = deserialize_message(&serialized, 1024 * 1024).unwrap();

        assert_eq!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(deserialized.manifest_id, "test-manifest-id");
    }

    #[test]
    fn test_manifest_response_roundtrip() {
        let manifest = create_test_manifest();
        let response = ManifestResponse {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest,
        };

        let serialized = serialize_message(&response).unwrap();
        let deserialized: ManifestResponse = deserialize_message(&serialized, 1024 * 1024).unwrap();

        assert_eq!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(deserialized.manifest.model_id, "test-model-id");
    }

    #[test]
    fn test_chunk_request_roundtrip() {
        let request = ChunkRequest {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest_id: "test-manifest-id".to_string(),
            chunk_index: 0,
        };

        let serialized = serialize_message(&request).unwrap();
        let deserialized: ChunkRequest = deserialize_message(&serialized, 1024 * 1024).unwrap();

        assert_eq!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(deserialized.manifest_id, "test-manifest-id");
        assert_eq!(deserialized.chunk_index, 0);
    }

    #[test]
    fn test_chunk_response_roundtrip() {
        let chunk_data = vec![1u8, 2u8, 3u8, 4u8];
        let response = ChunkResponse {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            chunk_index: 0,
            chunk_data: chunk_data.clone(),
        };

        let serialized = serialize_message(&response).unwrap();
        let deserialized: ChunkResponse = deserialize_message(&serialized, 1024 * 1024).unwrap();

        assert_eq!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(deserialized.chunk_index, 0);
        assert_eq!(deserialized.chunk_data, vec![1u8, 2u8, 3u8, 4u8]);
    }

    #[test]
    fn test_unknown_field_rejection() {
        let announcement = ManifestAnnouncement {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest: create_test_manifest(),
            signature: None,
        };

        let serialized = serialize_message(&announcement).unwrap();
        // Inject an unknown field
        let json_str = String::from_utf8(serialized).unwrap();
        let mut json_str = json_str;
        json_str.insert(json_str.len() - 1, ',');
        json_str.push_str("\"unknown_field\": 42}");

        let result: Result<ManifestAnnouncement> =
            deserialize_message(json_str.as_bytes(), 1024 * 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_oversize_rejection() {
        let large_data = vec![0u8; 10 * 1024 * 1024]; // 10 MB
        let result: Result<ManifestAnnouncement> = deserialize_message(&large_data, 1024);
        assert!(result.is_err());
    }

    #[test]
    fn test_version_mismatch_handling() {
        let announcement = ManifestAnnouncement {
            protocol_version: 999, // Wrong version
            manifest: create_test_manifest(),
            signature: None,
        };

        let serialized = serialize_message(&announcement).unwrap();
        let deserialized: ManifestAnnouncement =
            deserialize_message(&serialized, 1024 * 1024).unwrap();

        // Deserialization succeeds, but caller should check version
        assert_eq!(deserialized.protocol_version, 999);
        assert_ne!(deserialized.protocol_version, CURRENT_PROTOCOL_VERSION);
    }

    #[test]
    fn test_optional_signature_field() {
        // Without signature
        let announcement_no_sig = ManifestAnnouncement {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest: create_test_manifest(),
            signature: None,
        };
        let serialized_no_sig = serialize_message(&announcement_no_sig).unwrap();
        assert!(!serialized_no_sig.is_empty());

        // With signature
        let signature_bytes = vec![1u8, 2u8, 3u8, 4u8];
        let announcement_with_sig = ManifestAnnouncement {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            manifest: create_test_manifest(),
            signature: Some(signature_bytes),
        };
        let serialized_with_sig = serialize_message(&announcement_with_sig).unwrap();
        assert!(!serialized_with_sig.is_empty());
        assert!(serialized_with_sig.len() > serialized_no_sig.len());
    }
}
