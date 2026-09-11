//! Supernova observer: poll + per-tx tracking (O1-step2b).
//!
//! Two layers:
//! - `poll_once` — network snapshot (config + status → [`ObserverSnapshot`]);
//!   the runtime crate calls it on a bounded interval and serves the last
//!   snapshot on `GET /v1/mx/status`.
//! - `track` — full lifecycle of ONE tx hash: observation → execution
//!   result → finality. Finality is proven, never presumed: for a
//!   same-shard success tx we resolve the block at the chain's
//!   `highest_final_nonce` and require its round to pass the tx's round
//!   AND a proof to be present. Every other outcome is an honest `pending`
//!   with a machine-stable `detail` reason — never `final`.

use crate::proxy::MxProxy;
use crate::types::{
    ChainStatus, ExecutionResult, FinalityState, NetworkConfig, ObserverSnapshotDetails,
    SupernovaError, SupernovaStatus, TxObservation,
};

/// Minimum poll interval (ms): 600ms rounds exist, but the Proxy is shared
/// infrastructure — hammering it every round is abuse, not observation.
pub const MIN_POLL_INTERVAL_MS: u64 = 1_000;
/// Maximum poll interval (ms): beyond an hour the snapshot is stale enough
/// to be a lie; the caller re-polls instead.
pub const MAX_POLL_INTERVAL_MS: u64 = 3_600_000;

/// Validated observer configuration (pure; construction fails closed).
#[derive(Debug, Clone)]
pub struct ObserverConfig {
    /// Proxy base URL (e.g. the testnet constant owned by `settlement_tx`).
    pub api_base: String,
    /// Shard polled for the periodic snapshot.
    pub shard: u32,
    /// Snapshot poll interval (ms).
    pub poll_interval_ms: u64,
    /// Per-request timeout (ms).
    pub timeout_ms: u64,
    /// Per-response byte cap.
    pub max_bytes: usize,
    /// Operator override for the Supernova enable epoch (`None` = decide
    /// from the live 600ms-round heuristic).
    pub enable_epoch: Option<u64>,
    /// Expected chain id (`Some("T")` on testnet lane). Mismatch fails
    /// closed — observing mainnet with testnet goggles is a bug, not data.
    pub expected_chain_id: Option<String>,
}

impl ObserverConfig {
    /// Validate bounds (base/timeout/bytes reuse the [`MxProxy`] rules).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_base: impl Into<String>,
        shard: u32,
        poll_interval_ms: u64,
        timeout_ms: u64,
        max_bytes: usize,
        enable_epoch: Option<u64>,
        expected_chain_id: Option<String>,
    ) -> Result<Self, SupernovaError> {
        let api_base = api_base.into();
        MxProxy::new(&api_base, timeout_ms, max_bytes).map(|_| ())?;
        if !(MIN_POLL_INTERVAL_MS..=MAX_POLL_INTERVAL_MS).contains(&poll_interval_ms) {
            return Err(SupernovaError::Malformed(format!(
                "poll_interval_ms out of bounds: {poll_interval_ms}"
            )));
        }
        Ok(Self {
            api_base,
            shard,
            poll_interval_ms,
            timeout_ms,
            max_bytes,
            enable_epoch,
            expected_chain_id,
        })
    }

    /// Client for this observer's base/bounds.
    pub fn proxy(&self) -> Result<MxProxy, SupernovaError> {
        MxProxy::new(&self.api_base, self.timeout_ms, self.max_bytes)
    }
}

/// One network snapshot: identity + progress + activation verdict.
#[derive(Debug, Clone)]
pub struct ObserverSnapshot {
    /// Network identity/timing.
    pub net: NetworkConfig,
    /// Chain progress on the polled shard.
    pub chain: ChainStatus,
    /// Activation verdict.
    pub status: SupernovaStatus,
    /// Wall-clock (ms) the snapshot was taken.
    pub at_ms: u64,
}

/// Poll config + status once. Chain-id mismatch fails closed.
pub async fn poll_once(
    proxy: &MxProxy,
    cfg: &ObserverConfig,
    now_ms: u64,
) -> Result<ObserverSnapshot, SupernovaError> {
    let net = proxy.network_config().await?;
    if let Some(expected) = &cfg.expected_chain_id {
        if &net.chain_id != expected {
            return Err(SupernovaError::ActivationUnknown(format!(
                "chain id mismatch: observed {} expected {expected}",
                net.chain_id
            )));
        }
    }
    let chain = proxy.chain_status(cfg.shard).await?;
    // Explicit operator config wins; otherwise the live-verified heuristic
    // (600ms rounds ⇔ Supernova timing) decides. Unknown is never active —
    // that rule lives in SupernovaStatus::observe for the explicit path.
    let mut status = SupernovaStatus::observe(chain.epoch, chain.round, cfg.enable_epoch);
    if cfg.enable_epoch.is_none() {
        status.active = net.looks_supernova();
        status.round_duration_ms = net.round_duration_ms;
    }
    Ok(ObserverSnapshot {
        net,
        chain,
        status,
        at_ms: now_ms,
    })
}

/// Full lifecycle of one tx hash under Supernova rules.
#[derive(Debug, Clone)]
pub struct MxTrack {
    /// The tracked hash.
    pub tx_hash: String,
    /// Chain observation (`None` = not indexed yet).
    pub tx: Option<TxObservation>,
    /// Execution result (`None` until the tx is observed).
    pub execution: Option<ExecutionResult>,
    /// Finality (pending until proven — see `detail`).
    pub finality: FinalityState,
    /// Machine-stable reason: `not-indexed` | `not-executed` |
    /// `cross-shard` | `not-finalized` | `final-block-pruned` | `final`.
    pub detail: &'static str,
}

/// Track one tx: observation → execution → finality resolution.
///
/// Same-shard success txs resolve finality through the finalized head
/// (`highest_final_nonce` → block round must pass the tx round → proof
/// must be present). Cross-shard and pruned histories stay honestly
/// pending — a `success` status is execution, not finality.
pub async fn track(
    proxy: &MxProxy,
    snap: &ObserverSnapshot,
    hash: &str,
) -> Result<MxTrack, SupernovaError> {
    let pending = |detail: &'static str| MxTrack {
        tx_hash: hash.to_string(),
        tx: None,
        execution: None,
        finality: FinalityState::pending(hash),
        detail,
    };
    let Some(tx) = proxy.transaction(hash).await? else {
        return Ok(pending("not-indexed"));
    };
    let execution = tx.execution_result(tx.miniblock_hash.clone(), snap.status.active);
    if !tx.is_success() {
        return Ok(MxTrack {
            tx_hash: hash.to_string(),
            tx: Some(tx),
            execution: Some(execution),
            finality: FinalityState::pending(hash),
            detail: "not-executed",
        });
    }
    if tx.sender_shard != tx.receiver_shard {
        return Ok(MxTrack {
            tx_hash: hash.to_string(),
            tx: Some(tx),
            execution: Some(execution),
            finality: FinalityState::pending(hash),
            detail: "cross-shard",
        });
    }
    // Same-shard success: prove finality via the finalized head.
    let chain = proxy.chain_status(tx.sender_shard).await?;
    let finals = proxy
        .blocks_by_nonce(tx.sender_shard, chain.highest_final_nonce)
        .await?;
    let Some(head) = finals.first() else {
        return Ok(MxTrack {
            tx_hash: hash.to_string(),
            tx: Some(tx),
            execution: Some(execution),
            finality: FinalityState::pending(hash),
            detail: "final-block-pruned",
        });
    };
    if head.round <= tx.round {
        return Ok(MxTrack {
            tx_hash: hash.to_string(),
            tx: Some(tx),
            execution: Some(execution),
            finality: FinalityState::pending(hash),
            detail: "not-finalized",
        });
    }
    let proof_seen = head.proof_seen();
    // Execution seen (status success) AND proof seen ⇒ final.
    let finality = FinalityState {
        tx_hash: hash.to_string(),
        proof_seen,
        observed_round: head.round,
        block_height: head.nonce,
        finality_reached: proof_seen,
    };
    Ok(MxTrack {
        tx_hash: hash.to_string(),
        tx: Some(tx),
        execution: Some(execution),
        finality,
        detail: if proof_seen { "final" } else { "not-finalized" },
    })
}

/// Details carrier so runtime endpoints can serialize snapshots without
/// depending on internals.
impl ObserverSnapshot {
    /// Machine-readable projection (telemetry-safe: counters and ids only,
    /// never payloads).
    pub fn details(&self) -> ObserverSnapshotDetails {
        ObserverSnapshotDetails {
            chain_id: self.net.chain_id.clone(),
            epoch: self.chain.epoch,
            round: self.chain.round,
            active: self.status.active,
            round_duration_ms: self.status.round_duration_ms,
            nonce: self.chain.nonce,
            highest_final_nonce: self.chain.highest_final_nonce,
            last_executed_nonce: self.chain.last_executed_nonce,
            execution_lead: self.chain.execution_lead(),
            at_ms: self.at_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn cfg() -> ObserverConfig {
        ObserverConfig::new("http://127.0.0.1:1", 2, 30_000, 10_000, 262_144, None, None).unwrap()
    }

    fn snap() -> ObserverSnapshot {
        ObserverSnapshot {
            net: NetworkConfig {
                chain_id: "T".to_string(),
                round_duration_ms: 600,
                software_version: "T2.0.8.0".to_string(),
                rounds_per_epoch: 12000,
            },
            chain: ChainStatus {
                shard: 2,
                epoch: 5790,
                round: 28253981,
                nonce: 100,
                highest_final_nonce: 90,
                last_executed_nonce: 95,
            },
            status: SupernovaStatus::observe(5790, 28253981, None),
            at_ms: 1,
        }
    }

    /// Multi-route mock: serves each request by path-substring match.
    /// Loops until the test runtime tears down (each test makes a bounded
    /// number of requests; the listener dies with the test).
    async fn serve_seq(routes: Vec<(String, u16, String)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = vec![0u8; 8192];
                let Ok(n) = sock.read(&mut buf).await else {
                    break;
                };
                if n == 0 {
                    break;
                }
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let (_, status, body) = routes
                    .iter()
                    .find(|(p, _, _)| req.contains(p))
                    .cloned()
                    .unwrap_or((String::new(), 500, r#"{"message":"no route"}"#.to_string()));
                let reason = if status == 200 { "OK" } else { "ERR" };
                let reply = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                if sock.write_all(reply.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        format!("http://{addr}")
    }

    const TX_OK: &str = r#"{"txHash":"aabb","nonce":3,"sender":"erd1s",
        "receiver":"erd1s","senderShard":2,"receiverShard":2,"status":"success",
        "gasLimit":50000,"gasUsed":50000,"timestampMs":1789140223200,
        "round":100,"epoch":5790,"miniBlockHash":"4c78"}"#;
    const STATUS: &str = r#"{"data":{"status":{"erd_epoch_number":5790,
        "erd_current_round":28254010,"erd_nonce":200,
        "erd_highest_final_nonce":150,"erd_last_executed_nonce":155,
        "erd_extra":0}},"code":"successful"}"#;
    const FINAL_BLOCK: &str = r#"[{"hash":"ff11","nonce":150,"round":120,
        "epoch":5790,"shard":2,"timestampMs":1789140223200,
        "lastExecutionResultHash":"0f2a","lastExecutionResultNonce":149,
        "proof":{"aggregatedSignature":"sig","headerHash":"ff11",
        "headerEpoch":5790,"headerNonce":150,"headerRound":120}}]"#;

    #[test]
    fn config_bounds_fail_closed() {
        assert!(ObserverConfig::new("http://x", 0, 999, 1000, 1024, None, None).is_err());
        assert!(ObserverConfig::new("http://x", 0, 30_000, 1000, 1024, None, None).is_ok());
    }

    #[tokio::test]
    async fn poll_heuristic_and_override() {
        // Heuristic path: 600ms rounds → active.
        let base = serve_seq(vec![
            ("/network/config".to_string(), 200,
                r#"{"data":{"config":{"erd_chain_id":"T","erd_round_duration":600,"erd_latest_tag_software_version":"T2.0.8.0","erd_rounds_per_epoch":12000}}}"#.to_string()),
            ("/network/status/2".to_string(), 200, STATUS.to_string()),
        ])
        .await;
        let c = cfg();
        let s = poll_once(&MxProxy::with_defaults(&base).unwrap(), &c, 7)
            .await
            .unwrap();
        assert!(s.status.active);
        assert_eq!(s.at_ms, 7);
        // Explicit enable_epoch in the future wins over the heuristic.
        let c2 = ObserverConfig::new(&base, 2, 30_000, 10_000, 262_144, Some(9999), None).unwrap();
        let s2 = poll_once(&MxProxy::with_defaults(&base).unwrap(), &c2, 7)
            .await
            .unwrap();
        assert!(!s2.status.active);
    }

    #[tokio::test]
    async fn poll_chain_mismatch_fails_closed() {
        let base = serve_seq(vec![("/network/config".to_string(), 200,
            r#"{"data":{"config":{"erd_chain_id":"1","erd_round_duration":6000,"erd_latest_tag_software_version":"M","erd_rounds_per_epoch":100}}}"#.to_string())])
        .await;
        let c = ObserverConfig::new(
            &base,
            2,
            30_000,
            10_000,
            262_144,
            None,
            Some("T".to_string()),
        )
        .unwrap();
        assert!(matches!(
            poll_once(&MxProxy::with_defaults(&base).unwrap(), &c, 1).await,
            Err(SupernovaError::ActivationUnknown(_))
        ));
    }

    #[tokio::test]
    async fn track_missing_tx_is_pending() {
        let base = serve_seq(vec![(
            "/transactions/".to_string(),
            404,
            r#"{}"#.to_string(),
        )])
        .await;
        let t = track(
            &MxProxy::with_defaults(&base).unwrap(),
            &snap(),
            &"ab".repeat(32),
        )
        .await
        .unwrap();
        assert_eq!(t.detail, "not-indexed");
        assert!(!t.finality.finality_reached);
        assert!(t.execution.is_none());
    }

    #[tokio::test]
    async fn track_same_shard_success_reaches_final() {
        // tx round 100; finalized head nonce 150 at round 120 with proof.
        let base = serve_seq(vec![
            ("/transactions/".to_string(), 200, TX_OK.to_string()),
            ("/network/status/2".to_string(), 200, STATUS.to_string()),
            (
                "/blocks?nonce=150".to_string(),
                200,
                FINAL_BLOCK.to_string(),
            ),
        ])
        .await;
        let t = track(
            &MxProxy::with_defaults(&base).unwrap(),
            &snap(),
            &"aa".repeat(32),
        )
        .await
        .unwrap();
        assert_eq!(t.detail, "final");
        assert!(t.finality.finality_reached);
        assert!(t.finality.proof_seen);
        assert!(t.execution.is_some_and(|e| e.is_valid()));
    }

    #[tokio::test]
    async fn track_lagging_head_stays_pending() {
        // Finalized head round 90 < tx round 100 → not finalized.
        let lagging = FINAL_BLOCK.replace("\"round\":120", "\"round\":90");
        let base = serve_seq(vec![
            ("/transactions/".to_string(), 200, TX_OK.to_string()),
            ("/network/status/2".to_string(), 200, STATUS.to_string()),
            ("/blocks?nonce=150".to_string(), 200, lagging),
        ])
        .await;
        let t = track(
            &MxProxy::with_defaults(&base).unwrap(),
            &snap(),
            &"aa".repeat(32),
        )
        .await
        .unwrap();
        assert_eq!(t.detail, "not-finalized");
        assert!(!t.finality.finality_reached);
    }

    #[tokio::test]
    async fn track_cross_shard_honest_pending() {
        let xshard = TX_OK.replace("\"receiverShard\":2", "\"receiverShard\":0");
        let base = serve_seq(vec![("/transactions/".to_string(), 200, xshard)]).await;
        let t = track(
            &MxProxy::with_defaults(&base).unwrap(),
            &snap(),
            &"aa".repeat(32),
        )
        .await
        .unwrap();
        assert_eq!(t.detail, "cross-shard");
    }
}
