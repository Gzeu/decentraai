//! Worker discovery via mDNS and P2P gossip
//!
//! This crate provides automatic worker detection on LAN and P2P network,
//! with approval workflow from dashboard.

mod announcement;
mod approval;
mod service;
mod mdns_discovery;

pub use announcement::{WorkerAnnouncement, WorkerStatus, WorkerResources};
pub use approval::{WorkerApproval, TrustRecord, ApprovalStatus};
pub use service::DiscoveryService;
pub use mdns_discovery::MdnsDiscovery;

use std::collections::HashMap;
use std::time::Duration;
use libp2p::PeerId;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Pending worker cache with TTL
pub struct PendingWorkerCache {
    workers: HashMap<PeerId, WorkerAnnouncement>,
    approvals: HashMap<PeerId, WorkerApproval>,
    ttl_duration: Duration,
}

impl PendingWorkerCache {
    pub fn new(ttl_minutes: u64) -> Self {
        Self {
            workers: HashMap::new(),
            approvals: HashMap::new(),
            ttl_duration: Duration::from_secs(ttl_minutes * 60),
        }
    }

    pub fn add_pending(&mut self, announcement: WorkerAnnouncement) {
        self.workers.insert(announcement.peer_id.clone(), announcement);
    }

    pub fn get_pending(&self) -> Vec<&WorkerAnnouncement> {
        let now = Utc::now();
        self.workers
            .values()
            .filter(|w| w.status == WorkerStatus::Pending)
            .filter(|w| {
                let announced_at = DateTime::from_timestamp(w.timestamp as i64, 0).unwrap();
                now.signed_duration_since(announced_at) < chrono::Duration::from_std(self.ttl_duration).unwrap()
            })
            .collect()
    }

    pub fn approve(&mut self, peer_id: &PeerId, approval: WorkerApproval) {
        if let Some(worker) = self.workers.get_mut(peer_id) {
            worker.status = WorkerStatus::Active;
        }
        self.approvals.insert(peer_id.clone(), approval);
    }

    pub fn reject(&mut self, peer_id: &PeerId) {
        self.workers.remove(peer_id);
    }

    pub fn is_trusted(&self, peer_id: &PeerId) -> bool {
        self.approvals.contains_key(peer_id)
    }
}
