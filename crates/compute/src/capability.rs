//! Static hardware + software capability of a compute worker.

use serde::{Deserialize, Serialize};

/// GPU hardware specification. A worker without a GPU advertises `None`
/// and is a CPU-only node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuSpec {
    pub name: String,
    /// Total VRAM in MiB.
    pub vram_mb: u64,
    pub driver: String,
}

/// A model the node can execute, with its estimated memory footprint.
///
/// The estimates are what the scheduler needs for capability matching;
/// they are conservative memory budgets (model bytes + KV cache headroom),
/// not exact allocations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServedModel {
    pub model_hash: String,
    pub file_name: String,
    pub size_mb: u64,
    /// Estimated host RAM footprint when loaded (MiB).
    pub est_ram_mb: u64,
    /// Estimated VRAM footprint when loaded on GPU (MiB). `0` = CPU-only.
    pub est_vram_mb: u64,
    /// Real KV-cache context window (tokens) this worker allocates for the
    /// model (`--ctx-size` on llama-server). `0` = unknown. Coordinator uses
    /// this as the honest capacity for KV-headroom accounting (M20).
    /// Backward-compatible: older workers that predate M20 omit the field,
    /// which deserializes to `0` (unknown capacity).
    #[serde(default)]
    pub context_tokens: u32,
}

impl ServedModel {
    /// Pure, honest estimate of the model's VRAM footprint when fully offloaded
    /// to a GPU (MiB), and the KV-cache overhead for `ctx` tokens. Returning
    /// `0` means the model is CPU-only (no GPU offload). The estimate is the
    /// model bytes (the dominant VRAM cost at Q4/Q5) plus a small KV headroom
    /// so the capacity matcher does not over-commit a GPU worker.
    pub fn estimate_vram_mb(model_size_bytes: u64, gpu_offload: bool, ctx: u32) -> u64 {
        if !gpu_offload {
            return 0;
        }
        let model_mib = (model_size_bytes / (1024 * 1024)).max(1);
        // Approximate KV-cache headroom: ~2 MiB per 1024 ctx tokens per layer is
        // engine/model dependent; use a coarse 1 MiB per 256 tokens as a safe
        // ceiling so placement never over-commits VRAM.
        let kv_mib = (u64::from(ctx) / 256).max(1);
        model_mib + kv_mib
    }

    /// Pure, conservative estimate of the host RAM footprint (MiB) to LOAD a
    /// GGUF model fully into system memory — the CPU-only execution case.
    ///
    /// A Q4/Q5 GGUF's weights are ~the file size; loading into RAM needs that
    /// plus page/buffer overhead, so we budget 120% of the file. This is the
    /// single authoritative full-load RAM estimator; the "Models I can run"
    /// hub view routes through it. A worker that GPU-offloads the weights
    /// holds only a *working set* in RAM, which is a different (smaller) load
    /// mode — the distributed scheduler uses its own heuristic for that case
    /// and it must be validated on hardware, not swapped in blindly.
    pub fn estimate_ram_mb(model_size_bytes: u64) -> u64 {
        let mib = (model_size_bytes / (1024 * 1024)).max(1);
        mib * 120 / 100
    }
}

/// Immutable capability of a worker. Advertised on registration and changes
/// rarely (new model added, GPU swapped). Compare with
/// [`crate::availability::ComputeAvailability`] which changes every heartbeat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputeCapability {
    pub cpu_cores: u16,
    /// Total host RAM in MiB.
    pub ram_mb: u64,
    pub gpu: Option<GpuSpec>,
    /// Inference engine, e.g. "llama_server".
    pub engine: String,
    /// Models this node can execute (already local — compute nodes serve
    /// what they hold; model download is a separate policy-gated step).
    pub served_models: Vec<ServedModel>,
    /// Whether this worker will fetch a missing model on demand when a
    /// workload demands it and the coordinator routes to it (M14). Such a
    /// worker is eligible for a workload it does not yet serve, subject to
    /// the coordinator's `allow_provisioning` policy.
    pub can_provision: bool,
    /// All models this node has on disk (registry), regardless of whether
    /// they are currently loaded. Used by the coordinator's model picker
    /// to discover what a worker can serve — the worker swaps its engine
    /// on request to serve a model from its on-disk collection.
    #[serde(default)]
    pub available_models: Vec<ServedModel>,
}

impl ComputeCapability {
    /// Whether this worker serves the given model hash (currently loaded or
    /// available on disk).
    pub fn has_model(&self, model_hash: &str) -> bool {
        self.served_models
            .iter()
            .any(|m| m.model_hash == model_hash)
            || self
                .available_models
                .iter()
                .any(|m| m.model_hash == model_hash)
    }

    /// Whether this worker can handle `model_hash` either because it serves
    /// it today or because it will provision it on demand.
    pub fn serves_or_provisions(&self, model_hash: &str) -> bool {
        self.has_model(model_hash) || self.can_provision
    }

    /// The served model matching `model_hash`, if any.
    pub fn model(&self, model_hash: &str) -> Option<&ServedModel> {
        self.served_models
            .iter()
            .find(|m| m.model_hash == model_hash)
    }

    /// The model on disk (registry) matching `model_hash`, if any — present
    /// but not necessarily loaded into the engine right now.
    pub fn available_model(&self, model_hash: &str) -> Option<&ServedModel> {
        self.available_models
            .iter()
            .find(|m| m.model_hash == model_hash)
    }

    /// Marginal host RAM (MiB) a *new request* for `model_hash` costs this
    /// worker, given what is already resident.
    ///
    /// A model that is currently served (`served_models`) is already loaded
    /// into the engine: its weights are in RAM and already subtracted from
    /// `available_ram_mb` (the probe measures live free memory). Charging
    /// `est_ram_mb` again would double-count the resident weights and make
    /// the admission gate reject requests for the very model the engine is
    /// running — exactly what happened on the Desktop worker for its active
    /// Llama model (required 2240 MiB vs ~1992 MiB free). For a resident
    /// model the marginal cost is the KV-cache/context working set plus
    /// compute buffers, estimated conservatively from the advertised context
    /// window (`~128 KiB/token`, floored at 64 MiB — a coarse safe ceiling).
    ///
    /// A model that is only *available on disk* (not loaded yet) still costs
    /// its full load estimate. Unknown hashes cost nothing extra; the caller
    /// applies its own provisioning default.
    pub fn request_ram_mb(&self, model_hash: &str) -> u64 {
        if let Some(m) = self.model(model_hash) {
            match m.context_tokens {
                // Unknown context window (pre-M20 worker): assume a
                // conservative ~4k-token working set rather than a tiny floor.
                0 => 512,
                ctx => (u64::from(ctx) / 8).max(64),
            }
        } else if let Some(m) = self.available_model(model_hash) {
            m.est_ram_mb
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability() -> ComputeCapability {
        ComputeCapability {
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
        }
    }

    #[test]
    fn has_model_matches_hash() {
        let cap = capability();
        assert!(cap.has_model("abc"));
        assert!(!cap.has_model("nope"));
    }

    #[test]
    fn has_model_checks_available_models() {
        let mut cap = capability();
        assert!(!cap.has_model("available-on-disk"));
        cap.available_models.push(ServedModel {
            model_hash: "available-on-disk".into(),
            file_name: "other.gguf".into(),
            size_mb: 1024,
            est_ram_mb: 128,
            est_vram_mb: 0,
            context_tokens: 4096,
        });
        assert!(cap.has_model("available-on-disk"));
    }

    #[test]
    fn serves_or_provisions_accepts_provisioning_workers() {
        let mut cap = capability();
        assert!(!cap.serves_or_provisions("nope"));
        cap.can_provision = true;
        assert!(cap.serves_or_provisions("nope"));
        assert!(cap.serves_or_provisions("abc"));
    }

    #[test]
    fn model_returns_the_matching_entry() {
        let cap = capability();
        assert_eq!(cap.model("abc").unwrap().file_name, "model.gguf");
        assert!(cap.model("missing").is_none());
    }

    #[test]
    fn serializes_and_round_trips() {
        let cap = capability();
        let json = serde_json::to_string(&cap).unwrap();
        let back: ComputeCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cap);
    }

    #[test]
    fn estimate_vram_is_nonzero_only_with_gpu_offload() {
        // CPU-only: never advertises VRAM (0), so the capacity matcher treats
        // the worker as CPU-op and VRAM headroom is not committed.
        assert_eq!(ServedModel::estimate_vram_mb(1 << 30, false, 4096), 0);
        // GPU offload: model bytes dominate + KV headroom; a bigger model
        // needs more VRAM (pure, monotone), so GPU vs CPU workers differ.
        let small = ServedModel::estimate_vram_mb(2_147_483_648, true, 4096); // 2 GiB model
        let big = ServedModel::estimate_vram_mb(8_589_934_592, true, 8192); // 8 GiB model
        assert!(small > 0);
        assert!(big > small, "larger model must advertise more VRAM");
        assert!(
            small >= (2048 + (4096 / 256)) as u64,
            "model MiB + KV headroom"
        );
    }

    #[test]
    fn estimate_ram_is_full_load_footprint() {
        // Full-load RAM = 120% of the model bytes, monotone and floor-1.
        let small = ServedModel::estimate_ram_mb(1 << 30); // 1 GiB
        let big = ServedModel::estimate_ram_mb(8 << 30); // 8 GiB
        assert_eq!(small, 1024 * 120 / 100);
        assert!(big > small, "larger model needs more full-load RAM");
        // A tiny file still gets a floor of at least 1 MiB budget.
        assert!(ServedModel::estimate_ram_mb(1) >= 1);
    }

    #[test]
    fn request_ram_for_resident_model_is_context_only() {
        // The worker *serves* the model (weights already resident). A new
        // request must NOT be charged the full-load estimate again — that
        // double-counts the resident weights and rejects the very model the
        // engine is running. Only the KV/context working set is marginal.
        let cap = capability();
        // `capability()` uses context_tokens: 0 (unknown window, pre-M20): the
        // conservative 512 MiB default must still be far below a real full
        // load, proving the resident path never re-charges the weights.
        // Give the served model a realistic full-load est (Desktop Llama was
        // ~2240 MiB required vs ~1992 MiB free — the bug this fixes).
        let mut cap = capability();
        cap.served_models[0].est_ram_mb = 2240;
        let resident = cap.request_ram_mb("abc");
        assert_eq!(
            resident, 512,
            "unknown context window defaults conservatively"
        );
        assert!(
            resident < cap.model("abc").unwrap().est_ram_mb,
            "resident model charges context, not a second full load"
        );
        assert!(resident >= 64, "context floor still reserves headroom");

        // A known context window scales with it (128 KiB/token).
        let mut known = capability();
        known.served_models[0].context_tokens = 4096;
        assert_eq!(known.request_ram_mb("abc"), 512);
        known.served_models[0].context_tokens = 16384;
        assert_eq!(known.request_ram_mb("abc"), 2048);
    }

    #[test]
    fn request_ram_for_disk_only_model_is_full_load() {
        // A model present on disk but not loaded yet still costs the full
        // load estimate — serving it means loading the weights.
        let mut cap = capability();
        cap.available_models.push(ServedModel {
            model_hash: "disk-only".into(),
            file_name: "other.gguf".into(),
            size_mb: 1024,
            est_ram_mb: 128,
            est_vram_mb: 0,
            context_tokens: 4096,
        });
        assert_eq!(cap.request_ram_mb("disk-only"), 128);
        // Unknown hashes charge nothing; the caller applies its own default.
        assert_eq!(cap.request_ram_mb("unknown-hash"), 0);
    }
}
