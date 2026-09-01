use base64::Engine as _;
use bech32::{FromBase32, ToBase32, Variant, decode, encode};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEFAULT_NETWORK: &str = "multiversx-testnet";
const CHALLENGE_TTL_SECS: u64 = 300;
const SESSION_TTL_SECS: u64 = 24 * 60 * 60;

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
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

fn network_name() -> String {
    std::env::var("DECENTRAAI_MX_NETWORK").unwrap_or_else(|_| DEFAULT_NETWORK.to_string())
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
    let (hrp, data, variant) =
        decode(address).map_err(|_| WalletAuthError::InvalidAddress(address.to_string()))?;
    if hrp.as_str() != "erd" || variant != Variant::Bech32 {
        return Err(WalletAuthError::InvalidAddress(address.to_string()));
    }
    let bytes = Vec::<u8>::from_base32(&data)
        .map_err(|_| WalletAuthError::InvalidAddress(address.to_string()))?;
    if bytes.len() != 32 {
        return Err(WalletAuthError::InvalidAddress(address.to_string()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

pub fn encode_wallet_address(public_key: &[u8; 32]) -> Result<String, WalletAuthError> {
    encode("erd", public_key.to_base32(), Variant::Bech32)
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
        pk.verify(challenge.message.as_bytes(), &sig)
            .map_err(|_| WalletAuthError::SignatureInvalid)?;
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

    pub fn session_for_token(&mut self, token: &str, now: u64) -> Option<WalletSessionRecord> {
        self.cleanup(now);
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

    #[test]
    fn wallet_address_roundtrips_and_validates() {
        let identity = Identity::generate();
        let address = address_from_identity(&identity);
        let decoded = validate_wallet_address(&address).unwrap();
        assert_eq!(decoded, *identity.public_key().as_bytes());
        assert!(validate_wallet_address("erd1invalid").is_err());
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
