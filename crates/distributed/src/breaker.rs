//! Per-worker circuit breaker (P5).
//!
//! A consistently failing worker should not be re-selected and re-tried on
//! every request — booking a reservation, waiting out a timeouts, then
//! backoff/retrying, again and again. This breaker pulls such a worker out of
//! contention for a cooldown window after it accumulates enough consecutive
//! retryable failures.
//!
//! # Trip / recovery
//! - A worker "trips" (opens) after `threshold` **consecutive** retryable
//!   failures, all within `window`.
//! - It stays open for `cooldown`. During that time `allow()` returns `false`,
//!   so the planner never selects it and no reservation is booked on it.
//! - After `cooldown` it is re-eligible (a natural "half-open with real
//!   traffic": if still wedged it fails `threshold` times again and re-opens).
//! - A success resets the consecutive-failure run.
//!
//! # Relationship to reputation
//! This is local, coordinator-side operational hygiene — NOT reputation. It
//! never touches the reputation store, bans, or contribution scores. Only
//! [retryable][`crate::DistributedError::is_retryable`] transport outcomes
//! trip it (connection failures / timeouts); a definitive worker rejection or
//! a cancellation never trips it (no re-send ⇒ no reason to think the worker
//! is broken).
//!
//! Pure and I/O-free: `now` is injected so tests drive it with synthetic time.

use libp2p::PeerId;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Tunable trip/recovery parameters (P5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerConfig {
    /// Consecutive retryable failures before a worker opens.
    pub threshold: u32,
    /// Window (seconds) in which the consecutive failures must occur.
    pub window: Duration,
    /// How long an opened worker stays out of contention.
    pub cooldown: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            threshold: 3,
            window: Duration::from_secs(5),
            cooldown: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed { failures: u32, fresh_at: Instant },
    Open { until: Instant },
}

/// Per-worker circuit breaker.
#[derive(Debug)]
pub struct CircuitBreaker {
    workers: HashMap<PeerId, State>,
    cfg: BreakerConfig,
}

impl CircuitBreaker {
    pub fn new(cfg: BreakerConfig) -> Self {
        Self {
            workers: HashMap::new(),
            cfg,
        }
    }

    /// Whether a request may currently be routed to `peer`.
    pub fn allow(&self, peer: &PeerId, now: Instant) -> bool {
        match self.workers.get(peer) {
            None | Some(State::Closed { .. }) => true,
            Some(State::Open { until }) => now >= *until,
        }
    }

    /// Records a retryable failure for `peer`. Trips (opens) the worker when
    /// the consecutive-failure count within the window reaches the threshold.
    pub fn record_failure(&mut self, peer: &PeerId, now: Instant) {
        let cfg = self.cfg;
        let next = match self.workers.get(peer) {
            None => {
                if 1 >= cfg.threshold {
                    State::Open {
                        until: now + cfg.cooldown,
                    }
                } else {
                    State::Closed {
                        failures: 1,
                        fresh_at: now,
                    }
                }
            }
            Some(State::Closed { failures, fresh_at }) => {
                // A stale run (beyond the window) restarts the count.
                if now.duration_since(*fresh_at) > cfg.window || *failures >= cfg.threshold {
                    if 1 >= cfg.threshold {
                        State::Open {
                            until: now + cfg.cooldown,
                        }
                    } else {
                        State::Closed {
                            failures: 1,
                            fresh_at: now,
                        }
                    }
                } else {
                    let failures = failures + 1;
                    if failures >= cfg.threshold {
                        State::Open {
                            until: now + cfg.cooldown,
                        }
                    } else {
                        State::Closed {
                            failures,
                            fresh_at: *fresh_at,
                        }
                    }
                }
            }
            Some(State::Open { .. }) => State::Open {
                until: now + cfg.cooldown,
            },
        };
        self.workers.insert(*peer, next);
    }

    /// Records a success for `peer`, resetting any consecutive-failure run.
    pub fn record_success(&mut self, peer: &PeerId, now: Instant) {
        if let Some(State::Open { until }) = self.workers.get(peer) {
            // If it was open but now succeeded, close it cleanly.
            if now < *until {
                // (Shouldn't normally be routed while open, but be safe.)
                self.workers.insert(*peer, State::Closed {
                    failures: 0,
                    fresh_at: now,
                });
                return;
            }
        }
        self.workers.insert(*peer, State::Closed {
            failures: 0,
            fresh_at: now,
        });
    }

    /// Returns when `peer` is next eligible after an open, or `None` if closed.
    pub fn open_until(&self, peer: &PeerId) -> Option<Instant> {
        match self.workers.get(peer) {
            Some(State::Open { until }) => Some(*until),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerId {
        PeerId::random()
    }

    #[test]
    fn trips_after_threshold_failures() {
        let now = Instant::now();
        let mut b = CircuitBreaker::new(BreakerConfig {
            threshold: 3,
            window: Duration::from_secs(5),
            cooldown: Duration::from_secs(30),
        });
        let p = peer();
        assert!(b.allow(&p, now));
        b.record_failure(&p, now);
        b.record_failure(&p, now + Duration::from_millis(1));
        // Below threshold: still allowed.
        assert!(b.allow(&p, now + Duration::from_millis(2)));
        b.record_failure(&p, now + Duration::from_millis(3));
        // Tripped.
        assert!(!b.allow(&p, now + Duration::from_millis(4)));
    }

    #[test]
    fn recovers_after_cooldown() {
        let t0 = Instant::now();
        let mut b = CircuitBreaker::new(BreakerConfig {
            threshold: 1,
            window: Duration::from_secs(5),
            cooldown: Duration::from_secs(30),
        });
        let p = peer();
        b.record_failure(&p, t0);
        assert!(!b.allow(&p, t0 + Duration::from_secs(10)));
        // After cooldown, allowed again (natural re-probe via real traffic).
        assert!(b.allow(&p, t0 + Duration::from_secs(31)));
    }

    #[test]
    fn healthy_and_other_workers_are_unaffected() {
        let now = Instant::now();
        let mut b = CircuitBreaker::new(BreakerConfig {
            threshold: 1,
            window: Duration::from_secs(5),
            cooldown: Duration::from_secs(30),
        });
        let a = peer();
        let c = peer();
        b.record_failure(&a, now);
        assert!(!b.allow(&a, now + Duration::from_millis(1)));
        assert!(b.allow(&c, now + Duration::from_millis(1)), "other workers unaffected");
        assert!(b.allow(&peer(), now + Duration::from_millis(1)));
    }

    #[test]
    fn success_resets_the_failure_run() {
        let now = Instant::now();
        let mut b = CircuitBreaker::new(BreakerConfig {
            threshold: 3,
            window: Duration::from_secs(5),
            cooldown: Duration::from_secs(30),
        });
        let p = peer();
        b.record_failure(&p, now);
        b.record_failure(&p, now + Duration::from_millis(1));
        b.record_success(&p, now + Duration::from_millis(2));
        // Run reset: 2 more failures won't trip (needs 3 consecutive).
        b.record_failure(&p, now + Duration::from_millis(3));
        b.record_failure(&p, now + Duration::from_millis(4));
        assert!(b.allow(&p, now + Duration::from_millis(5)));
    }

    #[test]
    fn stale_run_beyond_window_restarts_without_tripping() {
        let t0 = Instant::now();
        let mut b = CircuitBreaker::new(BreakerConfig {
            threshold: 3,
            window: Duration::from_secs(5),
            cooldown: Duration::from_secs(30),
        });
        let p = peer();
        b.record_failure(&p, t0);
        b.record_failure(&p, t0 + Duration::from_millis(1));
        // 3rd failure well beyond the window: run restarts (count=1), no trip.
        b.record_failure(&p, t0 + Duration::from_secs(60));
        assert!(b.allow(&p, t0 + Duration::from_secs(61)));
    }
}