//! P2P RequestHandler for distributed inference messages
//!
//! This module provides a RequestHandler implementation that can process:
//! - WorkerAnnouncement: Register remote workers
//! - InferRequest: Process inference requests (if this node is a worker)

use anyhow::Result;
use decentraai_protocol::{InferRequest, WorkerAnnouncement};
use std::sync::Arc;

/// Inference request handler used by the legacy sync serving path. The
/// streaming worker path (queue → backend → progress) lives in
/// `DistributedInference::register_worker_backend` and is wired through the
/// P2PNode's `on_infer` callback instead.
type InferHandler = Arc<dyn Fn(InferRequest) -> Result<Vec<u8>> + Send + Sync>;

/// P2P RequestHandler for distributed inference
///
/// Processes WorkerAnnouncement and InferRequest messages received via P2P.
pub struct DistributedP2PHandler {
    /// Worker manager for processing announcements
    worker_manager: Option<Arc<crate::worker::WorkerManager>>,
    /// Inference request handler function (synchronous callback that may spawn async work)
    infer_handler: Option<InferHandler>,
    /// Optional tracker to deliver progress / final messages back to waiting coordinators
    tracker: Option<Arc<crate::tracker::RequestTracker>>,
}

impl DistributedP2PHandler {
    /// Creates a new handler with no callbacks (returns errors for all requests)
    pub fn new() -> Self {
        Self {
            worker_manager: None,
            infer_handler: None,
            tracker: None,
        }
    }

    /// Creates a new handler with worker manager for processing announcements
    pub fn with_worker_manager(worker_manager: Arc<crate::worker::WorkerManager>) -> Self {
        Self {
            worker_manager: Some(worker_manager),
            infer_handler: None,
            tracker: None,
        }
    }

    /// Creates a new handler with inference request handler
    pub fn with_infer_handler(
        infer_handler: impl Fn(InferRequest) -> Result<Vec<u8>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            worker_manager: None,
            infer_handler: Some(Arc::new(infer_handler)),
            tracker: None,
        }
    }

    /// Creates a new handler with both worker manager and inference handler
    pub fn with_both(
        worker_manager: Arc<crate::worker::WorkerManager>,
        infer_handler: impl Fn(InferRequest) -> Result<Vec<u8>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            worker_manager: Some(worker_manager),
            infer_handler: Some(Arc::new(infer_handler)),
            tracker: None,
        }
    }

    /// Attach a RequestTracker so progress messages are delivered to waiting coordinators
    pub fn set_tracker(&mut self, tracker: Arc<crate::tracker::RequestTracker>) {
        self.tracker = Some(tracker);
    }
}

impl Default for DistributedP2PHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl decentraai_p2p::RequestHandler for DistributedP2PHandler {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        use decentraai_protocol::{InferMessage, deserialize_message};

        // Try to deserialize as WorkerAnnouncement
        if let Ok(announcement) = deserialize_message::<WorkerAnnouncement>(request, request.len())
        {
            if let Some(manager) = &self.worker_manager {
                manager.process_announcement(announcement)?;
            }
            return Ok(Vec::new()); // No response for announcements
        }

        // Try to deserialize as a generic InferMessage (progress/response/etc)
        if let Ok(msg) = deserialize_message::<InferMessage>(request, request.len()) {
            // If a tracker is registered, deliver the message there for the waiting coordinator
            if let Some(tracker) = &self.tracker {
                let _ = futures::executor::block_on(tracker.deliver(msg));
                return Ok(Vec::new());
            }
            anyhow::bail!("No tracker configured to receive infer messages");
        }

        // Try to deserialize as InferRequest
        if let Ok(infer_request) = deserialize_message::<InferRequest>(request, request.len()) {
            if let Some(handler) = &self.infer_handler {
                // The handler returns raw bytes so it may serialize an InferAccepted
                // message and spawn background tasks for streaming progress.
                let response_bytes = handler(infer_request)?;
                return Ok(response_bytes);
            }
            anyhow::bail!("No inference handler configured");
        }

        // Not a distributed inference message
        anyhow::bail!("Not a distributed inference message")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use crate::worker::WorkerManager;
    use chrono::Utc;
    use decentraai_p2p::RequestHandler;
    use decentraai_protocol::{
        InferResponse, WorkerAnnouncement, deserialize_message, serialize_message,
    };
    use libp2p::identity::Keypair;

    fn create_test_peer_id() -> libp2p::PeerId {
        let keypair = Keypair::generate_ed25519();
        libp2p::PeerId::from(keypair.public())
    }

    /// Serializes an InferResponse for the legacy sync handler closures,
    /// which are typed `Fn(InferRequest) -> Result<Vec<u8>>`.
    fn resp_bytes(resp: InferResponse) -> anyhow::Result<Vec<u8>> {
        decentraai_protocol::serialize_message(&resp)
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
            let resp = InferResponse {
                request_id: request.request_id,
                trace_id: request.trace_id.clone(),
                worker_peer_id: peer_id,
                completed_at: Utc::now().to_rfc3339(),
                output: "test output".to_string(),
                tokens_used: 10,
                processing_time_ms: 100,
                success: true,
                error: None,
            };
            Ok(decentraai_protocol::serialize_message(&resp).unwrap())
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

    #[test]
    fn test_infer_request_with_both_handlers() {
        // Test that both worker_manager and infer_handler work together
        let peer_id = create_test_peer_id();
        let config = InferenceConfig::default();
        let worker_manager = Arc::new(WorkerManager::new(peer_id, config));

        let handler = DistributedP2PHandler::with_both(
            worker_manager.clone(),
            move |request: InferRequest| {
                resp_bytes(InferResponse {
                    request_id: request.request_id,
                    trace_id: request.trace_id.clone(),
                    worker_peer_id: peer_id,
                    completed_at: Utc::now().to_rfc3339(),
                    output: format!("Processed: {}", request.prompt),
                    tokens_used: request.max_tokens,
                    processing_time_ms: 50,
                    success: true,
                    error: None,
                })
            },
        );

        // Test worker announcement handling
        let announcement = WorkerAnnouncement {
            peer_id: create_test_peer_id(),
            node_name: "test-worker".to_string(),
            loaded_models: vec!["model1".to_string()],
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 100,
        };
        let announcement_payload = serialize_message(&announcement).unwrap();
        let result = handler.handle(&announcement_payload);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
        assert_eq!(worker_manager.worker_count_sync(), 1);

        // Test inference request handling
        let request = InferRequest::new("model-hash".to_string(), "test prompt".to_string(), 100);
        let request_payload = serialize_message(&request).unwrap();
        let result = handler.handle(&request_payload);
        assert!(result.is_ok());
        let response_bytes = result.unwrap();
        let response: InferResponse =
            deserialize_message(&response_bytes, response_bytes.len()).unwrap();
        assert_eq!(response.output, "Processed: test prompt");
        assert!(response.success);
    }

    #[test]
    fn test_infer_request_lifecycle_states() {
        // Test request lifecycle: received -> processed -> completed
        let peer_id = create_test_peer_id();

        // Track request states through the handler
        let handler = DistributedP2PHandler::with_infer_handler(move |request: InferRequest| {
            // Simulate request processing lifecycle
            // 1. Request received (implicit)
            // 2. Request validated (implicit in handler)
            // 3. Request processed
            // 4. Response completed

            resp_bytes(InferResponse {
                request_id: request.request_id,
                trace_id: request.trace_id.clone(),
                worker_peer_id: peer_id,
                completed_at: Utc::now().to_rfc3339(),
                output: "completed".to_string(),
                tokens_used: 100,
                processing_time_ms: 10,
                success: true,
                error: None,
            })
        });

        let request =
            InferRequest::new("model-hash".to_string(), "lifecycle test".to_string(), 100);
        let original_request_id = request.request_id;

        let payload = serialize_message(&request).unwrap();
        let result = handler.handle(&payload);

        assert!(result.is_ok());
        let response_bytes = result.unwrap();
        let response: InferResponse =
            deserialize_message(&response_bytes, response_bytes.len()).unwrap();

        // Verify request lifecycle completion
        assert_eq!(response.request_id, original_request_id);
        assert_eq!(response.output, "completed");
        assert!(response.success);
        assert_eq!(response.tokens_used, 100);
    }

    #[test]
    fn test_infer_request_model_not_available() {
        let peer_id = create_test_peer_id();

        let handler = DistributedP2PHandler::with_infer_handler(move |request: InferRequest| {
            if request.model_hash != "expected-hash" {
                anyhow::bail!("Model not available on this worker");
            }
            resp_bytes(InferResponse {
                request_id: request.request_id,
                trace_id: request.trace_id.clone(),
                worker_peer_id: peer_id,
                completed_at: Utc::now().to_rfc3339(),
                output: "test".to_string(),
                tokens_used: 0,
                processing_time_ms: 0,
                success: true,
                error: None,
            })
        });

        // Request with wrong model hash
        let request = InferRequest::new("wrong-hash".to_string(), "test".to_string(), 100);
        let payload = serialize_message(&request).unwrap();
        let result = handler.handle(&payload);

        // Should fail with model not available error
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Model not available")
        );
    }

    #[test]
    fn test_infer_request_validation_error() {
        let peer_id = create_test_peer_id();

        let handler = DistributedP2PHandler::with_infer_handler(move |request: InferRequest| {
            // Simulate validation error
            if request.prompt.is_empty() {
                anyhow::bail!("Prompt cannot be empty");
            }
            resp_bytes(InferResponse {
                request_id: request.request_id,
                trace_id: request.trace_id.clone(),
                worker_peer_id: peer_id,
                completed_at: Utc::now().to_rfc3339(),
                output: "test".to_string(),
                tokens_used: 0,
                processing_time_ms: 0,
                success: true,
                error: None,
            })
        });

        // Request with empty prompt
        let request = InferRequest::new("hash".to_string(), "".to_string(), 100);
        let payload = serialize_message(&request).unwrap();
        let result = handler.handle(&payload);

        // Should fail with validation error
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Prompt cannot be empty")
        );
    }

    #[test]
    fn test_infer_request_with_error_response() {
        let peer_id = create_test_peer_id();

        let handler = DistributedP2PHandler::with_infer_handler(move |request: InferRequest| {
            // Simulate a backend error that returns an error response
            if request.model_hash == "error-model" {
                return resp_bytes(InferResponse {
                    request_id: request.request_id,
                    trace_id: request.trace_id.clone(),
                    worker_peer_id: peer_id,
                    completed_at: Utc::now().to_rfc3339(),
                    output: "".to_string(),
                    tokens_used: 0,
                    processing_time_ms: 0,
                    success: false,
                    error: Some("Backend timeout".to_string()),
                });
            }
            resp_bytes(InferResponse {
                request_id: request.request_id,
                trace_id: request.trace_id.clone(),
                worker_peer_id: peer_id,
                completed_at: Utc::now().to_rfc3339(),
                output: "success".to_string(),
                tokens_used: 10,
                processing_time_ms: 10,
                success: true,
                error: None,
            })
        });

        // Request that triggers error response
        let request = InferRequest::new("error-model".to_string(), "test".to_string(), 100);
        let payload = serialize_message(&request).unwrap();
        let result = handler.handle(&payload);

        assert!(result.is_ok());
        let response_bytes = result.unwrap();
        let response: InferResponse =
            deserialize_message(&response_bytes, response_bytes.len()).unwrap();

        // Response should indicate failure
        assert!(!response.success);
        assert_eq!(response.error, Some("Backend timeout".to_string()));
    }
}
