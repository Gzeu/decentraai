//! Supernova integration facade (O1).
//!
//! The real implementation lives in `decentraai-mx-supernova`, whose shapes
//! were verified live against testnet (T2.0.8.0, 600ms rounds) and whose
//! activation rule mirrors `IsAsyncExecutionEnabledForEpochAndRound`
//! (`mx-chain-go` v2.0.8 `common/common.go`): epoch flag AND round flag,
//! both pinned by the operator. This module keeps the historical
//! intelligence-layer entry point working: validate untrusted plan output,
//! then report activation. Nothing here touches network, trust, or secrets.

pub use decentraai_mx_supernova::{
    ActivationConfig, BlockObservation, ChainStatus, DEFAULT_MAX_BYTES, DEFAULT_TIMEOUT_MS,
    ExecutionResult, FinalityState, MAX_BYTES_CEILING, MAX_POLL_INTERVAL_MS, MIN_POLL_INTERVAL_MS,
    MxProxy, MxTrack, NetworkConfig, ObserverConfig, ObserverSnapshot, ObserverSnapshotDetails,
    ProofSummary, SupernovaError, SupernovaStatus, TIMEOUT_CEILING_MS, TxObservation,
};
pub use decentraai_mx_supernova::{poll_once, track};

use crate::plan::{PlanError, TaskPlan};

/// Parse raw intel output (UNTRUSTED, closed schema) and report Supernova
/// activation for the given epoch/round under the pinned config.
///
/// `activation`: operator-pinned thresholds (`None` = unconfigured ⇒
/// inactive, fail-closed). There is deliberately NO timing heuristic here:
/// 600ms rounds are corroborating telemetry, never the verdict.
pub fn parse_and_check_supernova(
    raw: &str,
    epoch: u64,
    round: u64,
    activation: Option<ActivationConfig>,
) -> Result<SupernovaStatus, PlanError> {
    let _plan = TaskPlan::parse(raw)?;
    Ok(SupernovaStatus::observe(epoch, round, activation))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_plan() -> &'static str {
        r#"{
            "intent": "test_intent",
            "capabilities": [{"name": "ocr", "required": true}],
            "workflow": ["ocr"],
            "confidence": 0.9
        }"#
    }

    fn pinned() -> ActivationConfig {
        ActivationConfig {
            supernova_enable_epoch: 5,
            supernova_enable_round: 10,
        }
    }

    #[test]
    fn both_flags_decide() {
        let on = parse_and_check_supernova(raw_plan(), 5, 10, Some(pinned())).unwrap();
        assert!(on.active);
        // Epoch gate passed, round gate not (drain window) ⇒ inactive.
        let drain = parse_and_check_supernova(raw_plan(), 5, 9, Some(pinned())).unwrap();
        assert!(!drain.active);
        let off = parse_and_check_supernova(raw_plan(), 4, 999, Some(pinned())).unwrap();
        assert!(!off.active);
    }

    #[test]
    fn unknown_config_fails_closed() {
        let s = parse_and_check_supernova(raw_plan(), 5791, 32320452, None).unwrap();
        assert!(!s.active);
    }

    #[test]
    fn malformed_plan_rejected() {
        let bad = ActivationConfig {
            supernova_enable_epoch: 0,
            supernova_enable_round: 0,
        };
        assert!(parse_and_check_supernova("nope", 5, 10, Some(bad)).is_err());
    }
}
