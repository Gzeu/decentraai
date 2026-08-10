//! Local peer reputation (M5a) and deterministic provider ranking (M5c):
//! per-peer track records, temporary bans for repeated invalid chunks,
//! and atomic JSON persistence.
//!
//! Only cryptographic verification failures count toward the ban
//! threshold: a flaky connection is not proof of malice, a corrupted
//! chunk is. Network errors propagate without touching the score.

use anyhow::{Context, Result};
use libp2p::PeerId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::warn;

/// Per-peer track record, persisted between runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeerScore {
    pub verified_chunks: u64,
    pub failed_chunks: u64,
    /// Unix timestamp until which the peer is banned, if any.
    pub banned_until: Option<u64>,
}

/// Serializable snapshot of one peer for the dashboard (M7b).
#[derive(Debug, Clone, Serialize)]
pub struct PeerSummary {
    pub peer_id: String,
    pub verified: u64,
    pub failed: u64,
    pub score: f64,
    pub banned: bool,
}

#[derive(Serialize, Deserialize)]
struct ReputationFile {
    schema_version: u32,
    scores: BTreeMap<String, PeerScore>,
}

/// In-memory scores plus the path they persist to.
pub struct ReputationStore {
    scores: BTreeMap<String, PeerScore>,
    path: PathBuf,
    max_invalid_chunks: u8,
    ban_duration: Duration,
}

impl ReputationStore {
    /// Loads scores from `path`. A missing file starts empty; a corrupted
    /// file starts fresh with a warning, because download availability
    /// beats strictness of what is essentially a cache.
    pub fn load(path: &Path, max_invalid_chunks: u8, ban_duration: Duration) -> Result<Self> {
        let scores = match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<ReputationFile>(&content) {
                Ok(file) => file.scores,
                Err(e) => {
                    warn!(error = %e, path = %path.display(), "corrupt reputation file, starting fresh");
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e).context("reading reputation file"),
        };
        Ok(Self {
            scores,
            path: path.to_path_buf(),
            max_invalid_chunks,
            ban_duration,
        })
    }

    /// Persists scores atomically (tmp + sync + rename), like the registry.
    pub fn save(&self) -> Result<()> {
        let file = ReputationFile {
            schema_version: 1,
            scores: self.scores.clone(),
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

    /// A chunk from this peer passed BLAKE3 verification.
    pub fn record_success(&mut self, peer: &PeerId) {
        self.scores
            .entry(peer.to_string())
            .or_default()
            .verified_chunks += 1;
    }

    /// A chunk from this peer failed verification. Reaching the configured
    /// threshold bans the peer for the configured duration and resets the
    /// counter, so the score can recover after the ban expires.
    pub fn record_failure(&mut self, peer: &PeerId) {
        let entry = self.scores.entry(peer.to_string()).or_default();
        entry.failed_chunks += 1;
        if entry.failed_chunks >= u64::from(self.max_invalid_chunks) {
            entry.banned_until = Some(now_secs() + self.ban_duration.as_secs());
            entry.failed_chunks = 0;
            warn!(%peer, "peer banned for repeated invalid chunks");
        }
    }

    /// The ban expiry timestamp if the peer is banned right now.
    pub fn banned_until(&self, peer: &PeerId) -> Option<u64> {
        let until = self.scores.get(&peer.to_string())?.banned_until?;
        if until > now_secs() { Some(until) } else { None }
    }

    pub fn is_banned(&self, peer: &PeerId) -> bool {
        self.banned_until(peer).is_some()
    }

    /// Ranking input for the M5c scheduler: successes minus weighted failures.
    pub fn score(&self, peer: &PeerId) -> f64 {
        self.scores
            .get(&peer.to_string())
            .map(|e| e.verified_chunks as f64 - e.failed_chunks as f64 * 2.0)
            .unwrap_or(0.0)
    }

    pub fn successes(&self, peer: &PeerId) -> u64 {
        self.scores
            .get(&peer.to_string())
            .map(|e| e.verified_chunks)
            .unwrap_or(0)
    }

    pub fn failures(&self, peer: &PeerId) -> u64 {
        self.scores
            .get(&peer.to_string())
            .map(|e| e.failed_chunks)
            .unwrap_or(0)
    }

    /// Dashboard view: every tracked peer, sorted by score descending
    /// (ties by PeerId ascending, matching the scheduler's determinism).
    pub fn summaries(&self) -> Vec<PeerSummary> {
        let mut out: Vec<PeerSummary> = self
            .scores
            .iter()
            .map(|(peer_id, entry)| {
                let banned = entry
                    .banned_until
                    .is_some_and(|until| until > now_secs());
                PeerSummary {
                    peer_id: peer_id.clone(),
                    verified: entry.verified_chunks,
                    failed: entry.failed_chunks,
                    score: entry.verified_chunks as f64 - entry.failed_chunks as f64 * 2.0,
                    banned,
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.peer_id.cmp(&b.peer_id))
        });
        out
    }
}

/// Orders providers for a multi-provider download: eligible (non-banned)
/// peers sorted by score descending, ties broken by PeerId ascending.
/// The same peer set always produces the same order — determinism is a
/// hard requirement for reproducible scheduling.
pub fn rank_providers(peers: &[PeerId], reputation: &ReputationStore) -> Vec<PeerId> {
    let mut eligible: Vec<PeerId> = peers
        .iter()
        .filter(|peer| !reputation.is_banned(peer))
        .copied()
        .collect();
    eligible.sort_by(|a, b| {
        reputation
            .score(b)
            .partial_cmp(&reputation.score(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.to_string().cmp(&b.to_string()))
    });
    eligible
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

    fn open_store(dir: &Path, threshold: u8, ban_secs: u64) -> ReputationStore {
        ReputationStore::load(
            &dir.join("reputation.json"),
            threshold,
            Duration::from_secs(ban_secs),
        )
        .unwrap()
    }

    #[test]
    fn unknown_peer_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(dir.path(), 2, 60);
        let peer = PeerId::random();
        assert!(!store.is_banned(&peer));
        assert_eq!(store.score(&peer), 0.0);
        assert_eq!(store.successes(&peer), 0);
    }

    #[test]
    fn successes_raise_score() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(dir.path(), 2, 60);
        let peer = PeerId::random();
        for _ in 0..3 {
            store.record_success(&peer);
        }
        assert_eq!(store.score(&peer), 3.0);
        assert_eq!(store.successes(&peer), 3);
    }

    #[test]
    fn failures_below_threshold_do_not_ban() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(dir.path(), 2, 60);
        let peer = PeerId::random();
        store.record_failure(&peer);
        assert!(!store.is_banned(&peer));
        assert_eq!(store.failures(&peer), 1);
        assert_eq!(store.score(&peer), -2.0);
    }

    #[test]
    fn reaching_threshold_bans_and_resets_counter() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(dir.path(), 2, 3600);
        let peer = PeerId::random();
        store.record_failure(&peer);
        store.record_failure(&peer);
        assert!(store.is_banned(&peer));
        assert!(store.banned_until(&peer).is_some());
        assert_eq!(store.failures(&peer), 0, "counter resets so the score can recover");
    }

    #[test]
    fn ban_expires_after_duration() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(dir.path(), 1, 0);
        let peer = PeerId::random();
        store.record_failure(&peer);
        assert!(!store.is_banned(&peer), "a zero-length ban expires immediately");
    }

    #[test]
    fn scores_persist_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let peer = PeerId::random();
        {
            let mut store = open_store(dir.path(), 10, 60);
            store.record_success(&peer);
            store.record_failure(&peer);
            store.save().unwrap();
        }
        let reloaded = open_store(dir.path(), 10, 60);
        assert_eq!(reloaded.successes(&peer), 1);
        assert_eq!(reloaded.failures(&peer), 1);
    }

    #[test]
    fn corrupt_file_starts_fresh() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("reputation.json"), b"not json").unwrap();
        let store = open_store(dir.path(), 2, 60);
        assert_eq!(store.score(&PeerId::random()), 0.0);
    }

    #[test]
    fn ranking_excludes_banned_peers() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(dir.path(), 1, 3600);
        let good = PeerId::random();
        let banned = PeerId::random();
        store.record_failure(&banned);
        let ranked = rank_providers(&[good, banned], &store);
        assert_eq!(ranked, vec![good]);
    }

    #[test]
    fn ranking_prefers_higher_scores() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(dir.path(), 10, 60);
        let veteran = PeerId::random();
        let newcomer = PeerId::random();
        store.record_success(&veteran);
        store.record_success(&veteran);
        let ranked = rank_providers(&[newcomer, veteran], &store);
        assert_eq!(ranked, vec![veteran, newcomer]);
    }

    #[test]
    fn ranking_is_deterministic_on_ties() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(dir.path(), 10, 60);
        let a = PeerId::random();
        let b = PeerId::random();
        let first = rank_providers(&[a, b], &store);
        let second = rank_providers(&[b, a], &store);
        assert_eq!(first, second, "tie-breaking must not depend on input order");
        let mut expected = vec![a, b];
        expected.sort_by_key(|p| p.to_string());
        assert_eq!(first, expected, "ties break by PeerId ascending");
    }

    #[test]
    fn summaries_cover_every_peer_sorted_by_score() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = open_store(dir.path(), 1, 3600);
        let good = PeerId::random();
        let bad = PeerId::random();
        store.record_success(&good);
        store.record_success(&good);
        store.record_failure(&bad);
        let summaries = store.summaries();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].peer_id, good.to_string(), "highest score first");
        assert_eq!(summaries[0].score, 2.0);
        assert!(!summaries[0].banned);
        assert_eq!(summaries[1].peer_id, bad.to_string());
        assert!(summaries[1].banned);
    }
}
