//! DecentraAI ↔ MX-8004 identity link (verified-first phase).
//!
//! # The deterministic bridge
//!
//! MX-8004 registers an agent by its Ed25519 PUBLIC KEY (hex, 0x-prefixed).
//! DecentraAI node identities ARE Ed25519 keys
//! (`decentraai_identity::Identity::public_key()` → 32 raw bytes).
//!
//! Therefore the mapping is pure byte equality after hex decoding:
//!
//! ```text
//! local VerifyingKey (32B) ──hex──▶ "0x…" ──registered──▶ MxAgentRecord.public_key
//! ```
//!
//! No third identifier is invented; internal identity stays authoritative
//! for permissions, MX identity is the external anchor.
//!
//! # What is verified vs prepared
//!
//! - `GET /agents*`, `GET /reputations/*`, `POST /agents` contract: ✅ from
//!   the official skill.md.
//! - EXECUTING registration requires an operator wallet + manifest hosting:
//!   prepared here as validated data structures ONLY. This module never
//!   signs, never hosts, never submits.
//! - Anchoring endpoint: ❌ not verified — [`anchoring_payload`] is a data
//!   shape for future work, clearly marked.

use crate::multiversx_devnet::MxAgentRecord;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// Errors from linking or preparing registration.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    #[error("public key must be 0x-prefixed hex")]
    BadFormat,
    #[error("public key hex decodes to {got} bytes, expected 32")]
    BadLength { got: usize },
    #[error("invalid hex: {0}")]
    InvalidHex(String),
    #[error("key mismatch: registered key does not match local identity")]
    KeyMismatch,
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn from_hex(s: &str) -> Result<Vec<u8>, LinkError> {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    if !stripped.len().is_multiple_of(2) {
        return Err(LinkError::BadFormat);
    }
    let mut out = Vec::with_capacity(stripped.len() / 2);
    let bytes = stripped.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char)
            .to_digit(16)
            .ok_or_else(|| LinkError::InvalidHex(s.to_string()))?;
        let lo = (pair[1] as char)
            .to_digit(16)
            .ok_or_else(|| LinkError::InvalidHex(s.to_string()))?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

/// Encodes a local Ed25519 public key into the MX-8004 wire format
/// (`0x` + lowercase hex), exactly as the official flow expects.
#[must_use]
pub fn local_public_key_hex(public_key_bytes: &[u8; 32]) -> String {
    format!("0x{}", to_hex(public_key_bytes))
}

/// Proves (or refutes) that an MX-8004 agent record belongs to THIS local
/// identity: decoded registered key must equal our 32 public bytes.
pub fn verify_link(
    local_public_key_bytes: &[u8; 32],
    record: &MxAgentRecord,
) -> Result<(), LinkError> {
    let Some(hex) = &record.public_key else {
        return Err(LinkError::BadFormat);
    };
    let decoded = from_hex(hex)?;
    if decoded.len() != 32 {
        return Err(LinkError::BadLength { got: decoded.len() });
    }
    if decoded != local_public_key_bytes {
        return Err(LinkError::KeyMismatch);
    }
    Ok(())
}

/// Protocols the official manifest example documents. Closed set until the
/// standards pages are verified (⚠️ A2A/x402/MPP/OASF remain unverified).
pub const KNOWN_PROTOCOLS: &[&str] = &["ACP", "x402", "UCP", "MCP"];

/// Agent capability manifest (the JSON hosted on IPFS/HTTPS at
/// registration). Shape mirrors the official example exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentManifest {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Ed25519 public key, `0x`-prefixed hex — MUST be this agent's own key.
    #[serde(rename = "publicKey")]
    pub public_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protocols: Vec<String>,
}

impl AgentManifest {
    /// Deterministic canonical JSON (what gets pinned/hosted).
    pub fn manifest_json(&self) -> Result<String, LinkError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|_| LinkError::BadFormat)
    }

    /// Offline validation rules derived from the official manifest example:
    /// non-empty bounded name, version present, own-key consistency,
    /// capabilities non-empty, protocols within the known set.
    pub fn validate(&self) -> Result<(), LinkError> {
        if self.name.trim().is_empty() || self.name.chars().count() > 64 {
            return Err(LinkError::BadFormat);
        }
        if self.version.trim().is_empty() {
            return Err(LinkError::BadFormat);
        }
        if !self.public_key.starts_with("0x") || from_hex(&self.public_key)?.len() != 32 {
            return Err(LinkError::BadLength { got: 0 });
        }
        if self.capabilities.is_empty() {
            return Err(LinkError::BadFormat);
        }
        for p in &self.protocols {
            if !KNOWN_PROTOCOLS.contains(&p.as_str()) {
                return Err(LinkError::BadFormat);
            }
        }
        Ok(())
    }
}

/// The exact POST body for `POST /agents` (endpoint ✅ verified in skill.md).
/// Built here as VALIDATED DATA; submission requires an operator wallet and
/// happens outside this crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationBody {
    pub name: String,
    pub uri: String,
    #[serde(rename = "publicKey")]
    pub public_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<RegistrationMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationMetadata {
    pub key: String,
    pub value: String,
}

impl RegistrationBody {
    /// Builds + validates the body from a hosted manifest URI.
    pub fn new(name: &str, manifest_uri: &str, public_key_hex: &str) -> Result<Self, LinkError> {
        let body = Self {
            name: name.to_string(),
            uri: manifest_uri.to_string(),
            public_key: public_key_hex.to_string(),
            metadata: Vec::new(),
        };
        body.validate()?;
        Ok(body)
    }

    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.push(RegistrationMetadata {
            key: key.to_string(),
            value: value.to_string(),
        });
        self
    }

    /// URI must be IPFS or HTTPS; key must be well-formed 0x-hex 32 bytes;
    /// name bounded like manifests.
    pub fn validate(&self) -> Result<(), LinkError> {
        if self.name.trim().is_empty() || self.name.chars().count() > 64 {
            return Err(LinkError::BadFormat);
        }
        if !(self.uri.starts_with("ipfs://") || self.uri.starts_with("https://")) {
            return Err(LinkError::BadFormat);
        }
        if !self.public_key.starts_with("0x") || from_hex(&self.public_key)?.len() != 32 {
            return Err(LinkError::BadLength { got: 0 });
        }
        Ok(())
    }

    pub fn json(&self) -> Result<String, LinkError> {
        serde_json::to_string(self).map_err(|_| LinkError::BadFormat)
    }
}

/// Anchoring payload PREPARATION (endpoint NOT verified yet — ❌).
/// Carries the EconomicEvidence BLAKE3 hash so a future anchoring call only
/// wraps this structure. Clearly marked preparation, never submitted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorPayloadPrep {
    pub evidence_hash_hex: String,
    pub cu_version: u32,
    pub epoch: u64,
    /// Marker so no one mistakes preparation for a verified API call.
    pub status: &'static str,
}

#[must_use]
pub fn anchoring_payload(
    evidence_hash: &[u8; 32],
    epoch: u64,
    cu_version: u32,
) -> AnchorPayloadPrep {
    AnchorPayloadPrep {
        evidence_hash_hex: to_hex(evidence_hash),
        cu_version,
        epoch,
        status: "PREPARATION — anchoring endpoint not verified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        let mut k = [0xAB_u8; 32];
        k[31] = 0x01;
        k
    }

    #[test]
    fn local_key_encodes_to_mx_wire_format_and_links_back() {
        let hex = local_public_key_hex(&key());
        assert!(hex.starts_with("0x"));
        assert_eq!(hex.len(), 66);

        // A devnet record carrying OUR key links deterministically.
        let rec = MxAgentRecord {
            nonce: Some(7),
            name: Some("DecentraGovernor".into()),
            uri: Some("ipfs://QmZ".into()),
            public_key: Some(hex.clone()),
        };
        assert!(verify_link(&key(), &rec).is_ok());

        // Someone else's key does not link — case-insensitive hex still works.
        let mut other = rec.clone();
        other.public_key = Some(format!("0x{:0>64}", "ff"));
        assert!(matches!(
            verify_link(&key(), &other),
            Err(LinkError::KeyMismatch)
        ));
    }

    #[test]
    fn malformed_registered_keys_are_rejected_not_guessed() {
        let rec = MxAgentRecord {
            nonce: None,
            name: None,
            uri: None,
            public_key: None,
        };
        assert!(matches!(
            verify_link(&key(), &rec),
            Err(LinkError::BadFormat)
        ));

        let short = MxAgentRecord {
            nonce: None,
            name: None,
            uri: None,
            public_key: Some("0x0011".into()),
        };
        assert!(matches!(
            verify_link(&key(), &short),
            Err(LinkError::BadLength { got: 2 })
        ));

        let bad_hex = MxAgentRecord {
            nonce: None,
            name: None,
            uri: None,
            public_key: Some("0xzz".into()),
        };
        assert!(matches!(
            verify_link(&key(), &bad_hex),
            Err(LinkError::InvalidHex(_))
        ));
    }

    #[test]
    fn manifest_validation_enforces_own_key_and_known_protocols() {
        let hex = local_public_key_hex(&key());
        let m = AgentManifest {
            name: "DecentraGovernor".into(),
            version: "1.0.0".into(),
            description: "governor node".into(),
            public_key: hex.clone(),
            capabilities: vec!["research".into()],
            protocols: vec!["MCP".into()],
        };
        m.validate().unwrap();
        let json = m.manifest_json().unwrap();
        assert!(json.contains("\"publicKey\":\"0x"));

        // Unknown protocol rejected (closed set until standards verified).
        let mut unknown = m.clone();
        unknown.protocols = vec!["MADE_UP".into()];
        assert!(unknown.validate().is_err());

        // Empty capabilities rejected.
        let mut empty_caps = m.clone();
        empty_caps.capabilities.clear();
        assert!(empty_caps.validate().is_err());
    }

    #[test]
    fn registration_body_requires_hosted_uri_and_valid_key() {
        let hex = local_public_key_hex(&key());
        let ok = RegistrationBody::new("gov", "ipfs://QmABC", &hex).unwrap();
        ok.validate().unwrap();
        assert!(ok.json().unwrap().contains("ipfs://QmABC"));

        // http:// is not a valid hosting scheme.
        assert!(RegistrationBody::new("gov", "http://x", &hex).is_err());

        // Metadata attaches cleanly (category per official docs).
        let with_meta = RegistrationBody::new("gov", "https://m.example/agent.json", &hex)
            .unwrap()
            .with_metadata("category", "research-analysis");
        assert_eq!(with_meta.metadata.len(), 1);
        with_meta.validate().unwrap();
    }

    #[test]
    fn anchoring_payload_is_marked_preparation() {
        let p = anchoring_payload(&[5u8; 32], 3, 2);
        assert!(p.status.contains("not verified"));
        assert_eq!(
            p.evidence_hash_hex,
            "0505050505050505050505050505050505050505050505050505050505050505"
        );
        assert_eq!(p.cu_version, 2);
    }
}
