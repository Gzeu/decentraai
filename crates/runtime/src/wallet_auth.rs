use base64::Engine as _;
use bech32::primitives::decode::CheckedHrpstring;
use bech32::{Bech32, Hrp};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEFAULT_NETWORK: &str = "multiversx-testnet";
const CHALLENGE_TTL_SECS: u64 = 300;
const SESSION_TTL_SECS: u64 = 24 * 60 * 60;
/// Native-auth token TTL bounds (seconds): prevents forever-tokens.
/// Upper bound 86400 matches the official JS `sdk-native-auth-client`
/// default and third-party consoles built on it (Perchance orchestrator).
const NATIVE_AUTH_MIN_TTL: u64 = 60;
const NATIVE_AUTH_MAX_TTL: u64 = 86400;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletIdentityBinding {
    pub wallet_address: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub network: String,
    pub bound_at: u64,
    pub verified_at: u64,
    pub last_seen_at: u64,
    /// Name of the wallet-provisioned operator subscription token in the
    /// node token registry (`dsk_…`, tier 3, operator role). `None` = never
    /// provisioned (unlisted wallets never get one). The plaintext lives
    /// only in the issuance response; here just the name for idempotency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_token_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletChallengeRecord {
    pub challenge_id: String,
    pub wallet_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id_hint: Option<String>,
    pub purpose: String,
    pub nonce: String,
    pub message: String,
    pub issued_at: u64,
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletSessionRecord {
    pub session_token: String,
    pub wallet_address: String,
    pub agent_id: String,
    pub challenge_id: String,
    pub purpose: String,
    pub issued_at: u64,
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WalletAuthStore {
    pub bindings: BTreeMap<String, WalletIdentityBinding>,
    pub challenges: BTreeMap<String, WalletChallengeRecord>,
    pub sessions: BTreeMap<String, WalletSessionRecord>,
    /// Consumed native-auth tokens (token → used_at): one-time use, no replay.
    #[serde(default)]
    pub used_tokens: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletChallengeRequest {
    pub wallet_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletChallengeResponse {
    pub challenge_id: String,
    pub wallet_address: String,
    pub message: String,
    pub nonce: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pub network: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletVerifyRequest {
    pub wallet_address: String,
    pub challenge_id: String,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Native-auth login request (official MultiversX wallet flow).
/// Token: `b64url(origin).blockhash.ttl.b64url(extra)`; the wallet signs
/// the Elrond digest of `address + token` (legacy: `address + token + {}`).
/// Optional `network` pins the chain the block must live on (refuses
/// token↔network mismatch); absent, the server probes the known chains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletNativeAuthRequest {
    pub wallet_address: String,
    pub token: String,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Native-auth exchange request (third-party console contract, Shape B).
/// `accessToken = b64url(address).b64url(loginToken).sigHex` — the exact
/// envelope `@multiversx/sdk-native-auth-client` wallets produce and the
/// Perchance orchestrator sends to `POST /v1/auth/native`. Optional fields
/// are cross-checks only; the token itself is authoritative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletNativeExchangeRequest {
    #[serde(rename = "accessToken")]
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// Split an `accessToken` envelope into (address, login_token, sig_hex).
/// Fail-closed on shape: exactly 3 non-empty dot parts, first two valid
/// b64url UTF-8, third an even-length hex string.
pub fn parse_access_token(access_token: &str) -> Result<(String, String, String), WalletAuthError> {
    if access_token.len() > 4096 {
        return Err(WalletAuthError::MalformedToken);
    }
    let parts: Vec<&str> = access_token.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(WalletAuthError::MalformedToken);
    }
    let address = String::from_utf8(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| WalletAuthError::MalformedToken)?,
    )
    .map_err(|_| WalletAuthError::MalformedToken)?;
    let login_token = String::from_utf8(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| WalletAuthError::MalformedToken)?,
    )
    .map_err(|_| WalletAuthError::MalformedToken)?;
    let sig = parts[2];
    if sig.len() != 128 || !sig.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(WalletAuthError::MalformedToken);
    }
    Ok((address, login_token, sig.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletLoginResponse {
    pub wallet_address: String,
    pub agent_id: String,
    pub session_token: String,
    pub session_expires_at: u64,
    pub challenge_id: String,
    pub message: String,
    pub network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub pylon_identity_path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WalletAuthError {
    #[error("invalid wallet address: {0}")]
    InvalidAddress(String),
    #[error("wallet address does not match challenge")]
    AddressMismatch,
    #[error("challenge not found")]
    ChallengeNotFound,
    #[error("challenge expired")]
    ChallengeExpired,
    #[error("challenge already used")]
    ChallengeReplay,
    #[error("invalid signature encoding")]
    InvalidSignatureEncoding,
    #[error("signature verification failed")]
    SignatureInvalid,
    #[error("invalid session token")]
    InvalidSession,
    #[error("session expired")]
    SessionExpired,
    #[error("wallet binding conflict")]
    BindingConflict,
    #[error("malformed native-auth token")]
    MalformedToken,
    #[error("native-auth origin not allowed")]
    OriginNotAllowed,
    #[error("native-auth token expired or ttl out of range")]
    TokenExpired,
    #[error("native-auth token already used")]
    TokenReplay,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Elrond/MultiversX message-signing digest (what real wallet software
/// signs — NOT the raw message).
///
/// `keccak256("\x17Elrond Signed Message:\n" + len_ascii + msg)`, the
/// Ethereum-`personal_sign` shape adapted for Elrond (0x17 = 23 = length of
/// the prefix line). Proven live against DeFi-extension signatures (Sep
/// 2026): raw `addr+token` NEVER verifies from wallet software, the digest
/// does — in canonic (`addr+token`) and legacy (`addr+token+{}`) forms.
pub fn signable_digest(msg: &[u8]) -> [u8; 32] {
    use sha3::Digest as _;
    let mut h = sha3::Keccak256::new();
    h.update(b"\x17Elrond Signed Message:\n");
    h.update(msg.len().to_string().as_bytes());
    h.update(msg);
    h.finalize().into()
}

/// Verify a wallet signature against every known-good message form.
/// Order: wallet-native digests first (canonic, legacy), then the raw form
/// (own CLI/synthetic signers). All forms bind the same key to the same
/// server-issued bytes; one-time consumption upstream stops replays.
fn verify_sig_variants(
    pk: &VerifyingKey,
    sig: &Signature,
    address: &str,
    token_or_message: &str,
) -> bool {
    use sha3::Digest as _;
    let canonic = format!("{address}{token_or_message}");
    // 1-2. wallet digests (keccak, canonic + legacy).
    for candidate in [canonic.clone(), canonic.clone() + "{}"] {
        let mut h = sha3::Keccak256::new();
        h.update(b"\x17Elrond Signed Message:\n");
        h.update(candidate.len().to_string().as_bytes());
        h.update(candidate.as_bytes());
        let digest: [u8; 32] = h.finalize().into();
        if pk.verify(&digest, sig).is_ok() {
            return true;
        }
    }
    // 3. raw form (own CLI + synthetic test signers).
    pk.verify(canonic.as_bytes(), sig).is_ok()
}

/// Verify a bare server message (challenge flow): Elrond digest first
/// (wallet `signMessage` implementations digest like native-auth), then raw
/// (own CLI + manual paste of a raw signature).
fn verify_sig_message(pk: &VerifyingKey, sig: &Signature, message: &str) -> bool {
    use sha3::Digest as _;
    let mut h = sha3::Keccak256::new();
    h.update(b"\x17Elrond Signed Message:\n");
    h.update(message.len().to_string().as_bytes());
    h.update(message.as_bytes());
    let digest: [u8; 32] = h.finalize().into();
    pk.verify(&digest, sig).is_ok() || pk.verify(message.as_bytes(), sig).is_ok()
}

pub(crate) fn network_name() -> String {
    std::env::var("DECENTRAAI_MX_NETWORK").unwrap_or_else(|_| DEFAULT_NETWORK.to_string())
}

/// Origins allowed to mint native-auth tokens (exact match on the token's
/// decoded origin). Comma-separated override via DECENTRAAI_AUTH_ORIGINS.
/// Both bare hostnames (the official JS SDK default: `location.hostname`)
/// and full origins are accepted so first-party pages and third-party
/// consoles (e.g. a Perchance app with its Auth-origin field pointed at the
/// fabric host) work without server changes.
pub(crate) fn auth_origins() -> Vec<String> {
    std::env::var("DECENTRAAI_AUTH_ORIGINS")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_else(|_| {
            vec![
                "decentraai.duckdns.org".to_string(),
                "https://decentraai.duckdns.org".to_string(),
                "127.0.0.1".to_string(),
                "localhost".to_string(),
                "http://127.0.0.1:8080".to_string(),
                "http://localhost:8080".to_string(),
            ]
        })
}

/// Best-effort decode of a login token's origin (for error messages only —
/// never trusted). Lets an operator copy the exact string into
/// DECENTRAAI_AUTH_ORIGINS.
pub fn extract_token_origin(login_token: &str) -> Option<String> {
    let first = login_token.split('.').next()?;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(first)
        .ok()?;
    String::from_utf8(raw).ok()
}

/// Wallet addresses entitled to ELEVATED self-serve grants on the exchange
/// path (comma-separated `DECENTRAAI_WALLET_POWER_USERS`). A listed wallet
/// still receives a revocable, quota-gated `dca_` consumer key — never a
/// master/operator credential — but with wildcard scope and a large
/// ceiling, so the owner's console associates full powers automatically at
/// wallet login, with no human ever handling plaintext. Exact match on the
/// canonical address; empty by default (everyone else gets the modest
/// self-serve grant).
pub(crate) fn wallet_power_users() -> Vec<String> {
    std::env::var("DECENTRAAI_WALLET_POWER_USERS")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_else(|_| Vec::new())
}

/// MultiversX chain API base for a declared network (canonical lowercase).
/// Unknown names fall back to testnet — callers validate first and refuse
/// mismatches explicitly.
pub fn chain_api_base(network: &str) -> &'static str {
    match network {
        "mainnet" => "https://api.multiversx.com",
        "devnet" => "https://devnet-api.multiversx.com",
        _ => "https://testnet-api.multiversx.com",
    }
}

/// Whether a network name is one the fabric binds (`testnet`/`mainnet`/`devnet`).
pub fn is_known_network(network: &str) -> bool {
    matches!(network, "testnet" | "mainnet" | "devnet")
}

/// Public chain endpoints for a resolved network, so wallet clients adopt
/// them from the provisioning response instead of hardcoding. `mcp` is the
/// fabric's own public endpoint (override via `DECENTRAAI_PUBLIC_MCP_URL`).
pub fn chain_endpoints(network: &str) -> serde_json::Value {
    let (api, wallet, explorer) = match network {
        "mainnet" => (
            "https://api.multiversx.com",
            "https://wallet.multiversx.com",
            "https://explorer.multiversx.com",
        ),
        "devnet" => (
            "https://devnet-api.multiversx.com",
            "https://devnet-wallet.multiversx.com",
            "https://devnet-explorer.multiversx.com",
        ),
        _ => (
            "https://testnet-api.multiversx.com",
            "https://testnet-wallet.multiversx.com",
            "https://testnet-explorer.multiversx.com",
        ),
    };
    let mcp = std::env::var("DECENTRAAI_PUBLIC_MCP_URL")
        .unwrap_or_else(|_| "https://decentraai.duckdns.org/mcp".to_string());
    serde_json::json!({"api": api, "wallet": wallet, "explorer": explorer, "mcp": mcp})
}

/// Split a login token into (origin, blockhash, ttl_seconds). Shape +
/// ttl-range enforced here so handlers fail fast before any chain fetch.
pub fn parse_login_token(login_token: &str) -> Result<(String, String, u64), WalletAuthError> {
    let parts: Vec<&str> = login_token.split('.').collect();
    if parts.len() != 4
        || parts.iter().any(|p| p.is_empty())
        || parts[1].len() != 64
        || !parts[1].chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(WalletAuthError::MalformedToken);
    }
    let ttl: u64 = parts[2]
        .parse()
        .map_err(|_| WalletAuthError::MalformedToken)?;
    if !(NATIVE_AUTH_MIN_TTL..=NATIVE_AUTH_MAX_TTL).contains(&ttl) {
        return Err(WalletAuthError::TokenExpired);
    }
    let origin = String::from_utf8(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| WalletAuthError::MalformedToken)?,
    )
    .map_err(|_| WalletAuthError::MalformedToken)?;
    let extra = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[3])
        .map_err(|_| WalletAuthError::MalformedToken)?;
    let _: serde_json::Value =
        serde_json::from_slice(&extra).map_err(|_| WalletAuthError::MalformedToken)?;
    Ok((origin, parts[1].to_string(), ttl))
}

#[allow(dead_code)]
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn wallet_auth_path_for(repo_root: &Path) -> PathBuf {
    repo_root.join("db/wallet-auth.json")
}

pub fn validate_wallet_address(address: &str) -> Result<[u8; 32], WalletAuthError> {
    // Strict `Bech32` checksum (NOT Bech32m): `CheckedHrpstring::<Bech32>`
    // rejects bech32m payloads exactly like the old `variant != Bech32`
    // gate. Free `decode()` would accept both — must not use it here.
    let checked = CheckedHrpstring::new::<Bech32>(address)
        .map_err(|_| WalletAuthError::InvalidAddress(address.to_string()))?;
    if checked.hrp().as_str() != "erd" {
        return Err(WalletAuthError::InvalidAddress(address.to_string()));
    }
    let bytes: Vec<u8> = checked.byte_iter().collect();
    if bytes.len() != 32 {
        return Err(WalletAuthError::InvalidAddress(address.to_string()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn encode_wallet_address(public_key: &[u8; 32]) -> Result<String, WalletAuthError> {
    let hrp =
        Hrp::parse("erd").map_err(|_| WalletAuthError::InvalidAddress("encode".to_string()))?;
    bech32::encode::<Bech32>(hrp, public_key)
        .map_err(|_| WalletAuthError::InvalidAddress("encode".to_string()))
}

fn decode_signature(signature: &str) -> Result<[u8; 64], WalletAuthError> {
    let sig_bytes = if let Ok(bytes) = hex::decode(signature) {
        bytes
    } else if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(signature) {
        bytes
    } else if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(signature) {
        bytes
    } else {
        return Err(WalletAuthError::InvalidSignatureEncoding);
    };
    if sig_bytes.len() != 64 {
        return Err(WalletAuthError::InvalidSignatureEncoding);
    }
    let mut out = [0u8; 64];
    out.copy_from_slice(&sig_bytes);
    Ok(out)
}

fn challenge_message(
    wallet_address: &str,
    challenge_id: &str,
    nonce: &str,
    purpose: &str,
    agent_id_hint: Option<&str>,
    issued_at: u64,
    expires_at: u64,
) -> String {
    format!(
        "DecentraAI Wallet Login\nnetwork={}\nwallet_address={}\nchallenge_id={}\nnonce={}\npurpose={}\nagent_id={}\nissued_at={}\nexpires_at={}",
        network_name(),
        wallet_address,
        challenge_id,
        nonce,
        purpose,
        agent_id_hint.unwrap_or(""),
        issued_at,
        expires_at,
    )
}

fn session_token() -> String {
    format!("wx_{}", uuid::Uuid::new_v4())
}

fn default_agent_id(wallet_address: &str) -> String {
    wallet_address.to_string()
}

impl WalletAuthStore {
    pub fn load(path: &Path) -> Result<Self, WalletAuthError> {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(WalletAuthError::Io(e)),
        };
        Ok(serde_json::from_slice(&data)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), WalletAuthError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        let payload = serde_json::to_vec_pretty(self)?;
        std::fs::write(&tmp, &payload)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    fn cleanup(&mut self, now: u64) {
        self.challenges
            .retain(|_, c| c.expires_at > now && c.used_at.is_none());
        self.sessions.retain(|_, s| s.expires_at > now);
        self.used_tokens
            .retain(|_, used_at| *used_at + SESSION_TTL_SECS > now);
    }

    pub fn issue_challenge(
        &mut self,
        req: WalletChallengeRequest,
        now: u64,
    ) -> Result<WalletChallengeResponse, WalletAuthError> {
        self.cleanup(now);
        let wallet_bytes = validate_wallet_address(&req.wallet_address)?;
        let canonical_address = encode_wallet_address(&wallet_bytes)?;
        let challenge_id = format!("wch_{}", uuid::Uuid::new_v4().simple());
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let purpose = req.purpose.unwrap_or_else(|| "login".to_string());
        let issued_at = now;
        let expires_at = now.saturating_add(CHALLENGE_TTL_SECS);
        let message = challenge_message(
            &canonical_address,
            &challenge_id,
            &nonce,
            &purpose,
            req.agent_id.as_deref(),
            issued_at,
            expires_at,
        );
        let record = WalletChallengeRecord {
            challenge_id: challenge_id.clone(),
            wallet_address: canonical_address.clone(),
            agent_id_hint: req.agent_id.clone(),
            purpose: purpose.clone(),
            nonce: nonce.clone(),
            message: message.clone(),
            issued_at,
            expires_at,
            used_at: None,
        };
        self.challenges.insert(challenge_id.clone(), record);
        Ok(WalletChallengeResponse {
            challenge_id,
            wallet_address: canonical_address,
            message,
            nonce,
            issued_at,
            expires_at,
            network: network_name(),
        })
    }

    pub fn verify_and_login(
        &mut self,
        req: WalletVerifyRequest,
        now: u64,
    ) -> Result<WalletLoginResponse, WalletAuthError> {
        let wallet_bytes = validate_wallet_address(&req.wallet_address)?;
        let canonical_address = encode_wallet_address(&wallet_bytes)?;
        // Look up challenge BEFORE cleanup so expired challenges return
        // ChallengeExpired rather than ChallengeNotFound.
        let challenge = self
            .challenges
            .get_mut(&req.challenge_id)
            .ok_or(WalletAuthError::ChallengeNotFound)?;
        if challenge.wallet_address != canonical_address {
            return Err(WalletAuthError::AddressMismatch);
        }
        if challenge.expires_at <= now {
            return Err(WalletAuthError::ChallengeExpired);
        }
        if challenge.used_at.is_some() {
            return Err(WalletAuthError::ChallengeReplay);
        }
        // Note: no cleanup() here — we already hold a mutable borrow on a
        // challenge. Stale sessions/challenges are cleaned on the next
        // issue_challenge() or session_for_token() call.
        let sig_bytes = decode_signature(&req.signature)?;
        let sig = Signature::from_bytes(&sig_bytes);
        let pk = VerifyingKey::from_bytes(&wallet_bytes)
            .map_err(|_| WalletAuthError::SignatureInvalid)?;
        if !verify_sig_message(&pk, &sig, &challenge.message) {
            return Err(WalletAuthError::SignatureInvalid);
        }
        challenge.used_at = Some(now);

        let agent_id = if let Some(existing) = self.bindings.get(&canonical_address) {
            if let Some(requested) = req.agent_id.as_ref() {
                if requested != &existing.agent_id {
                    return Err(WalletAuthError::BindingConflict);
                }
            }
            existing.agent_id.clone()
        } else {
            req.agent_id
                .clone()
                .unwrap_or_else(|| default_agent_id(&canonical_address))
        };

        let display_name = req.display_name.clone().or_else(|| {
            self.bindings
                .get(&canonical_address)
                .and_then(|b| b.display_name.clone())
        });
        let binding = self
            .bindings
            .entry(canonical_address.clone())
            .or_insert_with(|| WalletIdentityBinding {
                wallet_address: canonical_address.clone(),
                agent_id: agent_id.clone(),
                display_name: display_name.clone(),
                network: network_name(),
                bound_at: now,
                verified_at: now,
                operator_token_name: None,
                last_seen_at: now,
            });
        binding.agent_id = agent_id.clone();
        binding.display_name = display_name.clone();
        binding.verified_at = now;
        binding.last_seen_at = now;

        let session_token = session_token();
        let session = WalletSessionRecord {
            session_token: session_token.clone(),
            wallet_address: canonical_address.clone(),
            agent_id: agent_id.clone(),
            challenge_id: challenge.challenge_id.clone(),
            purpose: challenge.purpose.clone(),
            issued_at: now,
            expires_at: now.saturating_add(SESSION_TTL_SECS),
            last_seen_at: Some(now),
        };
        self.sessions.insert(session_token.clone(), session.clone());

        Ok(WalletLoginResponse {
            wallet_address: canonical_address,
            agent_id: agent_id.clone(),
            session_token,
            session_expires_at: session.expires_at,
            challenge_id: challenge.challenge_id.clone(),
            message: challenge.message.clone(),
            network: network_name(),
            display_name,
            pylon_identity_path: format!("agents/{agent_id}/Identity.md"),
        })
    }

    /// Verify a native-auth login token (official MultiversX wallet flow).
    ///
    /// Token: `b64url(origin).blockhash.ttl.b64url(extra)`; the wallet signs
    /// `address + token`. Checks, all fail-closed: well-formed address and
    /// token shape, 64-hex blockhash, ttl in range, origin allow-listed,
    /// signature verifies, token never used before. On success it issues the
    /// same session shape as the challenge flow, so key issuance and rotation
    /// downstream are shared.
    pub fn verify_native_auth(
        &mut self,
        req: WalletNativeAuthRequest,
        now: u64,
    ) -> Result<WalletLoginResponse, WalletAuthError> {
        self.cleanup(now);
        let wallet_bytes = validate_wallet_address(&req.wallet_address)?;
        let canonical_address = encode_wallet_address(&wallet_bytes)?;
        // Shared shape/ttl/origin/extra gates (chain freshness is enforced
        // by the async handlers via the block timestamp, not here).
        let (origin, _blockhash, _ttl) = parse_login_token(&req.token)?;
        if !auth_origins().iter().any(|o| o == &origin) {
            return Err(WalletAuthError::OriginNotAllowed);
        }
        // One-time use: same token twice is a replay, even with a valid sig.
        if self.used_tokens.contains_key(&req.token) {
            return Err(WalletAuthError::TokenReplay);
        }
        let sig_bytes = decode_signature(&req.signature)?;
        let sig = Signature::from_bytes(&sig_bytes);
        let pk = VerifyingKey::from_bytes(&wallet_bytes)
            .map_err(|_| WalletAuthError::SignatureInvalid)?;
        // Wallet software signs the Elrond digest, not the raw message
        // (canonic + legacy forms); own CLI signs raw. Accept all proven
        // forms — every one binds this key to these exact token bytes.
        if !verify_sig_variants(&pk, &sig, &canonical_address, &req.token) {
            return Err(WalletAuthError::SignatureInvalid);
        }
        self.used_tokens.insert(req.token.clone(), now);

        let agent_id = if let Some(existing) = self.bindings.get(&canonical_address) {
            if let Some(requested) = req.agent_id.as_ref() {
                if requested != &existing.agent_id {
                    return Err(WalletAuthError::BindingConflict);
                }
            }
            existing.agent_id.clone()
        } else {
            req.agent_id
                .clone()
                .unwrap_or_else(|| default_agent_id(&canonical_address))
        };
        let display_name = req.display_name.clone().or_else(|| {
            self.bindings
                .get(&canonical_address)
                .and_then(|b| b.display_name.clone())
        });
        let binding = self
            .bindings
            .entry(canonical_address.clone())
            .or_insert_with(|| WalletIdentityBinding {
                wallet_address: canonical_address.clone(),
                agent_id: agent_id.clone(),
                display_name: display_name.clone(),
                network: network_name(),
                bound_at: now,
                verified_at: now,
                operator_token_name: None,
                last_seen_at: now,
            });
        binding.agent_id = agent_id.clone();
        binding.display_name = display_name.clone();
        binding.verified_at = now;
        binding.last_seen_at = now;

        let session_token = session_token();
        let token_tag = format!("native-{}", &req.token[..req.token.len().min(12)]);
        let session = WalletSessionRecord {
            session_token: session_token.clone(),
            wallet_address: canonical_address.clone(),
            agent_id: agent_id.clone(),
            challenge_id: token_tag.clone(),
            purpose: "native-auth".to_string(),
            issued_at: now,
            expires_at: now.saturating_add(SESSION_TTL_SECS),
            last_seen_at: Some(now),
        };
        self.sessions.insert(session_token.clone(), session.clone());

        Ok(WalletLoginResponse {
            wallet_address: canonical_address,
            agent_id: agent_id.clone(),
            session_token,
            session_expires_at: session.expires_at,
            challenge_id: token_tag,
            message: req.token,
            network: network_name(),
            display_name,
            pylon_identity_path: format!("agents/{agent_id}/Identity.md"),
        })
    }

    /// Verify a third-party `accessToken` envelope (Shape B exchange).
    ///
    /// Parses `b64url(address).b64url(loginToken).sigHex`, cross-checks the
    /// optional convenience fields (address/origin must match the token when
    /// present — fail-closed on confusion), then delegates to the exact same
    /// [`Self::verify_native_auth`] core: same shape/ttl/origin/signature
    /// gates, same one-time consumption. Returns the canonical address plus
    /// the issued session login (callers mint a credential off it).
    pub fn verify_access_token(
        &mut self,
        req: WalletNativeExchangeRequest,
        now: u64,
    ) -> Result<WalletLoginResponse, WalletAuthError> {
        if req.access_token.len() > 4096 {
            return Err(WalletAuthError::MalformedToken);
        }
        let (address, login_token, sig_hex) = parse_access_token(&req.access_token)?;
        if let Some(claimed) = req.address.as_ref() {
            if claimed != &address {
                return Err(WalletAuthError::AddressMismatch);
            }
        }
        if let Some(claimed_origin) = req.origin.as_ref() {
            match extract_token_origin(&login_token) {
                Some(token_origin) if &token_origin == claimed_origin => {}
                _ => return Err(WalletAuthError::OriginNotAllowed),
            }
        }
        self.verify_native_auth(
            WalletNativeAuthRequest {
                wallet_address: address,
                token: login_token,
                signature: sig_hex,
                network: None,
                agent_id: None,
                display_name: None,
            },
            now,
        )
    }

    pub fn session_for_token(&mut self, token: &str, now: u64) -> Option<WalletSessionRecord> {        self.cleanup(now);
        self.sessions
            .get(token)
            .cloned()
            .filter(|s| s.expires_at > now)
    }

    pub fn binding_for_wallet(&self, wallet_address: &str) -> Option<&WalletIdentityBinding> {
        self.bindings.get(wallet_address)
    }
}

pub fn update_identity_memory_frontmatter(
    memory: &mut decentraai_agent_personal_memory::schema::IdentityMemory,
    wallet_address: &str,
    agent_id: &str,
    display_name: Option<&str>,
    verified_at: u64,
) {
    memory.agent_id = agent_id.to_string();
    if let Some(name) = display_name {
        memory.name = name.to_string();
    } else if memory.name.trim().is_empty() {
        memory.name = wallet_address.to_string();
    }
    if memory.description.trim().is_empty() {
        memory.description = "MultiversX wallet-backed DecentraAI identity".to_string();
    }
    if memory.persona.trim().is_empty() {
        memory.persona = "wallet-backed operator".to_string();
    }
    if memory.values.is_empty() {
        memory.values = vec![
            "security".to_string(),
            "reliability".to_string(),
            "continuity".to_string(),
        ];
    }
    if memory.communication_style.trim().is_empty() {
        memory.communication_style = "direct, technical, concise".to_string();
    }
    memory.frontmatter.extra.insert(
        "wallet_address".to_string(),
        serde_json::json!(wallet_address),
    );
    memory.frontmatter.extra.insert(
        "wallet_network".to_string(),
        serde_json::json!(network_name()),
    );
    memory.frontmatter.extra.insert(
        "wallet_verified_at".to_string(),
        serde_json::json!(verified_at),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use decentraai_identity::Identity;
    use tempfile::tempdir;

    fn address_from_identity(identity: &Identity) -> String {
        encode_wallet_address(identity.public_key().as_bytes()).unwrap()
    }

    fn native_token(origin: &str, ttl: u64) -> String {
        use base64::Engine as _;
        let o = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(origin);
        let e = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"app":"t"}"#);
        format!("{o}.{}.{ttl}.{e}", "ab".repeat(32))
    }

    #[test]
    fn wallet_elrond_digest_verifies_real_extension_signatures() {
        // Interop lock (Sep 2026): real DeFi-extension signatures over
        // native-auth tokens. The wallet signs keccak256(prefix+len+msg),
        // NOT the raw message — in canonic (`addr+token`, mainnet wallet)
        // and legacy (`addr+token+{}`, testnet wallet) forms. Addresses and
        // signatures are public by construction (on-chain + sent to server).
        let cases = [
            (
                "erd154tm8nu953lnen33zqxwq3mc9hz8y37mclyvmpdrszv9umcyys7q0p3f90",
                "ZGVjZW50cmFhaS5kdWNrZG5zLm9yZw.ceeee9addff372ff736887e5289d8eb9f715eaa41a34be7ed75480b15e008340.86400.e30",
                "3942616ad7e267b2d538e48358923ccd3533729b47015cb06c86e90cd751fab6231cb308bb765fbadff0aebaa8fd2cf4dffba4e4939ca73876bea231eee62200",
            ),
            (
                "erd1ws6ery9eknznsl33ghf72hexkv72j5f6hsnvjrs04hwxyzm66pxq2qv0cn",
                "ZGVjZW50cmFhaS5kdWNrZG5zLm9yZw.6da5d2a31d2c74856a0a31f18fd7b3727df62b2154ceb92e1f256f7bf02da7fc.86400.e30",
                "914d471bf8bc753c9612ac45478a95271980fab7a72dbcba0ea85919bf3734dcf99ec8d1d057f2f4e45c50897ea150c1b3c7fe6a4fccb4ceaf15638f4e74cc0e",
            ),
        ];
        for (addr, token, sig_hex) in cases {
            let bytes = validate_wallet_address(addr).unwrap();
            let pk = VerifyingKey::from_bytes(&bytes).unwrap();
            let sig_bytes = decode_signature(sig_hex).unwrap();
            let sig = Signature::from_bytes(&sig_bytes);
            assert!(
                verify_sig_variants(&pk, &sig, addr, token),
                "real wallet signature must verify for {addr}"
            );
            // A tampered token must NOT verify under the same signature.
            assert!(!verify_sig_variants(&pk, &sig, addr, &format!("{token}tampered")));
        }
    }

    #[test]
    fn native_auth_login_ok_and_replay_rejected() {
        // Official flow shape: wallet signs `address + token`.
        let identity = Identity::generate();
        let address = address_from_identity(&identity);
        let mut store = WalletAuthStore::default();
        let token = native_token("https://decentraai.duckdns.org", 600);
        let sig = identity.sign(format!("{address}{token}").as_bytes());
        let login = store
            .verify_native_auth(
                WalletNativeAuthRequest {
                    wallet_address: address.clone(),
                    token: token.clone(),
                    signature: hex::encode(sig.to_bytes()),
                    network: None,
                    agent_id: None,
                    display_name: None,
                },
                100,
            )
            .unwrap();
        assert_eq!(login.wallet_address, address);
        assert_eq!(login.agent_id, address);
        assert!(login.session_token.starts_with("wx_"));
        assert!(store.session_for_token(&login.session_token, 105).is_some());
        // Same token twice = replay, even with a valid signature.
        assert!(matches!(
            store.verify_native_auth(
                WalletNativeAuthRequest {
                    wallet_address: address.clone(),
                    token,
                    signature: hex::encode(sig.to_bytes()),
                    network: None,
                    agent_id: None,
                    display_name: None,
                },
                106,
            ),
            Err(WalletAuthError::TokenReplay)
        ));
    }

    #[test]
    fn native_auth_rejects_bad_token_origin_ttl_sig() {
        let identity = Identity::generate();
        let address = address_from_identity(&identity);
        let mut store = WalletAuthStore::default();
        let good = native_token("https://decentraai.duckdns.org", 600);
        let sign = |msg: &str| hex::encode(identity.sign(msg.as_bytes()).to_bytes());
        // Bad origin.
        let bad_origin = native_token("https://evil.example", 600);
        assert!(matches!(
            store.verify_native_auth(
                WalletNativeAuthRequest {
                    wallet_address: address.clone(),
                    token: bad_origin.clone(),
                    signature: sign(&format!("{address}{bad_origin}")),
                    network: None,
                    agent_id: None,
                    display_name: None,
                },
                100,
            ),
            Err(WalletAuthError::OriginNotAllowed)
        ));
        // TTL out of range.
        let bad_ttl = native_token("https://decentraai.duckdns.org", 99999);
        assert!(matches!(
            store.verify_native_auth(
                WalletNativeAuthRequest {
                    wallet_address: address.clone(),
                    token: bad_ttl,
                    signature: sign(&format!("{address}{good}")),
                    network: None,
                    agent_id: None,
                    display_name: None,
                },
                100,
            ),
            Err(WalletAuthError::TokenExpired)
        ));
        // Malformed token.
        assert!(matches!(
            store.verify_native_auth(
                WalletNativeAuthRequest {
                    wallet_address: address.clone(),
                    token: "not.a.token".to_string(),
                    signature: sign("x"),
                    network: None,
                    agent_id: None,
                    display_name: None,
                },
                100,
            ),
            Err(WalletAuthError::MalformedToken)
        ));
        // Wrong signature (signed different bytes).
        assert!(matches!(
            store.verify_native_auth(
                WalletNativeAuthRequest {
                    wallet_address: address.clone(),
                    token: good.clone(),
                    signature: sign("wrong-bytes"),
                    network: None,
                    agent_id: None,
                    display_name: None,
                },
                100,
            ),
            Err(WalletAuthError::SignatureInvalid)
        ));
    }

    #[test]
    fn wallet_address_roundtrips_and_validates() {
        let identity = Identity::generate();
        let address = address_from_identity(&identity);
        let decoded = validate_wallet_address(&address).unwrap();
        assert_eq!(decoded, *identity.public_key().as_bytes());
        assert!(validate_wallet_address("erd1invalid").is_err());
    }

    #[test]
    fn chain_vectors_decode_and_reencode_identically() {
        // Golden cross-version proof (bech32 0.9 → 0.12): these are real
        // MultiversX chain addresses (ESDT system SC + devnet contracts),
        // produced long before this migration. The BIP-173 `Bech32`
        // checksum is unchanged, so 0.12 must decode them to 32 bytes and
        // re-encode byte-identically.
        for vector in [
            "erd1qqqqqqqqqqqqqpgqzcufga3vm5r44xe3ukzyl4dmhpsvalrkkgjqeyu68x",
            "erd1qqqqqqqqqqqqqpgqvax6z79cvyz9gkfwg57hqume352p7s7rd8ss4g3t43",
            "erd1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq6gq4hu",
        ] {
            let bytes = validate_wallet_address(vector).unwrap();
            assert_eq!(bytes.len(), 32);
            let reencoded = encode_wallet_address(&bytes).unwrap();
            assert_eq!(reencoded, vector);
        }
    }

    #[test]
    fn foreign_hrp_and_bech32m_checksum_are_rejected() {
        // BIP-350 `abc1…` vector: wrong HRP *and* a Bech32m checksum.
        // Rejected on the strict gate (either the HRP or the checksum leg
        // fires — the old code rejected it via `variant != Bech32`/HRP too).
        assert!(validate_wallet_address("abc14w46h2at4w46h2at4w46h2at958ngu").is_err());
        assert!(validate_wallet_address("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").is_err());
    }

    #[test]
    fn challenge_verify_login_and_replay_protection() {
        let identity = Identity::generate();
        let address = address_from_identity(&identity);
        let mut store = WalletAuthStore::default();
        let challenge = store
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address.clone(),
                    agent_id: Some("agent-wallet".into()),
                    display_name: Some("Wallet Agent".into()),
                    purpose: Some("world".into()),
                },
                100,
            )
            .unwrap();
        let sig = identity.sign(challenge.message.as_bytes());
        let login = store
            .verify_and_login(
                WalletVerifyRequest {
                    wallet_address: address.clone(),
                    challenge_id: challenge.challenge_id.clone(),
                    signature: hex::encode(sig.to_bytes()),
                    agent_id: Some("agent-wallet".into()),
                    display_name: Some("Wallet Agent".into()),
                },
                105,
            )
            .unwrap();
        assert_eq!(login.wallet_address, address);
        assert_eq!(login.agent_id, "agent-wallet");
        assert!(
            store
                .verify_and_login(
                    WalletVerifyRequest {
                        wallet_address: login.wallet_address.clone(),
                        challenge_id: challenge.challenge_id.clone(),
                        signature: hex::encode(sig.to_bytes()),
                        agent_id: Some("agent-wallet".into()),
                        display_name: Some("Wallet Agent".into()),
                    },
                    106,
                )
                .is_err()
        );
    }

    #[test]
    fn challenge_and_session_persist_across_reload() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("wallet-auth.json");
        let identity = Identity::generate();
        let address = address_from_identity(&identity);
        let mut store = WalletAuthStore::default();
        let challenge = store
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address.clone(),
                    agent_id: None,
                    display_name: None,
                    purpose: None,
                },
                200,
            )
            .unwrap();
        let sig = identity.sign(challenge.message.as_bytes());
        let login = store
            .verify_and_login(
                WalletVerifyRequest {
                    wallet_address: address.clone(),
                    challenge_id: challenge.challenge_id.clone(),
                    signature: base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
                    agent_id: None,
                    display_name: None,
                },
                205,
            )
            .unwrap();
        store.save(&path).unwrap();
        let mut loaded = WalletAuthStore::load(&path).unwrap();
        assert!(
            loaded
                .session_for_token(&login.session_token, 210)
                .is_some()
        );
        assert_eq!(
            loaded.binding_for_wallet(&address).unwrap().agent_id,
            address
        );
    }

    #[test]
    fn expired_and_invalid_signatures_are_rejected() {
        let identity = Identity::generate();
        let address = address_from_identity(&identity);
        let mut store = WalletAuthStore::default();
        let challenge = store
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address.clone(),
                    agent_id: None,
                    display_name: None,
                    purpose: None,
                },
                1,
            )
            .unwrap();
        let sig = identity.sign(challenge.message.as_bytes());
        assert!(matches!(
            store.verify_and_login(
                WalletVerifyRequest {
                    wallet_address: address.clone(),
                    challenge_id: challenge.challenge_id.clone(),
                    signature: hex::encode(sig.to_bytes()),
                    agent_id: None,
                    display_name: None,
                },
                challenge.expires_at + 1,
            ),
            Err(WalletAuthError::ChallengeExpired)
        ));

        let mut store = WalletAuthStore::default();
        let challenge = store
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address.clone(),
                    agent_id: None,
                    display_name: None,
                    purpose: None,
                },
                10,
            )
            .unwrap();
        assert!(matches!(
            store.verify_and_login(
                WalletVerifyRequest {
                    wallet_address: address.clone(),
                    challenge_id: challenge.challenge_id.clone(),
                    signature: "deadbeef".into(),
                    agent_id: None,
                    display_name: None,
                },
                11,
            ),
            Err(WalletAuthError::InvalidSignatureEncoding)
        ));
    }

    /// Full Definition of Done E2E flow:
    /// challenge → sign → verify → wx_ session → persistent binding →
    /// restart → same identity + same wallet.
    /// Plus: wallet conflict between two identities, idempotent re-bind,
    /// and wrong agent_id rejection.
    #[test]
    fn e2e_wallet_identity_lifecycle() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("wallet-auth.json");

        // --- Agent A: generate identity, derive wallet address ---
        let identity_a = Identity::generate();
        let address_a = address_from_identity(&identity_a);
        let mut store = WalletAuthStore::default();

        // --- Step 1: challenge → sign → verify → login ---
        let challenge_a = store
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address_a.clone(),
                    agent_id: Some("agent-alpha".into()),
                    display_name: Some("Alpha Agent".into()),
                    purpose: Some("world".into()),
                },
                1000,
            )
            .unwrap();
        assert_eq!(challenge_a.network, network_name());
        assert_eq!(challenge_a.wallet_address, address_a);

        let sig_a = identity_a.sign(challenge_a.message.as_bytes());
        let login_a = store
            .verify_and_login(
                WalletVerifyRequest {
                    wallet_address: address_a.clone(),
                    challenge_id: challenge_a.challenge_id.clone(),
                    signature: hex::encode(sig_a.to_bytes()),
                    agent_id: Some("agent-alpha".into()),
                    display_name: Some("Alpha Agent".into()),
                },
                1005,
            )
            .unwrap();

        // Session token is wx_-prefixed
        assert!(login_a.session_token.starts_with("wx_"));
        assert_eq!(login_a.agent_id, "agent-alpha");
        assert_eq!(login_a.wallet_address, address_a);
        assert!(login_a.session_expires_at > 1005);

        // Session is live
        assert!(
            store
                .session_for_token(&login_a.session_token, 1010)
                .is_some()
        );

        // Binding persisted in store
        let binding = store.binding_for_wallet(&address_a).unwrap();
        assert_eq!(binding.agent_id, "agent-alpha");
        assert_eq!(binding.network, network_name());
        assert_eq!(binding.bound_at, 1005);

        // --- Step 2: persist + reload (simulates restart) ---
        store.save(&path).unwrap();
        let mut reloaded = WalletAuthStore::load(&path).unwrap();

        // Session survives restart
        let session = reloaded.session_for_token(&login_a.session_token, 1010);
        assert!(session.is_some());
        let session = session.unwrap();
        assert_eq!(session.wallet_address, address_a);
        assert_eq!(session.agent_id, "agent-alpha");

        // Binding survives restart
        let binding = reloaded.binding_for_wallet(&address_a).unwrap();
        assert_eq!(binding.agent_id, "agent-alpha");
        assert_eq!(binding.verified_at, 1005);

        // --- Step 3: idempotent re-bind with same agent_id → OK ---
        let challenge_a2 = reloaded
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address_a.clone(),
                    agent_id: Some("agent-alpha".into()),
                    display_name: None,
                    purpose: None,
                },
                2000,
            )
            .unwrap();
        let sig_a2 = identity_a.sign(challenge_a2.message.as_bytes());
        let login_a2 = reloaded
            .verify_and_login(
                WalletVerifyRequest {
                    wallet_address: address_a.clone(),
                    challenge_id: challenge_a2.challenge_id.clone(),
                    signature: hex::encode(sig_a2.to_bytes()),
                    agent_id: Some("agent-alpha".into()),
                    display_name: None,
                },
                2005,
            )
            .unwrap();
        assert_eq!(login_a2.agent_id, "agent-alpha");
        // New login generates a new session token (old one still valid too)
        assert!(login_a2.session_token.starts_with("wx_"));
        assert_ne!(login_a2.session_token, login_a.session_token);
        // Both sessions are valid concurrently
        assert!(
            reloaded
                .session_for_token(&login_a.session_token, 2010)
                .is_some()
        );
        assert!(
            reloaded
                .session_for_token(&login_a2.session_token, 2010)
                .is_some()
        );

        // --- Step 4: different identity tries to claim same wallet → conflict ---
        let identity_b = Identity::generate();
        let challenge_a3 = reloaded
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address_a.clone(),
                    agent_id: Some("agent-beta".into()),
                    display_name: None,
                    purpose: None,
                },
                3000,
            )
            .unwrap();
        // Sign with identity_b but claim wallet address_a
        let sig_a3_wrong = identity_b.sign(challenge_a3.message.as_bytes());
        // This should fail: signature doesn't match wallet's public key
        assert!(
            reloaded
                .verify_and_login(
                    WalletVerifyRequest {
                        wallet_address: address_a.clone(),
                        challenge_id: challenge_a3.challenge_id.clone(),
                        signature: hex::encode(sig_a3_wrong.to_bytes()),
                        agent_id: Some("agent-beta".into()),
                        display_name: None,
                    },
                    3005,
                )
                .is_err()
        );

        // --- Step 5: same wallet + wrong agent_id → BindingConflict ---
        // Re-challenge with correct wallet address but wrong agent_id
        let challenge_a4 = reloaded
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address_a.clone(),
                    agent_id: Some("agent-wrong".into()),
                    display_name: None,
                    purpose: None,
                },
                3010,
            )
            .unwrap();
        let sig_a4 = identity_a.sign(challenge_a4.message.as_bytes());
        let result = reloaded.verify_and_login(
            WalletVerifyRequest {
                wallet_address: address_a.clone(),
                challenge_id: challenge_a4.challenge_id.clone(),
                signature: hex::encode(sig_a4.to_bytes()),
                agent_id: Some("agent-wrong".into()),
                display_name: None,
            },
            3015,
        );
        assert!(matches!(result, Err(WalletAuthError::BindingConflict)));

        // --- Step 6: agent B with its own wallet → independent identity ---
        let challenge_b = reloaded
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address_from_identity(&identity_b),
                    agent_id: Some("agent-beta".into()),
                    display_name: Some("Beta Agent".into()),
                    purpose: Some("world".into()),
                },
                4000,
            )
            .unwrap();
        let sig_b = identity_b.sign(challenge_b.message.as_bytes());
        let login_b = reloaded
            .verify_and_login(
                WalletVerifyRequest {
                    wallet_address: address_from_identity(&identity_b),
                    challenge_id: challenge_b.challenge_id.clone(),
                    signature: hex::encode(sig_b.to_bytes()),
                    agent_id: Some("agent-beta".into()),
                    display_name: Some("Beta Agent".into()),
                },
                4005,
            )
            .unwrap();
        assert_eq!(login_b.agent_id, "agent-beta");
        assert_ne!(login_b.wallet_address, address_a);
        assert_ne!(login_b.session_token, login_a.session_token);

        // Both bindings coexist
        assert!(reloaded.binding_for_wallet(&address_a).is_some());
        assert!(
            reloaded
                .binding_for_wallet(&address_from_identity(&identity_b))
                .is_some()
        );

        // --- Step 7: session expires ---
        let expired = reloaded.session_for_token(&login_a.session_token, 99999);
        assert!(expired.is_none());

        // --- Step 8: same wallet, no agent_id → defaults to wallet address ---
        let identity_c = Identity::generate();
        let address_c = address_from_identity(&identity_c);
        let challenge_c = reloaded
            .issue_challenge(
                WalletChallengeRequest {
                    wallet_address: address_c.clone(),
                    agent_id: None,
                    display_name: None,
                    purpose: None,
                },
                5000,
            )
            .unwrap();
        let sig_c = identity_c.sign(challenge_c.message.as_bytes());
        let login_c = reloaded
            .verify_and_login(
                WalletVerifyRequest {
                    wallet_address: address_c.clone(),
                    challenge_id: challenge_c.challenge_id.clone(),
                    signature: hex::encode(sig_c.to_bytes()),
                    agent_id: None,
                    display_name: None,
                },
                5005,
            )
            .unwrap();
        // Default agent_id = wallet address itself
        assert_eq!(login_c.agent_id, address_c);
    }
}
