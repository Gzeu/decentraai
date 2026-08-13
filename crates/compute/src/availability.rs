//! Time-varying availability of a compute worker plus the full
//! advertisement broadcast over the P2P network.

use libp2p::PeerId;
use serde::{Deserialize, Serialize};

use crate::capability::ComputeCapability;

/// Health of a worker as of the last heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerHealth {
    /// Accepting new workloads.
    Ready,
    /// Serving work back-to-back; still accepting (short queue).
    Busy,
    /// Running hot (high load, low headroom) — warn-only.
    Degraded,
    /// Not accepting work (over temperature, out of memory, ...).
    Unhealthy,
    /// No heartbeat within the stale window; coordinator-side only.
    Offline,
}

impl WorkerHealth {
    /// Whether the worker can accept new workloads right now.
    pub fn can_accept_work(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Runtime state that changes every heartbeat: free memory, load, queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputeAvailability {
    /// Free host RAM in MiB at probe time.
    pub available_ram_mb: u64,
    /// Free VRAM in MiB at probe time (`None` on CPU-only nodes).
    pub available_vram_mb: Option<u64>,
    /// Current load 0..100.
    pub load_percent: u8,
    pub queue_depth: u32,
    pub tokens_per_second: u32,
    pub current_latency_ms: u32,
    pub status: WorkerHealth,
}

impl ComputeAvailability {
    /// Whether the worker reports itself able to accept work.
    pub fn healthy(&self) -> bool {
        self.status.can_accept_work()
    }
}

/// One full advertisement: static [`ComputeCapability`] plus current
/// [`ComputeAvailability`]. This is what a compute node broadcasts on the
/// LAN so coordinators can build a [`crate::registry::ComputeRegistry`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputeAdvertisement {
    pub peer_id: PeerId,
    pub node_name: String,
    pub capability: ComputeCapability,
    pub availability: ComputeAvailability,
    /// Unix epoch milliseconds when this advertisement was produced.
    pub announced_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{GpuSpec, ServedModel};

    #[test]
    fn health_gates_acceptance() {
        assert!(WorkerHealth::Ready.can_accept_work());
        assert!(!WorkerHealth::Busy.can_accept_work());
        assert!(!WorkerHealth::Unhealthy.can_accept_work());
    }

    #[test]
    fn advertisement_round_trips_through_json() {
        let peer = crate::testutil::test_peer();
        let adv = ComputeAdvertisement {
            peer_id: peer,
            node_name: "gpu-rig".into(),
            capability: ComputeCapability {
                cpu_cores: 8,
                ram_mb: 16 * 1024,
                gpu: Some(GpuSpec {
                    name: "RTX 4090".into(),
                    vram_mb: 24 * 1024,
                    driver: "565".into(),
                }),
                engine: "llama_server".into(),
                served_models: vec![ServedModel {
                    model_hash: "abc".into(),
                    file_name: "model.gguf".into(),
                    size_mb: 2048,
                    est_ram_mb: 256,
                    est_vram_mb: 3072,
                }],
                can_provision: false,
            },
            availability: ComputeAvailability {
                available_ram_mb: 12 * 1024,
                available_vram_mb: Some(18 * 1024),
                load_percent: 32,
                queue_depth: 0,
                tokens_per_second: 60,
                current_latency_ms: 90,
                status: WorkerHealth::Ready,
            },
            announced_at_ms: 1_700_000_000_000,
        };

        let json = serde_json::to_string(&adv).unwrap();
        let back: ComputeAdvertisement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, adv);
        assert_eq!(back.peer_id, peer);
    }
}