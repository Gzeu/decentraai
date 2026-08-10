//! Worker scheduler with task placement and load balancing

use std::collections::HashMap;
use libp2p::PeerId;
use tracing::{info, warn, error};

use crate::{WorkerAnnouncement, WorkerStatus as WorkerState};
use protocol::{InferRequest, InferMessage, WorkerStatus, TaskPlacement};

/// Scheduler configuration
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub max_queue_depth: u32,
    pub min_available_capacity: f32,
    pub enable_load_balancing: bool,
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
#[derive(Debug, Clone)]
pub struct QueuedRequest {
    pub request: InferRequest,
    pub queued_at: u64,
    pub retries: u32,
}

/// Worker scheduler with task placement
pub struct WorkerScheduler {
    config: SchedulerConfig,
    workers: HashMap<PeerId, WorkerAnnouncement>,
    worker_status: HashMap<PeerId, WorkerStatus>,
    queues: HashMap<PeerId, Vec<QueuedRequest>>,
}

impl WorkerScheduler {
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config,
            workers: HashMap::new(),
            worker_status: HashMap::new(),
            queues: HashMap::new(),
        }
    }

    /// Register or update worker
    pub fn register_worker(&mut self, announcement: WorkerAnnouncement) {
        let peer_id = announcement.peer_id.clone();
        
        // Initialize status if new worker
        if !self.worker_status.contains_key(&peer_id) {
            self.worker_status.insert(
                peer_id.clone(),
                WorkerStatus {
                    peer_id,
                    loaded_models: announcement.loaded_models.clone(),
                    queue_depth: 0,
                    available_capacity: 1.0,
                    current_latency_ms: 100,
                    tokens_per_second: 50,
                },
            );
            self.queues.insert(peer_id.clone(), Vec::new());
        } else {
            // Update loaded models
            if let Some(status) = self.worker_status.get_mut(&peer_id) {
                status.loaded_models = announcement.loaded_models.clone();
            }
        }

        self.workers.insert(peer_id, announcement);
    }

    /// Remove worker
    pub fn remove_worker(&mut self, peer_id: &PeerId) {
        self.workers.remove(peer_id);
        self.worker_status.remove(peer_id);
        self.queues.remove(peer_id);
    }

    /// Select best worker for request using scoring algorithm
    pub fn select_worker(&self, request: &InferRequest) -> Option<TaskPlacement> {
        let candidates: Vec<_> = self.worker_status
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
                score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
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
    fn score_worker(&self, worker: &WorkerStatus, request: &InferRequest) -> f32 {
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
    fn estimate_wait_ms(&self, peer_id: &PeerId) -> u32 {
        let queue = self.queues.get(peer_id).unwrap();
        let queue_len = queue.len() as u32;
        
        // Assume 50 tokens/sec average, 100 tokens per request
        let avg_time_per_request_ms = 2000;
        queue_len * avg_time_per_request_ms
    }

    /// Estimate execution time for request
    fn estimate_execution_time(&self, request: &InferRequest, worker: &WorkerStatus) -> u32 {
        let tokens = request.max_tokens;
        let tps = worker.tokens_per_second.max(1);
        (tokens as f32 / tps as f32 * 1000.0) as u32
    }

    /// Calculate confidence score
    fn calculate_confidence(&self, worker: &WorkerStatus) -> f32 {
        // Higher capacity + lower queue = higher confidence
        worker.available_capacity * 0.5 + 
        (1.0 - (worker.queue_depth as f32 / self.config.max_queue_depth as f32)) * 0.5
    }

    /// Add request to worker queue
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
    pub fn record_completion(
        &mut self,
        worker_id: &PeerId,
        request_id: uuid::Uuid,
        success: bool,
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
    pub fn get_all_workers(&self) -> Vec<&WorkerAnnouncement> {
        self.workers.values().collect()
    }

    /// Get worker by peer ID
    pub fn get_worker(&self, peer_id: &PeerId) -> Option<&WorkerAnnouncement> {
        self.workers.get(peer_id)
    }

    /// Check if worker is trusted
    pub fn is_worker_trusted(&self, peer_id: &PeerId) -> bool {
        self.worker_status.contains_key(peer_id)
    }
}
