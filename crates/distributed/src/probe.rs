//! Bounded per-peer probe history and honest link-quality derivation (M9 P2).
//!
//! The coordinator pings each remote worker every ~5s (`InferPing`). A probe
//! either succeeds (we learn the round-trip time) or fails (the request errored
//! or timed out — a dropped packet). From a bounded window of those samples we
//! derive two link-quality signals the planner's `NetworkFacts` reads:
//!
//! - **jitter** — the mean absolute deviation (MAD) of the recent successful
//!   RTTs, in microseconds. A stable link has near-zero MAD; a toggling link
//!   shows a large MAD even if its mean RTT is low.
//! - **packet loss** — the fraction of failed probes in the window, as a
//!   percent. `0.0` = every probe succeeded (fully healthy window), `100.0` =
//!   every probe failed (likely offline / very lossy).
//!
//! This module is pure (no I/O): every rule is a function over a slice of
//! [`LinkSample`], so tests drive it with synthetic data. The raw RTT is owned
//! by the caller; the window is bounded so memory never grows unboundedly.

use std::collections::VecDeque;

/// How many probe samples are kept per peer for jitter/loss derivation. Enough
/// history to average out single blips, small enough to never matter.
pub const PROBE_WINDOW: usize = 12;

/// One coordinator→worker ping outcome.
///
/// - `Ok(rtt_us)`: the worker replied; `rtt_us` is the measured round-trip in
///   microseconds (the successful samples feed jitter).
/// - `Err`: the probe failed (request error / timeout). Counted as a lost
///   packet for the loss-rate derivation; it contributes no RTT.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinkSample {
    Ok(u32),
    Err,
}

/// A parser-independent window of samples for one peer. Callers push in arrival
/// order; the newest `PROBE_WINDOW` are kept.
#[derive(Debug, Clone, Default)]
pub struct ProbeWindow {
    samples: VecDeque<LinkSample>,
}

impl ProbeWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a sample; evicts the oldest once the window is full.
    pub fn push(&mut self, sample: LinkSample) {
        if self.samples.len() >= PROBE_WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The window's samples in arrival order (oldest first). Used by tests and
    /// by the derivation functions.
    pub fn samples(&self) -> &VecDeque<LinkSample> {
        &self.samples
    }
}

/// Mean absolute deviation of `rtus` around their mean. `None` when fewer than
/// two samples exist (a single point has no meaningful jitter).
fn mad(rtus: &[u32]) -> Option<u32> {
    if rtus.len() < 2 {
        return None;
    }
    let mean = rtus.iter().map(|&r| r as f64).sum::<f64>() / rtus.len() as f64;
    let dev = rtus.iter().map(|&r| (r as f64 - mean).abs()).sum::<f64>() / rtus.len() as f64;
    Some(dev.round() as u32)
}

/// Packet-loss rate as a percent in `0.0..=100.0` over the window. Empty window
/// = no signal → `None` (UNKNOWN). A full window of failures is `100.0`.
pub fn packet_loss_percent(samples: &VecDeque<LinkSample>) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let failed = samples.iter().filter(|s| matches!(s, LinkSample::Err)).count();
    Some((failed as f64 / samples.len() as f64) * 100.0)
}

/// Jitter (MAD over successful RTTs) in microseconds. `None` when there are
/// fewer than two successful samples (UNKNOWN, conservative).
pub fn jitter_us(samples: &VecDeque<LinkSample>) -> Option<u32> {
    let rtus: Vec<u32> = samples
        .iter()
        .filter_map(|s| match s {
            LinkSample::Ok(r) => Some(*r),
            LinkSample::Err => None,
        })
        .collect();
    mad(&rtus)
}

/// Derives both link-quality signals from a window in one pass. Pure and
/// deterministic; returns `(jitter_us, packet_loss_percent)` as insertion
/// order-independent.
pub fn derive_link_quality(samples: &VecDeque<LinkSample>) -> (Option<u32>, Option<f64>) {
    (jitter_us(samples), packet_loss_percent(samples))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(samples: &[LinkSample]) -> ProbeWindow {
        let mut w = ProbeWindow::new();
        for s in samples {
            w.push(*s);
        }
        w
    }

    #[test]
    fn empty_window_is_unknown() {
        let w = ProbeWindow::new();
        assert!(w.is_empty());
        assert_eq!(jitter_us(w.samples()), None);
        assert_eq!(packet_loss_percent(w.samples()), None);
    }

    #[test]
    fn constant_rtt_has_zero_jitter() {
        // All 12 peaks identical → MAD 0 (perfectly stable).
        let samples = vec![LinkSample::Ok(5_000); PROBE_WINDOW];
        let w = win(&samples);
        assert_eq!(jitter_us(w.samples()), Some(0));
        assert_eq!(packet_loss_percent(w.samples()), Some(0.0));
    }

    #[test]
    fn varying_rtt_shows_positive_jitter() {
        // Alternating 4ms/6ms → mean 5ms, MAD 1ms = 1000us.
        let samples = vec![
            LinkSample::Ok(4_000),
            LinkSample::Ok(6_000),
            LinkSample::Ok(4_000),
            LinkSample::Ok(6_000),
        ];
        let w = win(&samples);
        assert_eq!(jitter_us(w.samples()), Some(1_000));
    }

    #[test]
    fn single_rtt_has_no_jitter() {
        let w = win(&[LinkSample::Ok(5_000)]);
        assert_eq!(jitter_us(w.samples()), None);
    }

    #[test]
    fn half_loss_yields_50_percent() {
        let samples = vec![
            LinkSample::Ok(4_000),
            LinkSample::Err,
            LinkSample::Ok(4_000),
            LinkSample::Err,
        ];
        let w = win(&samples);
        assert_eq!(packet_loss_percent(w.samples()), Some(50.0));
        // Jitter computed over the two successful peaks only (both 4ms → 0).
        assert_eq!(jitter_us(w.samples()), Some(0));
    }

    #[test]
    fn failures_do_not_contribute_rtt_but_count_as_loss() {
        let samples = vec![LinkSample::Err; PROBE_WINDOW];
        let w = win(&samples);
        assert_eq!(packet_loss_percent(w.samples()), Some(100.0));
        assert_eq!(jitter_us(w.samples()), None);
    }

    #[test]
    fn window_is_bounded_to_probe_window() {
        let mut w = ProbeWindow::new();
        for i in 0..(PROBE_WINDOW * 2) {
            w.push(LinkSample::Ok(i as u32));
        }
        assert_eq!(w.len(), PROBE_WINDOW);
    }
}