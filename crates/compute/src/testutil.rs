//! Shared fixtures for the crate's unit tests. Only compiled under `cfg(test)`.

#![cfg(test)]

use libp2p::PeerId;

use crate::availability::{ComputeAdvertisement, ComputeAvailability, WorkerHealth};
use crate::capability::{ComputeCapability, GpuSpec, ServedModel};

pub(crate) fn test_peer() -> PeerId {
    let keypair = libp2p::identity::Keypair::generate_ed25519();
    PeerId::from(keypair.public())
}

pub(crate) fn test_advertisement(
    peer: PeerId,
    avail_ram: u64,
    avail_vram: Option<u64>,
    load: u8,
    queue: u32,
    health: WorkerHealth,
) -> ComputeAdvertisement {
    ComputeAdvertisement {
        peer_id: peer,
        node_name: "worker".into(),
        node_id: "dca-test01".into(),
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
        },
        availability: ComputeAvailability {
            available_ram_mb: avail_ram,
            available_vram_mb: avail_vram,
            load_percent: load,
            queue_depth: queue,
            tokens_per_second: 60,
            current_latency_ms: 90,
            status: health,
        },
        announced_at_ms: 1_700_000_000_000,
        accepts_remote_inference: true,
    }
}
