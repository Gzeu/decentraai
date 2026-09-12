//! A2 Supernova-aware probe: existing lane + new observer, one shot.
//!
//! Usage:
//!   a2_probe --dry-run            # default: prepare envelope, NO sign/broadcast
//!   a2_probe --live               # reserve nonce → sign → broadcast → track to final
//!
//! Env (live only): `DECENTRAAI_MX_SIGNER_HEX_FILE` (0600 seed file) or
//! `DECENTRAAI_MX_SIGNER_HEX`. Values are never printed, logged, or
//! returned — output is tx hash + statuses (public chain data) only.
//!
//! Testnet only: chain id `T` is asserted from the live operator address
//! flow; anything else fails closed inside `settlement_tx`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tokio::main]
async fn main() {
    let live = std::env::args().any(|a| a == "--live");
    let api_base = decentraai_runtime::settlement_tx::TESTNET_API_BASE;

    // Gate 1: operator signer present (fail-closed without it).
    let sender = match decentraai_runtime::settlement_tx::operator_address() {
        Ok(s) => s,
        Err(e) => {
            println!("GATE signer: unavailable ({e}) — live submit impossible, read-only only");
            if live {
                std::process::exit(2);
            }
            // Dry-run without signer: still demonstrate the observer half.
            demonstrate_observer(api_base).await;
            return;
        }
    };
    println!("OPERATOR {sender}");

    // Gate 2: live chain identity must be testnet `T`.
    let proxy = decentraai_mx_supernova::MxProxy::with_defaults(api_base).expect("proxy builds");
    let net = proxy.network_config().await.expect("network config reads");
    assert_eq!(net.chain_id, "T", "refusing non-testnet chain");
    println!(
        "CHAIN {} {} rounds_ms={}",
        net.chain_id, net.software_version, net.round_duration_ms
    );

    let intent = format!("supernova-a2-probe@{}", now_ms());
    if !live {
        // Dry-run: reserve (read-only fetch + process-local slot) + prepare
        // envelope. No signing, no broadcast.
        let nonce = decentraai_runtime::settlement_tx::reserve_nonce(api_base, &sender)
            .await
            .expect("nonce reads");
        let prepared =
            decentraai_runtime::settlement_tx::prepare_anchoring_tx(&sender, nonce, &intent);
        println!(
            "DRYRUN nonce={nonce} gas={} bytes={}",
            prepared.gas_limit,
            prepared.sign_bytes.len()
        );
        println!("DRYRUN ok — no signature created, nothing broadcast");
        return;
    }

    // Live: the existing lane (reserve → sign → broadcast), then observe.
    let t0 = now_ms();
    let (tx_hash, _, nonce) =
        decentraai_runtime::settlement_tx::auto_submit_proof(&intent, &sender, api_base)
            .await
            .expect("broadcast works");
    println!("BROADCAST hash={tx_hash} nonce={nonce}");
    println!("EXPLORER https://testnet-explorer.multiversx.com/transactions/{tx_hash}");

    // Supernova-aware tracking to finality (bounded: 60 polls × 2s).
    // Pinned activation: v2.0.8 template thresholds (epoch 2, round 440).
    let activation = Some(decentraai_mx_supernova::ActivationConfig::TEMPLATE_DEFAULT);
    let cfg = decentraai_mx_supernova::ObserverConfig::new(
        api_base,
        2,
        30_000,
        15_000,
        262_144,
        activation,
        Some("T".to_string()),
    )
    .expect("observer config builds");
    let snap = decentraai_mx_supernova::poll_once(&proxy, &cfg, now_ms())
        .await
        .expect("snapshot polls");
    for attempt in 1..=60u32 {
        tokio::time::sleep(Duration::from_millis(2000)).await;
        let t = decentraai_mx_supernova::track(&proxy, &snap, &tx_hash)
            .await
            .expect("track reads");
        println!(
            "POLL {attempt} detail={} elapsed_ms={}",
            t.detail,
            now_ms() - t0
        );
        if t.detail == "final" {
            println!(
                "FINAL hash={tx_hash} proof_seen={} execution_valid={}",
                t.finality.proof_seen,
                t.execution.as_ref().is_some_and(|e| e.is_valid())
            );
            return;
        }
    }
    println!("TIMEOUT hash={tx_hash} — track later via /v1/mx/track");
    std::process::exit(3);
}

/// Signer absent: still prove the read-only half end-to-end (snapshot +
/// track of the historical probe tx).
async fn demonstrate_observer(api_base: &str) {
    let proxy = decentraai_mx_supernova::MxProxy::with_defaults(api_base).expect("proxy builds");
    // Pinned activation: v2.0.8 template thresholds (epoch 2, round 440).
    let activation = Some(decentraai_mx_supernova::ActivationConfig::TEMPLATE_DEFAULT);
    let cfg = decentraai_mx_supernova::ObserverConfig::new(
        api_base,
        2,
        30_000,
        15_000,
        262_144,
        activation,
        Some("T".to_string()),
    )
    .expect("observer config builds");
    let snap = decentraai_mx_supernova::poll_once(&proxy, &cfg, now_ms())
        .await
        .expect("snapshot polls");
    println!(
        "READONLY active={} lead={}",
        snap.status.active,
        snap.chain.execution_lead()
    );
    let t = decentraai_mx_supernova::track(
        &proxy,
        &snap,
        "051793bbbc934d0d9a2d0e7f572842a426a427cd09c6f77b2667abaea79775c6",
    )
    .await
    .expect("track reads");
    println!(
        "READONLY probe detail={} final={}",
        t.detail, t.finality.finality_reached
    );
}
