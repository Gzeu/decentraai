//! Hard model-artifact size policy for the normal node model pool.
//!
//! Rule: eligibility is decided by the ACTUAL downloadable artifact size,
//! never by parameter count. A "0.5B" model with a 3 GB BF16 checkpoint is
//! NOT pool-eligible; a 7B model with a 1.9 GB Q2 artifact would be (though
//! quality gates are separate). Large artifacts remain welcome on external
//! or high-resource workers — this gate only governs what the fabric may
//! AUTO-provision onto resource-constrained nodes.

use serde::{Deserialize, Serialize};

/// HARD limit for auto-provisioned model artifacts: 2 GiB.
pub const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Preferred target for the normal pool: ≤ 1 GiB.
pub const RECOMMENDED_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;

/// Configurable ceiling (defaults to [`MAX_ARTIFACT_BYTES`]). Operators of
/// beefier nodes may raise it, but it can never be removed entirely by
/// config — the floor is the hard constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactLimit {
    pub max_bytes: u64,
}

impl Default for ArtifactLimit {
    fn default() -> Self {
        Self {
            max_bytes: MAX_ARTIFACT_BYTES,
        }
    }
}

impl ArtifactLimit {
    /// Whether an artifact of `size_bytes` may be provisioned.
    pub fn allows(&self, size_bytes: u64) -> bool {
        // The hard floor wins even if config tries to go above 2 GiB? No:
        // operators own their nodes. But zero/nonsense limits fail closed.
        self.max_bytes > 0 && size_bytes <= self.max_bytes && size_bytes <= MAX_ARTIFACT_BYTES.max(self.max_bytes)
    }

    /// Whether the artifact fits comfortably in the recommended envelope.
    pub fn recommended(&self, size_bytes: u64) -> bool {
        size_bytes <= RECOMMENDED_ARTIFACT_BYTES && self.allows(size_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limit_is_two_gib() {
        assert_eq!(ArtifactLimit::default().max_bytes, 2 * 1024 * 1024 * 1024);
        assert!(ArtifactLimit::default().allows(MAX_ARTIFACT_BYTES));
        assert!(!ArtifactLimit::default().allows(MAX_ARTIFACT_BYTES + 1));
    }

    #[test]
    fn bf16_checkpoint_of_small_model_is_rejected() {
        // Qwen2.5-Coder-1.5B BF16 ≈ 3.09 GB: parameter count says "small",
        // artifact size says NO. This exact case motivated the policy.
        let limit = ArtifactLimit::default();
        assert!(!limit.allows(3_090_000_000));
    }

    #[test]
    fn zero_limit_fails_closed() {
        let limit = ArtifactLimit { max_bytes: 0 };
        assert!(!limit.allows(1));
    }

    #[test]
    fn recommended_envelope_is_stricter() {
        let limit = ArtifactLimit::default();
        let one_and_a_half_gb = 1_500_000_000;
        assert!(limit.allows(one_and_a_half_gb), "allowed but not preferred");
        assert!(!limit.recommended(one_and_a_half_gb));
        assert!(limit.recommended(900_000_000));
    }
}
