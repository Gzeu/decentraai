//! Request routing for distributed inference
//!
//! This module provides intelligent request routing based on:
//! - Worker capacity and availability
//! - Current queue depth
//! - Historical latency and throughput
//! - Model availability

use std::collections::HashMap;
use std::sync::Arc;
use libp2p::PeerId;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use decentraai_discovery::scheduler::{WorkerScheduler, SchedulerConfig};
use decentraai_protocol::{InferRequest, InferResponse, WorkerAnnouncement, WorkerStatus, TaskPlacement};
use decentraai_p2p::P2PNode;

use crate::{DistributedError, InferenceConfig};

/// Manages request routing to distributed workers
///
/// The RequestRouter uses a scheduler to select the best worker for each request
/// and handles sending requests via the P2P network.
pub struct RequestRouter {
    /// The peer ID of the local node
    local_peer_id: PeerId,
    
    /// The worker scheduler for selecting optimal workers
    scheduler: Arc<Mutex<WorkerScheduler>>,
    
    /// Counters for statistics
    total_requests: Arc<Mutex<u64>>,
    successful_requests: Arc<Mutex<u64>>,
    failed_requests: Arc<Mutex<u64>>,
    pending_requests: Arc<Mutex<u64>>,
}

impl RequestRouter {
    /// Creates a new RequestRouter
    ///
    /// # Arguments
    ///
    /// * `local_peer_id` - The peer ID of the local node
    pub fn new(local_peer_id: PeerId) -> Self {
        let scheduler_config = SchedulerConfig {
            max_queue_depth: 10,
            min_available_capacity: 0.1,
            enable_load_balancing: true,
            fallback_timeout_ms: 5000,
        };
        
        let scheduler = WorkerScheduler::new(scheduler_config);

        Self {
            local_peer_id,
            scheduler: Arc::new(Mutex::new(scheduler)),
            total_requests: Arc::new(Mutex::new(0)),
            successful_requests: Arc::new(Mutex::new(0)),
            failed_requests: Arc::new(Mutex::new(0)),
            pending_requests: Arc::new(Mutex::new(0)),
        }
    }

    /// Updates the scheduler with the current list of workers
    ///
    /// Call this when the worker list changes to ensure the scheduler
    /// has up-to-date information.
    ///
    /// # Arguments
    ///
    /// * `workers` - List of current worker announcements
    pub async fn update_workers(&self, workers: Vec<WorkerAnnouncement>) {
        let mut scheduler = self.scheduler.lock().await;
        
        // Clear existing workers
        // Note: This is a simplification; in production, we'd want to
        // only update changed workers
        
        // Register all workers with the scheduler
        for worker in workers {
            scheduler.register_worker(worker);
        }
    }

    /// Selects the best worker for a request
    ///
    /// Uses the scheduler's multi-factor scoring algorithm to select
    /// the optimal worker for the given request.
    ///
    /// # Arguments
    ///
    /// * `request` - The inference request to route
    /// * `workers` - List of available workers
    ///
    /// # Returns
    ///
    /// A TaskPlacement with the selected worker and estimates, or an error
    pub fn select_worker(
        &self,
        request: &InferRequest,
        workers: &[WorkerAnnouncement],
    ) -> Result<TaskPlacement, DistributedError> {
        // Update the scheduler with current workers
        // Note: This is synchronous, so we use a blocking lock
        // In a real implementation, we'd want to maintain the worker list
        // separately or use an async-friendly approach
        
        let mut scheduler = self.scheduler.blocking_lock();
        
        // Clear and re-register workers
        // This is a simplification for now
        for worker in workers {
            scheduler.register_worker(worker.clone());
        }
        
        // Select the best worker
        match scheduler.select_worker(request) {
            Some(placement) => Ok(placement),
            None => Err(DistributedError::NoWorkersAvailable(request.model_hash.clone())),
        }
    }

    /// Sends a request to a specific worker
    ///
    /// Serializes the request and sends it via the P2P network to the selected worker.
    ///
    /// # Arguments
    ///
    /// * `p2p_node` - The P2P node for network communication
    /// * `request` - The inference request to send
    /// * `placement` - The task placement specifying the target worker
    ///
    /// # Returns
    ///
    /// The inference response from the worker, or an error
    pub async fn send_request(
        &self,
        p2p_node: &P2PNode,
        request: InferRequest,
        placement: TaskPlacement,
    ) -> Result<InferResponse, DistributedError> {
        // Increment counters
        self.increment_total().await;
        self.increment_pending().await;
        
        let worker_peer_id = placement.selected_worker;
        
        debug!(
            request_id = %request.request_id,
            worker_peer_id = %worker_peer_id,
            model_hash = %request.model_hash,
            "sending request to worker"
        );

        // Serialize the request
        let payload = Self::serialize_request(&request)?;
        
        // Send via P2P
        // Note: This is where we need to extend the P2P node to handle
        // InferRequest messages properly
        
        // For now, we'll use the generic request method
        // In production, we'd want a dedicated method for inference requests
        let response_bytes = match p2p_node.request(worker_peer_id, payload).await {
            Ok(bytes) => bytes,
            Err(e) => {
                self.increment_failed().await;
                self.decrement_pending().await;
                return Err(DistributedError::P2PError(e));
            }
        };

        // Deserialize the response
        let response = Self::deserialize_response(&response_bytes)?;
        
        // Decrement pending and increment success
        self.decrement_pending().await;
        self.increment_success().await;
        
        info!(
            request_id = %request.request_id,
            worker_peer_id = %worker_peer_id,
            tokens_used = response.tokens_used,
            time_ms = response.time_ms,
            "request completed successfully"
        );

        Ok(response)
    }

    /// Routes a request to the best available worker
    ///
    /// Combines worker selection and request sending into a single operation.
    ///
    /// # Arguments
    ///
    /// * `p2p_node` - The P2P node for network communication
    /// * `request` - The inference request to route
    /// * `workers` - List of available workers
    ///
    /// # Returns
    ///
    /// The inference response from a worker, or an error
    pub async fn route_request(
        &self,
        p2p_node: &P2PNode,
        request: InferRequest,
        workers: Vec<WorkerAnnouncement>,
    ) -> Result<InferResponse, DistributedError> {
        // Update the scheduler with current workers
        self.update_workers(workers.clone()).await;
        
        // Select the best worker
        let placement = self.select_worker(&request, &workers)?;
        
        // Send the request
        self.send_request(p2p_node, request, placement).await
    }

    /// Serializes an inference request to bytes
    fn serialize_request(request: &InferRequest) -> Result<Vec<u8>, DistributedError> {
        use decentraai_protocol::serialize_message;
        
        serialize_message(request)
            .map_err(|e| DistributedError::SerializationError(e.to_string()))
    }

    /// Deserializes an inference response from bytes
    fn deserialize_response(bytes: &[u8]) -> Result<InferResponse, DistributedError> {
        use decentraai_protocol::deserialize_message;
        
        deserialize_message::<InferResponse>(bytes, bytes.len())
            .map_err(|e| DistributedError::SerializationError(e.to_string()))
    }

    /// Increment total request counter
    async fn increment_total(&self) {
        let mut total = self.total_requests.lock().await;
        *total += 1;
    }

    /// Increment successful request counter
    async fn increment_success(&self) {
        let mut success = self.successful_requests.lock().await;
        *success += 1;
    }

    /// Increment failed request counter
    async fn increment_failed(&self) {
        let mut failed = self.failed_requests.lock().await;
        *failed += 1;
    }

    /// Increment pending request counter
    async fn increment_pending(&self) {
        let mut pending = self.pending_requests.lock().await;
        *pending += 1;
    }

    /// Decrement pending request counter
    async fn decrement_pending(&self) {
        let mut pending = self.pending_requests.lock().await;
        if *pending > 0 {
            *pending -= 1;
        }
    }

    /// Returns the number of total requests
    pub async fn total_requests(&self) -> u64 {
        *self.total_requests.lock().await
    }

    /// Returns the number of successful requests
    pub async fn successful_requests(&self) -> u64 {
        *self.successful_requests.lock().await
    }

    /// Returns the number of failed requests
    pub async fn failed_requests(&self) -> u64 {
        *self.failed_requests.lock().await
    }

    /// Returns the number of pending requests
    pub async fn pending_requests(&self) -> u64 {
        *self.pending_requests.lock().await
    }

    /// Returns the number of pending requests (synchronous)
    pub fn pending_requests_sync(&self) -> u64 {
        *self.pending_requests.blocking_lock()
    }

    /// Returns the number of total requests (synchronous)
    pub fn total_requests_sync(&self) -> u64 {
        *self.total_requests.blocking_lock()
    }

    /// Returns the number of successful requests (synchronous)
    pub fn successful_requests_sync(&self) -> u64 {
        *self.successful_requests.blocking_lock()
    }

    /// Returns the number of failed requests (synchronous)
    pub fn failed_requests_sync(&self) -> u64 {
        *self.failed_requests.blocking_lock()
    }
}

/// Handler for incoming inference requests
///
/// This can be used as a RequestHandler for the P2P node to process
/// InferRequest messages.
pub struct InferenceRequestHandler;

#[async_trait::async_trait]
impl decentraai_p2p::RequestHandler for InferenceRequestHandler {
    fn handle(&self, request: &[u8]) -> anyhow::Result<Vec<u8>> {
        // Try to deserialize as an InferRequest
        use decentraai_protocol::deserialize_message;
        
        let infer_request: InferRequest = deserialize_message(request, request.len())?;
        
        // In a real implementation, this would:
        // 1. Validate the request
        // 2. Check if we can serve the requested model
        // 3. Process the request (or queue it)
        // 4. Return a response or error
        
        // For now, return an error indicating this is not implemented
        Err(anyhow::anyhow!("Inference request handling not implemented in P2P handler"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    fn create_test_peer_id() -> PeerId {
        let keypair = Keypair::generate_ed25519();
        PeerId::from(keypair.public())
    }

    fn create_test_request() -> InferRequest {
        InferRequest::new(
            "test-model-hash".to_string(),
            "test prompt".to_string(),
            100,
        )
    }

    #[test]
    fn test_request_router_creation() {
        let peer_id = create_test_peer_id();
        let router = RequestRouter::new(peer_id);
        
        assert_eq!(router.local_peer_id, peer_id);
        assert_eq!(router.total_requests_sync(), 0);
        assert_eq!(router.successful_requests_sync(), 0);
        assert_eq!(router.failed_requests_sync(), 0);
        assert_eq!(router.pending_requests_sync(), 0);
    }

    #[test]
    fn test_serialize_deserialize_request() {
        let request = create_test_request();
        
        let serialized = RequestRouter::serialize_request(&request).unwrap();
        let deserialized: InferRequest = decentraai_protocol::deserialize_message(
            &serialized,
            serialized.len(),
        ).unwrap();
        
        assert_eq!(deserialized.request_id, request.request_id);
        assert_eq!(deserialized.model_hash, request.model_hash);
        assert_eq!(deserialized.prompt, request.prompt);
        assert_eq!(deserialized.max_tokens, request.max_tokens);
    }

    #[test]
    fn test_select_worker_with_workers() {
        let peer_id = create_test_peer_id();
        let router = RequestRouter::new(peer_id);
        
        let worker_peer_id = create_test_peer_id();
        let workers = vec![WorkerAnnouncement {
            peer_id: worker_peer_id,
            node_name: "test-worker".to_string(),
            loaded_models: vec!["test-model-hash".to_string()],
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 100,
        }];
        
        let request = create_test_request();
        let placement = router.select_worker(&request, &workers).unwrap();
        
        assert_eq!(placement.selected_worker, worker_peer_id);
    }

    #[test]
    fn test_select_worker_no_matching_model() {
        let peer_id = create_test_peer_id();
        let router = RequestRouter::new(peer_id);
        
        let worker_peer_id = create_test_peer_id();
        let workers = vec![WorkerAnnouncement {
            peer_id: worker_peer_id,
            node_name: "test-worker".to_string(),
            loaded_models: vec!["different-model-hash".to_string()], // Different model
            available_capacity: 1.0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 100,
        }];
        
        let request = create_test_request(); // Requests "test-model-hash"
        let result = router.select_worker(&request, &workers);
        
        assert!(matches!(result, Err(DistributedError::NoWorkersAvailable(_))));
    }
}
