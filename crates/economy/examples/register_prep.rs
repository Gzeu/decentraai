//! Generates the OPERATOR PREPARATION for the first DecentraAI Governor
//! registration on MX-8004 devnet — offline, keyless, submission-free.
//!
//! ```text
//! cargo run -p decentraai-economy --example register_prep -- \
//!     --name "DecentraGovernor" \
//!     --uri "ipfs://QmYourManifest" \
//!     --key 0x<64 hex chars of the agent Ed25519 public key> \
//!     --sender erd1<your wallet address> \
//!     [--gas 30000000]
//! ```
//!
//! Output: JSON with the exact data field + every tx field you must fill,
//! plus the verification commands to run AFTER confirmation.

use decentraai_economy::multiversx_tx::registration_preparation;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut name = String::from("DecentraGovernor");
    let mut uri = String::new();
    let mut key = String::new();
    let mut sender = String::new();
    let mut gas: u64 = 30_000_000;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--name" => name = args.next().unwrap_or_default(),
            "--uri" => uri = args.next().unwrap_or_default(),
            "--key" => key = args.next().unwrap_or_default(),
            "--sender" => sender = args.next().unwrap_or_default(),
            "--gas" => gas = args.next().unwrap_or_default().parse()?,
            other => return Err(format!("unknown arg {other}").into()),
        }
    }
    if uri.is_empty() || key.is_empty() || sender.is_empty() {
        return Err("required: --uri, --key, --sender".into());
    }

    let prep = registration_preparation(&name, &uri, &key, &sender, gas)?;
    println!("{}", serde_json::to_string_pretty(&prep)?);
    Ok(())
}
