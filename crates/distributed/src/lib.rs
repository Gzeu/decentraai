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
//! let mut distributed = DistributedInference::new(p2p_node, config, Some(worker_manager), None)?;
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

pub mod compute;
pub mod config;
pub mod fallback;
pub mod p2p_handler;
pub mod queue;
pub mod router;
pub mod tracker;
pub mod worker;

pub use compute::{ComputeManager, build_advertisement};
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
    InferMessage, InferRequest, InferResponse, TaskPlacement, WorkerAnnouncement, WorkerStatus,
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
    compute_manager: Option<Arc<crate::compute::ComputeManager>>,
    config: InferenceConfig,
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
            compute_manager: None,
            config,
        })
    }

    /// Attaches a [`crate::compute::ComputeManager`] so routing can select
    /// workers from the capability-aware compute registry (hardware + model
    /// matching + resource reservations). The legacy announcement-based
    /// routing remains as a fallback.
    pub fn set_compute_manager(&mut self, compute_manager: Arc<crate::compute::ComputeManager>) {
        self.compute_manager = Some(compute_manager);
    }

    /// Returns the attached compute manager, if any.
    pub fn compute_manager(&self) -> Option<&Arc<crate::compute::ComputeManager>> {
        self.compute_manager.as_ref()
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
        &self.queue_manager
    }

    /// Starts the worker discovery process
    ///
    /// This spawns background tasks for:
    /// - Broadcasting worker announcements (if this node is a worker)
    /// - Listening for worker announcements from peers
    /// - Managing worker heartbeat and stale detection
    pub async fn start_worker_discovery(&mut self) -> anyhow::Result<()> {
        let manager = self.worker_manager.clone();
        manager
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
    ///
    /// The request lifecycle guarantees exactly ONE terminal event per request:
    /// either an `InferResponse` (success) or an `InferFailed` (cancellation or
    /// backend error) — never both, never zero (queue-full is answered
    /// immediately with `InferFailed`).
    pub fn register_worker_backend(
        &mut self,
        backend: decentraai_inference_adapter::OpenAiCompatibleBackend,
        model_hash: String,
    ) -> anyhow::Result<()> {
        use decentraai_protocol::{
            InferMessage, InferResponse, serialize_message,
        };

        let local_peer = self.p2p_node.local_peer_id();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<decentraai_protocol::InferRequest>();
        let tx_clone = tx.clone();
        let p2p_clone = self.p2p_node.clone();
        let queue_mgr = self.queue_manager.clone();
        let backend_clone = backend.clone();
        let worker_manager = self.worker_manager.clone();
        let model_hash_clone = model_hash.clone();

        // Map inbound InferCancel frames to the queue manager so in-flight
        // requests are marked cancelled and the streaming loop aborts.
        let cancel_queue = self.queue_manager.clone();
        self.p2p_node.set_on_cancel_request(move |request_id| {
            let cancel_queue = cancel_queue.clone();
            tokio::spawn(async move {
                let _ = cancel_queue.cancel_request(request_id).await;
            });
        });

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
                return serialize_message(&InferMessage::InferResponse(resp));
            }

            // Send to processing channel (background task will enqueue). If the
            // channel is gone the worker is shutting down; tell the requester.
            if tx_clone.send(req.clone()).is_err() {
                let failed = InferMessage::InferFailed {
                    request_id: req.request_id,
                    worker_peer_id: local_peer,
                    error: "worker is shutting down".to_string(),
                    retryable: false,
                };
                return serialize_message(&failed);
            }

            // Respond with InferAccepted echoing the ORIGINAL request id so the
            // requester can correlate subsequent progress frames.
            let msg = InferMessage::InferAccepted {
                request_id: req.request_id,
                worker_peer_id: local_peer,
                estimated_wait_ms: 10,
            };
            serialize_message(&msg)
        });

        // Spawn background task to process queued requests and stream
        tokio::spawn(async move {
            while let Some(req) = rx.recv().await {
                // Queue the request; a full queue is answered immediately so the
                // requester is never left hanging.
                if !queue_mgr.queue_request(req.clone(), local_peer).await {
                    if let Ok(bytes) = serialize_message(&InferMessage::InferFailed {
                        request_id: req.request_id,
                        worker_peer_id: local_peer,
                        error: "worker queue is full".to_string(),
                        retryable: true,
                    }) {
                        let _ = p2p_clone.request(req.sender_peer_id, bytes).await;
                    }
                    continue;
                }

                // Dequeue and process the request to a single terminal event.
                if let Some(queued) = queue_mgr.dequeue_request(&local_peer).await {
                    stream_request_to_terminal(
                        &backend_clone,
                        &p2p_clone,
                        &queue_mgr,
                        &worker_manager,
                        queued,
                    )
                    .await;
                }
            }
        });

        Ok(())
    }

    /// Routes an inference request to the best available worker
    ///
    /// This will:
    /// 1. Select the best worker (capability-aware when a compute manager is
    ///    attached, otherwise the legacy capacity scheduler)
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
        // Capability-aware compute path: pick a worker that serves the model
        // and has RAM/VRAM headroom, and hold a reservation for the duration.
        if let Some(compute) = &self.compute_manager {
            if let Some(req) = compute.requirements_for(&request.model_hash).await {
                if let Some(placement) = compute.select(&req).await {
                    let task_placement = TaskPlacement {
                        selected_worker: placement.worker,
                        estimated_wait_ms: 10,
                        estimated_time_ms: 0,
                        confidence: placement.confidence,
                    };
                    let result = self
                        .request_router
                        .send_request(&self.p2p_node, request, task_placement)
                        .await;
                    // Release the booking whether or not the request succeeded.
                    compute.release(placement.reservation.reservation_id).await;
                    return result;
                }
            }
        }

        // Legacy path: get the current worker list from the manager (async,
        // never blocks the runtime).
        let workers = self.worker_manager.get_workers().await;

        // Select the best worker for this request
        let placement = self.request_router.select_worker(&request, &workers).await?;

        // Send the request and handle the response
        self.request_router
            .send_request(&self.p2p_node, request, placement)
            .await
    }

    /// Like [`route_request`](Self::route_request) but streams each received
    /// `InferProgress` chunk into `progress` as it arrives.
    pub async fn route_request_streamed(
        &self,
        request: InferRequest,
        progress: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> Result<InferResponse, DistributedError> {
        if let Some(compute) = &self.compute_manager {
            if let Some(req) = compute.requirements_for(&request.model_hash).await {
                if let Some(placement) = compute.select(&req).await {
                    let task_placement = TaskPlacement {
                        selected_worker: placement.worker,
                        estimated_wait_ms: 10,
                        estimated_time_ms: 0,
                        confidence: placement.confidence,
                    };
                    let result = self
                        .request_router
                        .send_request_streamed(&self.p2p_node, request, task_placement, progress)
                        .await;
                    compute.release(placement.reservation.reservation_id).await;
                    return result;
                }
            }
        }

        let workers = self.worker_manager.get_workers().await;
        let placement = self.request_router.select_worker(&request, &workers).await?;
        self.request_router
            .send_request_streamed(&self.p2p_node, request, placement, progress)
            .await
    }

    /// Cancels an in-flight request on a worker by sending an `InferCancel`
    /// frame. The worker aborts generation and replies with
    /// `InferFailed(cancelled)`, which the router reports as an error to the
    /// caller that is still awaiting the request.
    pub async fn cancel_request(
        &self,
        worker_peer_id: libp2p::PeerId,
        request_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        use decentraai_protocol::{InferMessage, serialize_message};
        let msg = InferMessage::InferCancel {
            request_id,
            reason: "cancelled by coordinator".to_string(),
        };
        let bytes = serialize_message(&msg)?;
        self.p2p_node.request(worker_peer_id, bytes).await?;
        Ok(())
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

/// Drives one queued request through the backend stream to exactly ONE
/// terminal event. Normal completion sends a single `InferResponse` with the
/// accumulated output, token count and wall time; cancellation and backend
/// errors each send a single `InferFailed` (retryable=false for cancellation,
/// retryable=true for transient backend failures). The queue is always
/// `complete_request`ed so the worker's in-flight set stays consistent.
async fn stream_request_to_terminal(
    backend: &decentraai_inference_adapter::OpenAiCompatibleBackend,
    p2p: &decentraai_p2p::P2PNode,
    queue: &RequestQueueManager,
    worker_manager: &WorkerManager,
    queued: QueuedRequest,
) {
    use decentraai_inference_adapter::{BackendRequest, InferenceBackend};
    use decentraai_protocol::{InferMessage, InferProgress, InferResponse};

    let request_id = queued.request_id;
    let sender = queued.request.sender_peer_id;
    let trace_id = queued.request.trace_id.clone();
    let local_peer = p2p.local_peer_id();
    let started = std::time::Instant::now();

    // Mark the worker busy for the duration of the request.
    let _ = worker_manager.update_local_capacity(0.0, 0, 0, 0);

    let backend_req = BackendRequest {
        request_id: request_id.to_string(),
        prompt: queued.request.prompt.clone(),
        max_tokens: queued.request.max_tokens,
        temperature: queued.request.temperature,
        top_p: queued.request.top_p,
    };

    let stream = match backend.stream(backend_req).await {
        Ok(stream) => stream,
        Err(e) => {
            send_infer(
                p2p,
                sender,
                InferMessage::InferFailed {
                    request_id,
                    worker_peer_id: local_peer,
                    error: format!("backend error: {e}"),
                    retryable: true,
                },
            )
            .await;
            let _ = queue.complete_request(request_id).await;
            let _ = worker_manager.update_local_capacity(1.0, 0, 50, 100);
            return;
        }
    };

    let mut stream = stream;
    let mut seq: u64 = 0;
    let mut output = String::new();
    let mut terminal_sent = false;

    while let Some(next) = next_stream_item(&mut stream, queue, request_id).await {
        match next {
            NextItem::Chunk(Ok(chunk)) => {
                seq += 1;
                output.push_str(&chunk.text);
                send_infer(
                    p2p,
                    sender,
                    InferMessage::InferProgress(InferProgress {
                        request_id,
                        worker_peer_id: local_peer,
                        tokens_generated: seq as u32,
                        partial_output: chunk.text.clone(),
                        percent_complete: 0.0,
                    }),
                )
                .await;
            }
            NextItem::Chunk(Err(e)) => {
                terminal_sent = true;
                send_infer(
                    p2p,
                    sender,
                    InferMessage::InferFailed {
                        request_id,
                        worker_peer_id: local_peer,
                        error: format!("backend error: {e}"),
                        retryable: true,
                    },
                )
                .await;
                break;
            }
            NextItem::Cancelled => {
                terminal_sent = true;
                send_infer(
                    p2p,
                    sender,
                    InferMessage::InferFailed {
                        request_id,
                        worker_peer_id: local_peer,
                        error: "cancelled".to_string(),
                        retryable: false,
                    },
                )
                .await;
                break;
            }
            NextItem::End => break, // normal completion, terminal below
        }
    }

    if !terminal_sent {
        send_infer(
            p2p,
            sender,
            InferMessage::InferResponse(InferResponse {
                request_id,
                trace_id,
                worker_peer_id: local_peer,
                completed_at: chrono::Utc::now().to_rfc3339(),
                output,
                tokens_used: seq as u32,
                processing_time_ms: started.elapsed().as_millis() as u32,
                success: true,
                error: None,
            }),
        )
        .await;
    }

    let _ = queue.complete_request(request_id).await;
    let _ = worker_manager.update_local_capacity(1.0, 0, 50, 100);
}

/// Sends one InferMessage frame to `sender` via the P2P request/response
/// channel. Errors are logged, never fatal: a dropped requester must not
/// take down the worker loop.
async fn send_infer(p2p: &decentraai_p2p::P2PNode, sender: libp2p::PeerId, msg: InferMessage) {
    if let Ok(bytes) = decentraai_protocol::serialize_message(&msg) {
        if let Err(e) = p2p.request(sender, bytes).await {
            tracing::debug!(%sender, error = %e, "failed to deliver infer frame");
        }
    }
}

/// One unit of a streamed backend response, enriched with the cancellation
/// signal so the caller can distinguish stream end from an abort.
enum NextItem {
    Chunk(Result<decentraai_inference_adapter::StreamChunk, decentraai_inference_adapter::BackendError>),
    End,
    Cancelled,
}

/// Awaits the next streamed chunk OR a cancellation signal, whichever fires
/// first. The cancellation poll makes aborts prompt even while the backend
/// is between chunks (token generation is CPU-bound and may stall briefly).
async fn next_stream_item<S>(
    stream: &mut S,
    queue: &RequestQueueManager,
    request_id: uuid::Uuid,
) -> Option<NextItem>
where
    S: futures::Stream<Item = Result<decentraai_inference_adapter::StreamChunk, decentraai_inference_adapter::BackendError>>
        + Unpin
        + Send,
{
    use futures::StreamExt;
    tokio::select! {
        item = stream.next() => match item {
            Some(r) => Some(NextItem::Chunk(r)),
            None => Some(NextItem::End),
        },
        _ = cancel_poll(queue, request_id) => Some(NextItem::Cancelled),
    }
}

/// Polls the queue's cancellation flag until it is set.
async fn cancel_poll(queue: &RequestQueueManager, request_id: uuid::Uuid) {
    loop {
        if queue.is_cancelled(request_id).await {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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
