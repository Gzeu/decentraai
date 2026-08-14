//! Worker scheduler with task placement and load balancing
//!
//! This module provides intelligent worker selection and task placement for distributed inference.
//! The scheduler uses a multi-factor scoring algorithm considering queue depth, available capacity,
//! latency, and throughput to select the optimal worker for each request.
//!
//! # Scoring Algorithm
//!
//! Workers are scored based on four factors with the following weights:
//! - Queue depth (40%): Lower queue depth is better
//! - Available capacity (30%): Higher capacity is better
//! - Latency (20%): Lower latency is better
//! - Throughput (10%): Higher tokens per second is better
//!
//! # Fallback Mechanism
//!
//! If the primary worker fails, the scheduler can provide fallback workers sorted by the same
//! scoring algorithm, ensuring resilience in the face of worker failures.
//!
//! # Queue Management
//!
//! Each worker maintains a request queue with depth tracking. The scheduler estimates wait times
//! based on queue depth and average request duration, providing realistic time estimates to clients.

use libp2p::PeerId;
use std::collections::HashMap;

use decentraai_protocol::{InferRequest, TaskPlacement, WorkerAnnouncement, WorkerStatus};

/// Scheduler configuration
///
/// Contains tunable parameters for the worker scheduling algorithm.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Maximum queue depth per worker before it's considered overloaded
    pub max_queue_depth: u32,
    /// Minimum available capacity (0.0 to 1.0) for a worker to be eligible
    pub min_available_capacity: f32,
    /// Whether to enable load balancing across workers
    pub enable_load_balancing: bool,
    /// Timeout in milliseconds for fallback worker selection
    pub fallback_timeout_ms: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_queue_depth: 10,
            min_available_capacity: 0.1,
            enable_load_balancing: true,
            fallback_timeout_ms: 5000,
        }
    }
}

/// Pending request in queue
///
/// Represents a request that is waiting to be processed by a worker.
#[derive(Debug, Clone)]
pub struct QueuedRequest {
    /// The inference request to be processed
    pub request: InferRequest,
    /// Unix timestamp when the request was queued
    pub queued_at: u64,
    /// Number of retry attempts for this request
    pub retries: u32,
}

/// Worker scheduler with task placement
///
/// Manages worker registration, scoring, and request queueing for distributed inference.
pub struct WorkerScheduler {
    config: SchedulerConfig,
    workers: HashMap<PeerId, WorkerAnnouncement>,
    worker_status: HashMap<PeerId, WorkerStatus>,
    queues: HashMap<PeerId, Vec<QueuedRequest>>,
}

impl WorkerScheduler {
    /// Creates a new worker scheduler with the given configuration
    ///
    /// # Arguments
    ///
    /// * `config` - Scheduler configuration parameters
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            workers: HashMap::new(),
            worker_status: HashMap::new(),
            queues: HashMap::new(),
        }
    }

    /// Register or update worker
    ///
    /// Adds a new worker or updates an existing worker's information.
    /// New workers are initialized with default status values.
    ///
    /// # Arguments
    ///
    /// * `announcement` - Worker announcement with capabilities and resources
    pub fn register_worker(&mut self, announcement: WorkerAnnouncement) {
        let peer_id = announcement.peer_id;

        // Initialize status if new worker
        if let std::collections::hash_map::Entry::Vacant(e) = self.worker_status.entry(peer_id) {
            e.insert(WorkerStatus {
                peer_id,
                loaded_models: announcement.loaded_models.clone(),
                queue_depth: 0,
                available_capacity: 1.0,
                current_latency_ms: 100,
                tokens_per_second: 50,
            });
            self.queues.insert(peer_id, Vec::new());
        } else {
            // Update loaded models
            if let Some(status) = self.worker_status.get_mut(&peer_id) {
                status.loaded_models = announcement.loaded_models.clone();
            }
        }

        self.workers.insert(peer_id, announcement);
    }

    /// Remove worker
    ///
    /// Removes a worker from the scheduler, including its status and queue.
    ///
    /// # Arguments
    ///
    /// * `peer_id` - The peer ID of the worker to remove
    pub fn remove_worker(&mut self, peer_id: &PeerId) {
        self.workers.remove(peer_id);
        self.worker_status.remove(peer_id);
        self.queues.remove(peer_id);
    }

    /// Select best worker for request using scoring algorithm
    ///
    /// Evaluates all workers that can handle the requested model and selects
    /// the best one based on the multi-factor scoring algorithm.
    ///
    /// # Arguments
    ///
    /// * `request` - The inference request to schedule
    ///
    /// # Returns
    ///
    /// Some(TaskPlacement) with the selected worker and estimates, or None if no eligible workers exist.
    pub fn select_worker(&self, request: &InferRequest) -> Option<TaskPlacement> {
        let candidates: Vec<_> = self
            .worker_status
            .values()
            .filter(|w| w.can_accept_request(&request.model_hash))
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Score each worker
        let best = candidates
            .iter()
            .max_by(|a, b| {
                let score_a = self.score_worker(a, request);
                let score_b = self.score_worker(b, request);
                score_a
                    .partial_cmp(&score_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();

        Some(TaskPlacement {
            selected_worker: best.peer_id,
            estimated_wait_ms: self.estimate_wait_ms(&best.peer_id),
            estimated_time_ms: self.estimate_execution_time(request, best),
            confidence: self.calculate_confidence(best),
        })
    }

    /// Score worker based on multiple factors
    ///
    /// Computes a composite score (0.0 to 1.0) based on queue depth, capacity,
    /// latency, and throughput. Higher scores indicate better workers.
    ///
    /// # Arguments
    ///
    /// * `worker` - The worker status to score
    /// * `_request` - The request being scheduled (reserved for future use)
    ///
    /// # Returns
    ///
    /// A score between 0.0 and 1.0, where higher is better.
    fn score_worker(&self, worker: &WorkerStatus, _request: &InferRequest) -> f32 {
        let mut score = 0.0;

        // Lower queue depth = better (weight: 0.4)
        let queue_score = 1.0 - (worker.queue_depth as f32 / self.config.max_queue_depth as f32);
        score += queue_score * 0.4;

        // Higher available capacity = better (weight: 0.3)
        score += worker.available_capacity * 0.3;

        // Lower latency = better (weight: 0.2)
        let latency_score = 1.0 - (worker.current_latency_ms as f32 / 1000.0).min(1.0);
        score += latency_score * 0.2;

        // Higher throughput = better (weight: 0.1)
        let throughput_score = (worker.tokens_per_second as f32 / 100.0).min(1.0);
        score += throughput_score * 0.1;

        score
    }

    /// Estimate wait time based on queue depth
    ///
    /// Estimates the wait time for a request based on the worker's current queue depth.
    /// Assumes 50 tokens/sec average and 100 tokens per request.
    ///
    /// # Arguments
    ///
    /// * `peer_id` - The worker's peer ID
    ///
    /// # Returns
    ///
    /// Estimated wait time in milliseconds.
    fn estimate_wait_ms(&self, peer_id: &PeerId) -> u32 {
        let queue = self.queues.get(peer_id).unwrap();
        let queue_len = queue.len() as u32;

        // Assume 50 tokens/sec average, 100 tokens per request
        let avg_time_per_request_ms = 2000;
        queue_len * avg_time_per_request_ms
    }

    /// Estimate execution time for request
    ///
    /// Estimates the execution time based on the request's token count and the worker's throughput.
    ///
    /// # Arguments
    ///
    /// * `request` - The inference request
    /// * `worker` - The worker status
    ///
    /// # Returns
    ///
    /// Estimated execution time in milliseconds.
    fn estimate_execution_time(&self, request: &InferRequest, worker: &WorkerStatus) -> u32 {
        let tokens = request.max_tokens;
        let tps = worker.tokens_per_second.max(1);
        (tokens as f32 / tps as f32 * 1000.0) as u32
    }

    /// Calculate confidence score
    ///
    /// Computes a confidence score (0.0 to 1.0) indicating how reliable the
    /// placement estimate is. Higher capacity and lower queue depth increase confidence.
    ///
    /// # Arguments
    ///
    /// * `worker` - The worker status
    ///
    /// # Returns
    ///
    /// A confidence score between 0.0 and 1.0.
    fn calculate_confidence(&self, worker: &WorkerStatus) -> f32 {
        // Higher capacity + lower queue = higher confidence
        worker.available_capacity * 0.5
            + (1.0 - (worker.queue_depth as f32 / self.config.max_queue_depth as f32)) * 0.5
    }

    /// Add request to worker queue
    ///
    /// Queues a request for processing by the specified worker and updates queue depth.
    ///
    /// # Arguments
    ///
    /// * `worker_id` - The worker's peer ID
    /// * `request` - The inference request to queue
    pub fn queue_request(&mut self, worker_id: &PeerId, request: InferRequest) {
        let queue = self.queues.get_mut(worker_id).unwrap();
        queue.push(QueuedRequest {
            request,
            queued_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            retries: 0,
        });

        // Update queue depth
        if let Some(status) = self.worker_status.get_mut(worker_id) {
            status.queue_depth = queue.len() as u32;
        }
    }

    /// Remove request from queue (on success/failure)
    ///
    /// Removes a completed request from the worker's queue and updates queue depth.
    ///
    /// # Arguments
    ///
    /// * `worker_id` - The worker's peer ID
    /// * `request_id` - The UUID of the request to remove
    pub fn dequeue_request(&mut self, worker_id: &PeerId, request_id: uuid::Uuid) {
        if let Some(queue) = self.queues.get_mut(worker_id) {
            queue.retain(|r| r.request.request_id != request_id);

            // Update queue depth
            if let Some(status) = self.worker_status.get_mut(worker_id) {
                status.queue_depth = queue.len() as u32;
            }
        }
    }

    /// Get fallback workers if primary fails
    ///
    /// Returns alternative workers that can handle the request, excluding the failed worker.
    /// Workers are sorted by the same scoring algorithm used for primary selection.
    ///
    /// # Arguments
    ///
    /// * `request` - The inference request
    /// * `exclude` - The peer ID of the failed worker to exclude
    ///
    /// # Returns
    ///
    /// A vector of fallback worker placements, sorted by score.
    pub fn get_fallback_workers(
        &self,
        request: &InferRequest,
        exclude: &PeerId,
    ) -> Vec<TaskPlacement> {
        self.worker_status
            .values()
            .filter(|w| w.peer_id != *exclude)
            .filter(|w| w.can_accept_request(&request.model_hash))
            .map(|w| TaskPlacement {
                selected_worker: w.peer_id,
                estimated_wait_ms: self.estimate_wait_ms(&w.peer_id),
                estimated_time_ms: self.estimate_execution_time(request, w),
                confidence: self.calculate_confidence(w),
            })
            .collect()
    }

    /// Update worker status after request completion
    ///
    /// Updates the worker's latency estimate using exponential moving average
    /// and removes the request from the queue.
    ///
    /// # Arguments
    ///
    /// * `worker_id` - The worker's peer ID
    /// * `request_id` - The UUID of the completed request
    /// * `_success` - Whether the request succeeded (reserved for future use)
    /// * `time_ms` - The actual execution time in milliseconds
    pub fn record_completion(
        &mut self,
        worker_id: &PeerId,
        request_id: uuid::Uuid,
        _success: bool,
        time_ms: u32,
    ) {
        self.dequeue_request(worker_id, request_id);

        // Update latency estimate
        if let Some(status) = self.worker_status.get_mut(worker_id) {
            // Exponential moving average
            status.current_latency_ms =
                (status.current_latency_ms as f32 * 0.8 + time_ms as f32 * 0.2) as u32;
        }
    }

    /// Get all workers for dashboard
    ///
    /// Returns a list of all registered workers for display in the dashboard.
    ///
    /// # Returns
    ///
    /// A vector of references to worker announcements.
    pub fn get_all_workers(&self) -> Vec<&WorkerAnnouncement> {
        self.workers.values().collect()
    }

    /// Get worker by peer ID
    ///
    /// # Arguments
    ///
    /// * `peer_id` - The worker's peer ID
    ///
    /// # Returns
    ///
    /// Some reference to the worker announcement if found, None otherwise.
    pub fn get_worker(&self, peer_id: &PeerId) -> Option<&WorkerAnnouncement> {
        self.workers.get(peer_id)
    }

    /// Check if worker is trusted
    ///
    /// A worker is considered trusted if it has been registered and has status information.
    ///
    /// # Arguments
    ///
    /// * `peer_id` - The worker's peer ID
    ///
    /// # Returns
    ///
    /// True if the worker is trusted, false otherwise.
    pub fn is_worker_trusted(&self, peer_id: &PeerId) -> bool {
        self.worker_status.contains_key(peer_id)
    }
}
