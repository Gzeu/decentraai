//! P2P RequestHandler for distributed inference messages
//!
//! This module provides a RequestHandler implementation that can process:
//! - WorkerAnnouncement: Register remote workers
//! - InferRequest: Process inference requests (if this node is a worker)

use anyhow::{Context, Result};
use decentraai_protocol::{InferRequest, InferResponse, WorkerAnnouncement};
use std::sync::Arc;

/// P2P RequestHandler for distributed inference
///
/// Processes WorkerAnnouncement and InferRequest messages received via P2P.
pub struct DistributedP2PHandler {
    /// Worker manager for processing announcements
    worker_manager: Option<Arc<crate::worker::WorkerManager>>,
    /// Inference request handler function
    infer_handler: Option<Arc<dyn Fn(InferRequest) -> Result<InferResponse> + Send + Sync>>,
}

impl DistributedP2PHandler {
    /// Creates a new handler with no callbacks (returns errors for all requests)
    pub fn new() -> Self {
        Self {
            worker_manager: None,
            infer_handler: None,
        }
    }

    /// Creates a new handler with worker manager for processing announcements
    pub fn with_worker_manager(worker_manager: Arc<crate::worker::WorkerManager>) -> Self {
        Self {
            worker_manager: Some(worker_manager),
            infer_handler: None,
        }
    }

    /// Creates a new handler with inference request handler
    pub fn with_infer_handler(
        infer_handler: impl Fn(InferRequest) -> Result<InferResponse> + Send + Sync + 'static,
    ) -> Self {
        Self {
            worker_manager: None,
            infer_handler: Some(Arc::new(infer_handler)),
        }
    }

    /// Creates a new handler with both worker manager and inference handler
    pub fn with_both(
        worker_manager: Arc<crate::worker::WorkerManager>,
        infer_handler: impl Fn(InferRequest) -> Result<InferResponse> + Send + Sync + 'static,
    ) -> Self {
        Self {
            worker_manager: Some(worker_manager),
            infer_handler: Some(Arc::new(infer_handler)),
        }
    }
}

impl decentraai_p2p::RequestHandler for DistributedP2PHandler {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        use decentraai_protocol::deserialize_message;

        // Try to deserialize as WorkerAnnouncement
        if let Ok(announcement) = deserialize_message::<WorkerAnnouncement>(request, request.len())
        {
            if let Some(manager) = &self.worker_manager {
                manager.process_announcement(announcement)?;
            }
            return Ok(Vec::new()); // No response for announcements
        }

        // Try to deserialize as InferRequest
        if let Ok(infer_request) = deserialize_message::<InferRequest>(request, request.len()) {
            if let Some(handler) = &self.infer_handler {
                let response = handler(infer_request)?;
                return Self::serialize_response(&response);
            }
            anyhow::bail!("No inference handler configured");
        }

        // Not a distributed inference message
        anyhow::bail!("Not a distributed inference message")
    }
}

impl DistributedP2PHandler {
    /// Serializes an InferResponse to bytes
    fn serialize_response(response: &InferResponse) -> Result<Vec<u8>> {
        use decentraai_protocol::serialize_message;
        serialize_message(response).context("Failed to serialize InferResponse")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use crate::worker::WorkerManager;
    use decentraai_p2p::RequestHandler;
    use decentraai_protocol::{WorkerAnnouncement, serialize_message, deserialize_message};
    use libp2p::identity::Keypair;

    fn create_test_peer_id() -> libp2p::PeerId {
        let keypair = Keypair::generate_ed25519();
        libp2p::PeerId::from(keypair.public())
    }

    #[test]
    fn test_worker_announcement_handling() {
        let peer_id = create_test_peer_id();
        let config = InferenceConfig::default();
        let worker_manager = Arc::new(WorkerManager::new(peer_id, config));

        let handler = DistributedP2PHandler::with_worker_manager(worker_manager.clone());

        let announcement = WorkerAnnouncement {
            peer_id: create_test_peer_id(),
            node_name: "test-worker".to_string(),
            loaded_models: vec!["model1".to_string()],
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 100,
        };

        let payload = serialize_message(&announcement).unwrap();

        let result = handler.handle(&payload);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty()); // No response expected

        // Verify worker was added
        assert_eq!(worker_manager.worker_count_sync(), 1);
    }

    #[test]
    fn test_infer_request_handling() {
        let peer_id = create_test_peer_id();

        let handler = DistributedP2PHandler::with_infer_handler(move |request| {
            Ok(InferResponse {
                request_id: request.request_id,
                worker_peer_id: peer_id,
                output: "test output".to_string(),
                tokens_used: 10,
                time_ms: 100,
                success: true,
                error: None,
            })
        });

        let request = InferRequest::new("model-hash".to_string(), "test prompt".to_string(), 100);

        let payload = serialize_message(&request).unwrap();

        let result = handler.handle(&payload);
        assert!(result.is_ok());

        let response_bytes = result.unwrap();
        let response: InferResponse =
            deserialize_message(&response_bytes, response_bytes.len()).unwrap();

        assert_eq!(response.output, "test output");
        assert!(response.success);
    }

    #[test]
    fn test_unknown_message() {
        let handler = DistributedP2PHandler::new();

        let result = handler.handle(b"unknown message");
        assert!(result.is_err());
    }
}
