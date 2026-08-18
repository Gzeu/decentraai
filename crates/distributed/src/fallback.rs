//! Fallback mechanism for distributed inference
//!
//! This module provides automatic fallback to alternative workers when
//! the primary worker fails to process a request.

use libp2p::PeerId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::debug;
use uuid::Uuid;

/// Manages fallback requests when primary workers fail
///
/// The FallbackHandler keeps track of retry attempts and provides
/// fallback workers sorted by the same scoring algorithm used for
/// primary selection.
#[derive(Clone)]
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
