//! Subscription token registry (P1).
//!
//! Everything is free; your tier reflects your contribution. The admin
//! issues API tokens (`dsk_<64 hex>`) from the CLI or the future admin
//! dashboard; each token maps to a tier that gates models and request
//! rates in the inference proxy.
//!
//! Tokens are shown exactly once at creation; the registry stores only
//! BLAKE3 hashes, so a leaked `tokens.json` reveals nothing usable —
//! the same posture as the reputation store and the API token file.

mod consumer;
mod tiers;

pub use consumer::{
    ConsumerKeyRecord, ConsumerKeyStore, KEY_PREFIX, PREFIX_LEN, hash_key, key_prefix,
};
pub use tiers::{SuggestedTier, TierChange, plan_tier_changes};

use anyhow::{Context, Result, bail};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

/// Subscription tier: 1 Guest, 2 Contributor, 3 Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Tier(pub u8);

/// Role bound to a subscription token (H4 role separation).
///
/// - `Client` — may run inference within its tier limits; no operational view.
/// - `Operator` — read-only operational views (status, workers, network,
///   execution, peers) but NOT token management.
///
/// The master API token is the `Admin` role (everything, incl. token
/// create/revoke); it is not stored in this registry — it lives only in
/// `runtime/api.token`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Client,
    Operator,
}

impl Role {
    pub const DEFAULT: Self = Self::Client;

    pub fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "client" | "operator" => {}, // fallthrough below
            _ => bail!("role must be 'client' or 'operator'"),
        }
        match value.to_ascii_lowercase().as_str() {
            "client" => Ok(Self::Client),
            _ => Ok(Self::Operator),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Operator => "operator",
        }
    }
}

impl Tier {
    pub const GUEST: Self = Self(1);
    pub const CONTRIBUTOR: Self = Self(2);
    pub const CORE: Self = Self(3);

    pub fn parse(value: u8) -> Result<Self> {
        if (1..=3).contains(&value) {
            Ok(Self(value))
        } else {
            bail!("tier must be 1 (guest), 2 (contributor), or 3 (core)")
        }
    }

    pub fn name(self) -> &'static str {
        match self.0 {
            1 => "guest",
            2 => "contributor",
            _ => "core",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub name: String,
    pub tier: u8,
    pub created_at: u64,
    pub revoked: bool,
    /// Unix seconds at which the token stops working, or `None` for no expiry.
    /// Backward-compatible: older registries without the field are `None`.
    #[serde(default)]
    pub expires_at: Option<u64>,
    /// Role bound to this token (H4). Backward-compatible default: client.
    #[serde(default)]
    pub role: Role,
}

#[derive(Serialize, Deserialize)]
struct TokenFile {
    schema_version: u32,
    tokens: BTreeMap<String, TokenRecord>,
}

/// The registry: hash(token) -> record, persisted atomically.
pub struct TokenStore {
    tokens: BTreeMap<String, TokenRecord>,
    path: PathBuf,
}

/// Hashes a plaintext token for storage and lookup.
pub fn hash_token(plaintext: &str) -> String {
    blake3::hash(plaintext.as_bytes()).to_hex().to_string()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl TokenStore {
    /// Loads the registry. A missing file starts empty; a corrupted file
    /// starts fresh with a warning (availability over strictness).
    pub fn load(path: &Path) -> Result<Self> {
        let tokens = match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<TokenFile>(&content) {
                Ok(file) => file.tokens,
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "corrupt token registry, starting fresh");
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).context("reading token registry"),
        };
        Ok(Self {
            tokens,
            path: path.to_path_buf(),
        })
    }

    /// Persists atomically (tmp + sync + rename).
    pub fn save(&self) -> Result<()> {
        let file = TokenFile {
            schema_version: 1,
            tokens: self.tokens.clone(),
        };
        let content = serde_json::to_string_pretty(&file)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("tmp");
        let mut out = std::fs::File::create(&tmp)?;
        out.write_all(content.as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Issues a new token. Returns the plaintext — show it once, then
    /// forget it; only the hash is kept. `expires_at` (unix seconds) makes the
    /// token stop working after that instant; `None` means no expiry. The
    /// token is bound to [`Role::DEFAULT`] (client).
    pub fn create(&mut self, name: &str, tier: Tier, expires_at: Option<u64>) -> Result<String> {
        self.create_with_role(name, tier, expires_at, Role::DEFAULT)
    }

    /// Like [`create`](Self::create) but binds an explicit [`Role`] (H4).
    pub fn create_with_role(
        &mut self,
        name: &str,
        tier: Tier,
        expires_at: Option<u64>,
        role: Role,
    ) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            bail!("token name must not be empty");
        }
        if self.tokens.values().any(|r| r.name == name && !r.revoked) {
            bail!("an active token named '{name}' already exists");
        }
        let mut bytes = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut bytes);
        let plaintext = format!("dsk_{}", hex::encode(bytes));
        self.tokens.insert(
            hash_token(&plaintext),
            TokenRecord {
                name: name.to_string(),
                tier: tier.0,
                created_at: now_secs(),
                revoked: false,
                expires_at,
                role,
            },
        );
        self.save()?;
        Ok(plaintext)
    }

    /// Reassigns an active token's role (H4). Returns the previous role and
    /// persists atomically. No-ops if already at `role`.
    pub fn set_role(&mut self, name: &str, role: Role) -> Result<Role> {
        let entry = self
            .tokens
            .values_mut()
            .find(|r| r.name == name && !r.revoked)
            .with_context(|| format!("no active token named '{name}'"))?;
        let from = entry.role;
        if from != role {
            entry.role = role;
            self.save()?;
        }
        Ok(from)
    }

    /// Revokes by name (the admin knows names, not hashes).
    pub fn revoke(&mut self, name: &str) -> Result<()> {
        let entry = self
            .tokens
            .values_mut()
            .find(|r| r.name == name && !r.revoked)
            .with_context(|| format!("no active token named '{name}'"))?;
        entry.revoked = true;
        self.save()
    }

    /// Reassigns an active token's tier (P4). Returns the previous tier so the
    /// caller can audit the change. No-ops if already at `tier`.
    pub fn set_tier(&mut self, name: &str, tier: Tier) -> Result<u8> {
        let entry = self
            .tokens
            .values_mut()
            .find(|r| r.name == name && !r.revoked)
            .with_context(|| format!("no active token named '{name}'"))?;
        let from = entry.tier;
        if from != tier.0 {
            entry.tier = tier.0;
            self.save()?;
        }
        Ok(from)
    }

    /// Resolves a plaintext token to its record, if active (not revoked and not
    /// expired). A token whose `expires_at` is in the past is inactive.
    pub fn lookup(&self, plaintext: &str) -> Option<&TokenRecord> {
        self.tokens
            .get(&hash_token(plaintext))
            .filter(|r| !r.revoked && Self::not_expired(r))
    }

    /// Whether a record is live at the current time: not revoked and, if it has
    /// an expiry, that expiry is still in the future.
    pub fn is_active(&self, record: &TokenRecord) -> bool {
        !record.revoked && Self::not_expired(record)
    }

    fn not_expired(record: &TokenRecord) -> bool {
        record.expires_at.is_none_or(|ts| ts > now_secs())
    }

    /// All records (active and revoked), newest first, for `token list`.
    pub fn list(&self) -> Vec<TokenRecord> {
        let mut out: Vec<TokenRecord> = self.tokens.values().cloned().collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(dir: &Path) -> TokenStore {
        TokenStore::load(&dir.join("tokens.json")).unwrap()
    }

    #[test]
    fn create_shows_plaintext_once_and_stores_only_the_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plaintext = store.create("alice", Tier::GUEST, None).unwrap();
        assert!(plaintext.starts_with("dsk_"));
        assert_eq!(plaintext.len(), 4 + 64);

        let on_disk = std::fs::read_to_string(dir.path().join("tokens.json")).unwrap();
        assert!(
            !on_disk.contains(&plaintext),
            "plaintext must never be persisted"
        );
        assert!(on_disk.contains(&hash_token(&plaintext)));

        let record = store.lookup(&plaintext).unwrap();
        assert_eq!(record.name, "alice");
        assert_eq!(record.tier, 1);
    }

    #[test]
    fn duplicate_active_names_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        store.create("alice", Tier::GUEST, None).unwrap();
        assert!(store.create("alice", Tier::CORE, None).is_err());
    }

    #[test]
    fn revoke_hides_the_token_but_keeps_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plaintext = store.create("bob", Tier::CONTRIBUTOR, None).unwrap();
        store.revoke("bob").unwrap();
        assert!(store.lookup(&plaintext).is_none());
        let listed = store.list();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].revoked);
        // A revoked name can be reused.
        store.create("bob", Tier::CORE, None).unwrap();
    }

    #[test]
    fn set_tier_reassigns_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plaintext = store.create("dana", Tier::GUEST, None).unwrap();
        store.set_tier("dana", Tier::CORE).unwrap();

        let reloaded = open(dir.path());
        let record = reloaded.lookup(&plaintext).unwrap();
        assert_eq!(record.tier, 3);
    }

    #[test]
    fn registry_survives_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let plaintext;
        {
            let mut store = open(dir.path());
            plaintext = store.create("carol", Tier::CORE, None).unwrap();
        }
        let reloaded = open(dir.path());
        let record = reloaded.lookup(&plaintext).unwrap();
        assert_eq!(record.tier, 3);
    }

    #[test]
    fn corrupt_registry_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tokens.json"), b"not json").unwrap();
        let store = open(dir.path());
        assert!(store.list().is_empty());
    }

    #[test]
    fn tier_validation() {
        assert!(Tier::parse(0).is_err());
        assert!(Tier::parse(4).is_err());
        assert_eq!(Tier::parse(2).unwrap().name(), "contributor");
    }

    #[test]
    fn expired_token_is_inactive_but_revoked_stays_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        // An expiry in the far past makes the token unusable immediately.
        let expired = store
            .create("dave", Tier::GUEST, Some(1))
            .unwrap();
        assert!(
            store.lookup(&expired).is_none(),
            "a token whose expiry has passed must not resolve"
        );
        let rec = store.tokens.get(&hash_token(&expired)).unwrap();
        assert!(!store.is_active(rec), "expired token must not be active");

        // A future or absent expiry stays active.
        let live = store.create("erin", Tier::GUEST, Some(now_secs() + 3600)).unwrap();
        assert!(store.lookup(&live).is_some());
        let never = store.create("frank", Tier::GUEST, None).unwrap();
        assert!(store.lookup(&never).is_some());
    }

    #[test]
    fn tokens_carry_a_role_and_set_role_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        // Default role is client.
        let t = store.create("grace", Tier::GUEST, None).unwrap();
        assert_eq!(store.lookup(&t).unwrap().role, Role::DEFAULT);
        assert_eq!(store.lookup(&t).unwrap().role, Role::Client);

        // Promote to operator; the change persists across reloads.
        store.set_role("grace", Role::Operator).unwrap();
        let reloaded = open(dir.path());
        assert_eq!(reloaded.lookup(&t).unwrap().role, Role::Operator);
        assert!(reloaded.lookup(&t).unwrap().role != Role::Client);

        // Role parsing accepts both spellings and rejects others.
        assert_eq!(Role::parse("client").unwrap(), Role::Client);
        assert_eq!(Role::parse("OPERATOR").unwrap(), Role::Operator);
        assert!(Role::parse("root").is_err());
    }
}
