//! Compute sharing coordinator (M12).
//!
//! Bridges the pure [`decentraai_compute`] scheduler into the async
//! distributed layer. The coordinator aggregates `ComputeAdvertisement`
//! frames received from peers, selects a worker for each workload, and
//! books/releases resource reservations. A node that wants to offer its
//! own GPU builds its advertisement from the real system probe and
//! broadcasts it on the announce interval.
//!
//! The compute path coexists with the legacy `WorkerAnnouncement`
//! discovery: new compute peers advertise hardware; the legacy path keeps
//! serving nodes that have not opted in to compute sharing yet.

use std::collections::HashSet;
use std::time::Instant;

use libp2p::PeerId;
use tokio::sync::Mutex;

pub use decentraai_compute::{
    ComputeAdvertisement, ComputeAvailability, ComputeCapability, GpuSpec, ResourceReservation,
    ServedModel, WorkloadRequirements, WorkerHealth,
};
use decentraai_compute::{
    CapabilityMatcher, ComputeRegistry, ComputeScheduler, Placement,
};

use decentraai_system_probe::{GpuProbeStatus, GpuSnapshot, SystemSnapshot};

const MIB: u64 = 1024 * 1024;

/// Interval between local compute advertisement broadcasts.
pub const DEFAULT_ADVERTISEMENT_INTERVAL_MS: u64 = 5_000;
/// Heartbeat gap after which a peer's advertisement is treated as stale.
pub const DEFAULT_STALE_AFTER_MS: u64 = 30_000;

/// The compute engine identifier this node runs (matching what the
/// advertisement reports).
pub const ENGINE_LLAMA_SERVER: &str = "llama_server";

/// Pure builder: turns a real hardware probe into a `ComputeAdvertisement`.
///
/// Kept as a free function so unit tests can drive it with synthetic
/// snapshots and GPU states without touching `nvidia-smi` or sysinfo.
pub fn build_advertisement(
    local_peer: PeerId,
    node_name: &str,
    engine: &str,
    snapshot: SystemSnapshot,
    gpu: GpuProbeStatus,
    served_models: Vec<ServedModel>,
    announced_at_ms: u64,
) -> ComputeAdvertisement {
    let (gpu_spec, free_vram_mib) = match &gpu {
        GpuProbeStatus::Nvidia(info) => (
            Some(GpuSpec {
                name: info.name.clone(),
                vram_mb: info.total_vram_mib * MIB / MIB,
                driver: "nvidia".into(),
            }),
            Some(info.free_vram_mib),
        ),
        GpuProbeStatus::Unavailable(_) => (None, None),
    };

    let load_percent = (snapshot.cpu_usage_percent.clamp(0.0, 100.0)) as u8;

    ComputeAdvertisement {
        peer_id: local_peer,
        node_name: node_name.to_string(),
        capability: ComputeCapability {
            cpu_cores: snapshot.logical_cpus.max(1) as u16,
            ram_mb: snapshot.total_memory_bytes / MIB,
            gpu: gpu_spec,
            engine: engine.to_string(),
            served_models,
        },
        availability: ComputeAvailability {
            available_ram_mb: snapshot.available_memory_bytes / MIB,
            available_vram_mb: free_vram_mib,
            load_percent,
            queue_depth: 0,
            tokens_per_second: 0,
            current_latency_ms: 0,
            status: WorkerHealth::Ready,
        },
        announced_at_ms,
    }
}

/// Coordinator-side compute manager.
pub struct ComputeManager {
    local_peer: PeerId,
    node_name: String,
    engine: String,
    advertisement_interval_ms: u64,
    scheduler: Mutex<ComputeScheduler>,
}

impl ComputeManager {
    /// Creates a manager with a fresh scheduler over the given trusted set.
    pub fn new(local_peer: PeerId, node_name: String, trusted: HashSet<PeerId>) -> Self {
        let registry = ComputeRegistry::new(std::time::Duration::from_millis(
            DEFAULT_STALE_AFTER_MS,
        ));
        let ledger = decentraai_compute::ReservationLedger::new(
            std::time::Duration::from_secs(60),
            4,
        );
        let scheduler = ComputeScheduler::new(
            registry,
            ledger,
            CapabilityMatcher::default(),
            trusted,
        );
        Self {
            local_peer,
            node_name,
            engine: ENGINE_LLAMA_SERVER.to_string(),
            advertisement_interval_ms: DEFAULT_ADVERTISEMENT_INTERVAL_MS,
            scheduler: Mutex::new(scheduler),
        }
    }

    pub fn local_peer(&self) -> PeerId {
        self.local_peer
    }

    pub fn advertisement_interval_ms(&self) -> u64 {
        self.advertisement_interval_ms
    }

    pub fn set_advertisement_interval_ms(&mut self, ms: u64) {
        self.advertisement_interval_ms = ms;
    }

    /// Marks `peer` as trusted (eligible to run workloads).
    pub async fn add_trusted(&self, peer: PeerId) {
        self.scheduler.lock().await.add_trusted(peer);
    }

    /// Whether `peer` is trusted to run workloads.
    pub async fn is_trusted(&self, peer: &PeerId) -> bool {
        self.scheduler.lock().await.is_trusted(peer)
    }

    /// Records the latest advertisement received from a peer.
    pub async fn process_advertisement(&self, adv: ComputeAdvertisement) {
        self.scheduler.lock().await.upsert(adv);
    }

    /// Marks a peer offline (stale heartbeat or explicit disconnect).
    pub async fn mark_offline(&self, peer: &PeerId) {
        self.scheduler.lock().await.mark_offline(peer);
    }

    /// Snapshot of live workers, newest-advertisement first.
    pub async fn workers(&self) -> Vec<ComputeAdvertisement> {
        self.scheduler.lock().await.registry().list()
    }

    /// Selects the best eligible worker and books a reservation.
    pub async fn select(&self, req: &WorkloadRequirements) -> Option<Placement> {
        self.scheduler.lock().await.select(req, Instant::now())
    }

    /// Releases a reservation (call on workload completion or failure).
    pub async fn release(&self, reservation_id: uuid::Uuid) {
        self.scheduler.lock().await.release(reservation_id);
    }

    /// Derives a `WorkloadRequirements` for `model_hash` from the union of
    /// what workers advertise they serve (taking the largest RAM/VRAM
    /// footprint so the coordinator never under-reserves). Returns `None`
    /// when no known worker serves the model — the compute path cannot
    /// schedule it.
    pub async fn requirements_for(&self, model_hash: &str) -> Option<WorkloadRequirements> {
        let workers = self.scheduler.lock().await.registry().list();
        let mut ram: u64 = 0;
        let mut vram: u64 = 0;
        for adv in &workers {
            if let Some(model) = adv.capability.model(model_hash) {
                ram = ram.max(model.est_ram_mb);
                vram = vram.max(model.est_vram_mb);
            }
        }
        if ram == 0 && vram == 0 {
            return None;
        }
        Some(WorkloadRequirements::new(model_hash.to_string(), ram, vram))
    }

    /// Builds this node's own advertisement from a real probe and records it
    /// locally (so the coordinator can schedule to itself when appropriate).
    pub async fn advertise_local(
        &self,
        snapshot: SystemSnapshot,
        gpu: GpuProbeStatus,
        served_models: Vec<ServedModel>,
    ) -> ComputeAdvertisement {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let adv = build_advertisement(
            self.local_peer,
            &self.node_name,
            &self.engine,
            snapshot,
            gpu,
            served_models,
            now,
        );
        self.scheduler.lock().await.upsert(adv.clone());
        adv
    }
}

/// Convenience: derive a `GpuSpec` and free-VRAM from a `GpuSnapshot`.
pub fn gpu_from_snapshot(info: &GpuSnapshot) -> (Option<GpuSpec>, Option<u64>) {
    (
        Some(GpuSpec {
            name: info.name.clone(),
            vram_mb: info.total_vram_mib,
            driver: "nvidia".into(),
        }),
        Some(info.free_vram_mib),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_compute::ServedModel;

    fn peer() -> PeerId {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        PeerId::from(keypair.public())
    }

    fn snapshot() -> SystemSnapshot {
        SystemSnapshot {
            logical_cpus: 8,
            cpu_usage_percent: 25.0,
            total_memory_bytes: 32 * 1024 * 1024 * 1024,
            available_memory_bytes: 16 * 1024 * 1024 * 1024,
            used_swap_bytes: 0,
            total_disk_free_bytes: 200 * 1024 * 1024 * 1024,
        }
    }

    fn gpu() -> GpuProbeStatus {
        GpuProbeStatus::Nvidia(GpuSnapshot {
            name: "RTX 4090".into(),
            total_vram_mib: 24564,
            free_vram_mib: 20000,
            utilization_percent: 10,
            temperature_celsius: 55,
            power_draw_watts: 150.0,
        })
    }

    fn model() -> ServedModel {
        ServedModel {
            model_hash: "abc".into(),
            file_name: "model.gguf".into(),
            size_mb: 2048,
            est_ram_mb: 256,
            est_vram_mb: 3072,
        }
    }

    #[test]
    fn builds_real_advertisement_from_probe() {
        let p = peer();
        let adv = build_advertisement(
            p,
            "gpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            gpu(),
            vec![model()],
            1_700_000_000_000,
        );
        assert_eq!(adv.peer_id, p);
        assert_eq!(adv.node_name, "gpu-rig");
        assert_eq!(adv.capability.cpu_cores, 8);
        assert_eq!(adv.capability.ram_mb, 32 * 1024);
        assert_eq!(adv.availability.available_ram_mb, 16 * 1024);
        assert_eq!(adv.availability.available_vram_mb, Some(20000));
        assert_eq!(adv.availability.load_percent, 25);
        assert!(adv.capability.has_model("abc"));
        let spec = adv.capability.gpu.unwrap();
        assert_eq!(spec.name, "RTX 4090");
        assert_eq!(spec.vram_mb, 24564);
    }

    #[test]
    fn builds_cpu_only_advertisement_when_gpu_missing() {
        let p = peer();
        let adv = build_advertisement(
            p,
            "cpu-rig",
            ENGINE_LLAMA_SERVER,
            snapshot(),
            GpuProbeStatus::Unavailable("nvidia-smi not found".into()),
            vec![model()],
            0,
        );
        assert!(adv.capability.gpu.is_none());
        assert_eq!(adv.availability.available_vram_mb, None);
    }

    #[tokio::test]
    async fn selects_and_releases_via_manager() {
        let p = peer();
        let manager = ComputeManager::new(p, "coordinator".into(), HashSet::from([p]));
        let adv = build_advertisement(p, "gpu-rig", ENGINE_LLAMA_SERVER, snapshot(), gpu(), vec![model()], 0);
        manager.process_advertisement(adv).await;

        let req = WorkloadRequirements::new("abc".into(), 256, 3072);
        let placement = manager.select(&req).await.expect("eligible worker");
        assert_eq!(placement.worker, p);
        assert!(manager.select(&req).await.is_some(), "only one reservation tracked per select");
        manager.release(placement.reservation.reservation_id).await;
        assert!(manager.select(&req).await.is_some());
    }
}
