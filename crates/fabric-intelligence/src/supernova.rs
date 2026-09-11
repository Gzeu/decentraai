//! Supernova integration facade (O1).
//!
//! The real implementation lives in `decentraai-mx-supernova`, whose shapes
//! were verified live against testnet (T2.0.8.0, 600ms rounds). This module
//! keeps the historical intelligence-layer entry point working: validate
//! untrusted plan output, then report Supernova activation for an
//! epoch/round pair. Nothing here touches the network, trust, or secrets.

pub use decentraai_mx_supernova::{
    BlockObservation, ChainStatus, DEFAULT_MAX_BYTES, DEFAULT_TIMEOUT_MS, ExecutionResult,
    FinalityState, MAX_BYTES_CEILING, MAX_POLL_INTERVAL_MS, MIN_POLL_INTERVAL_MS, MxProxy, MxTrack,
    NetworkConfig, ObserverConfig, ObserverSnapshot, ObserverSnapshotDetails, ProofSummary,
    SupernovaError, SupernovaStatus, TIMEOUT_CEILING_MS, TxObservation,
};
pub use decentraai_mx_supernova::{poll_once, track};

use crate::plan::{PlanError, TaskPlan};

/// Parse raw intel output (UNTRUSTED, closed schema) and report Supernova
/// activation for the given epoch/round.
///
/// `enable_epoch`: operator override (`None` = the live-verified 600ms
/// heuristic decides — matches the O1 observer rule). Unknown is never
/// active; malformed plans are rejections, not guesses.
pub fn parse_and_check_supernova(
    raw: &str,
    epoch: u64,
    round: u64,
    enable_epoch: Option<u64>,
) -> Result<SupernovaStatus, PlanError> {
    let _plan = TaskPlan::parse(raw)?;
    Ok(SupernovaStatus::observe(epoch, round, enable_epoch))
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

    #[test]
    fn explicit_enable_epoch_decides() {
        let on = parse_and_check_supernova(raw_plan(), 5, 10, Some(5)).unwrap();
        assert!(on.active);
        let off = parse_and_check_supernova(raw_plan(), 4, 10, Some(5)).unwrap();
        assert!(!off.active);
    }

    #[test]
    fn unknown_config_fails_closed() {
        let s = parse_and_check_supernova(raw_plan(), 5790, 28253981, None).unwrap();
        assert!(!s.active);
    }

    #[test]
    fn malformed_plan_rejected() {
        assert!(parse_and_check_supernova("nope", 5, 10, Some(0)).is_err());
    }
}
