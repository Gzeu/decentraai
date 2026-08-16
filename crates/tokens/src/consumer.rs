//! Consumer API keys (`dca_…`) for the Compute Contribution & Quota model
//! (Q2 — Consumer API Keys).
//!
//! # What a consumer key is
//!
//! A consumer key is an **access credential + quota ceiling** for an agent,
//! application or API client that consumes fabric compute. It does **not**
//! create quota — it is only an authorization layer on top of the existing
//! quota ledger ([`decentraai_compute::QuotaLedger`]). The key names its
//! owner account; that account holds the authoritative balance (earned from
//! measured contribution). The key can never let its caller consume more than
//! `min(account available, key quota_ceiling)`.
//!
//! # Security model (mirrors the existing `dsk_` subscription registry)
//!
//! - The plaintext `dca_…` secret is **shown exactly once at creation** and
//!   never stored; only its BLAKE3 hash is persisted (the on-disk map is keyed
//!   by the hash, exactly like the subscription registry).
//! - A leaked key store reveals nothing usable (only hashes).
//! - Keys are **revocable** by key id; a revoked key stops authenticating.
//! - A consumer key is strictly an inference credential: it can never grant
//!   admin/operator/master privileges (that boundary lives in the runtime
//!   `Auth` classification, not here).
//! - Secrets never appear in list metadata — only a short display prefix
//!   (e.g. `dca_ab12…`) so operators can recognize a key without leaking it.
//!
//! # Rate limit vs quota
//!
//! These are independent and both live on the key:
//!
//! - **rate limit** = how often the key may *request* (requests/minute);
//! - **quota ceiling** = how much compute the key may *consume* per request
//!   (units), capped further by the account's available balance.
//!
//! The quota ledger remains the single source of truth for balances; this
//! module only stores the authorization metadata.

use anyhow::{Context, Result, bail};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Key prefix distinguishing consumer API keys from `dsk_` subscription
/// tokens and the master token. `dca_` is the roadmap's consumer namespace.
pub const KEY_PREFIX: &str = "dca_";

/// Length of the display prefix shown in metadata (`dca_` + 4 hex chars), so
/// operators can recognize a key without leaking its secret.
pub const PREFIX_LEN: usize = 8;

/// One consumer API key record. The plaintext secret is never stored — only
/// its BLAKE3 hash (as the map key) — and never serialized in list metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumerKeyRecord {
    /// Stable key id (e.g. `ck-…`). The admin references a key by this id.
    pub key_id: String,
    /// Short display prefix (`dca_ab12…`) for recognition; NOT the secret.
    pub prefix: String,
    /// Owner account in the quota ledger (existing account identity — the
    /// same `AccountId` the provider/worker accounts use). The key draws
    /// quota from this account; it never holds its own balance.
    pub owner_account: String,
    /// Unix seconds the key was created.
    pub created_at: u64,
    /// Unix seconds the key was last used to authenticate a request.
    /// `None` = never used.
    pub last_used_at: Option<u64>,
    /// Whether the key was revoked (stops authenticating).
    pub revoked: bool,
    /// Per-request quota ceiling in quota units. A request may consume at
    /// most `min(account.available, quota_ceiling)`.
    pub quota_ceiling: u64,
    /// Per-key rate limit: max requests per minute. Independent of quota.
    pub rate_limit_per_minute: u32,
    /// Permission scopes (e.g. `["inference"]`). Currently informational /
    /// future: the key always runs inference within its ceiling. Kept as an
    /// explicit, inspectable field so scope policy can be enforced without a
    /// schema change.
    pub scopes: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct KeyFile {
    schema_version: u32,
    /// keyed by BLAKE3 hash of the plaintext (mirrors the subscription
    /// registry: a leaked file reveals only hashes).
    keys: BTreeMap<String, ConsumerKeyRecord>,
}

/// The consumer-key registry: `hash(plaintext) -> record`, persisted
/// atomically. Lookup hashes the presented token and reads the map directly.
pub struct ConsumerKeyStore {
    keys: BTreeMap<String, ConsumerKeyRecord>,
    path: PathBuf,
}

/// Hashes a plaintext key for storage and lookup.
pub fn hash_key(plaintext: &str) -> String {
    blake3::hash(plaintext.as_bytes()).to_hex().to_string()
}

/// Short recognisable prefix of a plaintext key (e.g. `dca_ab12`).
pub fn key_prefix(plaintext: &str) -> String {
    plaintext.chars().take(PREFIX_LEN).collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl ConsumerKeyStore {
    /// Loads the registry. A missing file starts empty; a corrupted file
    /// starts fresh with a warning (availability over strictness).
    pub fn load(path: &Path) -> Result<Self> {
        let keys = match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<KeyFile>(&content) {
                Ok(file) => file.keys,
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "corrupt consumer key registry, starting fresh");
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).context("reading consumer key registry"),
        };
        Ok(Self {
            keys,
            path: path.to_path_buf(),
        })
    }

    /// Persists atomically (tmp + sync + rename).
    pub fn save(&self) -> Result<()> {
        let file = KeyFile {
            schema_version: 1,
            keys: self.keys.clone(),
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

    /// Issues a new consumer API key for `owner_account`. Returns the
    /// plaintext secret — show it once, then forget it; only its hash and a
    /// short display prefix are kept. `quota_ceiling` bounds per-request
    /// consumption; `rate_limit_per_minute` bounds request frequency;
    /// `scopes` names permitted capabilities (informational for now).
    pub fn create(
        &mut self,
        owner_account: &str,
        quota_ceiling: u64,
        rate_limit_per_minute: u32,
        scopes: Vec<String>,
    ) -> Result<String> {
        let owner_account = owner_account.trim();
        if owner_account.is_empty() {
            bail!("owner account must not be empty");
        }
        if quota_ceiling == 0 {
            bail!("quota_ceiling must be > 0");
        }
        if rate_limit_per_minute == 0 {
            bail!("rate_limit_per_minute must be > 0");
        }
        let mut bytes = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut bytes);
        let plaintext = format!("{KEY_PREFIX}{}", hex::encode(bytes));
        let key_id = format!("ck-{}", hex::encode(&bytes[..4]));
        let record = ConsumerKeyRecord {
            key_id: key_id.clone(),
            prefix: key_prefix(&plaintext),
            owner_account: owner_account.to_string(),
            created_at: now_secs(),
            last_used_at: None,
            revoked: false,
            quota_ceiling,
            rate_limit_per_minute,
            scopes,
        };
        let hash = hash_key(&plaintext);
        self.keys.insert(hash, record);
        self.save()?;
        Ok(plaintext)
    }

    /// Resolves a plaintext key to its record, if active (not revoked). A
    /// revoked key does not resolve, so a stolen/revoked secret stops working.
    pub fn lookup(&self, plaintext: &str) -> Option<&ConsumerKeyRecord> {
        self.keys.get(&hash_key(plaintext)).filter(|r| !r.revoked)
    }

    /// Marks a key as used (updates `last_used_at`), if it exists. Best-effort
    /// on persistence: a failed write only logs, never breaks the request.
    pub fn touch_used(&mut self, key_id: &str) {
        if let Some(rec) = self.keys.values_mut().find(|r| r.key_id == key_id) {
            rec.last_used_at = Some(now_secs());
            if self.save().is_err() {
                tracing::warn!("failed to persist consumer key last_used_at");
            }
        }
    }

    /// Revokes a key by id; it stops authenticating immediately.
    pub fn revoke(&mut self, key_id: &str) -> Result<()> {
        let rec = self
            .keys
            .values_mut()
            .find(|r| r.key_id == key_id && !r.revoked)
            .with_context(|| format!("no active consumer key '{key_id}'"))?;
        rec.revoked = true;
        self.save()
    }

    /// All records (active and revoked), newest first. Metadata only — the
    /// plaintext secret never appears here.
    pub fn list(&self) -> Vec<ConsumerKeyRecord> {
        let mut out: Vec<ConsumerKeyRecord> = self.keys.values().cloned().collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(dir: &Path) -> ConsumerKeyStore {
        ConsumerKeyStore::load(&dir.join("consumer_keys.json")).unwrap()
    }

    #[test]
    fn create_shows_secret_once_and_stores_only_hash_and_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plaintext = store
            .create("acct-1", 100, 10, vec!["inference".to_string()])
            .unwrap();
        assert!(plaintext.starts_with(KEY_PREFIX));
        assert_eq!(plaintext.len(), KEY_PREFIX.len() + 64);

        let on_disk = std::fs::read_to_string(dir.path().join("consumer_keys.json")).unwrap();
        assert!(
            !on_disk.contains(&plaintext),
            "plaintext must never be persisted"
        );
        // The short display prefix is fine; the full secret is not.
        assert!(on_disk.contains(&key_prefix(&plaintext)));

        let rec = store.lookup(&plaintext).unwrap();
        assert_eq!(rec.owner_account, "acct-1");
        assert_eq!(rec.quota_ceiling, 100);
        assert_eq!(rec.rate_limit_per_minute, 10);
    }

    #[test]
    fn invalid_key_does_not_resolve() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        assert!(store.lookup("dca_0000000000000000").is_none());
    }

    #[test]
    fn revoked_key_stops_resolving() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plaintext = store.create("acct-1", 100, 10, vec![]).unwrap();
        let key_id = store.lookup(&plaintext).unwrap().key_id.clone();
        store.revoke(&key_id).unwrap();
        assert!(
            store.lookup(&plaintext).is_none(),
            "revoked key must not authenticate"
        );
        // Record stays listed as revoked for the admin.
        let listed = store.list();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].revoked);
    }

    #[test]
    fn metadata_never_leaks_the_secret() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plaintext = store.create("acct-1", 50, 5, vec![]).unwrap();
        let listed = store.list();
        let json = serde_json::to_string(&listed).unwrap();
        assert!(
            !json.contains(&plaintext),
            "list metadata must not contain the plaintext secret"
        );
        // Only the short display prefix appears (dca_ + 4 hex), never the
        // full 64-hex secret body.
        assert!(json.contains(&key_prefix(&plaintext)));
        let secret_body = &plaintext[KEY_PREFIX.len()..];
        assert!(
            !json.contains(secret_body),
            "the full secret body must not appear in metadata"
        );
    }

    #[test]
    fn registry_survives_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let plaintext;
        {
            let mut store = open(dir.path());
            plaintext = store.create("acct-1", 200, 20, vec![]).unwrap();
        }
        let reloaded = open(dir.path());
        let rec = reloaded.lookup(&plaintext).unwrap();
        assert_eq!(rec.quota_ceiling, 200);
        assert_eq!(rec.owner_account, "acct-1");
    }

    #[test]
    fn rejects_zero_ceiling_or_rate() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        assert!(store.create("acct-1", 0, 10, vec![]).is_err());
        assert!(store.create("acct-1", 100, 0, vec![]).is_err());
    }

    #[test]
    fn touch_used_records_last_used() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plaintext = store.create("acct-1", 100, 10, vec![]).unwrap();
        let key_id = store.lookup(&plaintext).unwrap().key_id.clone();
        assert!(store.lookup(&plaintext).unwrap().last_used_at.is_none());
        store.touch_used(&key_id);
        assert!(store.lookup(&plaintext).unwrap().last_used_at.is_some());
    }

    #[test]
    fn corrupt_registry_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("consumer_keys.json"), b"not json").unwrap();
        let store = open(dir.path());
        assert!(store.list().is_empty());
    }
}
