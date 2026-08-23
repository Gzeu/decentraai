//! DFCP v1 — the DecentraAI Fabric Communication Protocol messages for
//! resource sharing ("Sharing is Caring", M14/M15 milestone 1).
//!
//! Scope of this module: the NEGOTIATION envelope only. Task payloads reuse
//! the existing typed channels (`InferRequest` for inference, skills API for
//! tool workloads) and receipts reuse the signed P13 pipeline — DFCP never
//! duplicates them.
//!
//! Message flow for one assist:
//!
//! ```text
//! requester                    candidate worker
//!    │  RESOURCE_REQUEST          │
//!    │ ─────────────────────────▶ │   (broadcast on the mesh)
//!    │        RESOURCE_OFFER      │
//!    │ ◀───────────────────────── │   (owner-limits checked by SENDER)
//!    │  RESOURCE_RESERVE{offer}   │
//!    │ ─────────────────────────▶ │
//!    │     RESOURCE_RESERVED      │
//!    │ ◀───────────────────────── │   (lease starts; TTL enforced)
//!    │  ASSIST_TASK_ASSIGN        │
//!    │ ─────────────────────────▶ │
//!    │       ASSIST_TASK_RESULT   │
//!    │ ◀───────────────────────── │   → evidence/receipt → credit
//!    │  RESOURCE_RELEASE          │
//!    │ ─────────────────────────▶ │   (early release; TTL is the backstop)
//! ```
//!
//! Security invariants:
//! - every message carries `protocol_version` and a random id (replay aid);
//! - identity binding comes from the libp2p secure channel (peer id in the
//!   transport), NOT from any model-generated field;
//! - all strings/sizes bounded; unknown fields rejected;
//! - an OFFER is a CLAIM until the RESERVE handshake succeeds against the
//!   worker's authoritative reservation ledger — advertised capacity is
//!   never trusted blindly.

use serde::{Deserialize, Serialize};

/// Wire format version. Bump on breaking changes; receivers MUST reject
/// versions they do not understand instead of guessing.
pub const DFCP_VERSION: u32 = 1;

fn dfcp_version() -> u32 {
    DFCP_VERSION
}

/// Upper bound for any single DFCP message payload (bytes). Negotiation
/// metadata only — task payloads travel on the existing channels.
pub const MAX_DFCP_MESSAGE_BYTES: usize = 16 * 1024;

macro_rules! dfcp_id {
    ($name:ident) => {
        /// Random unique identifier (UUID v4). Lets both sides correlate a
        /// negotiation across the mesh and reject duplicates cheaply.
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub String);

        impl $name {
            pub fn new() -> Self {
                Self(uuid::Uuid::new_v4().to_string())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

dfcp_id!(ResourceRequestId);
dfcp_id!(ResourceOfferId);
dfcp_id!(ReservationId);
dfcp_id!(AssignmentId);

/// What the requesting node actually needs. Capability uses the hub taxonomy
/// snake_case name so offers can be matched deterministically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRequest {
    #[serde(default = "dfcp_version")]
    pub protocol_version: u32,
    pub request_id: ResourceRequestId,
    /// Hub taxonomy capability name (e.g. `embeddings`). Validated against
    /// `CapabilityKind` by the receiving side before any matching.
    pub capability: String,
    /// Desired CPU cores for the assist workload.
    pub cpu_cores: u16,
    /// Desired RAM headroom in MiB.
    pub ram_mb: u64,
    /// Maximum lease the requester is willing to work under (seconds).
    pub max_lease_seconds: u64,
}

impl ResourceRequest {
    pub fn new(
        capability: impl Into<String>,
        cpu_cores: u16,
        ram_mb: u64,
        max_lease_seconds: u64,
    ) -> Self {
        Self {
            protocol_version: DFCP_VERSION,
            request_id: ResourceRequestId::new(),
            capability: capability.into(),
            cpu_cores,
            ram_mb,
            max_lease_seconds,
        }
    }
}

/// A candidate worker's answer. The WORKER validates this against ITS owner
/// limits before sending — an offer is a pre-vetted claim, still subject to
/// reservation conflicts at RESERVE time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceOffer {
    #[serde(default = "dfcp_version")]
    pub protocol_version: u32,
    pub offer_id: ResourceOfferId,
    /// Echoes the request being answered.
    pub request_id: ResourceRequestId,
    pub capability: String,
    pub cpu_cores: u16,
    pub ram_mb: u64,
    /// Lease the worker grants (≤ request.max_lease_seconds).
    pub lease_seconds: u64,
    /// Requester-side freshness hint from the worker's last availability
    /// sample (seconds since epoch). Stale offers are rejected by the
    /// requester before selection.
    pub sampled_at_unix_ms: u64,
}

impl ResourceOffer {
    pub fn answering(
        request: &ResourceRequest,
        cpu_cores: u16,
        ram_mb: u64,
        lease_seconds: u64,
    ) -> Self {
        Self {
            protocol_version: DFCP_VERSION,
            offer_id: ResourceOfferId::new(),
            request_id: request.request_id.clone(),
            capability: request.capability.clone(),
            cpu_cores,
            ram_mb,
            lease_seconds: lease_seconds.min(request.max_lease_seconds),
            sampled_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }
}

/// Reserve handshake: the requester locks the offered capacity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReserve {
    #[serde(default = "dfcp_version")]
    pub protocol_version: u32,
    pub offer_id: ResourceOfferId,
    pub request_id: ResourceRequestId,
}

/// The worker's authoritative confirmation. Only after THIS message does the
/// lease exist; the reservation lives in the worker's existing ledger with
/// its TTL enforcement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReserved {
    #[serde(default = "dfcp_version")]
    pub protocol_version: u32,
    pub reservation_id: ReservationId,
    pub offer_id: ResourceOfferId,
    pub lease_seconds: u64,
}

/// Hand off one unit of assist work inside an active lease. Payload stays
/// small and opaque here; heavyweight artifacts keep using their dedicated
/// verified-transfer channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistTaskAssign {
    #[serde(default = "dfcp_version")]
    pub protocol_version: u32,
    pub assignment_id: AssignmentId,
    pub reservation_id: ReservationId,
    /// Hub taxonomy capability name (must match the negotiated one).
    pub capability: String,
    /// Bounded, self-describing JSON payload for the tool/skill runtime.
    #[serde(with = "serde_bytes_base64")]
    pub payload: Vec<u8>,
}

impl AssistTaskAssign {
    pub fn new(
        reservation_id: ReservationId,
        capability: impl Into<String>,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            protocol_version: DFCP_VERSION,
            assignment_id: AssignmentId::new(),
            reservation_id,
            capability: capability.into(),
            payload,
        }
    }
}

/// Worker's answer to an assignment. Success/failure is factual; credit is
/// awarded ONLY after evidence verification downstream — never from this
/// message alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistTaskResult {
    #[serde(default = "dfcp_version")]
    pub protocol_version: u32,
    pub assignment_id: AssignmentId,
    pub success: bool,
    /// Bounded result payload (e.g. the embedding vector, JSON-encoded).
    #[serde(with = "serde_bytes_base64")]
    pub payload: Vec<u8>,
    /// Short machine-readable error when `success == false`.
    #[serde(default)]
    pub error: Option<String>,
}

/// Early release of the lease (normal completion or abort). The worker-side
/// TTL remains the backstop if this message never arrives (crash, partition).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRelease {
    #[serde(default = "dfcp_version")]
    pub protocol_version: u32,
    pub reservation_id: ReservationId,
}

/// Base64 helper: keeps binary payloads transport-safe over the existing
/// JSON-framed request/response channel without inflating the schema.
mod serde_bytes_base64 {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_and_bounds() {
        let req = ResourceRequest::new("embeddings", 2, 512, 30);
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.len() < MAX_DFCP_MESSAGE_BYTES);
        let back: ResourceRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.capability, "embeddings");
        assert_eq!(back.protocol_version, 1);
    }

    #[test]
    fn offer_never_exceeds_requested_lease() {
        let req = ResourceRequest::new("ocr", 1, 256, 20);
        let offer = ResourceOffer::answering(&req, 4, 2048, 999);
        assert_eq!(offer.lease_seconds, 20, "worker grant clamped to request");
    }

    #[test]
    fn unknown_fields_are_rejected_everywhere() {
        let sneaky = r#"{"protocol_version":1,"request_id":"r","capability":"ocr",
            "cpu_cores":1,"ram_mb":10,"max_lease_seconds":5,"peer_override":"evil"}"#;
        assert!(serde_json::from_str::<ResourceRequest>(sneaky).is_err());
    }

    #[test]
    fn wrong_version_is_visible_to_receivers() {
        let old = r#"{"protocol_version":99,"request_id":"r","capability":"ocr",
            "cpu_cores":1,"ram_mb":10,"max_lease_seconds":5}"#;
        let req: ResourceRequest = serde_json::from_str(old).unwrap();
        assert_ne!(req.protocol_version, DFCP_VERSION);
    }

    #[test]
    fn payload_travels_base64_safely() {
        let assign =
            AssistTaskAssign::new(ReservationId::new(), "embeddings", vec![0u8, 159, 146, 255]);
        let json = serde_json::to_string(&assign).unwrap();
        assert!(!json.contains('\u{fffd}'), "binary must be base64-encoded");
        let back: AssistTaskAssign = serde_json::from_str(&json).unwrap();
        assert_eq!(back.payload, vec![0u8, 159, 146, 255]);
    }
}
