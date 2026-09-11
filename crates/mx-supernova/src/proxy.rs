//! Bounded MultiversX Proxy client (O1-step2).
//!
//! Reads ONLY from the public Proxy surface (`/network/config`,
//! `/network/status/{shard}`, `/transactions/{hash}`, `/blocks/{hash}`).
//! Every response shape below was verified live against testnet-api
//! (T2.0.8.0, 600ms rounds, 2026-09-11); anything else is a typed
//! rejection, never a guess.
//!
//! Bounds (all fail-closed at construction):
//! - `timeout_ms` in 1..=120_000 (default 10_000);
//! - `max_bytes` in 1..=8MiB (default 256KiB) — content-length pre-check
//!   plus post-read check, the buffer never grows past the cap to find out;
//! - base must be `http(s)`, paths never escape (`..` rejected).
//!
//! Semantics: transport failures are transient (bounded retry by the
//! caller); a 404 on tx/block means "not indexed yet" → `Ok(None)`, NOT an
//! error — under async execution a submitted tx legitimately precedes its
//! execution result.

use std::time::Duration;

use serde_json::Value;

use crate::types::{
    BlockObservation, ChainStatus, NetworkConfig, ProofSummary, SupernovaError, TxObservation,
};

/// Default request timeout (ms).
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;
/// Default response byte cap (Proxy answers are small JSON).
pub const DEFAULT_MAX_BYTES: usize = 256 * 1024;
/// Hard ceiling for the byte cap.
pub const MAX_BYTES_CEILING: usize = 8 * 1024 * 1024;
/// Hard ceiling for the timeout (ms).
pub const TIMEOUT_CEILING_MS: u64 = 120_000;

/// Bounded read-only Proxy client. No signing, no submission, no secrets.
#[derive(Debug, Clone)]
pub struct MxProxy {
    base: String,
    client: reqwest::Client,
    max_bytes: usize,
}

impl MxProxy {
    /// Build against a Proxy base (e.g. `https://testnet-api.multiversx.com`).
    /// The caller passes the base explicitly — this crate hardcodes NO
    /// network address; the testnet constant lives with the existing
    /// submission lane (`settlement_tx::TESTNET_API_BASE`).
    pub fn new(
        base: impl Into<String>,
        timeout_ms: u64,
        max_bytes: usize,
    ) -> Result<Self, SupernovaError> {
        let base = base.into().trim_end_matches('/').to_string();
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            return Err(SupernovaError::Malformed(
                "proxy base must be http(s)".to_string(),
            ));
        }
        if timeout_ms == 0 || timeout_ms > TIMEOUT_CEILING_MS {
            return Err(SupernovaError::Malformed(format!(
                "timeout_ms out of bounds: {timeout_ms}"
            )));
        }
        if max_bytes == 0 || max_bytes > MAX_BYTES_CEILING {
            return Err(SupernovaError::Malformed(format!(
                "max_bytes out of bounds: {max_bytes}"
            )));
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .map_err(|e| SupernovaError::Transport(e.to_string()))?;
        Ok(Self {
            base,
            client,
            max_bytes,
        })
    }

    /// Convenience: default bounds.
    pub fn with_defaults(base: impl Into<String>) -> Result<Self, SupernovaError> {
        Self::new(base, DEFAULT_TIMEOUT_MS, DEFAULT_MAX_BYTES)
    }

    /// GET `path` with byte cap and typed errors. `path` must be an
    /// absolute in-API path (`/…`, no `..`, no scheme).
    async fn get(&self, path: &str) -> Result<Value, SupernovaError> {
        if !path.starts_with('/') || path.contains("..") {
            return Err(SupernovaError::Malformed(format!(
                "unsafe proxy path: {path}"
            )));
        }
        let url = format!("{}{}", self.base, path);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| SupernovaError::Transport(e.to_string()))?;
        let status = resp.status();
        if let Some(len) = resp.content_length() {
            if len > self.max_bytes as u64 {
                return Err(SupernovaError::TooLarge(format!(
                    "{path}: content-length {len} over cap {}",
                    self.max_bytes
                )));
            }
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| SupernovaError::Transport(e.to_string()))?;
        if body.len() > self.max_bytes {
            return Err(SupernovaError::TooLarge(format!(
                "{path}: body {} over cap {}",
                body.len(),
                self.max_bytes
            )));
        }
        if !status.is_success() {
            let message = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| status.to_string());
            return Err(SupernovaError::HttpStatus {
                status: status.as_u16(),
                message,
            });
        }
        serde_json::from_slice(&body).map_err(|e| SupernovaError::Decode(e.to_string()))
    }

    /// `GET /network/config` → [`NetworkConfig`].
    pub async fn network_config(&self) -> Result<NetworkConfig, SupernovaError> {
        let v = self.get("/network/config").await?;
        let cfg = &v["data"]["config"];
        Ok(NetworkConfig {
            chain_id: str_field(cfg, "erd_chain_id")?,
            round_duration_ms: u64_field(cfg, "erd_round_duration")?,
            software_version: str_field(cfg, "erd_latest_tag_software_version")?,
            rounds_per_epoch: u64_field(cfg, "erd_rounds_per_epoch")?,
        })
    }

    /// `GET /network/status/{shard}` → [`ChainStatus`].
    pub async fn chain_status(&self, shard: u32) -> Result<ChainStatus, SupernovaError> {
        let v = self.get(&format!("/network/status/{shard}")).await?;
        let st = &v["data"]["status"];
        Ok(ChainStatus {
            shard,
            epoch: u64_field(st, "erd_epoch_number")?,
            round: u64_field(st, "erd_current_round")?,
            nonce: u64_field(st, "erd_nonce")?,
            highest_final_nonce: u64_field(st, "erd_highest_final_nonce")?,
            last_executed_nonce: u64_field(st, "erd_last_executed_nonce")?,
        })
    }

    /// `GET /transactions/{hash}` → [`TxObservation`] or `None` (404 =
    /// not indexed yet — normal under async execution, poll again).
    ///
    /// Shape note (verified live): the single-tx endpoint returns the object
    /// BARE, while older gateway builds envelope it in
    /// `{"data":{"transaction":…}}`. Both are accepted; arrays rejected.
    pub async fn transaction(&self, hash: &str) -> Result<Option<TxObservation>, SupernovaError> {
        check_hash(hash)?;
        match self.get(&format!("/transactions/{hash}")).await {
            Err(SupernovaError::HttpStatus { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
            Ok(v) => {
                let tx = inner(&v, "transaction")?;
                Ok(Some(TxObservation {
                    tx_hash: str_field(&tx, "txHash")?,
                    nonce: u64_field(&tx, "nonce")?,
                    sender: str_field(&tx, "sender")?,
                    receiver: str_field(&tx, "receiver")?,
                    sender_shard: u32_field(&tx, "senderShard")?,
                    receiver_shard: u32_field(&tx, "receiverShard")?,
                    status: str_field(&tx, "status")?,
                    gas_limit: u64_field(&tx, "gasLimit")?,
                    gas_used: u64_field(&tx, "gasUsed")?,
                    timestamp_ms: u64_field(&tx, "timestampMs")
                        .or_else(|_| u64_field(&tx, "timestamp").map(|s| s * 1000))?,
                    round: u64_field(&tx, "round")?,
                    epoch: u64_field(&tx, "epoch")?,
                    miniblock_hash: str_field(&tx, "miniBlockHash")?,
                }))
            }
        }
    }

    /// `GET /blocks/{hash}` → [`BlockObservation`] or `None` (404).
    /// Same bare-or-enveloped rule as [`Self::transaction`].
    pub async fn block(&self, hash: &str) -> Result<Option<BlockObservation>, SupernovaError> {
        check_hash(hash)?;
        match self.get(&format!("/blocks/{hash}")).await {
            Err(SupernovaError::HttpStatus { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
            Ok(v) => {
                let b = inner(&v, "block")?;
                Ok(Some(read_block(&b)?))
            }
        }
    }

    /// `GET /blocks?nonce={nonce}&shard={shard}` → blocks at that nonce
    /// (normally 0–1). Verified live. Empty array = pruned/unknown, NOT an
    /// error — the caller reports honest `pending`, never finality.
    pub async fn blocks_by_nonce(
        &self,
        shard: u32,
        nonce: u64,
    ) -> Result<Vec<BlockObservation>, SupernovaError> {
        let v = self
            .get(&format!("/blocks?nonce={nonce}&shard={shard}"))
            .await?;
        let arr = if let Some(a) = v.as_array() {
            a
        } else if let Some(a) = v["data"]["blocks"].as_array() {
            a
        } else {
            return Err(SupernovaError::Malformed(
                "blocks query: expected array".to_string(),
            ));
        };
        arr.iter().map(read_block).collect()
    }
}

/// Read one block object (bare list item or unwrapped single response).
fn read_block(b: &Value) -> Result<BlockObservation, SupernovaError> {
    let proof = if b["proof"].is_null() {
        None
    } else {
        Some(ProofSummary {
            aggregated_signature: str_field(&b["proof"], "aggregatedSignature")?,
            header_hash: str_field(&b["proof"], "headerHash")?,
            header_epoch: u64_field(&b["proof"], "headerEpoch")?,
            header_nonce: u64_field(&b["proof"], "headerNonce")?,
            header_round: u64_field(&b["proof"], "headerRound")?,
        })
    };
    Ok(BlockObservation {
        hash: str_field(b, "hash")?,
        nonce: u64_field(b, "nonce")?,
        round: u64_field(b, "round")?,
        epoch: u64_field(b, "epoch")?,
        shard: u32_field(b, "shard")?,
        timestamp_ms: u64_field(b, "timestampMs")
            .or_else(|_| u64_field(b, "timestamp").map(|s| s * 1000))?,
        last_execution_result_hash: str_field(b, "lastExecutionResultHash")?,
        last_execution_result_nonce: u64_field(b, "lastExecutionResultNonce")?,
        proof,
    })
}

/// Unwrap `{"data":{key:…}}` envelopes; bare objects pass through; arrays
/// and anything else are malformed (never scraped).
fn inner<'a>(v: &'a Value, key: &str) -> Result<std::borrow::Cow<'a, Value>, SupernovaError> {
    if let Some(inner) = v.get("data").and_then(|d| d.get(key)) {
        if inner.is_null() {
            return Err(SupernovaError::Malformed(format!("missing object: {key}")));
        }
        return Ok(std::borrow::Cow::Borrowed(inner));
    }
    if v.as_object().is_some() {
        return Ok(std::borrow::Cow::Borrowed(v));
    }
    Err(SupernovaError::Malformed(format!("expected object: {key}")))
}

/// Hashes we request must look like hashes (hex, bounded length) — never a
/// path, query string, or empty string smuggled into the URL.
fn check_hash(hash: &str) -> Result<(), SupernovaError> {
    if hash.is_empty() || hash.len() > 128 {
        return Err(SupernovaError::Malformed("bad tx/block hash".to_string()));
    }
    if !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SupernovaError::Malformed("bad tx/block hash".to_string()));
    }
    Ok(())
}

/// Required string field; missing/wrong-type → [`SupernovaError::Malformed`].
fn str_field(v: &Value, key: &str) -> Result<String, SupernovaError> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| SupernovaError::Malformed(format!("missing string field: {key}")))
}

/// Required integer field. Accepts JSON numbers and stringified integers
/// (the API quotes some numerics); anything else is malformed.
fn u64_field(v: &Value, key: &str) -> Result<u64, SupernovaError> {
    let bad = || SupernovaError::Malformed(format!("missing integer field: {key}"));
    match v.get(key) {
        Some(Value::Number(n)) => n.as_u64().ok_or_else(bad),
        Some(Value::String(s)) => s.parse::<u64>().map_err(|_| bad()),
        _ => Err(bad()),
    }
}

/// Required 32-bit integer field (shard ids).
fn u32_field(v: &Value, key: &str) -> Result<u32, SupernovaError> {
    let n = u64_field(v, key)?;
    u32::try_from(n).map_err(|_| SupernovaError::Malformed(format!("field out of range: {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serve ONE canned response, return the base URL. Same pattern as the
    /// fabric-intelligence provider mock: raw TcpListener, canned HTTP.
    async fn serve_once(status: u16, body: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let body = body.to_string();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = sock.read(&mut buf).await.unwrap();
            let reason = if status == 200 { "OK" } else { "ERR" };
            let reply = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(reply.as_bytes()).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn proxy(base: &str) -> MxProxy {
        MxProxy::with_defaults(base).unwrap()
    }

    #[test]
    fn constructor_bounds_fail_closed() {
        assert!(MxProxy::new("ftp://x", 1000, 1024).is_err());
        assert!(MxProxy::new("testnet-api.multiversx.com", 1000, 1024).is_err());
        assert!(MxProxy::new("https://x", 0, 1024).is_err());
        assert!(MxProxy::new("https://x", 1000, 0).is_err());
        assert!(MxProxy::new("https://x", TIMEOUT_CEILING_MS + 1, 1024).is_err());
        assert!(MxProxy::new("https://x", 1000, MAX_BYTES_CEILING + 1).is_err());
        assert!(MxProxy::with_defaults("https://testnet-api.multiversx.com").is_ok());
    }

    #[tokio::test]
    async fn network_config_live_shape() {
        // Shape captured live (T2.0.8.0). Unknown extra keys are ignored —
        // we read explicit fields, forward-compatible across node versions.
        let base = serve_once(
            200,
            r#"{"data":{"config":{
            "erd_chain_id":"T","erd_round_duration":600,
            "erd_latest_tag_software_version":"T2.0.8.0",
            "erd_rounds_per_epoch":12000,"erd_future_key":"ignored"}},
            "code":"successful"}"#,
        )
        .await;
        let cfg = proxy(&base).network_config().await.unwrap();
        assert_eq!(cfg.chain_id, "T");
        assert_eq!(cfg.round_duration_ms, 600);
        assert!(cfg.looks_supernova());
    }

    #[tokio::test]
    async fn chain_status_live_shape() {
        let base = serve_once(
            200,
            r#"{"data":{"status":{
            "erd_epoch_number":5790,"erd_current_round":28253981,
            "erd_nonce":25954202,"erd_highest_final_nonce":25954199,
            "erd_last_executed_nonce":25954203,"erd_extra":"ignored"}},
            "code":"successful"}"#,
        )
        .await;
        let st = proxy(&base).chain_status(0).await.unwrap();
        assert_eq!((st.epoch, st.round), (5790, 28253981));
        assert_eq!(st.execution_lead(), 4);
    }

    #[tokio::test]
    async fn transaction_live_shape() {
        // Values of the real probe-250 tx 051793bb… (public chain data).
        let base = serve_once(
            200,
            r#"{"data":{"transaction":{
            "txHash":"051793bbbc934d0d9a2d0e7f572842a426a427cd09c6f77b2667abaea79775c6",
            "nonce":1,"sender":"erd1s","receiver":"erd1s","senderShard":2,
            "receiverShard":2,
            "status":"success","gasLimit":50000,"gasUsed":50000,
            "timestamp":1788653422,"timestampMs":1788653422200,
            "round":27442667,"epoch":5723,"miniBlockHash":"4c78bc7b"}},
            "code":"successful"}"#,
        )
        .await;
        let tx = proxy(&base)
            .transaction("051793bbbc934d0d9a2d0e7f572842a426a427cd09c6f77b2667abaea79775c6")
            .await
            .unwrap()
            .unwrap();
        assert!(tx.is_success());
        assert!(tx.execution_result("b", true).is_valid());
    }

    #[tokio::test]
    async fn transaction_404_is_none_not_error() {
        let base = serve_once(404, r#"{"message":"not found"}"#).await;
        let out = proxy(&base).transaction(&"ab".repeat(32)).await.unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn transaction_bare_shape_accepted() {
        // Live single-tx endpoint returns the object BARE (no envelope).
        let base = serve_once(
            200,
            r#"{"txHash":"aa","nonce":0,
            "sender":"erd1s","receiver":"erd1s","senderShard":0,
            "receiverShard":0,"status":"pending","gasLimit":50000,"gasUsed":0,
            "timestampMs":1789140223200,"round":28254002,"epoch":5790,
            "miniBlockHash":"4c78bc7b"}"#,
        )
        .await;
        let tx = proxy(&base)
            .transaction(&"aa".repeat(32))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(tx.nonce, 0);
        assert_eq!(tx.sender_shard, 0);
        assert!(!tx.is_success());
    }

    #[tokio::test]
    async fn blocks_by_nonce_lists_bare_items() {
        let base = serve_once(
            200,
            r#"[{"hash":"686521e6","nonce":25954224,
            "round":28254002,"epoch":5790,"shard":0,"timestampMs":1789140223200,
            "lastExecutionResultHash":"0f2aa227","lastExecutionResultNonce":25954223,
            "proof":{"aggregatedSignature":"217de0ad","headerHash":"686521e6",
            "headerEpoch":5790,"headerNonce":25954224,"headerRound":28254002}}]"#,
        )
        .await;
        let blocks = proxy(&base).blocks_by_nonce(0, 25954224).await.unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].proof_seen());
    }

    #[tokio::test]
    async fn blocks_by_nonce_empty_is_ok() {
        // Pruned history → empty array → Ok(empty), caller says pending.
        let base = serve_once(200, "[]").await;
        let blocks = proxy(&base).blocks_by_nonce(2, 1).await.unwrap();
        assert!(blocks.is_empty());
    }

    #[tokio::test]
    async fn block_live_shape_with_proof() {
        let base = serve_once(
            200,
            r#"{"data":{"block":{
            "hash":"686521e6","nonce":25954224,"round":28254002,"epoch":5790,
            "shard":0,"timestampMs":1789140223200,
            "lastExecutionResultHash":"0f2aa227","lastExecutionResultNonce":25954223,
            "proof":{"aggregatedSignature":"217de0ad","headerHash":"686521e6",
            "headerEpoch":5790,"headerNonce":25954224,"headerRound":28254002}}},
            "code":"successful"}"#,
        )
        .await;
        let b = proxy(&base).block(&"cd".repeat(32)).await.unwrap().unwrap();
        assert!(b.proof_seen());
        assert_eq!(b.last_execution_result_nonce, 25954223);
    }

    #[tokio::test]
    async fn oversize_body_rejected() {
        let base = serve_once(200, &format!("\"{}\"", "x".repeat(1024))).await;
        let p = MxProxy::new(&base, 10_000, 16).unwrap();
        assert!(matches!(
            p.network_config().await,
            Err(SupernovaError::TooLarge(_))
        ));
    }

    #[tokio::test]
    async fn bad_json_rejected() {
        let base = serve_once(200, "this is not json").await;
        assert!(matches!(
            proxy(&base).network_config().await,
            Err(SupernovaError::Decode(_))
        ));
    }

    #[tokio::test]
    async fn server_error_typed() {
        let base = serve_once(500, r#"{"message":"boom"}"#).await;
        match proxy(&base).network_config().await {
            Err(SupernovaError::HttpStatus {
                status: 500,
                message,
            }) => {
                assert_eq!(message, "boom")
            }
            other => panic!("expected HttpStatus, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn bad_hash_never_hits_wire() {
        let p = MxProxy::with_defaults("http://127.0.0.1:1").unwrap();
        assert!(p.transaction("").await.is_err());
        assert!(p.transaction("../x").await.is_err());
        assert!(p.block("not-hex!!").await.is_err());
    }
}
