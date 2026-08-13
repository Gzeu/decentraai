//! What a workload needs, expressed in terms a worker's capability and
//! availability can be matched against.

/// Resource/time requirements of a workload to be executed remotely.
///
/// This is deliberately protocol-agnostic (no `InferRequest` dependency) so
/// the matching logic stays pure and testable. The distributed layer builds
/// one from an `InferRequest` plus the requested model's footprint.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadRequirements {
    pub model_hash: String,
    /// Estimated host RAM needed to run the model (MiB).
    pub est_ram_mb: u64,
    /// Estimated VRAM needed to run the model (MiB). `0` = CPU-only.
    pub est_vram_mb: u64,
    pub max_tokens: u32,
    pub stream: bool,
    pub priority: u8,
}

impl WorkloadRequirements {
    pub fn new(model_hash: String, est_ram_mb: u64, est_vram_mb: u64) -> Self {
        Self {
            model_hash,
            est_ram_mb,
            est_vram_mb,
            max_tokens: 1024,
            stream: true,
            priority: 128,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requirements_defaults_are_sane() {
        let req = WorkloadRequirements::new("hash".into(), 256, 3072);
        assert_eq!(req.max_tokens, 1024);
        assert!(req.stream);
        assert_eq!(req.priority, 128);
    }
}