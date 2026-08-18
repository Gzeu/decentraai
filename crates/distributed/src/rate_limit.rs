//! Per-peer sliding-window rate limiter (H1).
//!
//! Protects a worker from an abusive or anomalous coordinator: regardless of
//! how many requests a peer sends, it may not dispatch more than the allowed
//! burst within the window. Unlike the API's token-tier limiter
//! (`crates/runtime`), this gates the **P2P worker path** and is keyed by the
//! transport-authenticated peer id (see P2), not a token in the payload.
//!
//! Pure and I/O-free: `now` is injected so tests drive it with synthetic time.
//! State is bounded: a per-peer capacity cap and a TTL prune keep memory flat.

use libp2p::PeerId;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Sliding-window rate limiter keyed by peer.
#[derive(Debug)]
pub struct PeerRateLimiter {
    windows: HashMap<PeerId, Vec<Instant>>,
    window: Duration,
    max_per_window: usize,
    max_entries_per_peer: usize,
}

impl PeerRateLimiter {
    /// `window` is the sliding window length; `max_per_window` is the number of
    /// allowed requests per window; `max_entries_per_peer` bounds stored
    /// timestamps per peer (should be >= max_per_window) to cap memory.
    pub fn new(window: Duration, max_per_window: usize, max_entries_per_peer: usize) -> Self {
        Self {
            windows: HashMap::new(),
            window,
            max_per_window: max_per_window.max(1),
            max_entries_per_peer: max_entries_per_peer.max(1),
        }
    }

    /// Whether `peer` is allowed to send another request now. Records the
    /// request when allowed; `false` when the peer has exhausted its window.
    pub fn allow(&mut self, peer: &PeerId, now: Instant) -> bool {
        self.prune(now);
        let cutoff = now - self.window;
        let entries = self.windows.entry(*peer).or_default();
        // Drop timestamps outside the current window.
        entries.retain(|&t| t >= cutoff);
        if entries.len() >= self.max_per_window {
            return false;
        }
        entries.push(now);
        // Bound memory: keep only the most recent window timestamps.
        if entries.len() > self.max_entries_per_peer {
            entries.drain(0..(entries.len() - self.max_entries_per_peer));
        }
        true
    }

    /// Drops empty peer buckets. Called lazily on every `allow`.
    fn prune(&mut self, now: Instant) {
        let cutoff = now - self.window;
        self.windows.retain(|_, entries| {
            entries.retain(|&t| t >= cutoff);
            !entries.is_empty()
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
    fn allows_up_to_the_burst_then_rejects() {
        let now = Instant::now();
        let mut l = PeerRateLimiter::new(Duration::from_secs(60), 3, 8);
        let p = peer();
        assert!(l.allow(&p, now));
        assert!(l.allow(&p, now + Duration::from_millis(1)));
        assert!(l.allow(&p, now + Duration::from_millis(2)));
        // Burst of 3 exhausted.
        assert!(!l.allow(&p, now + Duration::from_millis(3)));
    }

    #[test]
    fn window_slides_so_expired_requests_free_capacity() {
        let now = Instant::now();
        let mut l = PeerRateLimiter::new(Duration::from_secs(60), 3, 8);
        let p = peer();
        for i in 0..3 {
            assert!(l.allow(&p, now + Duration::from_millis(i)), "burst fill");
        }
        assert!(!l.allow(&p, now + Duration::from_secs(30)));
        // After the window slides past the first request, capacity is freed.
        assert!(l.allow(&p, now + Duration::from_secs(61)));
    }

    #[test]
    fn limits_are_isolated_across_peers() {
        let now = Instant::now();
        let mut l = PeerRateLimiter::new(Duration::from_secs(60), 2, 8);
        let a = peer();
        let b = peer();
        l.allow(&a, now);
        l.allow(&a, now + Duration::from_millis(1));
        assert!(!l.allow(&a, now + Duration::from_millis(2)), "A burst full");
        // A different peer is unaffected.
        assert!(l.allow(&b, now + Duration::from_millis(2)));
    }

    #[test]
    fn ignores_disconnected_peers_once_expired() {
        let now = Instant::now();
        let mut l = PeerRateLimiter::new(Duration::from_secs(60), 3, 8);
        let p = peer();
        l.allow(&p, now);
        // Prune drops the bucket after expiry without panicking.
        l.prune(now + Duration::from_secs(120));
        assert!(l.allow(&p, now + Duration::from_secs(121)));
    }
}
