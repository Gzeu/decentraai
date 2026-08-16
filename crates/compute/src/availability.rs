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
    /// Real GPU thermal pressure (Celsius) from the last probe, when a GPU is
    /// present and reporting. `None` = no GPU / no measurement (UNKNOWN) —
    /// never fabricated. Foundation for thermal-aware adaptive contribution.
    #[serde(default)]
    pub gpu_temperature_celsius: Option<u8>,
    /// Real GPU utilization (0..100) from the last probe, when available.
    #[serde(default)]
    pub gpu_utilization_percent: Option<u8>,
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
    /// Whether the node accepts inference work routed from *remote* peers
    /// (coordinator-side policy). The local node always accepts its own
    /// work; this flag is the honest opt-in for remote resource sharing
    /// (config `inference.allow_remote_inference`). Backward-compatible:
    /// advertisements that predate the field deserialize to `false` — a
    /// conservative default — so an old worker is never scheduled remotely
    /// by a new coordinator without its operator opting in.
    #[serde(default)]
    pub accepts_remote_inference: bool,
    /// Compact, stable human-readable node identifier (e.g. `dca-8f2a3c`),
    /// derived from the node's peer id. Lets operators tell nodes apart at a
    /// glance when every machine would otherwise show the same default name.
    /// Backward-compatible: advertisements that predate the field deserialize
    /// to the empty string, and the dashboard falls back to deriving a short
    /// id client-side from the peer id.
    #[serde(default)]
    pub node_id: String,
    /// DecentraAI build version this node runs (e.g. a git SHA / crate
    /// version). Backward-compatible: older advertisements that predate the
    /// field deserialize to empty (UNKNOWN), so a coordinator can still
    /// show "unknown" rather than break. Lets operators see which fabric
    /// members are on which version (e.g. after an update).
    #[serde(default)]
    pub node_version: String,
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
                    context_tokens: 0,
                }],
                can_provision: false,
                available_models: vec![],
            },
            availability: ComputeAvailability {
                available_ram_mb: 12 * 1024,
                available_vram_mb: Some(18 * 1024),
                load_percent: 32,
                queue_depth: 0,
                tokens_per_second: 60,
                current_latency_ms: 90,
                status: WorkerHealth::Ready,
                gpu_temperature_celsius: None,
                gpu_utilization_percent: None,
            },
            announced_at_ms: 1_700_000_000_000,
            accepts_remote_inference: true,
            node_id: "dca-8f2a3c".into(),
            node_version: env!("CARGO_PKG_VERSION").to_string(),
        };

        let json = serde_json::to_string(&adv).unwrap();
        let back: ComputeAdvertisement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, adv);
        assert_eq!(back.peer_id, peer);
        assert_eq!(back.node_id, "dca-8f2a3c");
    }

    #[test]
    fn legacy_advertisement_without_node_id_deserializes_safely() {
        // A pre-node_id advertisement must keep deserializing: the field is
        // optional and defaults to empty, so old workers never break a new
        // coordinator (and vice versa).
        let peer = crate::testutil::test_peer();
        let legacy = format!(
            r#"{{"peer_id":"{peer}","node_name":"old-worker","capability":{{"cpu_cores":4,"ram_mb":8192,"gpu":null,"engine":"llama_server","served_models":[],"can_provision":false}},"availability":{{"available_ram_mb":4096,"available_vram_mb":null,"load_percent":10,"queue_depth":0,"tokens_per_second":40,"current_latency_ms":80,"status":"Ready"}},"announced_at_ms":1700000000000,"accepts_remote_inference":false}}"#
        );
        let adv: ComputeAdvertisement = serde_json::from_str(&legacy).unwrap();
        assert_eq!(adv.node_id, "");
        assert_eq!(adv.node_name, "old-worker");
        assert!(!adv.accepts_remote_inference);
    }
}
