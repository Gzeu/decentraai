//! DecentraAI compute-sharing domain (M11).
//!
//! This crate is the pure, decision-only core of **compute sharing**: it
//! answers "which node should execute this workload, and can it actually
//! run it right now?" It deliberately contains NO I/O and NO async — every
//! type is serde-serializable so advertisements can travel over the P2P
//! request/response channel, and every decision is a pure function that
//! unit tests can drive with synthetic inputs.
//!
//! # Why this exists next to model sharing
//!
//! DecentraAI's core product is people sharing compute/GPU capacity, not
//! merely model files. A [`ComputeAdvertisement`](availability::ComputeAdvertisement)
//! describes a node's static capability (GPU, VRAM, RAM, CPU, served models)
//! plus its current availability (free RAM/VRAM, load, health). The
//! [`CapabilityMatcher`](matcher::CapabilityMatcher) decides eligibility;
//! the [`ComputeScheduler`](scheduler::ComputeScheduler) picks the best
//! eligible worker and **reserves** its resources so two workloads can
//! never double-book the same VRAM.
//!
//! Model files remain a *supporting* artifact: a worker serves models it
//! already holds, and the matcher only routes a workload to a node that
//! serves the required model hash. Auto-download of models is a separate,
//! policy-gated concern (see `decentraai-p2p` sharing), never the
//! orchestration mechanism here.
//!
//! # Core concepts
//!
//! | Concept | Type | Question it answers |
//! |---|---|---|
//! | WorkerCapability | [`capability::ComputeCapability`] | What hardware + models does this node have? |
//! | ComputeAvailability | [`availability::ComputeAvailability`] | How free is it right now? |
//! | ComputeAdvertisement | [`availability::ComputeAdvertisement`] | Capability + availability, broadcast over P2P |
//! | WorkloadRequirements | [`requirements::WorkloadRequirements`] | What does this workload need? |
//! | ResourceReservation | [`reservation::ResourceReservation`] | How much did the coordinator book on this worker? |
//! | CapabilityMatcher | [`matcher::CapabilityMatcher`] | Can this worker run this workload now? |
//! | ComputeScheduler | [`scheduler::ComputeScheduler`] | Which node executes it? |
//! | ComputeRegistry | [`registry::ComputeRegistry`] | Who is on the network, and are they still alive? |

pub mod availability;
pub mod capability;
pub mod compensation;
pub mod contribution;
pub mod loadbalance;
pub mod matcher;
pub mod quota;
pub mod registry;
pub mod requirements;
pub mod reservation;
pub mod scheduler;

#[cfg(test)]
pub(crate) mod testutil;

pub use availability::{ComputeAdvertisement, ComputeAvailability, WorkerHealth};
pub use capability::{ComputeCapability, GpuSpec, ServedModel};
pub use compensation::{
    CompensationAccount, CompensationEvent, CompensationLedger, RewardPolicy, reward_tokens,
    total_attempts,
};
pub use contribution::{ContributionProfile, contribution_score, suggest_tier};
pub use loadbalance::{LoadShare, adaptive_load_shares};
pub use matcher::{CapabilityMatcher, MatchOutcome, MatchReason};
pub use quota::{
    AccountId, ContributionPolicy, QuotaAccount, QuotaError, QuotaEvent, QuotaLedger,
    QuotaReservation,
};
pub use registry::ComputeRegistry;
pub use requirements::WorkloadRequirements;
pub use reservation::{Admission, AdmitReason, ReservationLedger, ResourceReservation};
pub use scheduler::{ComputeScheduler, Placement};

/// Default heartbeat interval for compute advertisements (ms).
pub const DEFAULT_ADVERTISEMENT_INTERVAL_MS: u64 = 5_000;

/// How long before an advertisement is treated as stale/offline.
pub const DEFAULT_STALE_AFTER_MS: u64 = 30_000;
