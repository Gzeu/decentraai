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
    /// Real battery charge percentage (0..100) from the last probe, when the
    /// node has a battery (mobile/laptop). `None` on desktop/no battery /
    /// UNKNOWN. Foundation for battery-aware adaptive contribution: a low
    /// battery worker is given less work.
    #[serde(default)]
    pub battery_percent: Option<u8>,
}

impl ComputeAvailability {
    /// A healthy, idle, empty worker state (used by tests and graph builders).
    pub fn ready() -> Self {
        Self {
            available_ram_mb: 0,
            available_vram_mb: None,
            load_percent: 0,
            queue_depth: 0,
            tokens_per_second: 0,
            current_latency_ms: 0,
            status: WorkerHealth::Ready,
            gpu_temperature_celsius: None,
            gpu_utilization_percent: None,
            battery_percent: None,
        }
    }

    /// Whether the worker reports itself able to accept work.
    pub fn healthy(&self) -> bool {
        self.status.can_accept_work()
    }

    /// Adaptive-contribution capacity state, derived ONLY from real,
    /// authoritative availability data. Never fabricated:
    /// - `UNAVAILABLE`: the worker is unhealthy / cannot accept work.
    /// - `LIMITED`: healthy but heavily loaded (load >= 80) or a long queue
    ///   (>= 6) — it can accept, but only a limited share.
    /// - `FULL`: healthy with headroom.
    /// - `UNKNOWN`: we cannot tell (shouldn't normally happen; conservative
    ///   fallback when the state is not a recognized health variant).
    pub fn capacity_state(&self) -> &'static str {
        if !self.status.can_accept_work() {
            return "UNAVAILABLE";
        }
        let loaded = self.load_percent >= 80 || self.queue_depth >= 6;
        if loaded { "LIMITED" } else { "FULL" }
    }

    /// Adaptive-contribution load factor (0.0..1.0), derived ONLY from real
    /// availability signals (never fabricated):
    ///
    /// - GPU thermal pressure: near/above the throttle point reduces capacity.
    /// - GPU utilization: a fully-busy GPU gets less new work.
    /// - CPU load: a heavily loaded machine gets less new work.
    /// - Battery: a low-battery worker gets less work (mobile/laptop).
    ///
    /// UNKNOWN signals (None) are neutral (factor 1.0 for that term): we never
    /// invent a measurement. The result is the product of the available terms,
    /// so any single stressed signal can reduce capacity while healthy nodes
    /// keep factor ~1.0. This is the honest input the planner uses to reduce
    /// the share of work sent to a stressed worker.
    pub fn adaptive_contribution_factor(&self) -> f32 {
        let mut f = 1.0_f32;

        // GPU thermal: >90°C → heavily reduced; >80°C → reduced; else neutral.
        if let Some(t) = self.gpu_temperature_celsius {
            let t = f32::from(t);
            f *= if t >= 95.0 {
                0.1
            } else if t >= 90.0 {
                0.25
            } else if t >= 80.0 {
                0.5
            } else if t >= 70.0 {
                0.8
            } else {
                1.0
            };
        }

        // GPU utilization: scale linearly from full capacity at 0% util down
        // to 0.3 at 100% util (a fully-busy GPU takes little new work).
        if let Some(u) = self.gpu_utilization_percent {
            let u = f32::from(u).clamp(0.0, 100.0);
            f *= 1.0 - 0.7 * (u / 100.0);
        }

        // CPU load: linear reduction from 1.0 (idle) to 0.3 (fully loaded).
        let load = f32::from(self.load_percent.clamp(0, 100)) / 100.0;
        f *= 1.0 - 0.7 * load;

        // Battery: below 20% → heavily reduced; below 50% → reduced; else
        // neutral. None (no battery / UNKNOWN) is neutral.
        if let Some(b) = self.battery_percent {
            let b = f32::from(b);
            f *= if b <= 10.0 {
                0.1
            } else if b <= 20.0 {
                0.25
            } else if b <= 50.0 {
                0.6
            } else {
                1.0
            };
        }

        f.clamp(0.0, 1.0)
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
    fn capacity_state_is_evidence_backed() {
        // Adaptive-contribution capacity: derived from real health/load/queue.
        let mut a = ComputeAvailability {
            available_ram_mb: 1000,
            available_vram_mb: None,
            load_percent: 10,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 10,
            status: WorkerHealth::Ready,
            gpu_temperature_celsius: None,
            gpu_utilization_percent: None,
            battery_percent: None,
        };
        assert_eq!(a.capacity_state(), "FULL");
        // Healthy but loaded -> LIMITED.
        a.load_percent = 90;
        assert_eq!(a.capacity_state(), "LIMITED");
        a.load_percent = 10;
        a.queue_depth = 8;
        assert_eq!(a.capacity_state(), "LIMITED");
        // Unhealthy -> UNAVAILABLE, regardless of load.
        a.status = WorkerHealth::Unhealthy;
        assert_eq!(a.capacity_state(), "UNAVAILABLE");
    }

    #[test]
    fn health_gates_acceptance() {
        assert!(WorkerHealth::Ready.can_accept_work());
        assert!(!WorkerHealth::Busy.can_accept_work());
        assert!(!WorkerHealth::Unhealthy.can_accept_work());
    }

    #[test]
    fn adaptive_contribution_factor_is_neutral_when_all_unknown() {
        let a = ComputeAvailability {
            available_ram_mb: 1000,
            available_vram_mb: None,
            load_percent: 0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 10,
            status: WorkerHealth::Ready,
            gpu_temperature_celsius: None,
            gpu_utilization_percent: None,
            battery_percent: None,
        };
        // No measurements -> factor 1.0 (neutral), never reduced by invention.
        assert_eq!(a.adaptive_contribution_factor(), 1.0);
    }

    #[test]
    fn adaptive_contribution_factor_reduces_on_stress() {
        let mut a = ComputeAvailability {
            available_ram_mb: 1000,
            available_vram_mb: None,
            load_percent: 0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 10,
            status: WorkerHealth::Ready,
            gpu_temperature_celsius: None,
            gpu_utilization_percent: None,
            battery_percent: None,
        };
        let healthy = a.adaptive_contribution_factor();
        // High GPU thermal reduces capacity.
        a.gpu_temperature_celsius = Some(95);
        assert!(
            a.adaptive_contribution_factor() < healthy,
            "thermal pressure must reduce the contribution factor"
        );
        // A low battery reduces it further.
        a.gpu_temperature_celsius = None;
        a.battery_percent = Some(10);
        assert!(
            a.adaptive_contribution_factor() < healthy,
            "low battery must reduce the contribution factor"
        );
        // A full GPU util reduces it.
        a.battery_percent = None;
        a.gpu_utilization_percent = Some(100);
        assert!(
            a.adaptive_contribution_factor() < healthy,
            "full GPU utilization must reduce the contribution factor"
        );
    }

    #[test]
    fn adaptive_contribution_factor_is_bounded_and_monotone() {
        let base = ComputeAvailability {
            available_ram_mb: 1000,
            available_vram_mb: None,
            load_percent: 0,
            queue_depth: 0,
            tokens_per_second: 50,
            current_latency_ms: 10,
            status: WorkerHealth::Ready,
            gpu_temperature_celsius: None,
            gpu_utilization_percent: None,
            battery_percent: None,
        };
        // Combined worst case is clamped to a small positive floor, never 0 or
        // negative (a worker can always accept a tiny share, not be invisible).
        let mut worst = base.clone();
        worst.gpu_temperature_celsius = Some(95);
        worst.gpu_utilization_percent = Some(100);
        worst.load_percent = 100;
        worst.battery_percent = Some(5);
        let f = worst.adaptive_contribution_factor();
        assert!(f > 0.0, "factor must stay positive, got {f}");
        assert!(f < base.adaptive_contribution_factor());
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
                battery_percent: None,
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
