//! External demand signal — nodes announce what compute they need.
//!
//! Currently the fabric advertises **supply** (ComputeAdvertisement:
//! what a node can provide). Demand is implicit (AssistRequest is
//! transient, created per-request). This module adds a **persistent
//! demand signal** that nodes can announce to the network.
//!
//! A demand signal says: "I need capability X with Y resources for Z
//! seconds." Other nodes (or coordinators) can read these signals to
//! understand network demand and proactively offer resources.
//!
//! Storage: local JSON file, atomic writes. No P2P broadcast yet —
//! demand is node-local; future work can propagate via gossip.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Status of a demand signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemandStatus {
    /// Active — waiting for offers.
    Pending,
    /// Fulfilled — at least one offer was accepted.
    Fulfilled,
    /// Cancelled by the requester.
    Cancelled,
    /// Expired (past TTL without fulfillment).
    Expired,
}

/// A single demand signal: "I need this compute."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemandSignal {
    /// Stable id (e.g. `dem-{uuid}`).
    pub id: String,
    /// Capability needed (snake_case, from hub taxonomy).
    pub capability: String,
    /// CPU cores needed (0 = any).
    #[serde(default)]
    pub cpu_cores: u16,
    /// RAM needed in MB (0 = any).
    #[serde(default)]
    pub ram_mb: u64,
    /// VRAM needed in MB (0 = any / CPU-only).
    #[serde(default)]
    pub vram_mb: u64,
    /// Estimated duration in seconds (0 = unknown).
    #[serde(default)]
    pub duration_secs: u64,
    /// Maximum price per unit the requester is willing to pay (0 = free / any).
    #[serde(default)]
    pub max_price_per_unit: u64,
    /// Optional model hash needed (empty = any model for this capability).
    #[serde(default)]
    pub model_hash: String,
    /// Unix epoch seconds when this signal was created.
    pub created_at: u64,
    /// Unix epoch seconds when this signal expires (0 = never).
    #[serde(default)]
    pub expires_at: u64,
    /// Current status.
    pub status: DemandStatus,
    /// Free-text reason (for operator visibility).
    #[serde(default)]
    pub reason: String,
}

/// Node-local demand store. Persists to a JSON file.
pub struct DemandStore {
    demands: BTreeMap<String, DemandSignal>,
    path: PathBuf,
}

impl DemandStore {
    /// Opens or creates a demand store.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join("demand_signals.json");
        let demands = match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => BTreeMap::new(),
        };
        Self { demands, path }
    }

    /// Persists atomically (tmp + sync + rename).
    fn save(&self) -> std::io::Result<()> {
        let content = serde_json::to_string_pretty(&self.demands)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &content)?;
        let f = std::fs::File::open(&tmp)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Announces a new demand signal. Returns the assigned id.
    #[allow(clippy::too_many_arguments)]
    pub fn announce(
        &mut self,
        capability: String,
        cpu_cores: u16,
        ram_mb: u64,
        vram_mb: u64,
        duration_secs: u64,
        max_price_per_unit: u64,
        model_hash: String,
        ttl_secs: u64,
        reason: String,
    ) -> Result<String, String> {
        let now = now_secs();
        let id = format!("dem-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        let expires_at = if ttl_secs > 0 {
            now + ttl_secs
        } else {
            0
        };
        let signal = DemandSignal {
            id: id.clone(),
            capability,
            cpu_cores,
            ram_mb,
            vram_mb,
            duration_secs,
            max_price_per_unit,
            model_hash,
            created_at: now,
            expires_at,
            status: DemandStatus::Pending,
            reason,
        };
        self.demands.insert(id.clone(), signal);
        let _ = self.save();
        Ok(id)
    }

    /// Cancels a demand signal.
    pub fn cancel(&mut self, id: &str) -> Result<(), String> {
        let signal = self
            .demands
            .get_mut(id)
            .ok_or_else(|| format!("demand {id} not found"))?;
        if signal.status != DemandStatus::Pending {
            return Err(format!("demand {id} is not pending (status: {:?})", signal.status));
        }
        signal.status = DemandStatus::Cancelled;
        let _ = self.save();
        Ok(())
    }

    /// Marks a demand as fulfilled.
    pub fn fulfill(&mut self, id: &str) -> Result<(), String> {
        let signal = self
            .demands
            .get_mut(id)
            .ok_or_else(|| format!("demand {id} not found"))?;
        signal.status = DemandStatus::Fulfilled;
        let _ = self.save();
        Ok(())
    }

    /// Prunes expired demands. Returns count of expired signals.
    pub fn prune_expired(&mut self) -> usize {
        let now = now_secs();
        let mut count = 0;
        for signal in self.demands.values_mut() {
            if signal.status == DemandStatus::Pending
                && signal.expires_at > 0
                && now > signal.expires_at
            {
                signal.status = DemandStatus::Expired;
                count += 1;
            }
        }
        if count > 0 {
            let _ = self.save();
        }
        count
    }

    /// Lists all demand signals, optionally filtered by status.
    pub fn list(&self, status_filter: Option<&DemandStatus>) -> Vec<&DemandSignal> {
        self.demands
            .values()
            .filter(|s| status_filter.is_none_or(|f| &s.status == f))
            .collect()
    }

    /// Gets a specific demand signal.
    pub fn get(&self, id: &str) -> Option<&DemandSignal> {
        self.demands.get(id)
    }

    /// Summary stats.
    pub fn summary(&self) -> DemandSummary {
        let mut pending = 0u64;
        let mut fulfilled = 0u64;
        let mut cancelled = 0u64;
        let mut expired = 0u64;
        for signal in self.demands.values() {
            match signal.status {
                DemandStatus::Pending => pending += 1,
                DemandStatus::Fulfilled => fulfilled += 1,
                DemandStatus::Cancelled => cancelled += 1,
                DemandStatus::Expired => expired += 1,
            }
        }
        DemandSummary {
            total: self.demands.len() as u64,
            pending,
            fulfilled,
            cancelled,
            expired,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemandSummary {
    pub total: u64,
    pub pending: u64,
    pub fulfilled: u64,
    pub cancelled: u64,
    pub expired: u64,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (DemandStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = DemandStore::open(dir.path());
        (store, dir)
    }

    #[test]
    fn announce_and_list() {
        let (mut store, _dir) = temp_store();
        let id = store
            .announce("inference".into(), 4, 2048, 0, 3600, 0, "".into(), 7200, "need help".into())
            .unwrap();
        assert!(id.starts_with("dem-"));
        let signals = store.list(None);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].capability, "inference");
        assert_eq!(signals[0].status, DemandStatus::Pending);
    }

    #[test]
    fn cancel_demand() {
        let (mut store, _dir) = temp_store();
        let id = store.announce("ocr".into(), 0, 0, 0, 0, 0, "".into(), 0, "".into()).unwrap();
        store.cancel(&id).unwrap();
        assert_eq!(store.get(&id).unwrap().status, DemandStatus::Cancelled);
    }

    #[test]
    fn fulfill_demand() {
        let (mut store, _dir) = temp_store();
        let id = store.announce("ocr".into(), 0, 0, 0, 0, 0, "".into(), 0, "".into()).unwrap();
        store.fulfill(&id).unwrap();
        assert_eq!(store.get(&id).unwrap().status, DemandStatus::Fulfilled);
    }

    #[test]
    fn prune_expired() {
        let (mut store, _dir) = temp_store();
        // TTL of 1 second, already expired
        let id = store.announce("ocr".into(), 0, 0, 0, 0, 0, "".into(), 1, "".into()).unwrap();
        // Simulate expiry by backdating created_at
        store.demands.get_mut(&id).unwrap().expires_at = 1;
        let count = store.prune_expired();
        assert_eq!(count, 1);
        assert_eq!(store.get(&id).unwrap().status, DemandStatus::Expired);
    }

    #[test]
    fn summary() {
        let (mut store, _dir) = temp_store();
        let id1 = store.announce("ocr".into(), 0, 0, 0, 0, 0, "".into(), 0, "".into()).unwrap();
        let _id2 = store.announce("inference".into(), 0, 0, 0, 0, 0, "".into(), 0, "".into()).unwrap();
        store.fulfill(&id1).unwrap();
        let s = store.summary();
        assert_eq!(s.total, 2);
        assert_eq!(s.pending, 1);
        assert_eq!(s.fulfilled, 1);
    }

    #[test]
    fn cannot_cancel_fulfilled() {
        let (mut store, _dir) = temp_store();
        let id = store.announce("ocr".into(), 0, 0, 0, 0, 0, "".into(), 0, "".into()).unwrap();
        store.fulfill(&id).unwrap();
        assert!(store.cancel(&id).is_err());
    }
}
