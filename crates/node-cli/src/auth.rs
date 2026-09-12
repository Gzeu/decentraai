//! Agent self-onboarding: wallet challenge → sign → session → `dca_` key,
//! fully non-interactive so an AI agent can authenticate itself from the CLI.
//!
//! The agent OWNS a throwaway Ed25519 identity (0600 seed file, same pattern
//! as `wallet new`): no user funds, no custody, no delegation. The seed
//! never leaves the machine; the token is stored 0600 and never printed.
//!
//! - `auth new [--node URL]`: create/reuse identity, login, mint key, store.
//! - `auth whoami [--node]`: prove the stored key (GET /v1/models).
//! - `auth revoke [--node]`: fresh login, revoke key, scrub stored token.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ed25519_dalek::Signer as _;

/// Agent API-identity commands (self-onboarding, non-interactive).
#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Create (or reuse) the agent identity, login, mint and store a `dca_` key.
    New(AuthNewArgs),
    /// Prove the stored key against the node (read-only).
    Whoami(AuthWhoamiArgs),
    /// Revoke the stored key (fresh login) and scrub it locally.
    Revoke(AuthRevokeArgs),
}

#[derive(Debug, Args)]
pub struct AuthNewArgs {
    /// Node base URL (no trailing slash).
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    pub node: String,
    /// Agent seed file (created 0600 when missing; reused otherwise).
    #[arg(long)]
    pub secret_file: Option<PathBuf>,
    /// Destination key file (0600 JSON; overwritten on rotation).
    #[arg(long)]
    pub key_file: Option<PathBuf>,
    /// Challenge purpose recorded + signed (default agent-login).
    #[arg(long, default_value = "agent-login")]
    pub purpose: String,
}

#[derive(Debug, Args)]
pub struct AuthWhoamiArgs {
    /// Node base URL (no trailing slash).
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    pub node: String,
    /// Stored key file from `auth new`.
    #[arg(long)]
    pub key_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct AuthRevokeArgs {
    /// Node base URL (no trailing slash).
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    pub node: String,
    /// Agent seed file (fresh login proves ownership for revoke).
    #[arg(long)]
    pub secret_file: Option<PathBuf>,
    /// Stored key file to scrub.
    #[arg(long)]
    pub key_file: Option<PathBuf>,
}

pub async fn auth_command(command: AuthCommand) -> Result<()> {
    let client = reqwest::Client::new();
    match command {
        AuthCommand::New(args) => {
            let secret_path = args.secret_file.unwrap_or_else(default_seed_path);
            let seed = load_or_create_seed(&secret_path)?;
            let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
            let address =
                decentraai_runtime::wallet_auth::encode_wallet_address(&signing.verifying_key().to_bytes())
                    .map_err(|e| anyhow::anyhow!("address encode: {e}"))?;
            let session = login(&client, &args.node, &address, &args.purpose, &signing).await?;
            let issued: serde_json::Value = client
                .post(format!("{}/v1/auth/wallet/key", args.node))
                .json(&serde_json::json!({"session_token": session}))
                .send()
                .await
                .context("POST /v1/auth/wallet/key")?
                .json()
                .await
                .context("parsing issuance")?;
            if issued["ok"] != true {
                anyhow::bail!(
                    "issuance refused: {} (key_id: {})",
                    issued["error"].as_str().unwrap_or("unknown"),
                    issued["key_id"].as_str().unwrap_or("-")
                );
            }
            let key_path = args.key_file.unwrap_or_else(default_key_path);
            save_key_0600(
                &key_path,
                &address,
                issued["account"].as_str().unwrap_or(""),
                issued["key_id"].as_str().unwrap_or(""),
                issued["token"].as_str().unwrap_or(""),
                &args.node,
            )?;
            println!("agent:    {address}");
            println!("key_id:   {}", issued["key_id"].as_str().unwrap_or("-"));
            println!(
                "scopes:   {}",
                issued["scopes"]
                    .as_array()
                    .map(|a| a
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(","))
                    .unwrap_or_default()
            );
            println!("stored:   {} (0600, token never printed)", key_path.display());
            Ok(())
        }
        AuthCommand::Whoami(args) => {
            let key_path = args.key_file.unwrap_or_else(default_key_path);
            let stored = load_key(&key_path)?;
            let client = reqwest::Client::new();
            let r = client
                .get(format!("{}/v1/models", args.node))
                .header("Authorization", format!("Bearer {}", stored.token))
                .send()
                .await
                .context("GET /v1/models")?;
            if r.status() != 200 {
                anyhow::bail!("key rejected (HTTP {}) — rotate via `auth new`", r.status());
            }
            println!("ok:       {} ({}) @ {}", stored.address, stored.key_id, args.node);
            println!("note:     spendable quota is not self-visible yet (consumer calls are allow-listed); ceiling 1000, starter 100 on first issue");
            Ok(())
        }
        AuthCommand::Revoke(args) => {
            let secret_path = args.secret_file.unwrap_or_else(default_seed_path);
            let seed = load_seed(&secret_path)?;
            let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
            let address =
                decentraai_runtime::wallet_auth::encode_wallet_address(&signing.verifying_key().to_bytes())
                    .map_err(|e| anyhow::anyhow!("address encode: {e}"))?;
            let session = login(&client, &args.node, &address, "agent-login", &signing).await?;
            let r = client
                .delete(format!("{}/v1/auth/wallet/key", args.node))
                .json(&serde_json::json!({"session_token": session}))
                .send()
                .await
                .context("DELETE /v1/auth/wallet/key")?;
            if r.status() == 404 {
                println!("nothing to revoke (no active key)");
                return Ok(());
            }
            if r.status() != 200 {
                anyhow::bail!("revoke refused (HTTP {})", r.status());
            }
            let body: serde_json::Value = r.json().await.context("parsing revoke")?;
            let key_path = args.key_file.unwrap_or_else(default_key_path);
            scrub_key_token(&key_path)?;
            println!("revoked:  {}", body["key_id"].as_str().unwrap_or("-"));
            println!("scrubbed: {}", key_path.display());
            Ok(())
        }
    }
}

/// Challenge → sign → verify → live session token.
async fn login(
    client: &reqwest::Client,
    node: &str,
    address: &str,
    purpose: &str,
    signing: &ed25519_dalek::SigningKey,
) -> Result<String> {
    let chal: serde_json::Value = client
        .post(format!("{node}/v1/auth/wallet/challenge"))
        .json(&serde_json::json!({"wallet_address": address, "purpose": purpose}))
        .send()
        .await
        .context("POST /v1/auth/wallet/challenge")?
        .json()
        .await
        .context("parsing challenge")?;
    let message = chal["message"].as_str().context("challenge has no message")?;
    if chal["wallet_address"].as_str() != Some(address) {
        anyhow::bail!("challenge address mismatch (node canonicalized differently)");
    }
    let sig = signing.sign(message.as_bytes());
    let login: serde_json::Value = client
        .post(format!("{node}/v1/auth/wallet/verify"))
        .json(&serde_json::json!({
            "wallet_address": address,
            "challenge_id": chal["challenge_id"].as_str().unwrap_or(""),
            "signature": hex::encode(sig.to_bytes()),
        }))
        .send()
        .await
        .context("POST /v1/auth/wallet/verify")?
        .json()
        .await
        .context("parsing login")?;
    login["session_token"]
        .as_str()
        .map(|s| s.to_string())
        .context("login returned no session_token")
}

fn default_seed_path() -> PathBuf {
    home_dir().join(".decentraai/agent-auth.seed")
}

fn default_key_path() -> PathBuf {
    home_dir().join(".decentraai/agent-auth.json")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct StoredKey {
    address: String,
    account: String,
    key_id: String,
    token: String,
    node: String,
}

/// Load seed or create it 0600 (reuse = idempotent agent login).
fn load_or_create_seed(path: &Path) -> Result<[u8; 32]> {
    if path.exists() {
        return load_seed(path);
    }
    let mut seed = [0u8; 32];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut seed);
    write_0600(path, &hex_encode(&seed))?;
    Ok(seed)
}

fn load_seed(path: &Path) -> Result<[u8; 32]> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading seed file {}", path.display()))?;
    let raw = hex::decode(content.trim()).context("seed file is not hex")?;
    if raw.len() != 32 {
        anyhow::bail!("seed file must hold 32 bytes");
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&raw);
    Ok(seed)
}

fn save_key_0600(
    path: &Path,
    address: &str,
    account: &str,
    key_id: &str,
    token: &str,
    node: &str,
) -> Result<()> {
    let stored = StoredKey {
        address: address.to_string(),
        account: account.to_string(),
        key_id: key_id.to_string(),
        token: token.to_string(),
        node: node.to_string(),
    };
    let body = serde_json::to_string_pretty(&stored)?;
    write_0600(path, &body)
}

fn load_key(path: &Path) -> Result<StoredKey> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("reading key file {} (run `auth new` first)", path.display()))?;
    serde_json::from_str(&body).context("key file corrupt")
}

/// Replace the stored token with the empty string (post-revoke hygiene).
fn scrub_key_token(path: &Path) -> Result<()> {    if !path.exists() {
        return Ok(());
    }
    let mut stored = load_key(path)?;
    stored.token.clear();
    let body = serde_json::to_string_pretty(&stored)?;
    write_0600(path, &body)
}

/// Atomic-ish 0600 write from the first byte (no chmod window).
fn write_0600(path: &Path, content: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    use std::io::Write as _;
    file.write_all(content.as_bytes())?;
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

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "auth-test-{}-{}.json",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn keystore_round_trips_0600_and_scrubs() {
        let path = tmp("roundtrip");
        save_key_0600(&path, "erd1test", "acct", "ck-1", "dca_secret", "http://x").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(mode & 0o777, 0o600);
        let back = load_key(&path).unwrap();
        assert_eq!(back.token, "dca_secret");
        assert_eq!(back.key_id, "ck-1");
        scrub_key_token(&path).unwrap();
        assert!(load_key(&path).unwrap().token.is_empty());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn corrupt_keystore_fails_closed() {
        let path = tmp("corrupt");
        std::fs::write(&path, "{nope").unwrap();
        assert!(load_key(&path).is_err());
        assert!(load_seed(&path).is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn seed_create_is_0600_and_stable() {        let path = tmp("seed");
        std::fs::remove_file(&path).unwrap_or(());
        let s1 = load_or_create_seed(&path).unwrap();
        let s2 = load_or_create_seed(&path).unwrap();
        assert_eq!(s1, s2, "reuse must be idempotent");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_file(&path).unwrap();
    }
}
