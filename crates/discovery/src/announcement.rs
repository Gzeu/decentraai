//! Worker announcement with resources and status

use serde::{Deserialize, Serialize};
use libp2p::PeerId;
use system_probe::SystemInfo;

/// Worker announcement broadcast to network
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerAnnouncement {
    pub peer_id: PeerId,
    pub resources: WorkerResources,
    pub loaded_models: Vec<String>,  // model hashes
    pub status: WorkerStatus,
    pub timestamp: u64,
    pub node_name: String,
}

/// Worker compute resources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerResources {
    pub cpu_cores: u32,
    pub ram_gb: u32,
    pub gpu_vram_gb: Option<u32>,
    pub gpu_count: u32,
    pub bandwidth_mbps: u32,
    pub disk_available_gb: u64,
}

impl WorkerResources {
    pub fn from_system_info(info: &SystemInfo) -> Self {
        Self {
            cpu_cores: info.cpu.cores as u32,
            ram_gb: (info.memory.total_bytes / (1024 * 1024 * 1024)) as u32,
            gpu_vram_gb: info.gpu.as_ref().map(|g| (g.vram_bytes / (1024 * 1024 * 1024)) as u32),
            gpu_count: info.gpu.as_ref().map(|_| 1).unwrap_or(0),
            bandwidth_mbps: 1000,  // TODO: measure actual bandwidth
            disk_available_gb: info.storage.available_bytes / (1024 * 1024 * 1024),
        }
    }
}

/// Worker status in network
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerStatus {
    Pending,    // awaiting approval
    Active,     // approved and available
    Rejected,   // denied by user
    Offline,    // temporarily unavailable
}

impl WorkerAnnouncement {
    pub fn new(peer_id: PeerId, resources: WorkerResources, node_name: String) -> Self {
        Self {
            peer_id,
            resources,
            loaded_models: Vec::new(),
            status: WorkerStatus::Pending,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            node_name,
        }
    }

    pub fn add_model(&mut self, model_hash: String) {
        if !self.loaded_models.contains(&model_hash) {
            self.loaded_models.push(model_hash);
        }
    }

    pub fn can_serve_model(&self, model_hash: &str) -> bool {
        self.loaded_models.contains(&model_hash.to_string())
    }
}
