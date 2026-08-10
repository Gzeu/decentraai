//! Discovery service orchestrating mDNS and P2P gossip

use std::time::Duration;
use tokio::time::interval;
use tokio::sync::mpsc;
use libp2p::PeerId;
use p2p::P2PService;
use identity::Identity;
use system_probe::SystemProbe;
use tracing::{info, warn, error};

use crate::{
    WorkerAnnouncement, WorkerResources,
    WorkerApproval, TrustRecord,
    MdnsDiscovery, PendingWorkerCache,
};

/// Discovery service configuration
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    pub broadcast_interval_secs: u64,
    pub ttl_minutes: u64,
    pub node_name: String,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            broadcast_interval_secs: 30,
            ttl_minutes: 60,
            node_name: "decentraai-worker".to_string(),
        }
    }
}

/// Main discovery service
pub struct DiscoveryService {
    config: DiscoveryConfig,
    identity: Identity,
    p2p: P2PService,
    mdns: MdnsDiscovery,
    cache: PendingWorkerCache,
    system_probe: SystemProbe,
    new_worker_tx: mpsc::Sender<WorkerAnnouncement>,
}

impl DiscoveryService {
    pub fn new(
        config: DiscoveryConfig,
        identity: Identity,
        p2p: P2PService,
    ) -> Self {
        let (new_worker_tx, _) = mpsc::channel(100);
        Self {
            config,
            identity: identity.clone(),
            p2p,
            mdns: MdnsDiscovery::new(),
            cache: PendingWorkerCache::new(config.ttl_minutes),
            system_probe: SystemProbe::new(),
            new_worker_tx,
        }
    }

    /// Start discovery service
    pub async fn start(&mut self) -> anyhow::Result<()> {
        info!("Starting discovery service");

        // Start mDNS listener
        let mut mdns_stream = self.mdns.start().await?;

        // Start broadcast loop
        let mut broadcast_interval = interval(Duration::from_secs(self.config.broadcast_interval_secs));

        // Self announcement
        let my_announcement = self.create_self_announcement()?;
        self.broadcast_announcement(&my_announcement).await?;

        loop {
            tokio::select! {
                // Periodic broadcast
                _ = broadcast_interval.tick() => {
                    let announcement = self.create_self_announcement()?;
                    if let Err(e) = self.broadcast_announcement(&announcement).await {
                        error!("Failed to broadcast announcement: {}", e);
                    }
                }

                // mDNS discoveries
                Some(discovered) = mdns_stream.recv() => {
                    info!("Discovered worker via mDNS: {:?}", discovered.peer_id);
                    self.handle_discovered_worker(discovered).await?;
                }

                // P2P announcements
                // TODO: integrate with p2p event stream
            }
        }
    }

    /// Create self announcement from system probe
    fn create_self_announcement(&self) -> anyhow::Result<WorkerAnnouncement> {
        let system_info = self.system_probe.probe()?;
        let resources = WorkerResources::from_system_info(&system_info);

        let mut announcement = WorkerAnnouncement::new(
            self.identity.peer_id(),
            resources,
            self.config.node_name.clone(),
        );

        // Add loaded models from manifest
        // TODO: get from manifest crate
        announcement.status = WorkerStatus::Active; // Self is always active

        Ok(announcement)
    }

    /// Broadcast announcement to network
    async fn broadcast_announcement(&self, announcement: &WorkerAnnouncement) -> anyhow::Result<()> {
        // Serialize and broadcast via P2P gossip
        let data = serde_json::to_vec(announcement)?;
        self.p2p.gossip_broadcast("worker_announcement", data).await?;
        Ok(())
    }

    /// Handle discovered worker
    async fn handle_discovered_worker(&mut self, announcement: WorkerAnnouncement) -> anyhow::Result<()> {
        if announcement.peer_id == self.identity.peer_id() {
            return Ok(()); // Skip self
        }

        info!(
            "Worker discovered: {} - {} CPU, {}GB RAM, GPU: {:?}",
            announcement.node_name,
            announcement.resources.cpu_cores,
            announcement.resources.ram_gb,
            announcement.resources.gpu_vram_gb
        );

        // Add to pending cache
        self.cache.add_pending(announcement.clone());

        // Notify dashboard
        if let Err(e) = self.new_worker_tx.send(announcement).await {
            warn!("Failed to notify new worker: {}", e);
        }

        Ok(())
    }

    /// Approve a worker
    pub fn approve_worker(&mut self, worker_peer_id: &PeerId) -> anyhow::Result<WorkerApproval> {
        let approval = WorkerApproval::create(*worker_peer_id, &self.identity)?;
        self.cache.approve(worker_peer_id, approval.clone());
        Ok(approval)
    }

    /// Reject a worker
    pub fn reject_worker(&mut self, worker_peer_id: &PeerId) {
        self.cache.reject(worker_peer_id);
    }

    /// Get pending workers for dashboard
    pub fn get_pending_workers(&self) -> Vec<&WorkerAnnouncement> {
        self.cache.get_pending()
    }

    /// Check if worker is trusted
    pub fn is_worker_trusted(&self, worker_peer_id: &PeerId) -> bool {
        self.cache.is_trusted(worker_peer_id)
    }
}
