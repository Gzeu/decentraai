//! Worker P2P inference protocol

use decentraai_identity::Identity;
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{b64_opt, canonical_infer_request_bytes};

/// Inference request sent to worker (M10)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferRequest {
    /// Unique request identifier for the entire lifecycle
    pub request_id: Uuid,
    /// Trace ID for correlation across distributed components
    pub trace_id: String,
    /// When the request was created (ISO 8601)
    pub created_at: String,
    /// When the request must complete (ISO 8601)
    pub deadline_at: String,
    /// Hash of the model to use
    pub model_hash: String,
    /// Peer that initiated the request (for audit and routing)
    pub sender_peer_id: PeerId,
    /// User prompt
    pub prompt: String,
    /// Maximum tokens to generate
    pub max_tokens: u32,
    /// Sampling temperature (0.0-2.0)
    pub temperature: f32,
    /// Top-p (nucleus) sampling
    pub top_p: f32,
    /// Maximum time to wait in milliseconds
    pub timeout_ms: u32,
    /// Whether response should be streamed
    pub stream: bool,
    /// Request priority (0-255, higher = more urgent)
    pub priority: u8,
    /// Optional session identifier (M20). Continuation requests carry the
    /// same `session_id` as an earlier request so the coordinator can prefer
    /// the worker that already holds the KV prefix (cache locality). `None`
    /// for a cold / stateless request. Backward-compatible: older requests
    /// without the field deserialize to `None`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Monotonic per-destination counter set by the sender for replay
    /// protection (P4). Never reused for the same (sender, worker) edge.
    #[serde(default)]
    pub nonce: u64,
    /// Ed25519 public key (32 bytes) of the sender, when signed. Used by the
    /// worker to (a) verify the request signature and (b) confirm it matches
    /// the authenticated connected peer (P1/P2). Legacy unsigned frames omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_public_key: Option<[u8; 32]>,
    /// Ed25519 signature over canonical bytes of the request (including
    /// `nonce`). Produced with the sender identity (P1). `None` for a legacy
    /// unsigned frame. Base64 on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "b64_opt")]
    pub signature: Option<Vec<u8>>,
}

impl InferRequest {
    pub fn new(model_hash: String, prompt: String, max_tokens: u32) -> Self {
        let now = chrono::Utc::now();
        let deadline = now + chrono::Duration::seconds(30);
        Self {
            request_id: Uuid::new_v4(),
            trace_id: format!("tr_{}", Uuid::new_v4()),
            created_at: now.to_rfc3339(),
            deadline_at: deadline.to_rfc3339(),
            model_hash,
            prompt,
            max_tokens,
            sender_peer_id: PeerId::random(),
            temperature: 0.7,
            top_p: 0.9,
            timeout_ms: 30000,
            stream: false,
            priority: 128,
            session_id: None,
            nonce: 0,
            sender_public_key: None,
            signature: None,
        }
    }

    pub fn with_sender(mut self, peer_id: PeerId) -> Self {
        self.sender_peer_id = peer_id;
        self
    }

    pub fn with_session(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_streaming(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    pub fn with_nonce(mut self, nonce: u64) -> Self {
        self.nonce = nonce;
        self
    }

    /// Signs this request with the node identity (P1). Sets the sender public
    /// key and the Ed25519 signature over canonical bytes (which include the
    /// `nonce`, so a captured request cannot be re-minted without the key).
    /// Returns `self` so callers can chain.
    pub fn sign(mut self, identity: &Identity) -> Self {
        self.sender_public_key = Some(identity.public_key().to_bytes());
        let bytes = canonical_infer_request_bytes(&self);
        self.signature = Some(identity.sign(&bytes).to_bytes().to_vec());
        self
    }

    /// Whether this frame carries a valid signature for `expected_peer`.
    pub fn is_signed(&self) -> bool {
        self.signature.is_some() && self.sender_public_key.is_some()
    }
}

/// Inference response from worker (M10)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferResponse {
    /// Request ID from the original request
    pub request_id: Uuid,
    /// Trace ID from the original request (for correlation)
    pub trace_id: String,
    /// Worker that processed the request
    pub worker_peer_id: PeerId,
    /// When the request was completed (ISO 8601)
    pub completed_at: String,
    /// Complete output text
    pub output: String,
    /// Total tokens used (input + output)
    pub tokens_used: u32,
    /// Time spent in milliseconds (not including queue time)
    pub processing_time_ms: u32,
    /// Whether the request succeeded
    pub success: bool,
    /// Error message if failed (None if success=true)
    pub error: Option<String>,
}

/// Inference progress for streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferProgress {
    pub request_id: Uuid,
    pub worker_peer_id: PeerId,
    pub tokens_generated: u32,
    pub partial_output: String,
    pub percent_complete: f32,
}

/// P2P inference message types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
#[serde(rename_all = "snake_case")]
pub enum InferMessage {
    /// Request inference from worker
    InferRequest(InferRequest),

    /// Worker accepted request
    InferAccepted {
        request_id: Uuid,
        worker_peer_id: PeerId,
        estimated_wait_ms: u32,
    },

    /// Progress update (streaming)
    InferProgress(InferProgress),

    /// Final result
    InferResponse(InferResponse),

    /// Request failed
    InferFailed {
        request_id: Uuid,
        worker_peer_id: PeerId,
        error: String,
        retryable: bool,
    },

    /// Cancel request
    InferCancel { request_id: Uuid, reason: String },

    /// Health check
    InferPing { request_id: Uuid },

    /// Health response
    InferPong { request_id: Uuid, latency_ms: u32 },
}

impl InferMessage {
    pub fn request_id(&self) -> Uuid {
        match self {
            Self::InferRequest(req) => req.request_id,
            Self::InferAccepted { request_id, .. } => *request_id,
            Self::InferProgress(prog) => prog.request_id,
            Self::InferResponse(resp) => resp.request_id,
            Self::InferFailed { request_id, .. } => *request_id,
            Self::InferCancel { request_id, .. } => *request_id,
            Self::InferPing { request_id } => *request_id,
            Self::InferPong { request_id, .. } => *request_id,
        }
    }
}

/// Worker capacity status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub peer_id: PeerId,
    pub loaded_models: Vec<String>,
    pub queue_depth: u32,
    pub available_capacity: f32, // 0.0 - 1.0
    pub current_latency_ms: u32,
    pub tokens_per_second: u32,
}

impl WorkerStatus {
    pub fn can_accept_request(&self, model_hash: &str) -> bool {
        self.loaded_models.contains(&model_hash.to_string())
            && self.available_capacity > 0.1
            && self.queue_depth < 10
    }
}

/// Task placement result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPlacement {
    pub selected_worker: PeerId,
    pub estimated_wait_ms: u32,
    pub estimated_time_ms: u32,
    pub confidence: f32, // 0.0 - 1.0
}

/// Worker announcement for discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerAnnouncement {
    pub peer_id: PeerId,
    pub node_name: String,
    pub loaded_models: Vec<String>,
    pub available_capacity: f32,
    pub queue_depth: u32,
    pub tokens_per_second: u32,
    pub current_latency_ms: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_placement_serde_round_trip() {
        let original = TaskPlacement {
            selected_worker: PeerId::random(),
            estimated_wait_ms: 120,
            estimated_time_ms: 5000,
            confidence: 0.95,
        };

        let json = serde_json::to_string(&original).expect("serialize should succeed");
        let decoded: TaskPlacement =
            serde_json::from_str(&json).expect("deserialize should succeed");

        assert_eq!(decoded.selected_worker, original.selected_worker);
        assert_eq!(decoded.estimated_wait_ms, original.estimated_wait_ms);
        assert_eq!(decoded.estimated_time_ms, original.estimated_time_ms);
        assert_eq!(decoded.confidence, original.confidence);
    }
}
