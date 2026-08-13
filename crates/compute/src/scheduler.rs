//! Capability-aware compute scheduler.
//!
//! Answers the core compute-sharing question: **which node should execute
//! this workload?** It filters candidates through the capability matcher
//! (model served, RAM/VRAM headroom after reservations, health, load),
//! ranks them deterministically, books a resource reservation on the
//! winner, and returns the placement.

use std::collections::HashSet;
use std::time::Instant;

use libp2p::PeerId;

use crate::availability::ComputeAdvertisement;
use crate::matcher::{CapabilityMatcher, MatchOutcome};
use crate::registry::ComputeRegistry;
use crate::requirements::WorkloadRequirements;
use crate::reservation::{ReservationLedger, ResourceReservation};

/// Result of selecting a worker for a workload.
#[derive(Debug, Clone)]
pub struct Placement {
    pub worker: PeerId,
    /// The booked reservation; must be released when the workload ends.
    pub reservation: ResourceReservation,
    /// 0.0..1.0 confidence in the placement estimate.
    pub confidence: f32,
}

/// Combines the compute registry, capability matcher, and reservation
/// ledger into the automatic worker selector.
#[derive(Debug, Clone)]
pub struct ComputeScheduler {
    registry: ComputeRegistry,
    ledger: ReservationLedger,
    matcher: CapabilityMatcher,
    /// Coordinator-side trust set (pairing/trust store), not derived from
    /// advertisements.
    trusted: HashSet<PeerId>,
}

impl ComputeScheduler {
    pub fn new(
        registry: ComputeRegistry,
        ledger: ReservationLedger,
        matcher: CapabilityMatcher,
        trusted: HashSet<PeerId>,
    ) -> Self {
        Self {
            registry,
            ledger,
            matcher,
            trusted,
        }
    }

    pub fn registry(&self) -> &ComputeRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut ComputeRegistry {
        &mut self.registry
    }

    pub fn matcher(&self) -> &CapabilityMatcher {
        &self.matcher
    }

    pub fn ledger(&self) -> &ReservationLedger {
        &self.ledger
    }

    /// Adds or replaces the latest advertisement from a peer.
    pub fn upsert(&mut self, adv: ComputeAdvertisement) {
        self.registry.upsert(adv, Instant::now());
    }

    /// Marks a peer offline (stale heartbeat / explicit failure).
    pub fn mark_offline(&mut self, peer: &PeerId) {
        self.registry.mark_offline(peer);
    }

    /// Drops peers that stopped heartbeating; returns the dropped ids.
    pub fn prune_stale(&mut self) -> Vec<PeerId> {
        self.registry.prune_stale(Instant::now())
    }

    /// Whether the coordinator trusts `peer` to execute workloads.
    pub fn is_trusted(&self, peer: &PeerId) -> bool {
        self.trusted.contains(peer)
    }

    pub fn add_trusted(&mut self, peer: PeerId) {
        self.trusted.insert(peer);
    }

    /// Selects the best eligible worker for `req` and books a reservation
    /// on it. Returns `None` when no worker can run the workload right now.
    ///
    /// Determinism: candidates are ranked by score (desc), ties broken by
    /// PeerId (asc) — a stable choice regardless of hash-map iteration.
    pub fn select(&mut self, req: &WorkloadRequirements, now: Instant) -> Option<Placement> {
        self.ledger.prune_expired(now);
        self.registry.prune_stale(now);

        let mut candidates: Vec<ComputeAdvertisement> = self
            .registry
            .list()
            .into_iter()
            .filter(|adv| self.trusted.contains(&adv.peer_id))
            .filter(|adv| {
                matches!(
                    self.matcher.matches(adv, req, &self.ledger, true),
                    MatchOutcome::Eligible
                )
            })
            .collect();

        candidates.sort_by(|a, b| {
            let score_a = self.score(a, req);
            let score_b = self.score(b, req);
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.peer_id.to_string().cmp(&b.peer_id.to_string()))
        });

        let best = candidates.into_iter().next()?;
        let reservation = self
            .ledger
            .reserve(best.peer_id, req.est_ram_mb, req.est_vram_mb)?;
        Some(Placement {
            worker: best.peer_id,
            reservation,
            confidence: self.confidence(&best, req),
        })
    }

    /// Frees a reservation (call on workload completion/failure).
    pub fn release(&mut self, reservation_id: uuid::Uuid) {
        self.ledger.release(reservation_id);
    }

    /// Deterministic 0.0..1.0 score: lower load, less backlogged queue,
    /// higher throughput, lower latency, and more headroom than the
    /// workload requires all score better.
    fn score(&self, adv: &ComputeAdvertisement, req: &WorkloadRequirements) -> f32 {
        let a = &adv.availability;
        let max_queue = self.matcher.max_queue_depth as f32;

        let load = (a.load_percent as f32 / 100.0).clamp(0.0, 1.0);
        let load_score = 1.0 - load;

        let queue_score = 1.0 - (a.queue_depth as f32 / max_queue).min(1.0);

        let throughput_score = (a.tokens_per_second as f32 / 200.0).min(1.0);

        let latency_score = 1.0 - (a.current_latency_ms as f32 / 1000.0).min(1.0);

        let headroom = if req.est_ram_mb > 0 {
            (a.available_ram_mb as f32 / req.est_ram_mb as f32).min(1.0)
        } else {
            1.0
        };

        load_score * 0.30 + queue_score * 0.20 + throughput_score * 0.20
            + latency_score * 0.15
            + headroom * 0.15
    }

    /// Placement confidence: how much free headroom and how little backlog
    /// the winner has (0.0..1.0).
    fn confidence(&self, adv: &ComputeAdvertisement, req: &WorkloadRequirements) -> f32 {
        let a = &adv.availability;
        let max_queue = self.matcher.max_queue_depth as f32;
        let headroom = if req.est_ram_mb > 0 {
            (a.available_ram_mb as f32 / req.est_ram_mb as f32).min(1.0)
        } else {
            1.0
        };
        let queue = 1.0 - (a.queue_depth as f32 / max_queue).min(1.0);
        (headroom * 0.6 + queue * 0.4).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::availability::{ComputeAvailability, WorkerHealth};
    use crate::testutil::test_advertisement;
    use crate::requirements::WorkloadRequirements;
    use std::time::Duration;

    fn peer() -> PeerId {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        PeerId::from(keypair.public())
    }

    fn advertisement(peer: PeerId, ram: u64, vram: u64, load: u8, queue: u32, tps: u32, lat: u32) -> ComputeAdvertisement {
        test_advertisement(peer, ram, Some(vram), load, queue, WorkerHealth::Ready)
            .replace_availability(ComputeAvailability {
                available_ram_mb: ram,
                available_vram_mb: Some(vram),
                load_percent: load,
                queue_depth: queue,
                tokens_per_second: tps,
                current_latency_ms: lat,
                status: WorkerHealth::Ready,
            })
    }

    trait Replace {
        fn replace_availability(self, a: ComputeAvailability) -> ComputeAdvertisement;
    }
    impl Replace for ComputeAdvertisement {
        fn replace_availability(mut self, a: ComputeAvailability) -> ComputeAdvertisement {
            self.availability = a;
            self
        }
    }

    fn scheduler(trusted: HashSet<PeerId>) -> ComputeScheduler {
        ComputeScheduler::new(
            ComputeRegistry::new(Duration::from_secs(30)),
            ReservationLedger::new(Duration::from_secs(60), 4),
            CapabilityMatcher::default(),
            trusted,
        )
    }

    fn req() -> WorkloadRequirements {
        WorkloadRequirements::new("abc".into(), 256, 3072)
    }

    #[test]
    fn selects_best_capable_worker_and_reserves() {
        let p1 = peer();
        let p2 = peer();
        let mut sched = scheduler(HashSet::from([p1, p2]));
        sched.upsert(advertisement(p1, 12 * 1024, 18 * 1024, 80, 5, 40, 400));
        sched.upsert(advertisement(p2, 12 * 1024, 18 * 1024, 10, 0, 80, 60));

        let placement = sched.select(&req(), Instant::now()).expect("a worker is eligible");
        assert_eq!(placement.worker, p2, "the idle, faster worker wins");
        assert_eq!(sched.ledger().in_flight(&p2), 1, "resources are reserved");
    }

    #[test]
    fn rejects_when_no_worker_serves_the_model() {
        let p = peer();
        let mut sched = scheduler(HashSet::from([p]));
        sched.upsert(advertisement(p, 12 * 1024, 18 * 1024, 10, 0, 80, 60));
        let other = WorkloadRequirements::new("zzz".into(), 256, 3072);
        assert!(sched.select(&other, Instant::now()).is_none());
    }

    #[test]
    fn untrusted_workers_are_never_selected() {
        let p = peer();
        let mut sched = scheduler(HashSet::new());
        sched.upsert(advertisement(p, 12 * 1024, 18 * 1024, 10, 0, 80, 60));
        assert!(sched.select(&req(), Instant::now()).is_none());
    }

    #[test]
    fn reservations_prevent_double_booking() {
        let p = peer();
        let mut sched = scheduler(HashSet::from([p]));
        // One worker, enough headroom for exactly one workload at a time.
        sched.upsert(advertisement(p, 3072, 3072 + 512, 0, 0, 80, 60));

        let first = sched.select(&req(), Instant::now()).expect("first fits");
        assert!(sched.select(&req(), Instant::now()).is_none(), "second workload must be rejected");
        sched.release(first.reservation.reservation_id);
        assert!(sched.select(&req(), Instant::now()).is_some(), "release frees the slot");
    }

    #[test]
    fn offline_workers_are_skipped() {
        let p = peer();
        let mut sched = scheduler(HashSet::from([p]));
        let adv = advertisement(p, 12 * 1024, 18 * 1024, 10, 0, 80, 60);
        sched.upsert(adv);
        sched.mark_offline(&p);
        assert!(sched.select(&req(), Instant::now()).is_none());
    }

    #[test]
    fn selection_is_deterministic_for_equal_workers() {
        let p1 = peer();
        let p2 = peer();
        let p3 = peer();
        let mut sched = scheduler(HashSet::from([p1, p2, p3]));
        for p in [p1, p2, p3] {
            sched.upsert(advertisement(p, 12 * 1024, 18 * 1024, 10, 0, 80, 60));
        }

        let mut winners = Vec::new();
        for _ in 0..10 {
            let mut fresh = sched.clone();
            let w = fresh.select(&req(), Instant::now()).unwrap().worker;
            winners.push(w);
        }
        assert!(
            winners.windows(2).all(|w| w[0] == w[1]),
            "equal workers must always pick the same PeerId: {winners:?}"
        );
    }

    #[test]
    fn gpu_only_workers_need_vram() {
        let p = peer();
        let mut sched = scheduler(HashSet::from([p]));
        // GPU node with plenty RAM but only 1 GiB free VRAM.
        let adv = advertisement(p, 12 * 1024, 1024, 10, 0, 80, 60);
        sched.upsert(adv);
        assert!(sched.select(&req(), Instant::now()).is_none(), "3 GiB workload needs more VRAM");
    }
}