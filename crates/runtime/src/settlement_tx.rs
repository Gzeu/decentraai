//! Automated testnet settlement submission — Phase 7 execution half.
//!
//! Takes a [`decentraai_economy::multiversx_tx::UnsignedTxIntent`], wraps its
//! `data_field()` as the `data` of a 0-value SELF-transfer (sender ==
//! receiver == operator wallet), signs the canonical envelope, broadcasts to
//! MultiversX testnet, and polls confirmation.
//!
//! # Why self-transfer (documented, not hidden)
//!
//! No MX-8004 registry contract address is VERIFIED yet
//! (`docs/MULTIVERSX_MX8004_WRITE_PATH.md`), so there is no legitimate
//! `receiver` for a contract call. The 0-value self-transfer anchors the
//! `submit_proof@…` payload immutably on-chain under the operator's
//! signature — same evidence bytes, same builder, verifiable in any
//! explorer. When registry addresses verify, only `receiver` changes.
//!
//! # Security
//!
//! The operator seed is read ONLY via
//! [`decentraai_economy::signer::load_signer_from_env`] (file 0600 or env).
//! Nothing here logs, serializes, or returns key material. Addresses,
//! nonces, tx hashes and statuses are public chain data — safe to return.

use decentraai_economy::multiversx_tx::TESTNET_NETWORK;
use decentraai_economy::signer::{
    Ed25519Signer, TransactionSigner as _, bech32_address, load_signer_from_env,
};

/// Testnet API base (same constant family as the economy crate).
pub const TESTNET_API_BASE: &str = "https://testnet-api.multiversx.com";
/// Testnet chain id (verified live against `/network/config`).
pub const TESTNET_CHAIN_ID: &str = "T";
/// Minimum gas price on testnet.
pub const SETTLEMENT_GAS_PRICE: u64 = 1_000_000_000;
/// Minimum gas limit for a simple transfer.
pub const SETTLEMENT_BASE_GAS: u64 = 50_000;
/// Extra gas per data byte (verified live against `/network/config`).
pub const SETTLEMENT_GAS_PER_BYTE: u64 = 1_500;
/// Transaction version we broadcast.
pub const SETTLEMENT_TX_VERSION: u64 = 1;

/// Local nonce tracker: the next nonce this node will use.
/// `None` until the first reservation after boot.
///
/// WHY: the chain account nonce only advances on INCLUSION. Two rapid
/// submits both read the same confirmed nonce and broadcast different txs
/// with it — one hash silently dies (observed live: proof-29 vs proof-30
/// collided on nonce 5). Reservations serialize through this tracker:
/// `max(chain_nonce, last_reserved + 1, observed pendings + 1)`.
static NEXT_NONCE: std::sync::OnceLock<tokio::sync::Mutex<Option<u64>>> =
    std::sync::OnceLock::new();

fn nonce_tracker() -> &'static tokio::sync::Mutex<Option<u64>> {
    NEXT_NONCE.get_or_init(|| tokio::sync::Mutex::new(None))
}

/// Pure reservation rule (unit-tested): never go backwards, never reuse.
pub fn next_nonce(chain_nonce: u64, last: Option<u64>) -> u64 {
    match last {
        Some(l) => chain_nonce.max(l.saturating_add(1)),
        None => chain_nonce,
    }
}

/// Feed an externally known used nonce (e.g. a `Submitted` proof's nonce
/// reloaded after a restart) so the tracker never reissues it.
pub async fn observe_nonce(used: u64) {
    let mut guard = nonce_tracker().lock().await;
    *guard = Some(guard.unwrap_or(0).max(used));
}

/// Reserve the next sendable nonce for `sender`. Serialized: concurrent
/// submits queue here instead of colliding on a stale chain read.
pub async fn reserve_nonce(api_base: &str, sender: &str) -> Result<u64, String> {
    // Held across the fetch on purpose: reservation + record is one atomic
    // step, so no two callers can take the same nonce. (tokio Mutex —
    // no executor blocking, just queuing.)
    let mut guard = nonce_tracker().lock().await;
    let chain = fetch_nonce(api_base, sender).await?;
    let n = next_nonce(chain, *guard);
    *guard = Some(n);
    Ok(n)
}

/// Gas for an anchoring tx carrying `data_len_bytes` of payload.
pub fn gas_limit_for_data(data_len_bytes: usize) -> u64 {
    SETTLEMENT_BASE_GAS + SETTLEMENT_GAS_PER_BYTE * data_len_bytes as u64
}

/// Operator wallet address derived from the injected signer.
/// Fails closed (`NotConfigured`, …) when no secret is injected.
pub fn operator_address() -> Result<String, String> {
    let signer = load_signer_from_env().map_err(|e| format!("operator signer unavailable: {e}"))?;
    Ok(bech32_address(&signer.verifying_key_bytes()))
}

/// Canonical signable JSON for a MultiversX transaction.
///
/// Field order per `docs.multiversx.com/developers/signing-transactions`:
/// `nonce, value, receiver, sender, gasPrice, gasLimit, data?, chainID,
/// version` — compact, no spaces; `data` base64, omitted when empty.
#[allow(clippy::too_many_arguments)]
pub fn signable_json(
    nonce: u64,
    value: &str,
    receiver: &str,
    sender: &str,
    gas_price: u64,
    gas_limit: u64,
    data_b64: Option<&str>,
    chain_id: &str,
    version: u64,
) -> String {
    match data_b64 {
        Some(d) => format!(
            "{{\"nonce\":{nonce},\"value\":\"{value}\",\"receiver\":\"{receiver}\",\"sender\":\"{sender}\",\"gasPrice\":{gas_price},\"gasLimit\":{gas_limit},\"data\":\"{d}\",\"chainID\":\"{chain_id}\",\"version\":{version}}}"
        ),
        None => format!(
            "{{\"nonce\":{nonce},\"value\":\"{value}\",\"receiver\":\"{receiver}\",\"sender\":\"{sender}\",\"gasPrice\":{gas_price},\"gasLimit\":{gas_limit},\"chainID\":\"{chain_id}\",\"version\":{version}}}"
        ),
    }
}

/// A prepared anchoring tx: unsigned envelope JSON + its sign bytes.
pub struct PreparedTx {
    /// Signable envelope (no `signature` field yet).
    pub unsigned_json: String,
    /// Raw bytes the operator signs.
    pub sign_bytes: Vec<u8>,
    /// Gas limit charged for this payload.
    pub gas_limit: u64,
}

/// Build the 0-value self-transfer envelope for an intent `data_field()`.
pub fn prepare_anchoring_tx(sender: &str, nonce: u64, tx_data: &str) -> PreparedTx {
    use base64::Engine as _;
    let data_b64 = base64::engine::general_purpose::STANDARD.encode(tx_data.as_bytes());
    let gas_limit = gas_limit_for_data(tx_data.len());
    let unsigned_json = signable_json(
        nonce,
        "0",
        sender,
        sender,
        SETTLEMENT_GAS_PRICE,
        gas_limit,
        Some(&data_b64),
        TESTNET_CHAIN_ID,
        SETTLEMENT_TX_VERSION,
    );
    let sign_bytes = unsigned_json.clone().into_bytes();
    PreparedTx {
        unsigned_json,
        sign_bytes,
        gas_limit,
    }
}

/// Sign prepared bytes with the injected operator signer (hex signature).
pub fn sign_prepared(prepared: &PreparedTx) -> Result<String, String> {
    let signer: Ed25519Signer =
        load_signer_from_env().map_err(|e| format!("operator signer unavailable: {e}"))?;
    signer
        .sign_hex(&prepared.sign_bytes)
        .map_err(|e| format!("signing failed: {e}"))
}

/// Attach the hex signature → ready-to-broadcast JSON.
pub fn attach_signature(prepared: &PreparedTx, signature_hex: &str) -> String {
    let mut out = prepared.unsigned_json.clone();
    out.pop(); // strip trailing `}`
    out.push_str(&format!(",\"signature\":\"{signature_hex}\"}}"));
    out
}

/// Fetch the current account nonce (new accounts → 0).
pub async fn fetch_nonce(api_base: &str, address: &str) -> Result<u64, String> {
    let url = format!("{api_base}/accounts/{address}");
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("testnet api unreachable: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(0);
    }
    if !resp.status().is_success() {
        return Err(format!("account lookup failed: http {}", resp.status()));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("account lookup decode failed: {e}"))?;
    v.get("nonce")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| "account lookup missing nonce".to_string())
}

/// Broadcast a signed tx JSON. Returns the chain `txHash`.
pub async fn broadcast_tx(api_base: &str, signed_json: &str) -> Result<String, String> {
    let body: serde_json::Value =
        serde_json::from_str(signed_json).map_err(|e| format!("signed tx encode failed: {e}"))?;
    let resp = reqwest::Client::new()
        .post(format!("{api_base}/transactions"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("broadcast failed: {e}"))?;
    let status = resp.status();
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("broadcast decode failed: {e}"))?;
    if !status.is_success() {
        return Err(format!(
            "broadcast rejected (http {status}): {}",
            v.get("message").and_then(|m| m.as_str()).unwrap_or("?")
        ));
    }
    v.get("txHash")
        .and_then(|h| h.as_str())
        .map(|h| h.to_string())
        .ok_or_else(|| "broadcast response missing txHash".to_string())
}

/// Poll chain status for a tx hash (`pending` / `success` / `fail` / …).
pub async fn tx_status(api_base: &str, tx_hash: &str) -> Result<String, String> {
    let url = format!("{api_base}/transactions/{tx_hash}");
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("status lookup failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("status lookup: http {}", resp.status()));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("status decode failed: {e}"))?;
    v.get("status")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "status response missing status".to_string())
}

/// Full automation for one proof: reserve nonce → sign → broadcast.
/// `sender` MUST be [`operator_address()`] output — the caller enforces the
/// operator identity, this function enforces the chain mechanics.
/// Returns `(tx_hash, sender, nonce)`. Recording + persistence stay with
/// the caller (the nonce travels with the record for restart recovery).
pub async fn auto_submit_proof(
    intent_data: &str,
    sender: &str,
    api_base: &str,
) -> Result<(String, String, u64), String> {
    let nonce = reserve_nonce(api_base, sender).await?;
    let prepared = prepare_anchoring_tx(sender, nonce, intent_data);
    let sig = sign_prepared(&prepared)?;
    let signed = attach_signature(&prepared, &sig);
    let tx_hash = broadcast_tx(api_base, &signed).await?;
    Ok((tx_hash, sender.to_string(), nonce))
}

/// Network tag this module submits to (mirrors the intent network).
pub fn network_tag() -> &'static str {
    TESTNET_NETWORK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signable_json_matches_docs_vector_with_data() {
        // Exact vector from docs.multiversx.com/developers/signing-transactions.
        let got = signable_json(
            7,
            "10000000000000000000",
            "erd1cux02zersde0l7hhklzhywcxk4u9n4py5tdxyx7vrvhnza2r4gmq4vw35r",
            "erd1l453hd0gt5gzdp7czpuall8ggt2dcv5zwmfdf3sd3lguxseux2fsmsgldz",
            1_000_000_000,
            70_000,
            Some("Zm9yIHRoZSBib29r"),
            "1",
            1,
        );
        assert_eq!(
            got,
            "{\"nonce\":7,\"value\":\"10000000000000000000\",\"receiver\":\"erd1cux02zersde0l7hhklzhywcxk4u9n4py5tdxyx7vrvhnza2r4gmq4vw35r\",\"sender\":\"erd1l453hd0gt5gzdp7czpuall8ggt2dcv5zwmfdf3sd3lguxseux2fsmsgldz\",\"gasPrice\":1000000000,\"gasLimit\":70000,\"data\":\"Zm9yIHRoZSBib29r\",\"chainID\":\"1\",\"version\":1}"
        );
    }

    #[test]
    fn signable_json_omits_empty_data() {
        let got = signable_json(8, "0", "erd1r", "erd1s", 1_000_000_000, 50_000, None, "T", 1);
        assert!(!got.contains("data"));
        assert!(got.starts_with("{\"nonce\":8,"));
        assert!(got.ends_with("\"version\":1}"));
    }

    #[test]
    fn gas_scales_with_payload() {
        assert_eq!(gas_limit_for_data(0), 50_000);
        assert_eq!(gas_limit_for_data(100), 50_000 + 150_000);
    }

    #[test]
    fn prepare_self_transfer_shape() {
        let p = prepare_anchoring_tx("erd1sender", 3, "submit_proof@aa@bb");
        assert_eq!(p.gas_limit, gas_limit_for_data("submit_proof@aa@bb".len()));
        assert!(p.unsigned_json.contains("\"receiver\":\"erd1sender\""));
        assert!(p.unsigned_json.contains("\"sender\":\"erd1sender\""));
        assert!(p.unsigned_json.contains("\"value\":\"0\""));
        assert!(p.unsigned_json.contains("\"chainID\":\"T\""));
        assert!(!p.unsigned_json.contains("signature"));
        assert_eq!(p.sign_bytes, p.unsigned_json.as_bytes());
    }

    #[test]
    fn attach_signature_appends_single_field() {
        let p = prepare_anchoring_tx("erd1sender", 3, "submit_proof@aa@bb");
        let full = attach_signature(&p, &"ab".repeat(64));
        let v: serde_json::Value = serde_json::from_str(&full).unwrap();
        assert_eq!(v["signature"], serde_json::Value::String("ab".repeat(64)));
        assert_eq!(v["nonce"], serde_json::Value::from(3));
    }

    #[test]
    fn nonce_reservation_never_reuses_or_regresses() {
        // Fresh boot: chain decides.
        assert_eq!(next_nonce(5, None), 5);
        assert_eq!(next_nonce(0, None), 0);
        // Rapid submits: chain is stale, tracker advances.
        assert_eq!(next_nonce(5, Some(5)), 6);
        assert_eq!(next_nonce(5, Some(8)), 9);
        // Chain moved past us (restart, external tx): follow the chain.
        assert_eq!(next_nonce(12, Some(8)), 12);
    }

    #[tokio::test]
    async fn observe_nonce_feeds_the_tracker() {
        observe_nonce(41).await;
        let guard = nonce_tracker().lock().await;
        assert!(guard.unwrap_or(0) >= 41);
        // Leave the tracker ahead — other tests use explicit values only.
    }

    #[test]
    fn operator_address_fails_closed_without_injection() {        // No DECENTRAAI_MX_SIGNER_* in test env → deterministic failure.
        // (If the operator env leaks into tests, skip instead of failing.)
        if std::env::var("DECENTRAAI_MX_SIGNER_HEX").is_err()
            && std::env::var("DECENTRAAI_MX_SIGNER_HEX_FILE").is_err()
        {
            assert!(operator_address().is_err());
        }
    }
}
