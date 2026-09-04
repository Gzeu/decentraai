//! Operator tooling: generate a DEDICATED DecentraAI Settlement Operator
//! wallet (MultiversX testnet only).
//!
//! Usage (ONCE, on the operator laptop):
//! ```sh
//! cargo run -p decentraai-economy --example gen_operator_wallet -- /tmp/settlement-signer.hex
//! ```
//!
//! Prints ONLY the `erd1…` address to stdout. The 64-char hex seed is
//! written to the given path with `0600` permissions and NEVER printed.
//! Move the file to the VPS (`~/.decentraai/settlement-signer.hex`, 0600,
//! `DECENTRAAI_MX_SIGNER_HEX_FILE` pointing at it), fund the printed
//! address with testnet xEGLD, then delete every other copy.
//!
//! This wallet is SEPARATE from any personal wallet by design: it only ever
//! holds valueless testnet xEGLD and only signs settlement anchoring txs.

use decentraai_economy::signer::{Ed25519Signer, TransactionSigner as _, bech32_address};
use std::os::unix::fs::PermissionsExt as _;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: gen_operator_wallet <out-hex-path>");
    let mut rng = rand_core::OsRng;
    let mut seed = [0u8; 32];
    rand_core::RngCore::fill_bytes(&mut rng, &mut seed);
    let signer = Ed25519Signer::from_seed_bytes(&seed);
    let address = bech32_address(&signer.verifying_key_bytes());
    let hex_seed = hex::encode(seed);
    seed.fill(0);
    std::fs::write(&path, format!("{hex_seed}\n")).expect("seed file write failed");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("chmod 0600 failed");
    // ONLY the public address reaches stdout.
    println!("{address}");
}
