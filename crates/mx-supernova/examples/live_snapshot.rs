//! Live read-only demo: Supernova snapshot + tx track on testnet.
//!
//! Run: `cargo run -p decentraai-mx-supernova --example live_snapshot [TX_HASH]`
//! Public data only. Never signs, never submits. Used for O1/A2 verification.

use decentraai_mx_supernova::{MxProxy, ObserverConfig};

#[tokio::main]
async fn main() {
    let api_base = "https://testnet-api.multiversx.com";
    let cfg = ObserverConfig::new(
        api_base,
        2,
        30_000,
        15_000,
        262_144,
        None,
        Some("T".to_string()),
    )
    .expect("testnet observer config builds");
    let proxy = MxProxy::with_defaults(api_base).expect("proxy builds");

    let snap = decentraai_mx_supernova::poll_once(&proxy, &cfg, 0)
        .await
        .expect("snapshot polls");
    println!(
        "SNAPSHOT {}",
        serde_json::to_string_pretty(&snap.details()).unwrap()
    );

    let hash = std::env::args().nth(1).unwrap_or_else(|| {
        "051793bbbc934d0d9a2d0e7f572842a426a427cd09c6f77b2667abaea79775c6".to_string()
    });
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
