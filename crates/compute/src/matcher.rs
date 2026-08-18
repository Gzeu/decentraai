//! Pure capability matching: can a worker run this workload right now?

use crate::availability::ComputeAdvertisement;
use crate::requirements::WorkloadRequirements;
use crate::reservation::ReservationLedger;
use libp2p::PeerId;

/// Why a worker was rejected, so operators and logs can distinguish a
/// missing model from an overloaded GPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchReason {
    /// Coordinator has no trust record for this peer.
    NotTrusted,
    /// The worker does not serve the required model hash.
    ModelNotServed,
    /// The worker is not in a `Ready` health state.
    NotHealthy,
    /// The worker does not accept inference routed from remote peers
    /// (`accepts_remote_inference == false` in its advertisement). The local
    /// node itself is exempt: its own work is local, not remote.
    NotAcceptingRemote,
    /// Not enough free RAM after subtracting reservations.
    InsufficientRam { available: u64, required: u64 },
    /// Not enough free VRAM after subtracting reservations.
    InsufficientVram { available: u64, required: u64 },
    /// Load or queue depth above the configured ceiling.
    Overloaded,
    /// All of the worker's inference slots are already booked.
    AtReservationCap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    Eligible,
    Rejected(MatchReason),
}

/// Thresholds a coordinator applies to every worker before scheduling.
#[derive(Debug, Clone)]
pub struct CapabilityMatcher {
    /// Absolute floor of free RAM a worker must keep (its own reserve).
    pub min_free_ram_mb: u64,
    /// Absolute floor of free VRAM a worker must keep.
    pub min_free_vram_mb: u64,
    pub max_queue_depth: u32,
    pub max_load_percent: u8,
    /// When true, a worker that does not serve `model_hash` today but
    /// advertises `can_provision` is still eligible (it will fetch the model
    /// on demand once the workload lands, M14).
    pub allow_provisioning: bool,
}

impl Default for CapabilityMatcher {
    fn default() -> Self {
        Self {
            min_free_ram_mb: 1024,
            min_free_vram_mb: 512,
            max_queue_depth: 8,
            max_load_percent: 95,
            allow_provisioning: false,
        }
    }
}

impl CapabilityMatcher {
    /// Pure decision. `trusted` is coordinator-side state (pairing/trust
    /// store), never derived from the advertisement itself. `local_peer` is
    /// the coordinator's own peer id: the local node always accepts its own
    /// work, so it is exempt from the `accepts_remote_inference` gate (that
    /// flag governs *remote* resource sharing only).
    pub fn matches(
        &self,
        adv: &ComputeAdvertisement,
        req: &WorkloadRequirements,
        ledger: &ReservationLedger,
        trusted: bool,
        local_peer: Option<&PeerId>,
    ) -> MatchOutcome {
        if !trusted {
            return MatchOutcome::Rejected(MatchReason::NotTrusted);
        }
        if !adv.capability.has_model(&req.model_hash)
            && !(self.allow_provisioning && adv.capability.can_provision)
        {
            return MatchOutcome::Rejected(MatchReason::ModelNotServed);
        }
        // A remote worker that has not opted in to remote sharing is not an
        // eligible scheduling candidate (its own engine would reject the
        // request). The local node is exempt: local work is not remote.
        if !adv.accepts_remote_inference && Some(&adv.peer_id) != local_peer {
            return MatchOutcome::Rejected(MatchReason::NotAcceptingRemote);
        }
        if !adv.availability.healthy() {
            return MatchOutcome::Rejected(MatchReason::NotHealthy);
        }

        let booked_ram = ledger.reserved_ram(&adv.peer_id);
        let available_ram = adv.availability.available_ram_mb.saturating_sub(booked_ram);
        if available_ram < req.est_ram_mb + self.min_free_ram_mb {
            return MatchOutcome::Rejected(MatchReason::InsufficientRam {
                available: available_ram,
                required: req.est_ram_mb + self.min_free_ram_mb,
            });
        }

        if let Some(free_vram) = adv.availability.available_vram_mb {
            let booked_vram = ledger.reserved_vram(&adv.peer_id);
            let available_vram = free_vram.saturating_sub(booked_vram);
            if available_vram < req.est_vram_mb + self.min_free_vram_mb {
                return MatchOutcome::Rejected(MatchReason::InsufficientVram {
                    available: available_vram,
                    required: req.est_vram_mb + self.min_free_vram_mb,
                });
            }
        }

        if adv.availability.load_percent > self.max_load_percent
            || adv.availability.queue_depth >= self.max_queue_depth
        {
            return MatchOutcome::Rejected(MatchReason::Overloaded);
        }

        if ledger.in_flight(&adv.peer_id) >= ledger.max_per_worker() {
            return MatchOutcome::Rejected(MatchReason::AtReservationCap);
        }

        MatchOutcome::Eligible
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::availability::WorkerHealth;
    use crate::testutil::{test_advertisement, test_peer};
    use std::time::Duration;

    fn req() -> WorkloadRequirements {
        WorkloadRequirements::new("abc".into(), 256, 3072)
    }

    #[test]
    fn eligible_worker_passes() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            32,
            0,
            WorkerHealth::Ready,
        );
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Eligible
        );
    }

    #[test]
    fn remote_worker_without_remote_opt_in_rejected() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let mut adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            32,
            0,
            WorkerHealth::Ready,
        );
        adv.accepts_remote_inference = false;
        // A remote worker that has not opted in is never a scheduling candidate.
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::NotAcceptingRemote)
        );
        // The local node itself is exempt: local work is not remote sharing.
        let local = adv.peer_id;
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, Some(&local)),
            MatchOutcome::Eligible
        );
    }

    #[test]
    fn untrusted_peer_rejected() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            32,
            0,
            WorkerHealth::Ready,
        );
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, false, None),
            MatchOutcome::Rejected(MatchReason::NotTrusted)
        );
    }

    #[test]
    fn unhealthy_worker_rejected() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            32,
            0,
            WorkerHealth::Unhealthy,
        );
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::NotHealthy)
        );
    }

    #[test]
    fn missing_model_rejected() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            32,
            0,
            WorkerHealth::Ready,
        );
        let other = WorkloadRequirements::new("zzz".into(), 256, 3072);
        assert_eq!(
            matcher.matches(&adv, &other, &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::ModelNotServed)
        );
    }

    #[test]
    fn provisioning_worker_eligible_when_policy_allows() {
        let matcher = CapabilityMatcher {
            allow_provisioning: true,
            ..CapabilityMatcher::default()
        };
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let mut adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            32,
            0,
            WorkerHealth::Ready,
        );
        adv.capability.can_provision = true;
        let other = WorkloadRequirements::new("zzz".into(), 256, 3072);
        assert_eq!(
            matcher.matches(&adv, &other, &ledger, true, None),
            MatchOutcome::Eligible
        );
    }

    #[test]
    fn provisioning_worker_rejected_when_policy_forbids() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let mut adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            32,
            0,
            WorkerHealth::Ready,
        );
        adv.capability.can_provision = true;
        let other = WorkloadRequirements::new("zzz".into(), 256, 3072);
        assert_eq!(
            matcher.matches(&adv, &other, &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::ModelNotServed)
        );
    }

    #[test]
    fn insufficient_ram_rejected_with_numbers() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let adv = test_advertisement(
            test_peer(),
            512,
            Some(18 * 1024),
            32,
            0,
            WorkerHealth::Ready,
        );
        match matcher.matches(&adv, &req(), &ledger, true, None) {
            MatchOutcome::Rejected(MatchReason::InsufficientRam {
                available,
                required,
            }) => {
                assert!(available < required);
            }
            other => panic!("expected insufficient RAM, got {other:?}"),
        }
    }

    #[test]
    fn insufficient_vram_rejected() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(512),
            32,
            0,
            WorkerHealth::Ready,
        );
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::InsufficientVram {
                available: 512,
                required: 3072 + 512
            })
        );
    }

    #[test]
    fn reservations_consume_headroom() {
        let matcher = CapabilityMatcher::default();
        let mut ledger = ReservationLedger::new(Duration::from_secs(60), 4);
        let p = test_peer();
        let adv = test_advertisement(p, 8 * 1024, Some(8 * 1024), 0, 0, WorkerHealth::Ready);
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Eligible
        );

        // Book two workloads; the third must hit the RAM/VRAM ceiling.
        ledger.reserve(p, 3072, 4096);
        ledger.reserve(p, 3072, 4096);
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::InsufficientVram {
                available: 0,
                required: 3072 + 512
            })
        );
    }

    #[test]
    fn overloaded_worker_rejected() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            99,
            0,
            WorkerHealth::Ready,
        );
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::Overloaded)
        );
    }

    #[test]
    fn deep_queue_rejected() {
        let matcher = CapabilityMatcher::default();
        let ledger = ReservationLedger::new(Duration::from_secs(60), 2);
        let adv = test_advertisement(
            test_peer(),
            12 * 1024,
            Some(18 * 1024),
            0,
            8,
            WorkerHealth::Ready,
        );
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::Overloaded)
        );
    }

    #[test]
    fn reservation_cap_rejected() {
        let matcher = CapabilityMatcher::default();
        let mut ledger = ReservationLedger::new(Duration::from_secs(60), 1);
        let p = test_peer();
        let adv = test_advertisement(p, 12 * 1024, Some(18 * 1024), 0, 0, WorkerHealth::Ready);
        ledger.reserve(p, 256, 3072);
        assert_eq!(
            matcher.matches(&adv, &req(), &ledger, true, None),
            MatchOutcome::Rejected(MatchReason::AtReservationCap)
        );
    }
}
