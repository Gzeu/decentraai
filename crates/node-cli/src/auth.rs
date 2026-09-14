//! Agent self-onboarding: wallet challenge → sign → session → `dca_` key,
//! fully non-interactive so an AI agent can authenticate itself from the CLI.
//!
//! The agent OWNS a throwaway Ed25519 identity (0600 seed file, same pattern
//! as `wallet new`): no user funds, no custody, no delegation. The seed
//! never leaves the machine; the token is stored 0600 and never printed.
//!
//! - `auth new [--node URL]`: create/reuse identity, login, mint key, store.
//! - `auth login --pem FILE [--node URL] [--native] [--mint]`: login with an
//!   EXISTING MultiversX key (PEM: 32 B seed, 64 B seed+pub, or legacy
//!   hex-string). Challenge flow by default, official native-auth token flow
//!   with `--native`. Session printed; with `--mint` the `dca_` key is stored
//!   0600 (never printed — same rule as `auth new`).
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
    /// Login with an existing MultiversX PEM key (challenge or native-auth).
    Login(AuthLoginArgs),
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

/// Login with a pre-existing MultiversX key (test wallets, operator-held).
/// The PEM never leaves the machine: only signatures travel. The session is
/// printed (short-lived); a minted key is stored 0600, never printed.
#[derive(Debug, Args)]
pub struct AuthLoginArgs {
    /// Path to the `.pem` file (TEST wallets only — never the main seed).
    pub pem: PathBuf,
    /// Node base URL (no trailing slash).
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    pub node: String,
    /// Chain intent for the purpose + blockhash lane.
    #[arg(long, default_value = "testnet", value_parser = ["testnet", "mainnet"])]
    pub network: String,
    /// Use the official native-auth token flow instead of challenge flow.
    #[arg(long, default_value_t = false)]
    pub native: bool,
    /// Also mint a `dca_` key for the wallet agent (stored 0600).
    #[arg(long, default_value_t = false)]
    pub mint: bool,
    /// Destination key file (0600 JSON) for `--mint`.
    #[arg(long)]
    pub key_file: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct AuthWhoamiArgs {    /// Node base URL (no trailing slash).
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
        AuthCommand::Login(args) => {
            use base64::Engine as _;
            let seed = load_pem_seed(&args.pem)?;
            let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
            let address =
                decentraai_runtime::wallet_auth::encode_wallet_address(&signing.verifying_key().to_bytes())
                    .map_err(|e| anyhow::anyhow!("address encode: {e}"))?;
            let purpose = format!(
                "onboard:{}",
                if args.network == "mainnet" { "mainnet" } else { "testnet" }
            );
            let session = if args.native {
                // Official token flow: blockhash → token → sign the Elrond
                // digest (keccak256(prefix + len + msg)) — byte-identical to
                // what real wallet software signs (same verifier server-side).
                let bh: serde_json::Value = client
                    .get(format!(
                        "{}/v1/auth/wallet/blockhash?network={}",
                        args.node, args.network
                    ))
                    .send()
                    .await
                    .context("GET /v1/auth/wallet/blockhash")?
                    .json()
                    .await
                    .context("parsing blockhash")?;
                let hash = bh["hash"].as_str().context("blockhash has no hash")?;
                let origin = node_host(&args.node);
                let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
                let token = format!(
                    "{}.{}.{}.{}",
                    b64.encode(&origin),
                    hash,
                    86400,
                    b64.encode("{}")
                );
                let sig = sign_elrond_digest(&signing, format!("{address}{token}").as_bytes());
                let login: serde_json::Value = client
                    .post(format!("{}/v1/auth/wallet/native-auth", args.node))
                    .json(&serde_json::json!({
                        "wallet_address": address,
                        "token": token,
                        "signature": hex::encode(sig.to_bytes()),
                    }))
                    .send()
                    .await
                    .context("POST /v1/auth/wallet/native-auth")?
                    .json()
                    .await
                    .context("parsing native login")?;
                login["session_token"]
                    .as_str()
                    .map(|s| s.to_string())
                    .context("native login returned no session_token")?
            } else {
                login(&client, &args.node, &address, &purpose, &signing).await?
            };
            println!("agent:    {address}");
            println!("session:  {session}");
            if args.mint {
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
                println!("key_id:   {}", issued["key_id"].as_str().unwrap_or("-"));
                println!("stored:   {} (0600, token never printed)", key_path.display());
            }
            Ok(())
        }
        AuthCommand::Whoami(args) => {            let key_path = args.key_file.unwrap_or_else(default_key_path);
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

/// Load the 32-byte seed from a MultiversX `.pem` file (all known layouts).
/// Layouts: 32 B raw seed, 64 B seed+pubkey (mxpy standard), or the legacy
/// form (base64 of a 128-char hex string). Fail-closed on anything else.
/// The seed never leaves this function except into the in-memory signer.
fn load_pem_seed(path: &Path) -> Result<[u8; 32]> {
    use base64::Engine as _;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading PEM {}", path.display()))?;
    let body: String = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains("BEGIN") && !l.contains("END"))
        .collect();
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&body)
        .context("PEM body is not base64")?;
    let seed_slice: &[u8] = match raw.len() {
        32 => &raw,
        64 => &raw[..32],
        _ => {
            // Legacy: the base64 payload is itself hex text.
            let hex_text = std::str::from_utf8(&raw).context("PEM payload is not text")?;
            let key64 = hex::decode(hex_text.trim()).context("PEM payload is not hex")?;
            if key64.len() != 64 {
                anyhow::bail!("PEM key must decode to 32 or 64 bytes (got {})", key64.len());
            }
            // Borrow-check: re-slice from an owned buffer via a static leak
            // is overkill — copy through a fixed array instead.
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&key64[..32]);
            return Ok(seed);
        }
    };
    let mut seed = [0u8; 32];
    seed.copy_from_slice(seed_slice);
    Ok(seed)
}

/// Sign the Elrond message digest (what the fabric verifies for wallets):
/// `ed25519(signing, keccak256("\x17Elrond Signed Message:\n" + len + msg))`.
fn sign_elrond_digest(
    signing: &ed25519_dalek::SigningKey,
    msg: &[u8],
) -> ed25519_dalek::Signature {
    use sha3::Digest as _;
    let mut h = sha3::Keccak256::new();
    h.update(b"\x17Elrond Signed Message:\n");
    h.update(msg.len().to_string().as_bytes());
    h.update(msg);
    let digest: [u8; 32] = h.finalize().into();
    signing.sign(&digest)
}

/// Bare hostname of the node URL (native-auth origin convention: the
/// official JS SDK defaults to `location.hostname`).
fn node_host(node_url: &str) -> String {
    let s = node_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let s = s.split('/').next().unwrap_or(s);
    s.split(':').next().unwrap_or(s).to_string()
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

    fn write_pem(path: &Path, body_b64: &str) {
        std::fs::write(
            path,
            format!("-----BEGIN PRIVATE KEY for erd1test-----\n{body_b64}\n-----END PRIVATE KEY for erd1test-----\n"),
        )
        .unwrap();
    }

    #[test]
    fn pem_parses_all_three_layouts() {
        use base64::Engine as _;
        // 32 B raw seed.
        let seed32 = [0x11u8; 32];
        let p = tmp("pem32");
        write_pem(&p, &base64::engine::general_purpose::STANDARD.encode(seed32));
        assert_eq!(load_pem_seed(&p).unwrap(), seed32);
        // 64 B seed+pubkey (mxpy standard): first half wins.
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&seed32);
        both[32..].copy_from_slice(&[0x22u8; 32]);
        let p = tmp("pem64");
        write_pem(&p, &base64::engine::general_purpose::STANDARD.encode(both));
        assert_eq!(load_pem_seed(&p).unwrap(), seed32);
        // Legacy: base64 of a 128-char hex string.
        let hexstr = "33".repeat(64);
        assert_eq!(hexstr.len(), 128);
        let p = tmp("pemlegacy");
        write_pem(&p, &base64::engine::general_purpose::STANDARD.encode(&hexstr));
        assert_eq!(load_pem_seed(&p).unwrap(), [0x33u8; 32]);
        // Garbage fails closed.
        let p = tmp("pembad");
        write_pem(&p, &base64::engine::general_purpose::STANDARD.encode([0x44u8; 16]));
        assert!(load_pem_seed(&p).is_err());
    }

    #[test]
    fn node_host_strips_scheme_port_path() {
        assert_eq!(node_host("https://decentraai.duckdns.org"), "decentraai.duckdns.org");
        assert_eq!(node_host("http://127.0.0.1:8080"), "127.0.0.1");
        assert_eq!(node_host("http://localhost:8080/v1"), "localhost");
    }
}
