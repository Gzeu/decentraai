//! Worker management for distributed inference
//!
//! This module provides functionality for:
//! - Worker registration and discovery
//! - Real-time capacity reporting
//! - Worker heartbeat and stale detection
//! - Worker status tracking

use libp2p::PeerId;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use decentraai_p2p::P2PNode;
use decentraai_protocol::{WorkerAnnouncement, WorkerStatus};

use crate::config::InferenceConfig;

/// Manages worker discovery, registration, and status tracking
///
/// The WorkerManager maintains a registry of all known workers in the network,
/// including their capabilities, capacity, and last seen timestamp. It handles:
/// - Registering the local node as a worker
/// - Receiving and processing worker announcements from peers
/// - Broadcasting local worker status updates
/// - Detecting and removing stale workers
/// - Providing worker information to the scheduler
#[derive(Clone)]
pub struct WorkerManager {
    /// The peer ID of the local node
    local_peer_id: PeerId,

    /// Configuration for distributed inference
    config: InferenceConfig,

    /// Map of peer ID to worker announcement
    workers: Arc<Mutex<HashMap<PeerId, WorkerAnnouncement>>>,

    /// Map of peer ID to worker status (for scheduling)
    worker_status: Arc<Mutex<HashMap<PeerId, WorkerStatus>>>,

    /// Map of peer ID to last seen timestamp
    last_seen: Arc<Mutex<HashMap<PeerId, Instant>>>,

    /// Whether this node is registered as a worker
    is_worker: Arc<Mutex<bool>>,

    /// Local worker announcement (if registered as worker)
    local_announcement: Arc<Mutex<Option<WorkerAnnouncement>>>,

    /// Local worker status (if registered as worker)
    local_status: Arc<Mutex<Option<WorkerStatus>>>,
}

impl WorkerManager {
    /// Creates a new WorkerManager
    ///
    /// # Arguments
    ///
    /// * `local_peer_id` - The peer ID of the local node
    /// * `config` - Distributed inference configuration
    pub fn new(local_peer_id: PeerId, config: InferenceConfig) -> Self {
        Self {
            local_peer_id,
            config,
            workers: Arc::new(Mutex::new(HashMap::new())),
            worker_status: Arc::new(Mutex::new(HashMap::new())),
            last_seen: Arc::new(Mutex::new(HashMap::new())),
            is_worker: Arc::new(Mutex::new(false)),
            local_announcement: Arc::new(Mutex::new(None)),
            local_status: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the local peer ID
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// Returns the distributed inference configuration for this manager.
    pub fn config(&self) -> &InferenceConfig {
        &self.config
    }

    /// Returns whether this node is registered as a worker
    pub async fn is_registered(&self) -> bool {
        *self.is_worker.lock().await
    }

    /// Returns whether this node is registered as a worker (synchronous)
    pub fn is_registered_sync(&self) -> bool {
        *self.is_worker.blocking_lock()
    }

    /// Returns the number of known workers
    pub async fn worker_count(&self) -> usize {
        self.workers.lock().await.len()
    }

    /// Returns the number of known workers (synchronous)
    pub fn worker_count_sync(&self) -> usize {
        self.workers.blocking_lock().len()
    }

    /// Registers this node as a worker with the given capabilities
    ///
    /// # Arguments
    ///
    /// * `node_name` - Human-readable name for this worker
    /// * `loaded_models` - List of model hashes this worker can serve
    /// * `initial_capacity` - Initial available capacity (0.0 - 1.0)
    pub fn register_as_worker(
        &self,
        node_name: String,
        loaded_models: Vec<String>,
        initial_capacity: f32,
    ) -> anyhow::Result<()> {
        let announcement = WorkerAnnouncement {
            peer_id: self.local_peer_id,
            node_name: node_name.clone(),
            loaded_models: loaded_models.clone(),
            available_capacity: initial_capacity,
            queue_depth: 0,
            tokens_per_second: 50,   // Default TPS
            current_latency_ms: 100, // Default latency
        };

        let status = WorkerStatus {
            peer_id: self.local_peer_id,
            loaded_models: loaded_models.clone(),
            queue_depth: 0,
            available_capacity: initial_capacity,
            current_latency_ms: 100,
            tokens_per_second: 50,
        };

        // Use block_in_place to avoid blocking the async runtime when invoked
        // from async contexts.
        tokio::task::block_in_place(|| {
            *self.is_worker.blocking_lock() = true;
            *self.local_announcement.blocking_lock() = Some(announcement);
            *self.local_status.blocking_lock() = Some(status);
        });

        // Register with ourselves
        self.add_worker(
            self.local_peer_id,
            loaded_models.clone(),
            initial_capacity,
            0,
            50,
            100,
        );

        info!(
            peer_id = %self.local_peer_id,
            node_name = %node_name,
            models = ?loaded_models,
            capacity = initial_capacity,
            "registered as worker"
        );

        Ok(())
    }

    /// Unregisters this node as a worker
    pub fn unregister_as_worker(&self) -> anyhow::Result<()> {
        tokio::task::block_in_place(|| {
            *self.is_worker.blocking_lock() = false;
            *self.local_announcement.blocking_lock() = None;
            *self.local_status.blocking_lock() = None;
        });

        // Remove ourselves from the worker list
        self.remove_worker(self.local_peer_id);

        info!(peer_id = %self.local_peer_id, "unregistered as worker");

        Ok(())
    }

    /// Starts the worker discovery process.
    ///
    /// Spawns a background task that, while this node is registered as a
    /// worker, broadcasts the local `WorkerAnnouncement` to every connected
    /// peer every `announcement_interval_ms` (immediately first). This is
    /// what makes workers visible to coordinators: without it the scheduler
    /// always reports "No workers available".
    pub async fn start_discovery(
        self: &Arc<Self>,
        p2p_node: &P2PNode,
        announcement_interval_ms: u64,
    ) -> anyhow::Result<()> {
        info!(peer_id = %self.local_peer_id, "worker discovery started");
        if announcement_interval_ms == 0 {
            return Ok(());
        }
        let interval = Duration::from_millis(announcement_interval_ms);
        let p2p_node = p2p_node.clone();
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                if me.is_registered().await {
                    if let Err(e) = me.broadcast_status(&p2p_node).await {
                        debug!(error = %e, "failed to broadcast worker status");
                    }
                }
                tokio::time::sleep(interval).await;
            }
        });
        Ok(())
    }

    /// Processes a received worker announcement
    ///
    /// This is called when a WorkerAnnouncement message is received from a peer.
    /// It updates the worker registry and marks the worker as recently seen.
    ///
    /// # Arguments
    ///
    /// * `announcement` - The received worker announcement
    pub fn process_announcement(&self, announcement: WorkerAnnouncement) -> anyhow::Result<()> {
        let peer_id = announcement.peer_id;

        // Skip our own announcements
        if peer_id == self.local_peer_id {
            return Ok(());
        }

        // Update the worker registry
        self.add_worker(
            peer_id,
            announcement.loaded_models.clone(),
            announcement.available_capacity,
            announcement.queue_depth,
            announcement.tokens_per_second,
            announcement.current_latency_ms,
        );

        // Update last seen timestamp
        self.update_last_seen(peer_id);

        debug!(
            peer_id = %peer_id,
            node_name = %announcement.node_name,
            models = ?announcement.loaded_models,
            capacity = announcement.available_capacity,
            "received worker announcement"
        );

        Ok(())
    }

    /// Adds a worker to the registry
    fn add_worker(
        &self,
        peer_id: PeerId,
        loaded_models: Vec<String>,
        available_capacity: f32,
        queue_depth: u32,
        tokens_per_second: u32,
        current_latency_ms: u32,
    ) {
        tokio::task::block_in_place(|| {
            let mut workers = self.workers.blocking_lock();
            let mut worker_status = self.worker_status.blocking_lock();

            let announcement = WorkerAnnouncement {
                peer_id,
                node_name: format!("peer-{}", peer_id),
                loaded_models: loaded_models.clone(),
                available_capacity,
                queue_depth,
                tokens_per_second,
                current_latency_ms,
            };

            workers.insert(peer_id, announcement);

            let status = WorkerStatus {
                peer_id,
                loaded_models,
                queue_depth,
                available_capacity,
                current_latency_ms,
                tokens_per_second,
            };

            worker_status.insert(peer_id, status);
        });
    }

    /// Removes a worker from the registry
    fn remove_worker(&self, peer_id: PeerId) {
        tokio::task::block_in_place(|| {
            let mut workers = self.workers.blocking_lock();
            let mut worker_status = self.worker_status.blocking_lock();
            let mut last_seen = self.last_seen.blocking_lock();

            workers.remove(&peer_id);
            worker_status.remove(&peer_id);
            last_seen.remove(&peer_id);
        });
    }

    /// Updates the last seen timestamp for a worker
    fn update_last_seen(&self, peer_id: PeerId) {
        tokio::task::block_in_place(|| {
            let mut last_seen = self.last_seen.blocking_lock();
            last_seen.insert(peer_id, Instant::now());
        });
    }

    /// Updates the capacity of the local worker
    ///
    /// # Arguments
    ///
    /// * `available_capacity` - New available capacity (0.0 - 1.0)
    /// * `queue_depth` - Current queue depth
    /// * `tokens_per_second` - Current throughput
    /// * `current_latency_ms` - Current latency estimate
    pub fn update_local_capacity(
        &self,
        available_capacity: f32,
        queue_depth: u32,
        tokens_per_second: u32,
        current_latency_ms: u32,
    ) -> anyhow::Result<()> {
        // All blocking_lock calls must live inside block_in_place so this is
        // safe from both sync callers and the async streaming task.
        let loaded_models = tokio::task::block_in_place(|| {
            if !*self.is_worker.blocking_lock() {
                return Err(anyhow::anyhow!("Node is not registered as a worker"));
            }

            let mut local_status = self.local_status.blocking_lock();
            let mut local_announcement = self.local_announcement.blocking_lock();

            if let Some(status) = local_status.as_mut() {
                status.available_capacity = available_capacity;
                status.queue_depth = queue_depth;
                status.tokens_per_second = tokens_per_second;
                status.current_latency_ms = current_latency_ms;
            }

            if let Some(announcement) = local_announcement.as_mut() {
                announcement.available_capacity = available_capacity;
                announcement.queue_depth = queue_depth;
                announcement.tokens_per_second = tokens_per_second;
                announcement.current_latency_ms = current_latency_ms;
                Ok(announcement.loaded_models.clone())
            } else {
                Ok(Vec::new())
            }
        })?;

        if !loaded_models.is_empty() {
            self.add_worker(
                self.local_peer_id,
                loaded_models,
                available_capacity,
                queue_depth,
                tokens_per_second,
                current_latency_ms,
            );
        }

        debug!(
            peer_id = %self.local_peer_id,
            capacity = available_capacity,
            queue_depth,
            tps = tokens_per_second,
            latency = current_latency_ms,
            "updated local worker capacity"
        );

        Ok(())
    }

    /// Broadcasts the current worker status to all connected peers.
    ///
    /// Uses async locks so it is safe to call from inside the Tokio runtime
    /// (e.g. the discovery task); `blocking_lock` would panic on a worker
    /// thread.
    pub async fn broadcast_status(&self, p2p_node: &P2PNode) -> anyhow::Result<()> {
        if !self.is_registered().await {
            return Err(anyhow::anyhow!("Node is not registered as a worker"));
        }

        let local_announcement = self.local_announcement.lock().await;

        if let Some(announcement) = local_announcement.as_ref() {
            let payload = Self::serialize_announcement(announcement)?;
            p2p_node.announce(payload);
            debug!(peer_id = %announcement.peer_id, "broadcasted worker status");
        }

        Ok(())
    }

    /// Serializes a worker announcement to bytes
    fn serialize_announcement(announcement: &WorkerAnnouncement) -> anyhow::Result<Vec<u8>> {
        use decentraai_protocol::serialize_message;

        let message = WorkerAnnouncement {
            peer_id: announcement.peer_id,
            node_name: announcement.node_name.clone(),
            loaded_models: announcement.loaded_models.clone(),
            available_capacity: announcement.available_capacity,
            queue_depth: announcement.queue_depth,
            tokens_per_second: announcement.tokens_per_second,
            current_latency_ms: announcement.current_latency_ms,
        };

        serialize_message(&message)
            .map_err(|e| anyhow::anyhow!("Failed to serialize announcement: {}", e))
    }

    /// Deserializes a worker announcement from bytes
    pub fn deserialize_announcement(bytes: &[u8]) -> anyhow::Result<WorkerAnnouncement> {
        use decentraai_protocol::{CURRENT_PROTOCOL_VERSION, deserialize_message};

        let announcement: WorkerAnnouncement = deserialize_message(
            bytes,
            decentraai_p2p::DEFAULT_MAX_MESSAGE_BYTES,
        )?;

        // Verify protocol version
        if CURRENT_PROTOCOL_VERSION != 1 {
            // In the future, we might need to handle version compatibility
            warn!(
                current_version = CURRENT_PROTOCOL_VERSION,
                "protocol version check not implemented for worker announcements"
            );
        }

        Ok(announcement)
    }

    /// Gets a list of all known workers
    pub async fn get_workers(&self) -> Vec<WorkerAnnouncement> {
        self.workers.lock().await.values().cloned().collect()
    }

    /// Gets a list of all known workers (synchronous)
    pub fn get_workers_sync(&self) -> Vec<WorkerAnnouncement> {
        self.workers.blocking_lock().values().cloned().collect()
    }

    /// Gets the worker status for all known workers
    pub async fn get_all_worker_status(&self) -> Vec<WorkerStatus> {
        self.worker_status.lock().await.values().cloned().collect()
    }

    /// Gets the worker status for all known workers (synchronous)
    pub fn get_all_worker_status_sync(&self) -> Vec<WorkerStatus> {
        self.worker_status
            .blocking_lock()
            .values()
            .cloned()
            .collect()
    }

    /// Gets a specific worker by peer ID
    pub async fn get_worker(&self, peer_id: &PeerId) -> Option<WorkerAnnouncement> {
        self.workers.lock().await.get(peer_id).cloned()
    }

    /// Gets a specific worker by peer ID (synchronous)
    pub fn get_worker_sync(&self, peer_id: &PeerId) -> Option<WorkerAnnouncement> {
        self.workers.blocking_lock().get(peer_id).cloned()
    }

    /// Gets the status of a specific worker by peer ID
    pub async fn get_worker_status_by_id(&self, peer_id: &PeerId) -> Option<WorkerStatus> {
        self.worker_status.lock().await.get(peer_id).cloned()
    }

    /// Gets the status of a specific worker by peer ID (synchronous)
    pub fn get_worker_status_by_id_sync(&self, peer_id: &PeerId) -> Option<WorkerStatus> {
        self.worker_status.blocking_lock().get(peer_id).cloned()
    }

    /// Checks if a worker is known and trusted
    pub async fn is_worker_trusted(&self, peer_id: &PeerId) -> bool {
        self.worker_status.lock().await.contains_key(peer_id)
    }

    /// Checks if a worker is known and trusted (synchronous)
    pub fn is_worker_trusted_sync(&self, peer_id: &PeerId) -> bool {
        self.worker_status.blocking_lock().contains_key(peer_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::PeerId;

    fn create_test_peer_id() -> PeerId {
        use libp2p::identity::Keypair;
        let keypair = Keypair::generate_ed25519();
        PeerId::from(keypair.public())
    }

    #[test]
    fn test_worker_manager_creation() {
        let peer_id = create_test_peer_id();
        let config = InferenceConfig::default();
        let manager = WorkerManager::new(peer_id, config);

        assert_eq!(manager.local_peer_id(), peer_id);
        assert!(!manager.is_registered_sync());
        assert_eq!(manager.worker_count_sync(), 0);
    }

    #[test]
    fn test_register_as_worker() {
        let peer_id = create_test_peer_id();
        let config = InferenceConfig::default();
        let manager = WorkerManager::new(peer_id, config);

        manager
            .register_as_worker(
                "test-worker".to_string(),
                vec!["model1".to_string(), "model2".to_string()],
                0.8,
            )
            .unwrap();

        assert!(manager.is_registered_sync());
        assert_eq!(manager.worker_count_sync(), 1); // Self-registered
    }

    #[test]
    fn test_process_announcement() {
        let peer_id = create_test_peer_id();
        let config = InferenceConfig::default();
        let manager = WorkerManager::new(peer_id, config);

        // Create a test announcement from a different peer
        let other_peer_id = create_test_peer_id();
        let announcement = WorkerAnnouncement {
            peer_id: other_peer_id,
            node_name: "other-worker".to_string(),
            loaded_models: vec!["model3".to_string()],
            available_capacity: 0.6,
            queue_depth: 1,
            tokens_per_second: 40,
            current_latency_ms: 150,
        };

        manager.process_announcement(announcement).unwrap();

        assert_eq!(manager.worker_count_sync(), 1);
        assert!(manager.is_worker_trusted_sync(&other_peer_id));
    }

    #[test]
    fn test_serialize_deserialize_announcement() {
        let peer_id = create_test_peer_id();
        let announcement = WorkerAnnouncement {
            peer_id,
            node_name: "test-worker".to_string(),
            loaded_models: vec!["model1".to_string()],
            available_capacity: 0.5,
            queue_depth: 2,
            tokens_per_second: 30,
            current_latency_ms: 200,
        };

        let serialized = WorkerManager::serialize_announcement(&announcement).unwrap();
        let deserialized = WorkerManager::deserialize_announcement(&serialized).unwrap();

        assert_eq!(deserialized.peer_id, announcement.peer_id);
        assert_eq!(deserialized.node_name, announcement.node_name);
        assert_eq!(deserialized.loaded_models, announcement.loaded_models);
        assert!(
            (deserialized.available_capacity - announcement.available_capacity).abs()
                < f32::EPSILON
        );
        assert_eq!(deserialized.queue_depth, announcement.queue_depth);
        assert_eq!(
            deserialized.tokens_per_second,
            announcement.tokens_per_second
        );
        assert_eq!(
            deserialized.current_latency_ms,
            announcement.current_latency_ms
        );
    }

    #[test]
    fn test_update_local_capacity() {
        let peer_id = create_test_peer_id();
        let config = InferenceConfig::default();
        let manager = WorkerManager::new(peer_id, config);

        manager
            .register_as_worker("test-worker".to_string(), vec!["model1".to_string()], 1.0)
            .unwrap();

        manager.update_local_capacity(0.5, 3, 100, 50).unwrap();

        let status = manager.get_worker_status_by_id_sync(&peer_id).unwrap();
        assert!((status.available_capacity - 0.5).abs() < f32::EPSILON);
        assert_eq!(status.queue_depth, 3);
        assert_eq!(status.tokens_per_second, 100);
        assert_eq!(status.current_latency_ms, 50);
    }
}
