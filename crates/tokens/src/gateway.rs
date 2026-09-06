//! BYOA agent credentials (`dga_…`) for the M16 Agent Gateway.
//!
//! Mirrors the proven `dca_` consumer-key discipline line for line —
//! BLAKE3-hashed map, atomic persistence, show-once plaintext, revocation
//! by id — with the agent differences REQUIRED by the scope:
//!
//! - distinct `dga_` prefix (never `dca_`, never `dsk_`; key ids `gk-…`);
//! - expiry is MANDATORY (`u64`, future-only; no never-expiring agents);
//! - `capabilities` are hub-taxonomy names (validated at issuance in the
//!   CLI; stored verbatim here) instead of free-form scopes;
//! - `owner_account` stays REQUIRED so quota draws on the authoritative
//!   ledger through the exact same path as `dca_` (no second ledger,
//!   no new money semantics).
//!
//! The plaintext secret is shown exactly once at creation and never
//! stored; only its hash and a short display prefix are kept.

use anyhow::{Context, Result, bail};
use rand_core::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::consumer::hash_key;

/// Key prefix distinguishing gateway agent credentials from `dca_`
/// consumer keys, `dsk_` subscription tokens and the master token.
pub const GATEWAY_KEY_PREFIX: &str = "dga_";

/// Length of the display prefix shown in metadata (`dga_` + 4 hex chars).
pub const GATEWAY_PREFIX_LEN: usize = 8;

/// One gateway agent credential record. Plaintext never stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayKeyRecord {
    /// Stable key id (`gk-…`). Revocation handle; safe to log.
    pub key_id: String,
    /// Short display prefix (`dga_ab12…`); NOT the secret.
    pub prefix: String,
    /// Agent display name.
    pub agent_name: String,
    /// Granted hub-taxonomy capability names (non-empty).
    pub capabilities: Vec<String>,
    /// Owner account in the quota ledger (same `AccountId` semantics as
    /// `dca_` — the credential draws quota from this account).
    pub owner_account: String,
    /// Unix seconds the credential was created.
    pub created_at: u64,
    /// Unix seconds last used. `None` = never used.
    pub last_used_at: Option<u64>,
    /// Whether revoked (stops authenticating immediately).
    pub revoked: bool,
    /// Per-request quota ceiling in quota units.
    pub quota_ceiling: u64,
    /// Max requests per minute.
    pub rate_limit_per_minute: u32,
    /// MANDATORY expiry Unix seconds. Enforced at auth time.
    pub expires_at: u64,
}

#[derive(Serialize, Deserialize)]
struct GatewayKeyFile {
    schema_version: u32,
    /// keyed by BLAKE3 hash of the plaintext (a leaked file reveals hashes).
    keys: BTreeMap<String, GatewayKeyRecord>,
}

/// The gateway credential registry: `hash(plaintext) -> record`.
pub struct GatewayKeyStore {
    keys: BTreeMap<String, GatewayKeyRecord>,
    path: PathBuf,
}

/// Short recognisable prefix of a plaintext key (e.g. `dga_ab12`).
pub fn gateway_key_prefix(plaintext: &str) -> String {
    plaintext.chars().take(GATEWAY_PREFIX_LEN).collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl GatewayKeyStore {
    /// Loads the registry. Missing file starts empty; corrupt file starts
    /// fresh with a warning (same availability posture as consumer keys).
    pub fn load(path: &Path) -> Result<Self> {
        let keys = match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<GatewayKeyFile>(&content) {
                Ok(file) => file.keys,
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "corrupt gateway key registry, starting fresh");
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).context("reading gateway key registry"),
        };
        Ok(Self {
            keys,
            path: path.to_path_buf(),
        })
    }

    /// Persists atomically (tmp + sync + rename).
    pub fn save(&self) -> Result<()> {
        let file = GatewayKeyFile {
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

    /// Issues a gateway credential. Returns the plaintext secret — show it
    /// once, then forget it. Expiry is mandatory and must be future.
    pub fn create(
        &mut self,
        agent_name: &str,
        capabilities: Vec<String>,
        owner_account: &str,
        quota_ceiling: u64,
        rate_limit_per_minute: u32,
        expires_at: u64,
    ) -> Result<String> {
        let agent_name = agent_name.trim();
        if agent_name.is_empty() {
            bail!("agent name must not be empty");
        }
        if capabilities.is_empty() {
            bail!("capabilities must not be empty (deny-by-default needs a grant)");
        }
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
        if expires_at <= now_secs() {
            bail!("expires_at must be in the future (agent credentials never live forever)");
        }
        let mut bytes = [0u8; 32];
        rand_core::OsRng.fill_bytes(&mut bytes);
        let plaintext = format!("{GATEWAY_KEY_PREFIX}{}", hex::encode(bytes));
        let key_id = format!("gk-{}", hex::encode(&bytes[..4]));
        let record = GatewayKeyRecord {
            key_id: key_id.clone(),
            prefix: gateway_key_prefix(&plaintext),
            agent_name: agent_name.to_string(),
            capabilities,
            owner_account: owner_account.to_string(),
            created_at: now_secs(),
            last_used_at: None,
            revoked: false,
            quota_ceiling,
            rate_limit_per_minute,
            expires_at,
        };
        let hash = hash_key(&plaintext);
        self.keys.insert(hash, record);
        self.save()?;
        Ok(plaintext)
    }

    /// Resolves a plaintext key to its record, if active (not revoked, not
    /// expired). Expired and revoked share this single inactive path.
    pub fn lookup(&self, plaintext: &str) -> Option<&GatewayKeyRecord> {
        self.keys.get(&hash_key(plaintext)).filter(|r| {
            if r.revoked {
                return false;
            }
            now_secs() < r.expires_at
        })
    }

    /// Marks a key as used (updates `last_used_at`). Best-effort persist.
    pub fn touch_used(&mut self, key_id: &str) {
        if let Some(rec) = self.keys.values_mut().find(|r| r.key_id == key_id) {
            rec.last_used_at = Some(now_secs());
            if self.save().is_err() {
                tracing::warn!("failed to persist gateway key last_used_at");
            }
        }
    }

    /// Revokes a key by id; immediate effect (auth resolves live store).
    pub fn revoke(&mut self, key_id: &str) -> Result<()> {
        let rec = self
            .keys
            .values_mut()
            .find(|r| r.key_id == key_id && !r.revoked)
            .with_context(|| format!("no active gateway credential '{key_id}'"))?;
        rec.revoked = true;
        self.save()
    }

    /// All records (active and revoked), newest first. Metadata only.
    pub fn list(&self) -> Vec<GatewayKeyRecord> {
        let mut out: Vec<GatewayKeyRecord> = self.keys.values().cloned().collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.created_at));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(dir: &Path) -> GatewayKeyStore {
        GatewayKeyStore::load(&dir.join("gateway_keys.json")).unwrap()
    }

    fn future() -> u64 {
        now_secs() + 7 * 86_400
    }

    #[test]
    fn create_shows_secret_once_and_stores_only_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let plaintext = store
            .create(
                "ext-1",
                vec!["ocr".to_string()],
                "acct-1",
                100,
                60,
                future(),
            )
            .unwrap();
        assert!(plaintext.starts_with(GATEWAY_KEY_PREFIX));
        assert!(!plaintext.starts_with("dca_") && !plaintext.starts_with("dsk_"));
        assert_eq!(plaintext.len(), GATEWAY_KEY_PREFIX.len() + 64);
        let raw = std::fs::read_to_string(dir.path().join("gateway_keys.json")).unwrap();
        assert!(!raw.contains(&plaintext), "plaintext must never be stored");
        let rec = store.lookup(&plaintext).expect("fresh credential resolves");
        assert!(rec.key_id.starts_with("gk-"));
        assert_eq!(rec.prefix.len(), GATEWAY_PREFIX_LEN);
        assert_eq!(rec.capabilities, vec!["ocr".to_string()]);
    }

    #[test]
    fn expiry_is_mandatory_and_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        assert!(
            store
                .create("x", vec!["ocr".to_string()], "a", 1, 1, now_secs() - 1)
                .is_err()
        );
        let pt = store
            .create("x", vec!["ocr".to_string()], "a", 1, 1, now_secs() + 30)
            .unwrap();
        assert!(store.lookup(&pt).is_some());
    }

    #[test]
    fn empty_grant_empty_account_zero_quota_zero_rate_all_fail() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        assert!(store.create("x", vec![], "a", 1, 1, future()).is_err());
        assert!(
            store
                .create("x", vec!["ocr".to_string()], "", 1, 1, future())
                .is_err()
        );
        assert!(
            store
                .create("x", vec!["ocr".to_string()], "a", 0, 1, future())
                .is_err()
        );
        assert!(
            store
                .create("x", vec!["ocr".to_string()], "a", 1, 0, future())
                .is_err()
        );
    }

    #[test]
    fn revoke_is_immediate() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open(dir.path());
        let pt = store
            .create("x", vec!["ocr".to_string()], "a", 1, 1, future())
            .unwrap();
        let id = store.lookup(&pt).unwrap().key_id.clone();
        store.revoke(&id).unwrap();
        assert!(store.lookup(&pt).is_none());
        assert!(store.revoke(&id).is_err(), "double revoke fails closed");
    }
}
