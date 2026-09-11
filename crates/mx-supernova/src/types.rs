//! Pure Supernova observation types.
//!
//! Everything here is deterministic and I/O-free: construction from explicit
//! arguments, validation as pure predicates, deserialization of OUR OWN
//! schemas only. Chain JSON enters through the `Raw*` readers in
//! `proxy.rs` (O1 step 2), never here.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors produced while turning chain data into observations.
///
/// Malformed chain answers are rejections with a typed reason — the caller
/// (observer poll loop) maps these to bounded retry, never to reputation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SupernovaError {
    /// Required field missing or wrong shape in a chain response.
    #[error("malformed chain response: {0}")]
    Malformed(String),
    /// Activation config unknown or contradictory — fail closed.
    #[error("supernova activation unknown: {0}")]
    ActivationUnknown(String),
    /// Transport failure (DNS, connect, timeout). Transient: bounded retry,
    /// never reputation — network errors never punish anyone.
    #[error("transport: {0}")]
    Transport(String),
    /// Non-2xx HTTP status with optional chain message.
    #[error("http {status}: {message}")]
    HttpStatus { status: u16, message: String },
    /// Body exceeded the byte cap. Never grows the buffer to find out more.
    #[error("response too large: {0}")]
    TooLarge(String),
    /// 2xx body that does not decode as JSON.
    #[error("decode: {0}")]
    Decode(String),
}

/// Whether Supernova async execution is active on the observed network.
///
/// Derived ONLY from explicit inputs (epoch/round observed on chain +
/// enable configuration). Unknown config → `active == false` (fail-closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupernovaStatus {
    /// Epoch observed on chain.
    pub epoch: u64,
    /// Round observed on chain.
    pub round: u64,
    /// Enable epoch from `/network/config` (`None` = not advertised).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_epoch: Option<u64>,
    /// Round duration the network runs: 6000ms pre-Supernova, 600ms after.
    pub round_duration_ms: u64,
    /// True only when activation is PROVEN from the inputs.
    pub active: bool,
}

impl SupernovaStatus {
    /// Pre-Supernova round duration (ms), per `vm/common.go` test vectors.
    pub const LEGACY_ROUND_MS: u64 = 6000;
    /// Supernova round duration (ms).
    pub const SUPERNOVA_ROUND_MS: u64 = 600;

    /// Build from explicit inputs. `enable_epoch == None` (config not
    /// advertised) yields `active == false`: unknown never means active.
    pub fn observe(epoch: u64, round: u64, enable_epoch: Option<u64>) -> Self {
        let active = matches!(enable_epoch, Some(e) if epoch >= e);
        Self {
            epoch,
            round,
            enable_epoch,
            round_duration_ms: if active {
                Self::SUPERNOVA_ROUND_MS
            } else {
                Self::LEGACY_ROUND_MS
            },
            active,
        }
    }
}

/// Pointer to one MultiversX transaction under observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MxTxRef {
    /// Chain transaction hash (hex).
    pub tx_hash: String,
    /// Sender bech32 address (public chain data).
    pub sender: String,
    /// Chain id the tx was issued on (`T` on testnet — anything else is
    /// rejected at the Actor boundary, never here).
    pub chain_id: String,
}

impl MxTxRef {
    /// Structural check: non-empty hash/sender/chain. Network membership
    /// (testnet-only) is enforced by the Actor lane, not here.
    pub fn is_well_formed(&self) -> bool {
        !self.tx_hash.is_empty() && !self.sender.is_empty() && !self.chain_id.is_empty()
    }
}

/// Execution result as tracked post-Supernova (indexer #366/#372/#373/#374:
/// last-execution nonce+hash on block, shard id, header gas data, timestamp).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResult {
    /// Execution nonce.
    pub nonce: u64,
    /// Execution result hash (hex).
    pub hash: String,
    /// Gas consumed.
    pub gas_used: u64,
    /// Gas limit of the executed tx/block.
    pub gas_limit: u64,
    /// Unix timestamp (ms) carried post-activation.
    pub timestamp_ms: u64,
    /// Shard where execution happened (0 is valid).
    pub shard_id: u32,
    /// Block hash anchoring this result.
    pub block_hash: String,
    /// Whether the result was observed after Supernova activation.
    pub post_supernova: bool,
}

impl ExecutionResult {
    /// Structural validation (never consensus):
    /// hash present, gas within limit, timestamp set.
    /// `nonce == 0` (first tx) and `shard_id == 0` (shard 0) are valid.
    pub fn is_valid(&self) -> bool {
        !self.hash.is_empty()
            && self.gas_used <= self.gas_limit
            && self.timestamp_ms > 0
            && !self.block_hash.is_empty()
    }

    /// Parse OUR OWN canonical JSON (closed schema). Chain JSON enters via
    /// `proxy.rs` readers, never here.
    pub fn parse_canonical(raw: &str) -> Result<Self, SupernovaError> {
        serde_json::from_str(raw).map_err(|e| SupernovaError::Malformed(e.to_string()))
    }
}

/// Finality as OBSERVED (never presumed).
///
/// Supernova converges at roughly 10 blocks/sec on healthy networks, but
/// the observer measures (`proof_seen`, `observed_round`) instead of
/// extrapolating `blocks * 100ms` — a partitioned network converges slower
/// and the type must say so honestly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalityState {
    /// The observed tx (or block) this state refers to.
    pub tx_hash: String,
    /// Whether a proof was seen before accepting finality (#7810).
    pub proof_seen: bool,
    /// Round in which finality was observed.
    pub observed_round: u64,
    /// Block height at which finality was observed.
    pub block_height: u64,
    /// True only when both execution result AND proof converged.
    pub finality_reached: bool,
}

impl FinalityState {
    /// Pending state: nothing observed yet. Honest default — finality is
    /// never assumed from submission alone (async execution decouples them).
    pub fn pending(tx_hash: impl Into<String>) -> Self {
        Self {
            tx_hash: tx_hash.into(),
            proof_seen: false,
            observed_round: 0,
            block_height: 0,
            finality_reached: false,
        }
    }

    /// Converge: execution result seen AND proof seen before broadcast.
    /// Either alone is insufficient.
    pub fn converge(
        mut self,
        execution_seen: bool,
        observed_round: u64,
        block_height: u64,
    ) -> Self {
        self.observed_round = observed_round;
        self.block_height = block_height;
        self.finality_reached = execution_seen && self.proof_seen;
        self
    }
}

/// Network identity and timing from `GET /network/config`.
///
/// Field names verified live against testnet-api (T2.0.8.0, 2026-09-11):
/// `data.config.erd_chain_id`, `erd_round_duration`, etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    /// Chain id (`T` on testnet — verified live).
    pub chain_id: String,
    /// Round duration in ms (600 on Supernova-active testnet — verified live).
    pub round_duration_ms: u64,
    /// Node software tag (e.g. `T2.0.8.0` — verified live).
    pub software_version: String,
    /// Rounds per epoch (12000 — verified live).
    pub rounds_per_epoch: u64,
}

impl NetworkConfig {
    /// Supernova heuristic, live-verified: 600ms rounds mean the network
    /// runs the Supernova timing. Combined with an operator `enable_epoch`
    /// override in [`SupernovaStatus::observe`] (explicit config wins).
    pub fn looks_supernova(&self) -> bool {
        self.round_duration_ms == SupernovaStatus::SUPERNOVA_ROUND_MS
    }
}

/// Chain progress from `GET /network/status/{shard}`.
///
/// Field names verified live: `data.status.erd_epoch_number`,
/// `erd_current_round`, `erd_nonce`, `erd_highest_final_nonce`,
/// `erd_last_executed_nonce`, `erd_proposed_nonce`. The proposed/executed/
/// final nonce spread IS the async-execution decoupling, observed directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainStatus {
    /// Shard this status was read for.
    pub shard: u32,
    /// Current epoch.
    pub epoch: u64,
    /// Current round.
    pub round: u64,
    /// Head nonce (proposed).
    pub nonce: u64,
    /// Highest nonce with finality proof.
    pub highest_final_nonce: u64,
    /// Highest executed nonce (may lead finality under async execution).
    pub last_executed_nonce: u64,
}

impl ChainStatus {
    /// How far execution leads finality. Positive under Supernova async
    /// execution; NOT an error, just the decoupling made visible.
    pub fn execution_lead(&self) -> u64 {
        self.last_executed_nonce
            .saturating_sub(self.highest_final_nonce)
    }
}

/// One transaction as returned by `GET /transactions/{hash}`.
///
/// Field names verified live (probe-250 `051793bb…`, 2026-09-11):
/// `data.transaction.{txHash,nonce,sender,receiver,senderShard,
/// gasLimit,gasPrice,gasUsed,status,timestamp,timestampMs,round,epoch,
/// miniBlockHash}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TxObservation {
    /// Transaction hash (hex).
    pub tx_hash: String,
    /// Account nonce (0 is valid — first tx).
    pub nonce: u64,
    /// Sender bech32.
    pub sender: String,
    /// Receiver bech32.
    pub receiver: String,
    /// Sender shard (0 is valid).
    pub sender_shard: u32,
    /// Receiver shard (0 is valid). Same-shard settles fastest; cross-shard
    /// finality needs the destination shard too (track() says so honestly).
    pub receiver_shard: u32,
    /// Chain status string (`success`, `pending`, `fail`, …).
    pub status: String,
    /// Gas limit / used (verified live: 50000/50000 on the probe).
    pub gas_limit: u64,
    /// Gas used.
    pub gas_used: u64,
    /// Execution timestamp (ms).
    pub timestamp_ms: u64,
    /// Round / epoch of inclusion.
    pub round: u64,
    /// Epoch of inclusion.
    pub epoch: u64,
    /// Miniblock anchoring the tx.
    pub miniblock_hash: String,
}

impl TxObservation {
    /// Terminal success on chain (any other status = not yet usable).
    pub fn is_success(&self) -> bool {
        self.status == "success"
    }

    /// Lift into an [`ExecutionResult`] anchored at the given block hash.
    /// Gas/timestamp come from the tx itself (verified live fields).
    pub fn execution_result(
        &self,
        block_hash: impl Into<String>,
        post_supernova: bool,
    ) -> ExecutionResult {
        ExecutionResult {
            nonce: self.nonce,
            hash: self.tx_hash.clone(),
            gas_used: self.gas_used,
            gas_limit: self.gas_limit,
            timestamp_ms: self.timestamp_ms,
            shard_id: self.sender_shard,
            block_hash: block_hash.into(),
            post_supernova,
        }
    }
}

/// Proof summary attached to a block (`#7810`: proof before broadcast).
///
/// Field names verified live: `proof.{aggregatedSignature,headerHash,
/// headerEpoch,headerNonce,headerRound}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofSummary {
    /// Aggregated BLS signature (hex).
    pub aggregated_signature: String,
    /// Header hash the proof certifies.
    pub header_hash: String,
    /// Header epoch / nonce / round.
    pub header_epoch: u64,
    /// Header nonce.
    pub header_nonce: u64,
    /// Header round.
    pub header_round: u64,
}

/// One block as returned by `GET /blocks/{hash}`.
///
/// Field names verified live (shard-0 head, 2026-09-11):
/// `hash,nonce,round,epoch,shard,timestampMs,lastExecutionResultHash,
/// lastExecutionResultNonce,proof{…}` — exactly indexer #366 + #7810.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockObservation {
    /// Block hash (hex).
    pub hash: String,
    /// Block nonce.
    pub nonce: u64,
    /// Block round.
    pub round: u64,
    /// Block epoch.
    pub epoch: u64,
    /// Block shard (0 is valid).
    pub shard: u32,
    /// Block timestamp (ms).
    pub timestamp_ms: u64,
    /// Last execution result hash on this block (indexer #366).
    pub last_execution_result_hash: String,
    /// Last execution result nonce on this block (indexer #366).
    pub last_execution_result_nonce: u64,
    /// Finality proof (`None` = not yet proven — honest, not an error).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<ProofSummary>,
}

impl BlockObservation {
    /// Whether the block carries a finality proof.
    pub fn proof_seen(&self) -> bool {
        self.proof
            .as_ref()
            .is_some_and(|p| !p.aggregated_signature.is_empty())
    }
}

/// Telemetry-safe snapshot projection (counters and ids only, never
/// payloads) for dashboards and read-only endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserverSnapshotDetails {
    /// Observed chain id.
    pub chain_id: String,
    /// Observed epoch / round.
    pub epoch: u64,
    /// Observed round.
    pub round: u64,
    /// Supernova active verdict.
    pub active: bool,
    /// Round duration the verdict implies (ms).
    pub round_duration_ms: u64,
    /// Head / finalized / executed nonces + visible async lead.
    pub nonce: u64,
    /// Highest finalized nonce.
    pub highest_final_nonce: u64,
    /// Highest executed nonce.
    pub last_executed_nonce: u64,
    /// `last_executed - highest_final` (saturating).
    pub execution_lead: u64,
    /// Wall-clock (ms) of the snapshot.
    pub at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_config_fails_closed() {
        let s = SupernovaStatus::observe(10, 5, None);
        assert!(!s.active);
        assert_eq!(s.round_duration_ms, SupernovaStatus::LEGACY_ROUND_MS);
    }

    #[test]
    fn activation_boundary_is_inclusive() {
        assert!(SupernovaStatus::observe(5, 1, Some(5)).active);
        assert!(!SupernovaStatus::observe(4, 999, Some(5)).active);
        let s = SupernovaStatus::observe(6, 1, Some(5));
        assert!(s.active);
        assert_eq!(s.round_duration_ms, SupernovaStatus::SUPERNOVA_ROUND_MS);
    }

    #[test]
    fn shard_zero_and_nonce_zero_are_valid() {
        let r = ExecutionResult {
            nonce: 0,
            hash: "ab".to_string(),
            gas_used: 50_000,
            gas_limit: 50_000,
            timestamp_ms: 1_700_000_000,
            shard_id: 0,
            block_hash: "cd".to_string(),
            post_supernova: true,
        };
        assert!(r.is_valid());
    }

    #[test]
    fn malformed_results_rejected() {
        let base = ExecutionResult {
            nonce: 1,
            hash: "ab".to_string(),
            gas_used: 50_000,
            gas_limit: 50_000,
            timestamp_ms: 1_700_000_000,
            shard_id: 1,
            block_hash: "cd".to_string(),
            post_supernova: true,
        };
        assert!(
            !ExecutionResult {
                hash: String::new(),
                ..base.clone()
            }
            .is_valid()
        );
        assert!(
            !ExecutionResult {
                gas_used: 50_001,
                ..base.clone()
            }
            .is_valid(),
            "over-execution is malformed"
        );
        assert!(
            !ExecutionResult {
                timestamp_ms: 0,
                ..base.clone()
            }
            .is_valid()
        );
        assert!(
            !ExecutionResult {
                block_hash: String::new(),
                ..base
            }
            .is_valid()
        );
    }

    #[test]
    fn canonical_schema_rejects_unknown_fields() {
        let raw = r#"{"nonce":1,"hash":"ab","gas_used":1,"gas_limit":2,
            "timestamp_ms":3,"shard_id":1,"block_hash":"cd",
            "post_supernova":true,"evil":"x"}"#;
        assert!(matches!(
            ExecutionResult::parse_canonical(raw),
            Err(SupernovaError::Malformed(_))
        ));
    }

    #[test]
    fn canonical_roundtrip() {
        let r = ExecutionResult {
            nonce: 7,
            hash: "ab".to_string(),
            gas_used: 1,
            gas_limit: 2,
            timestamp_ms: 3,
            shard_id: 2,
            block_hash: "cd".to_string(),
            post_supernova: false,
        };
        let back: ExecutionResult =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn finality_needs_both_legs() {
        let tx = "f7825a44";
        let pending = FinalityState::pending(tx);
        assert!(!pending.finality_reached);
        // Execution seen but no proof → not final.
        let no_proof = pending.clone().converge(true, 10, 500);
        assert!(!no_proof.finality_reached);
        // Proof seen + execution seen → final.
        let mut with_proof = pending;
        with_proof.proof_seen = true;
        let done = with_proof.converge(true, 10, 500);
        assert!(done.finality_reached);
        assert_eq!(done.observed_round, 10);
        assert_eq!(done.block_height, 500);
    }

    #[test]
    fn tx_ref_well_formedness() {
        let good = MxTxRef {
            tx_hash: "051793bb".to_string(),
            sender: "erd1…".to_string(),
            chain_id: "T".to_string(),
        };
        assert!(good.is_well_formed());
        assert!(
            !MxTxRef {
                tx_hash: String::new(),
                ..good.clone()
            }
            .is_well_formed()
        );
    }

    #[test]
    fn live_config_shape_marks_supernova() {
        // Values captured live from testnet-api (T2.0.8.0, 600ms rounds).
        let cfg = NetworkConfig {
            chain_id: "T".to_string(),
            round_duration_ms: 600,
            software_version: "T2.0.8.0".to_string(),
            rounds_per_epoch: 12000,
        };
        assert!(cfg.looks_supernova());
        assert!(
            !NetworkConfig {
                round_duration_ms: 6000,
                ..cfg
            }
            .looks_supernova()
        );
    }

    #[test]
    fn async_gap_is_measured_not_feared() {
        // Nonces captured live: proposed 25954203, executed 25954203,
        // final 25954199 — execution leads finality by 4.
        let st = ChainStatus {
            shard: 0,
            epoch: 5790,
            round: 28253981,
            nonce: 25954202,
            highest_final_nonce: 25954199,
            last_executed_nonce: 25954203,
        };
        assert_eq!(st.execution_lead(), 4);
        assert_eq!(
            ChainStatus {
                last_executed_nonce: 100,
                highest_final_nonce: 100,
                ..st
            }
            .execution_lead(),
            0
        );
    }

    #[test]
    fn probe_tx_lifts_to_valid_execution_result() {
        // Shape + values of the live probe-250 tx 051793bb… (self-transfer).
        let tx = TxObservation {
            tx_hash: "051793bbbc934d0d9a2d0e7f572842a426a427cd09c6f77b2667abaea79775c6".to_string(),
            nonce: 1,
            sender: "erd18ju7q8fluce5ns4y8tze7k4au4yrj47csfkph9lgpyqg0rnzculqxl2j0x".to_string(),
            receiver: "erd18ju7q8fluce5ns4y8tze7k4au4yrj47csfkph9lgpyqg0rnzculqxl2j0x".to_string(),
            sender_shard: 2,
            receiver_shard: 2,
            status: "success".to_string(),
            gas_limit: 50_000,
            gas_used: 50_000,
            timestamp_ms: 1788653422200,
            round: 27442667,
            epoch: 5723,
            miniblock_hash: "4c78bc7b".to_string(),
        };
        assert!(tx.is_success());
        let r = tx.execution_result("block-hash", true);
        assert!(r.is_valid());
        assert_eq!(r.nonce, 1);
        assert_eq!(r.shard_id, 2);
        assert!(
            !TxObservation {
                status: "pending".to_string(),
                ..tx
            }
            .is_success()
        );
    }

    #[test]
    fn live_block_shape_carries_execution_and_proof() {
        // Shape captured live from a shard-0 head block.
        let b = BlockObservation {
            hash: "686521e6".to_string(),
            nonce: 25954224,
            round: 28254002,
            epoch: 5790,
            shard: 0,
            timestamp_ms: 1789140223200,
            last_execution_result_hash: "0f2aa227".to_string(),
            last_execution_result_nonce: 25954223,
            proof: Some(ProofSummary {
                aggregated_signature: "217de0ad".to_string(),
                header_hash: "686521e6".to_string(),
                header_epoch: 5790,
                header_nonce: 25954224,
                header_round: 28254002,
            }),
        };
        assert!(b.proof_seen());
        assert!(!BlockObservation { proof: None, ..b }.proof_seen());
    }
}
