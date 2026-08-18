//! Replay protection for inference requests (P4).
//!
//! A captured ``InferRequest`` frame must not be re-executable. Requests carry
//! a sender-set `nonce` (see [`InferRequest::nonce`]); this guard is the
//! receiving side's per-peer set of already-seen nonces. A request whose nonce
//! is already present for the same authenticated sender is a replay and is
//! rejected *before* it reaches model admission, the queue, or the backend —
//! so output/tokens/KV are never duplicated.
//!
//! Design notes:
//! - Keyed by the transport-authenticated sender peer, **not** the
//!   attacker-controllable `sender_peer_id` payload (see P2).
//! - Bounded: a per-peer capacity cap plus a TTL ensures the set cannot grow
//!   without bound; stale entries are pruned so legitimate fresh requests (and
//!   the natural re-use of a nonce only after it has expired) still pass.
//! - Only *signed* requests advance/consult the guard (P1): an attacker cannot
//!   mint a fresh nonce and re-sign, so the guard is only ever fed authentic
//!   senders. This is the documented composition with P1, mirroring the
//!   manifest pipeline (signature anchors integrity, checklist anchors
//!   freshness).
//! - Pure and I/O-free: tests drive it with synthetic ``Instant`` values.
//! - In-memory only: a restart resets the set. Replay protection is a
//!   freshness guarantee for in-flight/captured traffic, not a durable
//!   anti-double-spend ledger.

use libp2p::PeerId;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Bounded, TTL'd seen-nonce set per authenticated sender peer.
#[derive(Debug)]
pub struct ReplayGuard {
    seen: HashMap<PeerId, HashMap<u64, Instant>>,
    window: Duration,
    max_per_peer: usize,
}

/// Outcome of checking a nonce.
#[derive(Debug, PartialEq, Eq)]
pub enum ReplayCheck {
    /// First time this sender has used this nonce; accepted and recorded.
    Accepted,
    /// The sender reused a nonce still within the freshness window: a replay.
    Rejected,
}

impl ReplayGuard {
    /// Creates a guard. `window` is how long a seen nonce stays live;
    /// `max_per_peer` bounds how many distinct nonces are remembered per peer.
    pub fn new(window: Duration, max_per_peer: usize) -> Self {
        Self {
            seen: HashMap::new(),
            window,
            max_per_peer: max_per_peer.max(1),
        }
    }

    /// Record a `nonce` for `peer` at time `now`. Returns `Rejected` if the
    /// nonce is already present and unexpired (a replay); otherwise records it
    /// and returns `Accepted`.
    pub fn check_and_mark(&mut self, peer: &PeerId, nonce: u64, now: Instant) -> ReplayCheck {
        self.prune(now);
        let slots = self.seen.entry(*peer).or_default();
        if let Some(&recorded) = slots.get(&nonce) {
            if now.duration_since(recorded) < self.window {
                return ReplayCheck::Rejected;
            }
        }
        // Enforce the per-peer bound: drop the oldest entry if full.
        if slots.len() >= self.max_per_peer {
            if let Some((&oldest, _)) = slots.iter().min_by_key(|(_, t)| **t) {
                slots.remove(&oldest);
            }
        }
        slots.insert(nonce, now);
        ReplayCheck::Accepted
    }

    /// Drops expired entries and empty peer buckets.
    pub fn prune(&mut self, now: Instant) {
        self.seen.retain(|_, slots| {
            slots.retain(|_, &mut t| now.duration_since(t) < self.window);
            !slots.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerId {
        PeerId::random()
    }

    #[test]
    fn replay_of_the_same_nonce_is_rejected() {
        let mut guard = ReplayGuard::new(Duration::from_secs(60), 8);
        let p = peer();
        let t = Instant::now();
        assert_eq!(guard.check_and_mark(&p, 42, t), ReplayCheck::Accepted);
        assert_eq!(guard.check_and_mark(&p, 42, t), ReplayCheck::Rejected);
    }

    #[test]
    fn fresh_distinct_nonces_pass() {
        // Parallelism-safe: no strict +1 ordering is required, only uniqueness
        // within the window.
        let mut guard = ReplayGuard::new(Duration::from_secs(60), 8);
        let p = peer();
        let t = Instant::now();
        for n in [7u64, 8, 9] {
            assert_eq!(guard.check_and_mark(&p, n, t), ReplayCheck::Accepted);
        }
    }

    #[test]
    fn nonces_are_isolated_across_peers() {
        let mut guard = ReplayGuard::new(Duration::from_secs(60), 8);
        let a = peer();
        let b = peer();
        let t = Instant::now();
        // Same nonce from two different senders must never collide.
        assert_eq!(guard.check_and_mark(&a, 5, t), ReplayCheck::Accepted);
        assert_eq!(guard.check_and_mark(&b, 5, t), ReplayCheck::Accepted);
    }

    #[test]
    fn expired_nonce_may_be_replayed_after_ttl() {
        let mut guard = ReplayGuard::new(Duration::from_secs(60), 8);
        let p = peer();
        let t0 = Instant::now();
        assert_eq!(guard.check_and_mark(&p, 1, t0), ReplayCheck::Accepted);
        // After the window elapses, the nonce is pruned and may be reused.
        let later = t0 + Duration::from_secs(61);
        assert_eq!(guard.check_and_mark(&p, 1, later), ReplayCheck::Accepted);
    }

    #[test]
    fn prune_only_removes_expired_entries() {
        let mut guard = ReplayGuard::new(Duration::from_secs(60), 8);
        let p = peer();
        let t0 = Instant::now();
        guard.check_and_mark(&p, 1, t0);
        guard.check_and_mark(&p, 2, t0);
        let later = t0 + Duration::from_secs(30);
        guard.prune(later);
        // Both still within window.
        assert_eq!(guard.check_and_mark(&p, 1, later), ReplayCheck::Rejected);
        let far = later + Duration::from_secs(60); // now older than 60s from t0
        guard.prune(far);
        assert_eq!(guard.check_and_mark(&p, 1, far), ReplayCheck::Accepted);
    }

    #[test]
    fn per_peer_capacity_is_bounded() {
        let mut guard = ReplayGuard::new(Duration::from_secs(60), 3);
        let p = peer();
        let t = Instant::now();
        for n in [10u64, 11, 12] {
            assert_eq!(guard.check_and_mark(&p, n, t), ReplayCheck::Accepted);
        }
        // Adding a 4th evicts the oldest so the guard stays bounded.
        assert_eq!(guard.check_and_mark(&p, 13, t), ReplayCheck::Accepted);
    }
}
