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
    /// Optional compute manager to process ComputeAdvertisement frames
    compute_manager: Option<Arc<crate::compute::ComputeManager>>,
}

impl DistributedP2PHandler {
    /// Creates a new handler with no callbacks (returns errors for all requests)
    pub fn new() -> Self {
        Self {
            worker_manager: None,
            infer_handler: None,
            tracker: None,
            compute_manager: None,
        }
    }

    /// Creates a new handler with worker manager for processing announcements
    pub fn with_worker_manager(worker_manager: Arc<crate::worker::WorkerManager>) -> Self {
        Self {
            worker_manager: Some(worker_manager),
            infer_handler: None,
            tracker: None,
            compute_manager: None,
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
            compute_manager: None,
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
            compute_manager: None,
        }
    }

    /// Attach a RequestTracker so progress messages are delivered to waiting coordinators
    pub fn set_tracker(&mut self, tracker: Arc<crate::tracker::RequestTracker>) {
        self.tracker = Some(tracker);
    }

    /// Attach a ComputeManager so ComputeAdvertisement frames are recorded
    /// into the compute registry and peers can be selected as workers.
    pub fn set_compute_manager(&mut self, compute_manager: Arc<crate::compute::ComputeManager>) {
        self.compute_manager = Some(compute_manager);
    }
}

impl Default for DistributedP2PHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl decentraai_p2p::RequestHandler for DistributedP2PHandler {
    fn handle(&self, request: &[u8]) -> Result<Vec<u8>> {
        use decentraai_protocol::{
            InferMessage, SignedComputeAdvertisement, deserialize_message, serialize_message,
            verify_signed_compute_advertisement,
        };

        // P3: a signed compute advertisement is verified before being trusted.
        // The signer's public key must map to the advertisement's own peer_id
        // (an attacker cannot forge a signature for a peer they don't control).
        if let Ok(signed) = deserialize_message::<SignedComputeAdvertisement>(request, request.len())
        {
            if let Ok(inner) = deserialize_message::<decentraai_compute::ComputeAdvertisement>(
                &signed.advertisement,
                signed.advertisement.len(),
            ) {
                let claiming_peer = inner.peer_id;
                match verify_signed_compute_advertisement(&signed, &claiming_peer) {
                    Ok(()) => {
                        if let Some(manager) = &self.compute_manager {
                            let manager = manager.clone();
                            tokio::spawn(async move {
                                manager.process_advertisement(inner).await;
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!(%claiming_peer, error = %e, "rejected signed advertisement");
                    }
                }
            }
            return Ok(Vec::new()); // No response for advertisements
        }

        // Try to deserialize as a compute advertisement (legacy unsigned)
        if let Ok(adv) =
            deserialize_message::<decentraai_compute::ComputeAdvertisement>(request, request.len())
        {
            if let Some(manager) = &self.compute_manager {
                let manager = manager.clone();
                tokio::spawn(async move {
                    manager.process_advertisement(adv).await;
                });
                return Ok(Vec::new()); // No response for advertisements
            }
            anyhow::bail!("No compute manager configured");
        }

        // Try to deserialize as WorkerAnnouncement
        if let Ok(announcement) = deserialize_message::<WorkerAnnouncement>(request, request.len())
        {
            if let Some(manager) = &self.worker_manager {
                manager.process_announcement(announcement)?;
            }
            return Ok(Vec::new()); // No response for announcements
        }

        // Try to deserialize as a generic InferMessage (progress/response/etc)
        if let Ok(InferMessage::InferPing { .. }) =
            deserialize_message::<InferMessage>(request, request.len())
        {
            // Network probe (M19): a coordinator measures its own wall-clock
            // RTT to this worker by timing the round trip; we just reply with
            // a Pong carrying a nominal processing latency so stats stay honest.
            let pong = InferMessage::InferPong {
                request_id: uuid::Uuid::new_v4(),
                latency_ms: 1,
            };
            let bytes = serialize_message(&pong)?;
            return Ok(bytes);
        }

        // Try to deserialize as a generic InferMessage (progress/response/etc)
        if let Ok(msg) = deserialize_message::<InferMessage>(request, request.len()) {
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
        InferMessage, InferResponse, WorkerAnnouncement, deserialize_message, serialize_message,
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
    fn ping_is_answered_with_pong() {
        let handler = DistributedP2PHandler::new();
        let ping = InferMessage::InferPing {
            request_id: uuid::Uuid::new_v4(),
        };
        let payload = serialize_message(&ping).unwrap();
        let result = handler.handle(&payload);
        assert!(result.is_ok(), "network probe must be answered");
        let bytes = result.unwrap();
        let pong: InferMessage =
            deserialize_message(&bytes, bytes.len()).expect("response is an InferMessage");
        assert!(matches!(pong, InferMessage::InferPong { .. }));
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

    #[tokio::test]
    async fn test_compute_advertisement_handling() {
        use decentraai_compute::{GpuSpec, ServedModel};

        let peer_id = create_test_peer_id();
        let manager = Arc::new(crate::compute::ComputeManager::new(
            peer_id,
            "coordinator".into(),
            std::collections::HashSet::new(),
        ));

        let mut handler = DistributedP2PHandler::new();
        handler.set_compute_manager(manager.clone());
        handler.set_tracker(Arc::new(crate::tracker::RequestTracker::new()));

        let adv = decentraai_compute::ComputeAdvertisement {
            peer_id: create_test_peer_id(),
            node_name: "gpu-rig".into(),
            capability: decentraai_compute::ComputeCapability {
                cpu_cores: 8,
                ram_mb: 32 * 1024,
                gpu: Some(GpuSpec {
                    name: "RTX 4090".into(),
                    vram_mb: 24 * 1024,
                    driver: "565".into(),
                }),
                engine: "llama_server".into(),
                served_models: vec![ServedModel {
                    model_hash: "abc".into(),
                    file_name: "model.gguf".into(),
                    size_mb: 2048,
                    est_ram_mb: 256,
                    est_vram_mb: 3072,
                    context_tokens: 0,
                }],
                can_provision: false,
            },
            availability: decentraai_compute::ComputeAvailability {
                available_ram_mb: 16 * 1024,
                available_vram_mb: Some(18 * 1024),
                load_percent: 10,
                queue_depth: 0,
                tokens_per_second: 60,
                current_latency_ms: 90,
                status: decentraai_compute::WorkerHealth::Ready,
            },
            announced_at_ms: 1_700_000_000_000,
        };

        let payload = serialize_message(&adv).unwrap();
        let result = handler.handle(&payload);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());

        // The advertisement is processed on a spawned task; yield so it runs.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let workers = manager.workers().await;
        assert_eq!(
            workers.len(),
            1,
            "advertisement lands in the compute registry"
        );
        assert_eq!(workers[0].node_name, "gpu-rig");
    }
}
