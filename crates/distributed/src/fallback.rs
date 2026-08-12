//! Fallback mechanism for distributed inference
//!
//! This module provides automatic fallback to alternative workers when
//! the primary worker fails to process a request.

use libp2p::PeerId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use decentraai_discovery::scheduler::WorkerScheduler;
use decentraai_p2p::P2PNode;
use decentraai_protocol::{InferRequest, InferResponse, TaskPlacement, WorkerAnnouncement};

use crate::{DistributedError, RequestRouter};

/// Manages fallback requests when primary workers fail
///
/// The FallbackHandler keeps track of retry attempts and provides
/// fallback workers sorted by the same scoring algorithm used for
/// primary selection.
pub struct FallbackHandler {
    /// Maximum number of retry attempts
    max_retries: u32,

    /// Current retry count (per request)
    retries: Arc<Mutex<HashMap<Uuid, u32>>>,

    /// Excluded workers (per request) - workers that have already failed
    excluded_workers: Arc<Mutex<HashMap<Uuid, Vec<PeerId>>>>,
}

impl FallbackHandler {
    /// Creates a new FallbackHandler
    ///
    /// # Arguments
    ///
    /// * `max_retries` - Maximum number of retry attempts per request
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            retries: Arc::new(Mutex::new(HashMap::new())),
            excluded_workers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Returns the maximum number of retries
    pub fn max_retries(&self) -> u32 {
        self.max_retries
    }

    /// Resets the retry state for a request
    ///
    /// Call this when starting to process a new request.
    ///
    /// # Arguments
    ///
    /// * `request_id` - The request ID to reset
    pub async fn reset_request(&self, request_id: Uuid) {
        let mut retries = self.retries.lock().await;
        let mut excluded = self.excluded_workers.lock().await;

        retries.remove(&request_id);
        excluded.remove(&request_id);
    }

    /// Records a failed attempt for a request
    ///
    /// This increments the retry counter and adds the worker to the
    /// excluded list for this request.
    ///
    /// # Arguments
    ///
    /// * `request_id` - The request ID
    /// * `failed_worker` - The peer ID of the worker that failed
    ///
    /// # Returns
    ///
    /// True if more retries are allowed, false if max retries exceeded
    pub async fn record_failure(&self, request_id: Uuid, failed_worker: PeerId) -> bool {
        let mut retries = self.retries.lock().await;
        let mut excluded = self.excluded_workers.lock().await;

        let current_retries = retries.entry(request_id).or_insert(0);
        *current_retries += 1;

        // Add to excluded list
        excluded.entry(request_id).or_default().push(failed_worker);

        *current_retries < self.max_retries
    }

    /// Gets the number of retries for a request
    pub async fn get_retries(&self, request_id: Uuid) -> u32 {
        *self.retries.lock().await.get(&request_id).unwrap_or(&0)
    }

    /// Gets the list of excluded workers for a request
    pub async fn get_excluded_workers(&self, request_id: Uuid) -> Vec<PeerId> {
        self.excluded_workers
            .lock()
            .await
            .get(&request_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Gets fallback workers for a request
    ///
    /// Returns alternative workers that can handle the request, excluding
    /// workers that have already failed for this request.
    ///
    /// # Arguments
    ///
    /// * `request` - The inference request
    /// * `scheduler` - The worker scheduler
    /// * `excluded` - List of peer IDs to exclude
    ///
    /// # Returns
    ///
    /// A vector of fallback worker placements, sorted by score
    pub async fn get_fallback_workers(
        &self,
        _request: &InferRequest,
        _scheduler: &WorkerScheduler,
        _excluded: &[PeerId],
    ) -> Vec<TaskPlacement> {
        // Get all workers that can handle this request
        // This requires access to the scheduler's internal state
        // For now, we'll use a simplified approach

        // In a real implementation, we'd use the scheduler's get_fallback_workers method
        // which already implements this logic

        // For this implementation, we'll return an empty vector
        // The actual fallback logic is in the scheduler
        Vec::new()
    }

    /// Attempts to route a request with fallback
    ///
    /// This will:
    /// 1. Try to send the request to the primary worker
    /// 2. On failure, get fallback workers and retry
    /// 3. Continue until success or max retries exceeded
    ///
    /// # Arguments
    ///
    /// * `p2p_node` - The P2P node for network communication
    /// * `request` - The inference request to route
    /// * `router` - The request router for sending requests
    /// * `workers` - List of available workers
    /// * `primary_worker` - The initially selected worker
    ///
    /// # Returns
    ///
    /// The inference response from a worker, or an error if all attempts fail
    pub async fn route_with_fallback(
        &self,
        p2p_node: &P2PNode,
        request: InferRequest,
        router: &RequestRouter,
        workers: Vec<WorkerAnnouncement>,
        primary_worker: TaskPlacement,
    ) -> Result<InferResponse, DistributedError> {
        let request_id = request.request_id;

        // Reset retry state for this request
        self.reset_request(request_id).await;

        // Create a mutable request we can modify
        let current_request = request;

        // Try the primary worker first
        match router
            .send_request(p2p_node, current_request.clone(), primary_worker.clone())
            .await
        {
            Ok(response) => {
                info!(
                    request_id = %request_id,
                    worker_peer_id = %primary_worker.selected_worker,
                    "request succeeded on primary worker"
                );
                return Ok(response);
            }
            Err(e) => {
                warn!(
                    request_id = %request_id,
                    worker_peer_id = %primary_worker.selected_worker,
                    error = %e,
                    "primary worker failed, trying fallback"
                );

                // Record the failure
                let can_retry = self
                    .record_failure(request_id, primary_worker.selected_worker)
                    .await;
                if !can_retry {
                    return Err(DistributedError::AllWorkersFailed(request_id.to_string()));
                }
            }
        }

        // Get fallback workers
        // For now, we'll use the scheduler's get_fallback_workers method
        // This requires us to have access to the scheduler

        // Simplified: just try all other workers
        // In production, we'd use the scheduler's scoring algorithm

        let excluded = self.get_excluded_workers(request_id).await;

        for worker in &workers {
            if excluded.contains(&worker.peer_id) {
                continue;
            }

            // Create a placement for this fallback worker
            let placement = TaskPlacement {
                selected_worker: worker.peer_id,
                estimated_wait_ms: 0, // Will be calculated by scheduler
                estimated_time_ms: 0,
                confidence: 0.0,
            };

            debug!(
                request_id = %request_id,
                worker_peer_id = %worker.peer_id,
                "trying fallback worker"
            );

            match router
                .send_request(p2p_node, current_request.clone(), placement)
                .await
            {
                Ok(response) => {
                    info!(
                        request_id = %request_id,
                        worker_peer_id = %worker.peer_id,
                        "request succeeded on fallback worker"
                    );
                    return Ok(response);
                }
                Err(e) => {
                    warn!(
                        request_id = %request_id,
                        worker_peer_id = %worker.peer_id,
                        error = %e,
                        "fallback worker failed"
                    );

                    // Record the failure
                    let can_retry = self.record_failure(request_id, worker.peer_id).await;
                    if !can_retry {
                        break;
                    }
                }
            }
        }

        // All attempts failed
        error!(
            request_id = %request_id,
            "all workers failed"
        );
        Err(DistributedError::AllWorkersFailed(request_id.to_string()))
    }

    /// Calculates backoff duration for a retry attempt
    ///
    /// Uses exponential backoff: base * 2^attempt
    ///
    /// # Arguments
    ///
    /// * `attempt` - The current attempt number (0-indexed)
    /// * `base_ms` - The base backoff time in milliseconds
    ///
    /// # Returns
    ///
    /// The backoff duration
    pub fn calculate_backoff(attempt: u32, base_ms: u64) -> Duration {
        let multiplier = 1u64 << attempt; // 2^attempt
        Duration::from_millis(base_ms * multiplier)
    }

    /// Waits with backoff before the next retry
    ///
    /// # Arguments
    ///
    /// * `attempt` - The current attempt number (0-indexed)
    /// * `base_ms` - The base backoff time in milliseconds
    pub async fn wait_backoff(&self, attempt: u32, base_ms: u64) {
        let backoff = Self::calculate_backoff(attempt, base_ms);
        debug!(
            attempt,
            backoff_ms = backoff.as_millis(),
            "waiting before retry"
        );
        tokio::time::sleep(backoff).await;
    }
}

/// Extended request router with fallback support
///
/// Combines the basic RequestRouter with the FallbackHandler to provide
/// automatic retry with fallback workers.
pub struct RequestRouterWithFallback {
    router: RequestRouter,
    fallback_handler: FallbackHandler,
}

impl RequestRouterWithFallback {
    /// Creates a new RequestRouterWithFallback
    ///
    /// # Arguments
    ///
    /// * `local_peer_id` - The peer ID of the local node
    /// * `max_retries` - Maximum number of retry attempts
    pub fn new(local_peer_id: PeerId, max_retries: u32) -> Self {
        Self {
            router: RequestRouter::new(local_peer_id),
            fallback_handler: FallbackHandler::new(max_retries),
        }
    }

    /// Routes a request with automatic fallback on failure
    ///
    /// # Arguments
    ///
    /// * `p2p_node` - The P2P node for network communication
    /// * `request` - The inference request to route
    /// * `workers` - List of available workers
    ///
    /// # Returns
    ///
    /// The inference response from a worker, or an error if all attempts fail
    pub async fn route_with_fallback(
        &self,
        p2p_node: &P2PNode,
        request: InferRequest,
        workers: Vec<WorkerAnnouncement>,
    ) -> Result<InferResponse, DistributedError> {
        // Select the primary worker
        let placement = self.router.select_worker(&request, &workers)?;

        // Try with fallback
        self.fallback_handler
            .route_with_fallback(p2p_node, request, &self.router, workers, placement)
            .await
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

    #[test]
    fn test_fallback_handler_creation() {
        let handler = FallbackHandler::new(3);

        assert_eq!(handler.max_retries(), 3);
    }

    #[tokio::test]
    async fn test_reset_request() {
        let handler = FallbackHandler::new(3);
        let request_id = Uuid::new_v4();

        // Initially, no retries
        assert_eq!(handler.get_retries(request_id).await, 0);

        // Record a failure
        let peer_id = create_test_peer_id();
        handler.record_failure(request_id, peer_id).await;

        // Should have 1 retry
        assert_eq!(handler.get_retries(request_id).await, 1);

        // Reset
        handler.reset_request(request_id).await;

        // Should be back to 0
        assert_eq!(handler.get_retries(request_id).await, 0);
    }

    #[tokio::test]
    async fn test_record_failure_max_retries() {
        let handler = FallbackHandler::new(3);
        let request_id = Uuid::new_v4();
        let peer_id = create_test_peer_id();

        // With max_retries = 3, we allow 2 retries after the first attempt
        // Record first failure: retries = 1, can_retry = true (1 < 3)
        let can_retry = handler.record_failure(request_id, peer_id).await;
        assert!(can_retry);

        // Record second failure: retries = 2, can_retry = true (2 < 3)
        let can_retry = handler.record_failure(request_id, peer_id).await;
        assert!(can_retry);

        // Record third failure: retries = 3, can_retry = false (3 < 3 is false)
        let can_retry = handler.record_failure(request_id, peer_id).await;
        assert!(!can_retry);
    }

    #[tokio::test]
    async fn test_excluded_workers() {
        let handler = FallbackHandler::new(3);
        let request_id = Uuid::new_v4();
        let peer1 = create_test_peer_id();
        let peer2 = create_test_peer_id();

        // Record failures for two different workers
        handler.record_failure(request_id, peer1).await;
        handler.record_failure(request_id, peer2).await;

        // Both should be in the excluded list
        let excluded = handler.get_excluded_workers(request_id).await;
        assert!(excluded.contains(&peer1));
        assert!(excluded.contains(&peer2));
    }

    #[test]
    fn test_calculate_backoff() {
        // Base of 100ms
        assert_eq!(
            FallbackHandler::calculate_backoff(0, 100),
            Duration::from_millis(100)
        );
        assert_eq!(
            FallbackHandler::calculate_backoff(1, 100),
            Duration::from_millis(200)
        );
        assert_eq!(
            FallbackHandler::calculate_backoff(2, 100),
            Duration::from_millis(400)
        );
        assert_eq!(
            FallbackHandler::calculate_backoff(3, 100),
            Duration::from_millis(800)
        );
    }
}
