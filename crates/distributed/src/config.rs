//! Configuration for distributed inference

use std::time::Duration;

/// Configuration for distributed inference
///
/// Contains tunable parameters for worker discovery, request routing,
/// and fallback handling.
#[derive(Debug, Clone)]
pub struct InferenceConfig {
    /// Interval in milliseconds for broadcasting worker announcements
    /// Default: 10000 (10 seconds)
    pub announcement_interval_ms: u64,

    /// Interval in milliseconds for checking for stale workers
    /// Default: 5000 (5 seconds)
    pub discovery_interval_ms: u64,

    /// Timeout in milliseconds for removing stale workers
    /// Default: 30000 (30 seconds)
    pub stale_worker_timeout_ms: u64,

    /// Maximum number of retry attempts for a request
    /// Default: 3
    pub max_retries: u32,

    /// Backoff time in milliseconds between retries
    /// Default: 1000 (1 second)
    pub retry_backoff_ms: u64,

    /// Timeout in milliseconds for waiting for a response from a worker
    /// Default: 30000 (30 seconds)
    pub request_timeout_ms: u64,

    /// Maximum queue depth per worker before it's considered overloaded
    /// Default: 10
    pub max_queue_depth: u32,

    /// Minimum available capacity (0.0 to 1.0) for a worker to be eligible
    /// Default: 0.1 (10%)
    pub min_available_capacity: f32,

    /// Whether to enable load balancing across workers
    /// Default: true
    pub enable_load_balancing: bool,

    /// Whether to use reputation for worker selection
    /// Default: true
    pub use_reputation: bool,

    /// Base reputation reward for successful inference
    /// Default: 1.0
    pub base_reputation_reward: f32,

    /// Base reputation penalty for failed inference
    /// Default: 0.5
    pub base_reputation_penalty: f32,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            announcement_interval_ms: 10000,
            discovery_interval_ms: 5000,
            stale_worker_timeout_ms: 30000,
            max_retries: 3,
            retry_backoff_ms: 1000,
            request_timeout_ms: 30000,
            max_queue_depth: 10,
            min_available_capacity: 0.1,
            enable_load_balancing: true,
            use_reputation: true,
            base_reputation_reward: 1.0,
            base_reputation_penalty: 0.5,
        }
    }
}

impl InferenceConfig {
    /// Creates a new InferenceConfig with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the announcement interval
    pub fn with_announcement_interval(mut self, interval_ms: u64) -> Self {
        self.announcement_interval_ms = interval_ms;
        self
    }

    /// Sets the discovery interval
    pub fn with_discovery_interval(mut self, interval_ms: u64) -> Self {
        self.discovery_interval_ms = interval_ms;
        self
    }

    /// Sets the stale worker timeout
    pub fn with_stale_worker_timeout(mut self, timeout_ms: u64) -> Self {
        self.stale_worker_timeout_ms = timeout_ms;
        self
    }

    /// Sets the maximum number of retries
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Sets the retry backoff time
    pub fn with_retry_backoff(mut self, backoff_ms: u64) -> Self {
        self.retry_backoff_ms = backoff_ms;
        self
    }

    /// Sets the request timeout
    pub fn with_request_timeout(mut self, timeout_ms: u64) -> Self {
        self.request_timeout_ms = timeout_ms;
        self
    }

    /// Sets the maximum queue depth
    pub fn with_max_queue_depth(mut self, max_depth: u32) -> Self {
        self.max_queue_depth = max_depth;
        self
    }

    /// Sets the minimum available capacity
    pub fn with_min_available_capacity(mut self, min_capacity: f32) -> Self {
        self.min_available_capacity = min_capacity;
        self
    }

    /// Enables or disables load balancing
    pub fn with_load_balancing(mut self, enable: bool) -> Self {
        self.enable_load_balancing = enable;
        self
    }

    /// Enables or disables reputation-based worker selection
    pub fn with_reputation(mut self, enable: bool) -> Self {
        self.use_reputation = enable;
        self
    }

    /// Sets the base reputation reward
    pub fn with_base_reward(mut self, reward: f32) -> Self {
        self.base_reputation_reward = reward;
        self
    }

    /// Sets the base reputation penalty
    pub fn with_base_penalty(mut self, penalty: f32) -> Self {
        self.base_reputation_penalty = penalty;
        self
    }

    /// Returns the announcement interval as a Duration
    pub fn announcement_interval(&self) -> Duration {
        Duration::from_millis(self.announcement_interval_ms)
    }

    /// Returns the discovery interval as a Duration
    pub fn discovery_interval(&self) -> Duration {
        Duration::from_millis(self.discovery_interval_ms)
    }

    /// Returns the stale worker timeout as a Duration
    pub fn stale_worker_timeout(&self) -> Duration {
        Duration::from_millis(self.stale_worker_timeout_ms)
    }

    /// Returns the retry backoff as a Duration
    pub fn retry_backoff(&self) -> Duration {
        Duration::from_millis(self.retry_backoff_ms)
    }

    /// Returns the request timeout as a Duration
    pub fn request_timeout(&self) -> Duration {
        Duration::from_millis(self.request_timeout_ms)
    }
}

/// Configuration for a worker node
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Human-readable name for this worker
    pub node_name: String,

    /// List of model hashes this worker can serve
    pub loaded_models: Vec<String>,

    /// Initial available capacity (0.0 - 1.0)
    pub initial_capacity: f32,

    /// Initial queue depth
    pub initial_queue_depth: u32,

    /// Initial tokens per second throughput estimate
    pub initial_tokens_per_second: u32,

    /// Initial latency estimate in milliseconds
    pub initial_latency_ms: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            node_name: "default-worker".to_string(),
            loaded_models: Vec::new(),
            initial_capacity: 1.0,
            initial_queue_depth: 0,
            initial_tokens_per_second: 50,
            initial_latency_ms: 100,
        }
    }
}

impl WorkerConfig {
    /// Creates a new WorkerConfig with default values
    pub fn new(node_name: String) -> Self {
        Self {
            node_name,
            ..Self::default()
        }
    }

    /// Sets the loaded models
    pub fn with_models(mut self, models: Vec<String>) -> Self {
        self.loaded_models = models;
        self
    }

    /// Sets the initial capacity
    pub fn with_capacity(mut self, capacity: f32) -> Self {
        self.initial_capacity = capacity.clamp(0.0, 1.0);
        self
    }

    /// Sets the initial queue depth
    pub fn with_queue_depth(mut self, depth: u32) -> Self {
        self.initial_queue_depth = depth;
        self
    }

    /// Sets the initial tokens per second
    pub fn with_tokens_per_second(mut self, tps: u32) -> Self {
        self.initial_tokens_per_second = tps;
        self
    }

    /// Sets the initial latency
    pub fn with_latency(mut self, latency_ms: u32) -> Self {
        self.initial_latency_ms = latency_ms;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = InferenceConfig::default();

        assert_eq!(config.announcement_interval_ms, 10000);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.max_queue_depth, 10);
        assert!((config.min_available_capacity - 0.1).abs() < f32::EPSILON);
        assert!(config.enable_load_balancing);
    }

    #[test]
    fn test_config_builder() {
        let config = InferenceConfig::new()
            .with_announcement_interval(5000)
            .with_max_retries(5)
            .with_max_queue_depth(20)
            .with_min_available_capacity(0.2)
            .with_load_balancing(false);

        assert_eq!(config.announcement_interval_ms, 5000);
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.max_queue_depth, 20);
        assert!((config.min_available_capacity - 0.2).abs() < f32::EPSILON);
        assert!(!config.enable_load_balancing);
    }

    #[test]
    fn test_worker_config_default() {
        let config = WorkerConfig::default();

        assert_eq!(config.node_name, "default-worker");
        assert!(config.loaded_models.is_empty());
        assert!((config.initial_capacity - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_worker_config_builder() {
        let config = WorkerConfig::new("test-worker".to_string())
            .with_models(vec!["model1".to_string(), "model2".to_string()])
            .with_capacity(0.8)
            .with_queue_depth(2)
            .with_tokens_per_second(100)
            .with_latency(50);

        assert_eq!(config.node_name, "test-worker");
        assert_eq!(config.loaded_models.len(), 2);
        assert!((config.initial_capacity - 0.8).abs() < f32::EPSILON);
        assert_eq!(config.initial_queue_depth, 2);
        assert_eq!(config.initial_tokens_per_second, 100);
        assert_eq!(config.initial_latency_ms, 50);
    }
}
