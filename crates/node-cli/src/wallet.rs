//! Operator wallet ops for the bounded Testnet lane (read-only + keygen).
//!
//! Two subcommands, both spend-free by construction:
//!
//! - `wallet new --secret-file <path>`: generate a FRESH 32-byte Ed25519
//!   seed from the OS CSPRNG, write it 0600 (refuse to overwrite unless
//!   `--force`), print ONLY the derived bech32 address. The seed is never
//!   printed, logged, or returned.
//! - `wallet address [--secret-file <path>]`: print the bech32 address of the
//!   configured signer plus its source, read-only (never the secret value).
//!
//! Preflight companion to `experiment autonomous-cycle --enable-live-testnet`:
//! generate → verify address → fund via faucet/drip → single bounded cycle.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

/// Testnet wallet operator commands (no spend path anywhere in this module).
#[derive(Debug, Subcommand)]
pub enum WalletCommand {
    /// Generate a fresh settlement seed (0600 file) + print its address.
    New(WalletNewArgs),
    /// Print the bech32 address of the configured signer (read-only).
    Address(WalletAddressArgs),
}

#[derive(Debug, Args)]
pub struct WalletNewArgs {
    /// Destination seed file (created 0600; never overwritten w/o --force).
    #[arg(long)]
    pub secret_file: PathBuf,
    /// Overwrite an existing seed file (default: refuse).
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct WalletAddressArgs {
    /// Override the signer seed file instead of env resolution.
    #[arg(long)]
    pub secret_file: Option<PathBuf>,
}

pub fn wallet_command(command: WalletCommand) -> Result<()> {
    use decentraai_economy::signer::TransactionSigner as _;
    match command {
        WalletCommand::New(args) => {
            if args.secret_file.exists() && !args.force {
                anyhow::bail!(
                    "refusing to overwrite existing {} (pass --force to rotate)",
                    args.secret_file.display()
                );
            }
            let seed = random_seed()?;
            write_seed_0600(&args.secret_file, &seed)?;
            let signer = decentraai_economy::signer::Ed25519Signer::from_seed_bytes(&seed);
            let address = decentraai_economy::signer::bech32_address(&signer.verifying_key_bytes());
            println!("seed file:  {}", args.secret_file.display());
            println!("address:    {address}");
            println!("chain:      MultiversX testnet only (this key must NEVER fund mainnet)");
            Ok(())
        }
        WalletCommand::Address(args) => {
            let (address, source) = resolve_address(args.secret_file.as_deref())?;
            println!("address:    {address}");
            println!("source:     {source}");
            Ok(())
        }
    }
}

/// Resolve the signer exactly like the live lane, returning address + source.
pub fn resolve_address(secret_file: Option<&Path>) -> Result<(String, String)> {
    use decentraai_economy::signer::TransactionSigner as _;
    if let Some(path) = secret_file {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading secret file {}", path.display()))?;
        let signer = decentraai_economy::signer::Ed25519Signer::from_seed_hex(content.trim())
            .map_err(|e| anyhow::anyhow!("secret file invalid: {e}"))?;
        let address = decentraai_economy::signer::bech32_address(&signer.verifying_key_bytes());
        return Ok((address, format!("file {}", path.display())));
    }
    let signer = decentraai_economy::signer::load_signer_from_env()
        .map_err(|e| anyhow::anyhow!("signer unavailable: {e}"))?;
    let address = decentraai_economy::signer::bech32_address(&signer.verifying_key_bytes());
    let source = if std::env::var(decentraai_economy::signer::SIGNER_HEX_FILE_ENV).is_ok() {
        decentraai_economy::signer::SIGNER_HEX_FILE_ENV.to_string()
    } else {
        decentraai_economy::signer::SIGNER_HEX_ENV.to_string()
    };
    Ok((address, format!("env {source}")))
}

/// 32 bytes from the OS CSPRNG (`/dev/urandom`, Unix).
fn random_seed() -> Result<[u8; 32]> {
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .context("opening /dev/urandom")?
        .read_exact(&mut seed)
        .context("reading 32 seed bytes")?;
    Ok(seed)
}

/// Write hex seed with 0600 from the first byte (no chmod window).
fn write_seed_0600(path: &Path, seed: &[u8; 32]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let hex_seed = hex_encode(seed);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating secret file {}", path.display()))?;
    use std::io::Write as _;
    file.write_all(hex_seed.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "wallet-test-{}-{}.hex",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn new_seed_round_trips_to_stable_erd1_address() {
        let path = tmp_path("roundtrip");
        wallet_command(WalletCommand::New(WalletNewArgs {
            secret_file: path.clone(),
            force: false,
        }))
        .unwrap();
        let (a1, _) = resolve_address(Some(&path)).unwrap();
        let (a2, _) = resolve_address(Some(&path)).unwrap();
        assert_eq!(a1, a2);
        assert!(a1.starts_with("erd1"), "{a1}");
        assert_eq!(a1.len(), 62);
        // 0600 enforced at creation.
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn new_refuses_to_overwrite_without_force() {
        let path = tmp_path("no-overwrite");
        std::fs::write(&path, "x").unwrap();
        let err = wallet_command(WalletCommand::New(WalletNewArgs {
            secret_file: path.clone(),
            force: false,
        }))
        .unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn garbage_seed_fails_closed() {
        let path = tmp_path("garbage");
        std::fs::write(&path, "not-hex-at-all").unwrap();
        assert!(resolve_address(Some(&path)).is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn two_wallets_differ() {
        let p1 = tmp_path("a");
        let p2 = tmp_path("b");
        wallet_command(WalletCommand::New(WalletNewArgs {
            secret_file: p1.clone(),
            force: false,
        }))
        .unwrap();
        wallet_command(WalletCommand::New(WalletNewArgs {
            secret_file: p2.clone(),
            force: false,
        }))
        .unwrap();
        let (a1, _) = resolve_address(Some(&p1)).unwrap();
        let (a2, _) = resolve_address(Some(&p2)).unwrap();
        assert_ne!(a1, a2, "CSPRNG must not repeat");
        std::fs::remove_file(&p1).unwrap();
        std::fs::remove_file(&p2).unwrap();
    }
}
