//! Reproducible tokenomics scenario runner.
//!
//! ```text
//! cargo run -p decentraai-economy --example simulate -- \
//!     configs/economy/example-params.json 10000 800
//! ```
//!
//! Prints one JSON SimulationReport to stdout: same params → same bytes.

use decentraai_economy::tokenomics::{TokenomicsParams, simulate};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let params_path = args
        .next()
        .unwrap_or_else(|| "configs/economy/example-params.json".into());
    let nodes: u32 = args.next().map(|a| a.parse()).transpose()?.unwrap_or(1_000);
    let avg_award: u64 = args.next().map(|a| a.parse()).transpose()?.unwrap_or(2_000);

    let params: TokenomicsParams = serde_json::from_str(&std::fs::read_to_string(&params_path)?)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&simulate(&params, nodes, avg_award)?)?
    );
    Ok(())
}
