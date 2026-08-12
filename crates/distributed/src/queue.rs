//! Request queue management for distributed inference
//!
//! This module provides functionality for:
//! - Tracking pending requests per worker in FIFO order
//! - Managing request timeouts and cancellation
//! - Tracking queue depth per worker for capacity management

use libp2p::PeerId;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::warn;
use uuid::Uuid;

use decentraai_protocol::{InferRequest, InferResponse};

/// Represents a queued inference request
#[derive(Debug, Clone)]
pub struct QueuedRequest {
    /// The original inference request
    pub request: InferRequest,
    /// When this request was queued
    pub queued_at: Instant,
    /// Request ID (same as infer_request.request_id)
    pub request_id: Uuid,
    /// Target worker peer ID
    pub worker_peer_id: PeerId,
    /// Priority (higher = more urgent)
    pub priority: u8,
    /// Optional timeout for this request
    pub timeout: Option<Duration>,
}

impl QueuedRequest {
    pub fn new(request: InferRequest, worker_peer_id: PeerId) -> Self {
        let request_id = request.request_id;
        let priority = request.priority;
        let timeout_ms = request.timeout_ms;
        Self {
            request,
            queued_at: Instant::now(),
            request_id,
            worker_peer_id,
            priority,
            timeout: Some(Duration::from_millis(timeout_ms as u64)),
        }
    }

    /// Checks if this request has timed out
    pub fn is_timed_out(&self) -> bool {
        match self.timeout {
            Some(timeout) => self.queued_at.elapsed() >= timeout,
            None => false,
        }
    }

    /// Time spent waiting in queue
    pub fn wait_time(&self) -> Duration {
        self.queued_at.elapsed()
    }
}

/// Request queue for a single worker
#[derive(Debug, Clone)]
pub struct WorkerRequestQueue {
    /// The worker's peer ID
    peer_id: PeerId,
    /// Queued requests in FIFO order (but with priority support)
    requests: VecDeque<QueuedRequest>,
    /// Maximum queue depth for this worker
    max_depth: usize,
    /// Whether the worker is currently processing a request
    is_processing: bool,
}

impl WorkerRequestQueue {
    pub fn new(peer_id: PeerId, max_depth: usize) -> Self {
        Self {
            peer_id,
            requests: VecDeque::new(),
            max_depth,
            is_processing: false,
        }
    }

    /// Adds a request to the queue
    ///
    /// Returns true if the request was added, false if the queue is full
    pub fn enqueue(&mut self, request: QueuedRequest) -> bool {
        if self.requests.len() >= self.max_depth {
            warn!(
                peer_id = %self.peer_id,
                current_depth = self.requests.len(),
                max_depth = self.max_depth,
                "worker queue is full, rejecting request"
            );
            return false;
        }

        // Add to the queue
        self.requests.push_back(request);
        true
    }

    /// Removes and returns the next request to process
    ///
    /// This uses a simple FIFO approach. For priority support, we'd scan for
    /// the highest priority request.
    pub fn dequeue(&mut self) -> Option<QueuedRequest> {
        self.requests.pop_front()
    }

    /// Peeks at the next request without removing it
    pub fn peek(&self) -> Option<&QueuedRequest> {
        self.requests.front()
    }

    /// Returns the number of requests in the queue
    pub fn len(&self) -> usize {
        self.requests.len()
    }

    /// Returns true if the queue is empty
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Returns the current queue depth
    pub fn depth(&self) -> usize {
        self.requests.len()
    }

    /// Removes a specific request from the queue
    pub fn remove(&mut self, request_id: Uuid) -> Option<QueuedRequest> {
        let mut found_index = None;
        for (index, req) in self.requests.iter().enumerate() {
            if req.request_id == request_id {
                found_index = Some(index);
                break;
            }
        }

        found_index.map(|idx| self.requests.remove(idx).unwrap())
    }

    /// Removes all timed out requests
    pub fn remove_timed_out(&mut self) -> Vec<QueuedRequest> {
        let mut timed_out = Vec::new();

        // Filter out timed out requests
        self.requests.retain(|req| {
            if req.is_timed_out() {
                timed_out.push(req.clone());
                false
            } else {
                true
            }
        });

        timed_out
    }

    /// Sets whether the worker is currently processing a request
    pub fn set_processing(&mut self, is_processing: bool) {
        self.is_processing = is_processing;
    }

    /// Returns whether the worker is currently processing a request
    pub fn is_processing(&self) -> bool {
        self.is_processing
    }

    /// Returns the peer ID of the worker this queue is for
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }
}

/// Manages all request queues for distributed workers
/// Uses Arc<Mutex<HashMap>> for shared state across all accesses
#[derive(Debug)]
pub struct RequestQueueManager {
    /// Map of worker peer ID to their queue - shared mutable state
    queues: Arc<Mutex<HashMap<PeerId, Arc<Mutex<WorkerRequestQueue>>>>>,
    /// Maximum queue depth per worker (default)
    default_max_depth: usize,
    /// Global timeout for requests
    default_timeout: Duration,
}

impl RequestQueueManager {
    pub fn new(default_max_depth: usize, default_timeout: Duration) -> Self {
        Self {
            queues: Arc::new(Mutex::new(HashMap::new())),
            default_max_depth,
            default_timeout,
        }
    }

    /// Creates a new RequestQueueManager with default settings
    pub fn with_defaults() -> Self {
        Self::new(100, Duration::from_secs(60))
    }

    /// Gets or creates a queue for a worker
    /// Returns a shared reference to the queue (same Arc for all callers)
    pub async fn get_or_create_queue(&self, peer_id: PeerId) -> Arc<Mutex<WorkerRequestQueue>> {
        let mut queues = self.queues.lock().await;

        // Check if queue already exists
        if let Some(queue_arc) = queues.get(&peer_id) {
            return queue_arc.clone();
        }

        // Create new queue with shared state
        let queue = Arc::new(Mutex::new(WorkerRequestQueue::new(
            peer_id,
            self.default_max_depth,
        )));
        queues.insert(peer_id, queue.clone());
        queue
    }

    /// Gets a queue for a worker if it exists
    pub async fn get_queue(&self, peer_id: &PeerId) -> Option<Arc<Mutex<WorkerRequestQueue>>> {
        let queues = self.queues.lock().await;
        queues.get(peer_id).cloned()
    }

    /// Queues a request for a specific worker
    ///
    /// Returns true if the request was queued, false if the queue is full
    pub async fn queue_request(&self, request: InferRequest, worker_peer_id: PeerId) -> bool {
        let queue = self.get_or_create_queue(worker_peer_id).await;
        let queued_request = QueuedRequest::new(request, worker_peer_id);

        let mut queue_lock = queue.lock().await;
        queue_lock.enqueue(queued_request)
    }

    /// Dequeues the next request for a worker
    pub async fn dequeue_request(&self, worker_peer_id: &PeerId) -> Option<QueuedRequest> {
        let queue = self.get_queue(worker_peer_id).await?;
        let mut queue_lock = queue.lock().await;
        queue_lock.dequeue()
    }

    /// Gets the current queue depth for a worker
    pub async fn queue_depth(&self, worker_peer_id: &PeerId) -> usize {
        match self.get_queue(worker_peer_id).await {
            Some(queue) => {
                let queue_lock = queue.lock().await;
                queue_lock.depth()
            }
            None => 0,
        }
    }

    /// Gets the total number of queued requests across all workers
    pub async fn total_queued(&self) -> usize {
        let queues = self.queues.lock().await;
        queues
            .values()
            .map(|q| {
                let queue_lock = q.blocking_lock();
                queue_lock.len()
            })
            .sum()
    }

    /// Removes a request from any queue by its ID
    pub async fn cancel_request(&self, request_id: Uuid) -> bool {
        let queues = self.queues.lock().await;
        let mut cancelled = false;

        for (_, queue_arc) in queues.iter() {
            let mut queue_lock = queue_arc.lock().await;
            if queue_lock.remove(request_id).is_some() {
                cancelled = true;
                break;
            }
        }

        cancelled
    }

    /// Removes all timed out requests from all queues
    pub async fn cleanup_timed_out(&self) -> Vec<QueuedRequest> {
        let queues = self.queues.lock().await;
        let mut timed_out = Vec::new();

        for (_, queue_arc) in queues.iter() {
            let mut queue_lock = queue_arc.lock().await;
            timed_out.extend(queue_lock.remove_timed_out());
        }

        timed_out
    }

    /// Gets all worker peer IDs that have queues
    pub async fn workers_with_queues(&self) -> Vec<PeerId> {
        let queues = self.queues.lock().await;
        queues.keys().cloned().collect()
    }

    /// Checks if a worker has requests in its queue
    pub async fn has_pending(&self, worker_peer_id: &PeerId) -> bool {
        let queue = self.get_queue(worker_peer_id).await;
        match queue {
            Some(q) => {
                let queue_lock = q.lock().await;
                !queue_lock.is_empty()
            }
            None => false,
        }
    }
}

/// Result of processing a queued request
#[derive(Debug)]
pub enum QueueProcessResult {
    /// Request was successfully processed and completed
    Completed { response: InferResponse },
    /// Request was processed but failed
    Failed { error: String, request_id: Uuid },
    /// Request timed out
    TimedOut { request_id: Uuid },
    /// No requests available to process
    NoRequests,
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
    fn test_worker_queue_enqueue_dequeue() {
        let peer_id = create_test_peer_id();
        let mut queue = WorkerRequestQueue::new(peer_id, 10);

        let request = create_test_request();
        let queued = QueuedRequest::new(request, peer_id);

        assert!(queue.enqueue(queued.clone()));
        assert_eq!(queue.len(), 1);

        let dequeued = queue.dequeue();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().request_id, queued.request_id);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_worker_queue_full() {
        let peer_id = create_test_peer_id();
        let mut queue = WorkerRequestQueue::new(peer_id, 2);

        let request = create_test_request();

        // Fill the queue
        assert!(queue.enqueue(QueuedRequest::new(request.clone(), peer_id)));
        assert!(queue.enqueue(QueuedRequest::new(request.clone(), peer_id)));

        // Third request should fail
        assert!(!queue.enqueue(QueuedRequest::new(request, peer_id)));
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn test_queued_request_timeout() {
        let request = InferRequest {
            request_id: Uuid::new_v4(),
            model_hash: "test".to_string(),
            prompt: "test".to_string(),
            max_tokens: 100,
            temperature: 0.7,
            top_p: 0.9,
            timeout_ms: 100, // 100ms timeout
            stream: false,
            priority: 128,
        };

        let peer_id = create_test_peer_id();
        let queued = QueuedRequest::new(request, peer_id);

        // Should not be timed out immediately
        assert!(!queued.is_timed_out());

        // Wait for timeout
        std::thread::sleep(Duration::from_millis(150));

        // Should be timed out now
        assert!(queued.is_timed_out());
    }
}
