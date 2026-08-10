//! Worker P2P inference protocol

use serde::{Deserialize, Serialize};
use libp2p::PeerId;
use uuid::Uuid;

/// Inference request sent to worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferRequest {
    pub request_id: Uuid,
    pub model_hash: String,
    pub prompt: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub timeout_ms: u32,
    pub stream: bool,
    pub priority: u8,  // 0-255, higher = more urgent
}

impl InferRequest {
    pub fn new(
        model_hash: String,
        prompt: String,
        max_tokens: u32,
    ) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            model_hash,
            prompt,
            max_tokens,
            temperature: 0.7,
            top_p: 0.9,
            timeout_ms: 30000,
            stream: false,
            priority: 128,
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_streaming(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }
}

/// Inference response from worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferResponse {
    pub request_id: Uuid,
    pub worker_peer_id: PeerId,
    pub output: String,
    pub tokens_used: u32,
    pub time_ms: u32,
    pub success: bool,
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
    InferCancel {
        request_id: Uuid,
        reason: String,
    },
    
    /// Health check
    InferPing {
        request_id: Uuid,
    },
    
    /// Health response
    InferPong {
        request_id: Uuid,
        latency_ms: u32,
    },
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
    pub available_capacity: f32,  // 0.0 - 1.0
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
#[derive(Debug, Clone)]
pub struct TaskPlacement {
    pub selected_worker: PeerId,
    pub estimated_wait_ms: u32,
    pub estimated_time_ms: u32,
    pub confidence: f32,  // 0.0 - 1.0
}
