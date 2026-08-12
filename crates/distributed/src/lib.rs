//! Distributed inference for DecentraAI (M9)
//!
//! This crate provides distributed inference capabilities, enabling:
//! - Worker discovery and registration with real-time capacity reporting
//! - Intelligent request routing based on worker capacity, latency, and throughput
//! - Automatic fallback to alternative workers when primary workers fail
//! - Reputation-based compensation for worker contributions
//!
//! # Architecture
//!
//! The distributed inference system consists of three main components:
//!
//! 1. **Worker Manager**: Manages worker registration, heartbeats, and capacity updates
//! 2. **Request Router**: Routes inference requests to the best available worker
//! 3. **Fallback Handler**: Manages retry logic with fallback workers
//!
//! # Example Usage
//!
//! ```no_run
//! use decentraai_distributed::{DistributedInference, InferenceConfig, WorkerManager};
//! use decentraai_p2p::P2PNode;
//! use decentraai_identity::Identity;
//! use std::path::Path;
//! use std::sync::Arc;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let identity = Identity::load(Path::new("identity/key.pem"))?;
//! let p2p_node = P2PNode::new(&identity, 1024 * 1024, 1024 * 1024 * 100, None)?;
//!
//! let config = InferenceConfig::default();
//! let worker_manager = Arc::new(WorkerManager::new(p2p_node.local_peer_id(), config.clone()));
//! let mut distributed = DistributedInference::new(p2p_node, config, Some(worker_manager))?;
//!
//! // Start worker discovery
//! distributed.start_worker_discovery().await?;
//!
//! // Route an inference request
//! // let request = InferRequest::new("model-hash".to_string(), "prompt".to_string(), 100);
//! // let response = distributed.route_request(request).await?;
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

pub mod config;
pub mod fallback;
pub mod p2p_handler;
pub mod queue;
pub mod router;
pub mod tracker;
pub mod worker;

pub use config::InferenceConfig;
pub use error::DistributedError;
pub use fallback::FallbackHandler;
pub use p2p_handler::DistributedP2PHandler;
pub use queue::{QueueProcessResult, QueuedRequest, RequestQueueManager, WorkerRequestQueue};
pub use router::RequestRouter;
pub use tracker::RequestTracker;
pub use worker::WorkerManager;

/// Re-export protocol types for convenience
pub use decentraai_protocol::{
    InferRequest, InferResponse, TaskPlacement, WorkerAnnouncement, WorkerStatus,
};

/// Module for error types
mod error {
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum DistributedError {
        #[error("No workers available for model {0}")]
        NoWorkersAvailable(String),

        #[error("All workers failed for request {0}")]
        AllWorkersFailed(String),

        #[error("Request timeout after {0}ms")]
        RequestTimeout(u64),

        #[error("Worker {0} rejected request: {1}")]
        WorkerRejected(String, String),

        #[error("P2P communication error: {0}")]
        P2PError(#[from] anyhow::Error),

        #[error("Serialization error: {0}")]
        SerializationError(String),

        #[error("Worker {0} is not trusted")]
        UntrustedWorker(String),
    }
}

/// Main distributed inference coordinator
///
/// Combines worker discovery, request routing, and fallback handling
/// into a single interface for distributed inference.
pub struct DistributedInference {
    p2p_node: decentraai_p2p::P2PNode,
    worker_manager: Arc<WorkerManager>,
    request_router: RequestRouter,
    fallback_handler: FallbackHandler,
    queue_manager: Arc<RequestQueueManager>,
    config: InferenceConfig,
    tracker: Option<Arc<RequestTracker>>,
}

impl DistributedInference {
    /// Creates a new distributed inference coordinator
    ///
    /// # Arguments
    ///
    /// * `p2p_node` - The P2P node for network communication
    /// * `config` - Distributed inference configuration
    /// * `worker_manager` - Optional worker manager to use (if None, a new one will be created)
    pub fn new(
        p2p_node: decentraai_p2p::P2PNode,
        config: InferenceConfig,
        worker_manager: Option<Arc<WorkerManager>>,
        tracker: Option<Arc<RequestTracker>>,
    ) -> anyhow::Result<Self> {
        let worker_manager = worker_manager.unwrap_or_else(|| {
            Arc::new(WorkerManager::new(p2p_node.local_peer_id(), config.clone()))
        });
        let request_router = RequestRouter::new(p2p_node.local_peer_id(), tracker.clone());
        let fallback_handler = FallbackHandler::new(config.max_retries);
        let queue_manager = Arc::new(RequestQueueManager::new(
            config.max_queue_depth as usize,
            std::time::Duration::from_millis(config.request_timeout_ms),
        ));

        Ok(Self {
            p2p_node,
            worker_manager,
            request_router,
            fallback_handler,
            queue_manager,
            config,
            tracker,
        })
    }

    /// Returns a reference to the underlying P2P node
    pub fn p2p_node(&self) -> &decentraai_p2p::P2PNode {
        &self.p2p_node
    }

    /// Returns a reference to the worker manager
    pub fn worker_manager(&self) -> &WorkerManager {
        &self.worker_manager
    }

    /// Returns a mutable reference to the worker manager
    pub fn worker_manager_mut(&mut self) -> &mut WorkerManager {
        Arc::get_mut(&mut self.worker_manager).expect("WorkerManager is shared elsewhere")
    }

    /// Returns a clone of the Arc-wrapped worker manager
    pub fn worker_manager_arc(&self) -> Arc<WorkerManager> {
        self.worker_manager.clone()
    }

    /// Returns a reference to the request router
    pub fn request_router(&self) -> &RequestRouter {
        &self.request_router
    }

    /// Returns a reference to the fallback handler
    pub fn fallback_handler(&self) -> &FallbackHandler {
        &self.fallback_handler
    }

    /// Returns a reference to the queue manager
    pub fn queue_manager(&self) -> &RequestQueueManager {
        &*self.queue_manager
    }

    /// Starts the worker discovery process
    ///
    /// This spawns background tasks for:
    /// - Broadcasting worker announcements (if this node is a worker)
    /// - Listening for worker announcements from peers
    /// - Managing worker heartbeat and stale detection
    pub async fn start_worker_discovery(&mut self) -> anyhow::Result<()> {
        self.worker_manager
            .start_discovery(&self.p2p_node, self.config.announcement_interval_ms)
            .await
    }

    /// Registers this node as a worker with the given capabilities
    ///
    /// # Arguments
    ///
    /// * `node_name` - Human-readable name for this worker
    /// * `loaded_models` - List of model hashes this worker can serve
    /// * `initial_capacity` - Initial available capacity (0.0 - 1.0)
    pub fn register_as_worker(
        &mut self,
        node_name: String,
        loaded_models: Vec<String>,
        initial_capacity: f32,
    ) -> anyhow::Result<()> {
        self.worker_manager
            .register_as_worker(node_name, loaded_models, initial_capacity)
    }

    /// Registers a backend to serve inference on this node and installs the
    /// P2P on_infer handler that accepts requests, enqueues them, and streams
    /// progress back to the requester. This method must be called after the
    /// DistributedInference is constructed and takes ownership of the backend.
    pub fn register_worker_backend(
        &mut self,
        backend: decentraai_inference_adapter::OpenAiCompatibleBackend,
        model_hash: String,
    ) -> anyhow::Result<()> {
        use decentraai_inference_adapter::InferenceBackend;
        use decentraai_protocol::{InferProgress, InferMessage, InferResponse, serialize_message};

        let local_peer = self.p2p_node.local_peer_id();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<decentraai_protocol::InferRequest>();
        let tx_clone = tx.clone();
        let p2p_clone = self.p2p_node.clone();
        let queue_mgr = self.queue_manager.clone();
        let backend_clone = backend.clone();
        let model_hash_clone = model_hash.clone();

        // Register sync on_infer handler that enqueues the request and returns Accept
        self.p2p_node.set_on_infer_request(move |req: decentraai_protocol::InferRequest| -> anyhow::Result<Vec<u8>> {
            // Only accept requests for the configured model
            if req.model_hash != model_hash_clone {
                let resp = InferResponse {
                    request_id: req.request_id,
                    trace_id: req.trace_id.clone(),
                    worker_peer_id: local_peer,
                    completed_at: chrono::Utc::now().to_rfc3339(),
                    output: "".to_string(),
                    tokens_used: 0,
                    processing_time_ms: 0,
                    success: false,
                    error: Some("Model not available on this worker".to_string()),
                };
                return Ok(serialize_message(&InferMessage::InferResponse(resp))?);
            }

            // Send to processing channel (background task will enqueue)
            tx_clone.send(req).map_err(|e| anyhow::anyhow!("failed to enqueue infer request: {}", e))?;

            // Respond with InferAccepted (use the original request_id)
            let msg = InferMessage::InferAccepted {
                request_id: uuid::Uuid::new_v4(),
                worker_peer_id: local_peer,
                estimated_wait_ms: 10,
            };
            Ok(serialize_message(&msg)?)
        });

        // Spawn background task to process queued requests and stream
        tokio::spawn(async move {
            use futures::StreamExt;
            while let Some(req) = rx.recv().await {
                // Queue the request
                let _ = queue_mgr.queue_request(req.clone(), local_peer).await;

                // Try to dequeue and process
                while let Some(queued) = queue_mgr.dequeue_request(&local_peer).await {
                    let request_id = queued.request_id;
                    let backend_req = decentraai_inference_adapter::BackendRequest {
                        request_id: request_id.to_string(),
                        prompt: queued.request.prompt.clone(),
                        max_tokens: queued.request.max_tokens,
                        temperature: queued.request.temperature,
                        top_p: queued.request.top_p,
                    };

                    match backend_clone.stream(backend_req).await {
                        Ok(mut stream) => {
                            let mut seq: u64 = 0;
                            while let Some(chunk_res) = stream.next().await {
                                match chunk_res {
                                    Ok(chunk) => {
                                        seq += 1;
                                        let progress = InferProgress {
                                            request_id,
                                            worker_peer_id: local_peer,
                                            tokens_generated: seq as u32,
                                            partial_output: chunk.text.clone(),
                                            percent_complete: 0.0,
                                        };
                                        if let Ok(bytes) = serialize_message(&InferMessage::InferProgress(progress.clone())) {
                                            let _ = p2p_clone.request(queued.request.sender_peer_id.clone(), bytes).await;
                                        }

                                        // Check cancellation
                                        if queue_mgr.is_cancelled(request_id).await {
                                            if let Ok(bytes) = serialize_message(&InferMessage::InferFailed { request_id, worker_peer_id: local_peer, error: "cancelled".to_string(), retryable: false }) {
                                                let _ = p2p_clone.request(queued.request.sender_peer_id.clone(), bytes).await;
                                            }
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        if let Ok(bytes) = serialize_message(&InferMessage::InferFailed { request_id, worker_peer_id: local_peer, error: format!("backend error: {}", e), retryable: false }) {
                                            let _ = p2p_clone.request(queued.request.sender_peer_id.clone(), bytes).await;
                                        }
                                        break;
                                    }
                                }
                            }

                            // On completion send final InferResponse
                            let final_resp = InferResponse {
                                request_id,
                                trace_id: queued.request.trace_id.clone(),
                                worker_peer_id: local_peer,
                                completed_at: chrono::Utc::now().to_rfc3339(),
                                output: "".to_string(),
                                tokens_used: 0,
                                processing_time_ms: 0,
                                success: true,
                                error: None,
                            };
                            if let Ok(bytes) = serialize_message(&InferMessage::InferResponse(final_resp)) {
                                let _ = p2p_clone.request(queued.request.sender_peer_id.clone(), bytes).await;
                            }
                            let _ = queue_mgr.complete_request(request_id).await;
                        }
                        Err(e) => {
                            if let Ok(bytes) = serialize_message(&InferMessage::InferFailed { request_id: queued.request.request_id, worker_peer_id: local_peer, error: format!("backend error: {}", e), retryable: false }) {
                                let _ = p2p_clone.request(queued.request.sender_peer_id.clone(), bytes).await;
                            }
                            let _ = queue_mgr.complete_request(queued.request.request_id).await;
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Routes an inference request to the best available worker
    ///
    /// This will:
    /// 1. Select the best worker using the scheduler
    /// 2. Send the request to that worker
    /// 3. Handle the response or trigger fallback on failure
    ///
    /// # Arguments
    ///
    /// * `request` - The inference request to route
    ///
    /// # Returns
    ///
    /// The inference response from a worker, or an error if all workers fail
    pub async fn route_request(
        &self,
        request: InferRequest,
    ) -> Result<InferResponse, DistributedError> {
        // Get the current worker list from the manager (sync version for now)
        let workers = self.worker_manager.get_workers_sync();

        // Select the best worker for this request
        let placement = self.request_router.select_worker(&request, &workers)?;

        // Send the request and handle the response
        self.request_router
            .send_request(&self.p2p_node, request, placement)
            .await
    }

    /// Broadcasts this worker's current status to all connected peers
    pub async fn broadcast_worker_status(&self) -> anyhow::Result<()> {
        self.worker_manager.broadcast_status(&self.p2p_node).await
    }

    /// Updates the capacity of the local worker
    ///
    /// Call this when the worker's capacity changes (e.g., after loading/unloading models)
    ///
    /// # Arguments
    ///
    /// * `available_capacity` - New available capacity (0.0 - 1.0)
    /// * `queue_depth` - Current queue depth
    /// * `tokens_per_second` - Current throughput
    /// * `current_latency_ms` - Current latency estimate
    pub fn update_local_capacity(
        &mut self,
        available_capacity: f32,
        queue_depth: u32,
        tokens_per_second: u32,
        current_latency_ms: u32,
    ) -> anyhow::Result<()> {
        self.worker_manager.update_local_capacity(
            available_capacity,
            queue_depth,
            tokens_per_second,
            current_latency_ms,
        )
    }

    /// Gets statistics about the distributed inference system (synchronous)
    /// Note: queued_requests will be 0 as it requires async access
    pub fn get_stats(&self) -> DistributedStats {
        DistributedStats {
            worker_count: self.worker_manager.worker_count_sync(),
            local_worker_registered: self.worker_manager.is_registered_sync(),
            pending_requests: self.request_router.pending_requests_sync(),
            total_requests: self.request_router.total_requests_sync(),
            successful_requests: self.request_router.successful_requests_sync(),
            failed_requests: self.request_router.failed_requests_sync(),
            queued_requests: 0, // Async access needed for accurate count
        }
    }

    /// Gets statistics about the distributed inference system (async)
    /// Includes queue information which requires async access
    pub async fn get_stats_async(&self) -> DistributedStats {
        DistributedStats {
            worker_count: self.worker_manager.worker_count_sync(),
            local_worker_registered: self.worker_manager.is_registered_sync(),
            pending_requests: self.request_router.pending_requests_sync(),
            total_requests: self.request_router.total_requests_sync(),
            successful_requests: self.request_router.successful_requests_sync(),
            failed_requests: self.request_router.failed_requests_sync(),
            queued_requests: self.queue_manager.total_queued().await,
        }
    }

    /// Shuts down the underlying P2P node
    pub fn shutdown(self) {
        self.p2p_node.shutdown();
    }
}

/// Statistics about the distributed inference system
#[derive(Debug, Clone, serde::Serialize)]
pub struct DistributedStats {
    pub worker_count: usize,
    pub local_worker_registered: bool,
    pub pending_requests: u64,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub queued_requests: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distributed_stats_default() {
        let stats = DistributedStats {
            worker_count: 0,
            local_worker_registered: false,
            pending_requests: 0,
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            queued_requests: 0,
        };

        assert!(!stats.local_worker_registered);
        assert_eq!(stats.worker_count, 0);
    }
}
