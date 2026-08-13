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
}

impl ComputeCapability {
    /// Whether this worker serves the given model hash.
    pub fn has_model(&self, model_hash: &str) -> bool {
        self.served_models
            .iter()
            .any(|m| m.model_hash == model_hash)
    }

    /// The served model matching `model_hash`, if any.
    pub fn model(&self, model_hash: &str) -> Option<&ServedModel> {
        self.served_models
            .iter()
            .find(|m| m.model_hash == model_hash)
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
            }],
        }
    }

    #[test]
    fn has_model_matches_hash() {
        let cap = capability();
        assert!(cap.has_model("abc"));
        assert!(!cap.has_model("nope"));
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
}