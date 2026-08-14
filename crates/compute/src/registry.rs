//! Coordinator-side store of the latest compute advertisement per peer.
//!
//! This is the **ComputeRegistry**: who is on the network with what
//! hardware and how healthy they currently are. It is deliberately distinct
//! from `ModelRegistry` (which tracks artifacts on one node). A compute
//! node may advertise models it serves; a coordinator aggregates those into
//! a network-wide compute view.

use std::collections::HashMap;
use std::time::Instant;

use libp2p::PeerId;

use crate::availability::{ComputeAdvertisement, WorkerHealth};

/// Aggregates advertisements from every known compute peer.
#[derive(Debug, Clone)]
pub struct ComputeRegistry {
    workers: HashMap<PeerId, ComputeAdvertisement>,
    last_seen: HashMap<PeerId, Instant>,
    stale_after: std::time::Duration,
}

impl ComputeRegistry {
    /// `stale_after` is the heartbeat gap after which a peer is considered
    /// offline (its stored advertisement is flipped to `Offline`).
    pub fn new(stale_after: std::time::Duration) -> Self {
        Self {
            workers: HashMap::new(),
            last_seen: HashMap::new(),
            stale_after,
        }
    }

    pub fn len(&self) -> usize {
        self.workers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.workers.is_empty()
    }

    /// Records (or replaces) the latest advertisement for a peer.
    pub fn upsert(&mut self, adv: ComputeAdvertisement, now: Instant) {
        let peer = adv.peer_id;
        self.last_seen.insert(peer, now);
        self.workers.insert(peer, adv);
    }

    pub fn get(&self, peer: &PeerId) -> Option<&ComputeAdvertisement> {
        self.workers.get(peer)
    }

    /// Seconds elapsed since this peer's last heartbeat, or `None` when the
    /// peer is unknown to the registry. Surfaced by the metrics API so the
    /// dashboard can show how fresh a worker's advertisement is.
    pub fn last_seen_secs(&self, peer: &PeerId, now: Instant) -> Option<u64> {
        self.last_seen
            .get(peer)
            .map(|seen| now.duration_since(*seen).as_secs())
    }

    /// All known advertisements, newest-updated first (stable order).
    pub fn list(&self) -> Vec<ComputeAdvertisement> {
        let mut all: Vec<_> = self.workers.values().cloned().collect();
        all.sort_by(|a, b| {
            let ta = self.last_seen.get(&a.peer_id);
            let tb = self.last_seen.get(&b.peer_id);
            tb.cmp(&ta)
                .then_with(|| a.peer_id.to_string().cmp(&b.peer_id.to_string()))
        });
        all
    }

    /// Marks a peer offline without dropping its last-known advertisement,
    /// so operators still see what hardware it had.
    pub fn mark_offline(&mut self, peer: &PeerId) {
        if let Some(adv) = self.workers.get_mut(peer) {
            adv.availability.status = WorkerHealth::Offline;
        }
    }

    pub fn remove(&mut self, peer: &PeerId) {
        self.workers.remove(peer);
        self.last_seen.remove(peer);
    }

    /// Flips peers that have not heartbeated within `stale_after` to
    /// `Offline` and returns their peer ids (for audit/logging).
    pub fn prune_stale(&mut self, now: Instant) -> Vec<PeerId> {
        let stale: Vec<PeerId> = self
            .last_seen
            .iter()
            .filter(|(_, seen)| now.duration_since(**seen) > self.stale_after)
            .map(|(peer, _)| *peer)
            .collect();
        for peer in &stale {
            self.mark_offline(peer);
        }
        stale
    }

    /// Live (non-offline) peers, i.e. usable scheduling candidates.
    pub fn live_peers(&self) -> Vec<PeerId> {
        self.workers
            .values()
            .filter(|adv| adv.availability.status != WorkerHealth::Offline)
            .map(|adv| adv.peer_id)
            .collect()
    }

    /// Removes peers that have been offline for longer than `grace` and
    /// returns the removed records (peer + node name) so the coordinator can
    /// audit the eviction. This is the operator-facing "automatic removal of
    /// unhealthy workers" step (M24): first [`prune_stale`](Self::prune_stale)
    /// flips stale peers offline (they may yet rejoin), then repeated calls to
    /// this method evict those that stay gone past the grace window.
    pub fn reap_offline(
        &mut self,
        now: Instant,
        grace: std::time::Duration,
    ) -> Vec<(PeerId, String)> {
        let evicted: Vec<(PeerId, String)> = self
            .last_seen
            .iter()
            .filter(|(_, seen)| now.duration_since(**seen) > self.stale_after + grace)
            .filter_map(|(peer, _)| {
                self.workers
                    .get(peer)
                    .map(|adv| (*peer, adv.node_name.clone()))
            })
            .collect();
        for (peer, _) in &evicted {
            self.remove(peer);
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::availability::{ComputeAvailability, WorkerHealth};
    use crate::capability::{ComputeCapability, ServedModel};
    use crate::testutil::{test_advertisement, test_peer};

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn upsert_replaces_and_lists() {
        let mut reg = ComputeRegistry::new(std::time::Duration::from_secs(30));
        let p = test_peer();
        let a = test_advertisement(p, 1024, Some(2048), 10, 0, WorkerHealth::Ready);
        reg.upsert(a.clone(), now());
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get(&p).unwrap().node_name, a.node_name);

        let b = test_advertisement(p, 512, Some(1024), 50, 2, WorkerHealth::Busy);
        reg.upsert(b, now());
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get(&p).unwrap().availability.load_percent, 50);
    }

    #[test]
    fn mark_offline_keeps_advertisement() {
        let mut reg = ComputeRegistry::new(std::time::Duration::from_secs(30));
        let p = test_peer();
        reg.upsert(
            test_advertisement(p, 1024, Some(2048), 10, 0, WorkerHealth::Ready),
            now(),
        );
        reg.mark_offline(&p);
        let adv = reg.get(&p).unwrap();
        assert_eq!(adv.availability.status, WorkerHealth::Offline);
        assert!(!reg.live_peers().contains(&p));
    }

    #[test]
    fn stale_peers_are_flipped_offline() {
        let stale_after = std::time::Duration::from_millis(10);
        let mut reg = ComputeRegistry::new(stale_after);
        let p = test_peer();
        reg.upsert(
            test_advertisement(p, 1024, Some(2048), 10, 0, WorkerHealth::Ready),
            now(),
        );
        std::thread::sleep(std::time::Duration::from_millis(30));
        let stale = reg.prune_stale(now());
        assert_eq!(stale, vec![p]);
        assert_eq!(
            reg.get(&p).unwrap().availability.status,
            WorkerHealth::Offline
        );
    }

    #[test]
    fn advertisement_round_trips_for_p2p_transport() {
        let p = test_peer();
        let adv = ComputeAdvertisement {
            peer_id: p,
            node_name: "rig".into(),
            node_id: "dca-rig01".into(),
            capability: ComputeCapability {
                cpu_cores: 4,
                ram_mb: 8 * 1024,
                gpu: None,
                engine: "llama_server".into(),
                served_models: vec![ServedModel {
                    model_hash: "abc".into(),
                    file_name: "m.gguf".into(),
                    size_mb: 500,
                    est_ram_mb: 1000,
                    est_vram_mb: 0,
                    context_tokens: 0,
                }],
                can_provision: false,
            },
            availability: ComputeAvailability {
                available_ram_mb: 4 * 1024,
                available_vram_mb: None,
                load_percent: 20,
                queue_depth: 1,
                tokens_per_second: 30,
                current_latency_ms: 200,
                status: WorkerHealth::Ready,
            },
            announced_at_ms: 1_700_000_000_000,
            accepts_remote_inference: true,
        };
        let json = serde_json::to_string(&adv).unwrap();
        let back: ComputeAdvertisement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, adv);
    }
}
