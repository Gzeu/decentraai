//! Live read-only demo: Supernova snapshot + tx track.
//!
//! Run: `cargo run -p decentraai-mx-supernova --example live_snapshot [TX_HASH] [API_BASE] [CHAIN_ID]`
//! Defaults: testnet probe tx + testnet API + `T`. Mainnet read-only:
//! pass `https://api.multiversx.com 1` (no tx hash → snapshot only).
//! Public data only. Never signs, never submits.

use decentraai_mx_supernova::{ActivationConfig, MxProxy, ObserverConfig};

#[tokio::main]
async fn main() {
    let api_base = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "https://testnet-api.multiversx.com".to_string());
    let chain_id = std::env::args().nth(3).unwrap_or_else(|| "T".to_string());
    let cfg = ObserverConfig::new(
        &api_base,
        2,
        30_000,
        15_000,
        262_144,
        Some(ActivationConfig::TEMPLATE_DEFAULT),
        Some(chain_id),
    )
    .expect("observer config builds");
    let proxy = MxProxy::with_defaults(api_base).expect("proxy builds");

    let snap = decentraai_mx_supernova::poll_once(&proxy, &cfg, 0)
        .await
        .expect("snapshot polls");
    println!(
        "SNAPSHOT {}",
        serde_json::to_string_pretty(&snap.details()).unwrap()
    );

    // Track only an explicitly passed hash (a testnet default on mainnet
    // would just 404 — honest but noise).
    if let Some(hash) = std::env::args().nth(1) {
        let t = decentraai_mx_supernova::track(&proxy, &snap, &hash)
            .await
            .expect("track works");
        println!(
            "TRACK detail={} finality_reached={} proof_seen={} execution_valid={}",
            t.detail,
            t.finality.finality_reached,
            t.finality.proof_seen,
            t.execution.as_ref().is_some_and(|e| e.is_valid())
        );
    }
}
