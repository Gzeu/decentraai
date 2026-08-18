//! Coordinator-side resource reservations.
//!
//! When the scheduler picks a worker it books the workload's estimated
//! RAM/VRAM so a second workload can never double-book the same memory on
//! a node whose advertisement only refreshes every few seconds. Reservations
//! are local to the coordinator (never broadcast) and expire after a TTL so
//! a crashed request cannot leak a permanent booking.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use libp2p::PeerId;
use uuid::Uuid;

/// A reservation held by the coordinator on a worker for the duration of a
/// workload. Released explicitly on completion/failure or by TTL expiry.
#[derive(Debug, Clone)]
pub struct ResourceReservation {
    pub reservation_id: Uuid,
    pub worker: PeerId,
    pub est_ram_mb: u64,
    pub est_vram_mb: u64,
    pub created_at: Instant,
    pub ttl: Duration,
}

impl ResourceReservation {
    pub fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.created_at) > self.ttl
    }
}

/// Bookkeeping of outstanding reservations per worker.
#[derive(Debug, Clone)]
pub struct ReservationLedger {
    reservations: HashMap<PeerId, Vec<ResourceReservation>>,
    ttl: Duration,
    max_per_worker: usize,
}

impl ReservationLedger {
    /// `ttl` bounds how long a reservation lives before it is pruned;
    /// `max_per_worker` caps how many concurrent workloads may be booked
    /// on a single node (its inference slot limit).
    pub fn new(ttl: Duration, max_per_worker: usize) -> Self {
        Self {
            reservations: HashMap::new(),
            ttl,
            max_per_worker: max_per_worker.max(1),
        }
    }

    /// Maximum concurrent reservations allowed on a single worker.
    pub fn max_per_worker(&self) -> usize {
        self.max_per_worker
    }

    /// Books the workload on `worker`. Returns `None` when the worker is
    /// already at its reservation cap.
    pub fn reserve(
        &mut self,
        worker: PeerId,
        est_ram_mb: u64,
        est_vram_mb: u64,
    ) -> Option<ResourceReservation> {
        let bucket = self.reservations.entry(worker).or_default();
        bucket.retain(|r| !r.is_expired(Instant::now()));
        if bucket.len() >= self.max_per_worker {
            return None;
        }
        let reservation = ResourceReservation {
            reservation_id: Uuid::new_v4(),
            worker,
            est_ram_mb,
            est_vram_mb,
            created_at: Instant::now(),
            ttl: self.ttl,
        };
        bucket.push(reservation.clone());
        Some(reservation)
    }

    /// Releases a reservation by id (no-op if unknown).
    pub fn release(&mut self, reservation_id: Uuid) {
        for bucket in self.reservations.values_mut() {
            bucket.retain(|r| r.reservation_id != reservation_id);
        }
        self.reservations.retain(|_, bucket| !bucket.is_empty());
    }

    /// Total RAM currently booked on `worker` (MiB).
    pub fn reserved_ram(&self, worker: &PeerId) -> u64 {
        self.reservations
            .get(worker)
            .map(|bucket| bucket.iter().map(|r| r.est_ram_mb).sum())
            .unwrap_or(0)
    }

    /// Total VRAM currently booked on `worker` (MiB).
    pub fn reserved_vram(&self, worker: &PeerId) -> u64 {
        self.reservations
            .get(worker)
            .map(|bucket| bucket.iter().map(|r| r.est_vram_mb).sum())
            .unwrap_or(0)
    }

    /// Number of in-flight workloads booked on `worker`.
    pub fn in_flight(&self, worker: &PeerId) -> usize {
        self.reservations
            .get(worker)
            .map(|bucket| bucket.len())
            .unwrap_or(0)
    }

    /// Removes expired reservations; returns the number freed.
    pub fn prune_expired(&mut self, now: Instant) -> usize {
        let mut freed = 0;
        for bucket in self.reservations.values_mut() {
            let before = bucket.len();
            bucket.retain(|r| !r.is_expired(now));
            freed += before - bucket.len();
        }
        self.reservations.retain(|_, bucket| !bucket.is_empty());
        freed
    }
}

/// Why a worker-local workload was rejected for lack of headroom (M15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitReason {
    InsufficientRam { available: u64, required: u64 },
    InsufficientVram { available: u64, required: u64 },
}

/// The capacity context against which a worker admits a workload (M15):
/// what the worker advertises as free plus the absolute floor it must keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admission {
    pub available_ram_mb: u64,
    pub available_vram_mb: Option<u64>,
    pub min_free_ram_mb: u64,
    pub min_free_vram_mb: u64,
}

impl ReservationLedger {
    /// Worker-side reservation enforcement (M15): can a workload of
    /// `req_ram_mb`/`req_vram_mb` be admitted on top of the reservations
    /// already booked for `worker`, given the free capacity `capacity`
    /// describes (advertised free minus the absolute floor to keep)?
    ///
    /// Mirrors the coordinator's `CapabilityMatcher` so both ends agree on
    /// headroom: the coordinator refuses to route when the ledger it holds
    /// would overbook a worker, and the worker refuses to serve even if a
    /// buggy or malicious coordinator sends more than it booked.
    pub fn admit(
        &self,
        worker: &PeerId,
        capacity: Admission,
        req_ram_mb: u64,
        req_vram_mb: u64,
    ) -> Result<(), AdmitReason> {
        let total_ram = req_ram_mb.saturating_add(capacity.min_free_ram_mb);
        let free_ram = capacity
            .available_ram_mb
            .saturating_sub(self.reserved_ram(worker));
        if free_ram < total_ram {
            return Err(AdmitReason::InsufficientRam {
                available: free_ram,
                required: total_ram,
            });
        }
        if let Some(free_vram) = capacity.available_vram_mb {
            let free_vram = free_vram.saturating_sub(self.reserved_vram(worker));
            let total_vram = req_vram_mb.saturating_add(capacity.min_free_vram_mb);
            if free_vram < total_vram {
                return Err(AdmitReason::InsufficientVram {
                    available: free_vram,
                    required: total_vram,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerId {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        PeerId::from(keypair.public())
    }

    #[test]
    fn reserve_and_release_round_trip() {
        let mut ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let p = peer();
        let r = ledger.reserve(p, 256, 3072).expect("first slot free");
        assert_eq!(ledger.in_flight(&p), 1);
        assert_eq!(ledger.reserved_ram(&p), 256);
        assert_eq!(ledger.reserved_vram(&p), 3072);
        ledger.release(r.reservation_id);
        assert_eq!(ledger.in_flight(&p), 0);
        assert_eq!(ledger.reserved_ram(&p), 0);
    }

    #[test]
    fn per_worker_cap_blocks_overbooking() {
        let mut ledger = ReservationLedger::new(Duration::from_secs(60), 1);
        let p = peer();
        assert!(ledger.reserve(p, 256, 3072).is_some());
        assert!(
            ledger.reserve(p, 256, 3072).is_none(),
            "second workload must be rejected at the cap"
        );
    }

    #[test]
    fn reservations_from_different_workers_do_not_conflict() {
        let mut ledger = ReservationLedger::new(Duration::from_secs(60), 1);
        let a = peer();
        let b = peer();
        assert!(ledger.reserve(a, 256, 3072).is_some());
        assert!(ledger.reserve(b, 256, 3072).is_some());
        assert_eq!(ledger.in_flight(&a), 1);
        assert_eq!(ledger.in_flight(&b), 1);
    }

    #[test]
    fn expired_reservations_are_pruned() {
        let ttl = Duration::from_millis(50);
        let mut ledger = ReservationLedger::new(ttl, 2);
        let p = peer();
        ledger.reserve(p, 256, 3072);
        assert_eq!(ledger.in_flight(&p), 1);
        std::thread::sleep(Duration::from_millis(80));
        let freed = ledger.prune_expired(Instant::now());
        assert_eq!(freed, 1);
        assert_eq!(ledger.in_flight(&p), 0);
    }

    #[test]
    fn admit_allows_with_headroom() {
        let mut ledger = ReservationLedger::new(Duration::from_secs(60), 8);
        let p = peer();
        ledger.reserve(p, 256, 3072);
        let cap = Admission {
            available_ram_mb: 12 * 1024,
            available_vram_mb: Some(18 * 1024),
            min_free_ram_mb: 1024,
            min_free_vram_mb: 512,
        };
        assert_eq!(ledger.admit(&p, cap, 256, 3072), Ok(()));
    }

    #[test]
    fn admit_rejects_when_ram_would_overbook() {
        let mut ledger = ReservationLedger::new(Duration::from_secs(60), 8);
        let p = peer();
        ledger.reserve(p, 10 * 1024, 0);
        let cap = Admission {
            available_ram_mb: 12 * 1024,
            available_vram_mb: Some(18 * 1024),
            min_free_ram_mb: 1024,
            min_free_vram_mb: 512,
        };
        assert_eq!(
            ledger.admit(&p, cap, 2048, 0),
            Err(AdmitReason::InsufficientRam {
                available: 2 * 1024,
                required: 3072,
            })
        );
    }

    #[test]
    fn admit_rejects_when_vram_would_overbook() {
        let mut ledger = ReservationLedger::new(Duration::from_secs(60), 8);
        let p = peer();
        ledger.reserve(p, 0, 16 * 1024);
        let cap = Admission {
            available_ram_mb: 12 * 1024,
            available_vram_mb: Some(18 * 1024),
            min_free_ram_mb: 1024,
            min_free_vram_mb: 512,
        };
        assert_eq!(
            ledger.admit(&p, cap, 0, 4096),
            Err(AdmitReason::InsufficientVram {
                available: 2 * 1024,
                required: 4608,
            })
        );
    }

    #[test]
    fn admit_skips_vram_check_when_worker_has_no_gpu() {
        let ledger = ReservationLedger::new(Duration::from_secs(60), 8);
        let p = peer();
        let cap = Admission {
            available_ram_mb: 12 * 1024,
            available_vram_mb: None,
            min_free_ram_mb: 1024,
            min_free_vram_mb: 512,
        };
        assert_eq!(ledger.admit(&p, cap, 2048, 999_999), Ok(()));
    }
}
