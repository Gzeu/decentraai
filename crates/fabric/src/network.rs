//! Network-aware scheduling inputs (M19).
//!
//! Execution planning must not rank workers in isolation: moving a model or a
//! prompt to a worker costs network resources, and a worker that is fast to a
//! *request* may be expensive if it sits far away. This module models the
//! inter-node *link* metrics the planner feeds on: measured round-trip latency,
//! bandwidth, locality (same host / VLAN / LAN), and a transfer-cost estimator.
//!
//! It is pure and I/O-free so the planner can be tested with synthetic link
//! graphs and the coordinator can populate it with real measurements (RTT via
//! the P2P ping/pong channel, throughput from live transfer accounting).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Topological locality between two nodes. Coarse but robust: DecentraAI runs
/// on trusted LANs, so "which subnet does this peer share" is a strong signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Locality {
    /// The peer is this node (loopback). Zero-copy, effectively free.
    Local,
    /// Same host (different container/process) — near-zero cost.
    SameHost,
    /// Same private network (LAN) — trusted, low latency.
    Lan,
    /// Reachable only over a wider network (WAN / relayed).
    Remote,
}

impl Locality {
    /// A unit-round-trip latency multiplier relative to loopback, used as a
    /// soft prior before a measured value exists.
    pub fn prior_rtt_us(self) -> u32 {
        match self {
            Self::Local => 0,
            Self::SameHost => 200,
            Self::Lan => 2_000,
            Self::Remote => 50_000,
        }
    }
}

/// Soft (fluent) prior bandwidth in megabits/sec per locality.
pub fn prior_bandwidth_mbps(locality: Locality) -> u32 {
    match locality {
        Locality::Local => u32::MAX,
        Locality::SameHost => 10_000,
        Locality::Lan => 1_000,
        Locality::Remote => 50,
    }
}

/// Live, serde-serializable measurements of the link from *this* node to a
/// remote worker. Populated by the coordinator from real activity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinkMetrics {
    /// Round-trip time to the peer in microseconds (measured ping/pong).
    pub rtt_us: u32,
    /// Measured throughput to the peer in megabits/sec. `0` = unmeasured.
    pub bandwidth_mbps: u32,
    /// Estimated transfer cost of moving 1 MiB to this peer in milliseconds.
    /// Derived, deterministic; owned here so callers read one number.
    pub transfer_ms_per_mib: u32,
    pub locality: Locality,
}

impl LinkMetrics {
    /// Builds metrics for a locality, using soft priors when nothing has been
    /// measured yet.
    pub fn prior(locality: Locality, measured_rtt_us: Option<u32>) -> Self {
        let rtt_us = measured_rtt_us.unwrap_or_else(|| locality.prior_rtt_us());
        let bandwidth_mbps = prior_bandwidth_mbps(locality);
        Self {
            rtt_us,
            bandwidth_mbps,
            transfer_ms_per_mib: transfer_ms_per_mib(bandwidth_mbps),
            locality,
        }
    }

    /// Recomputes the derived transfer estimate from the current bandwidth.
    pub fn refresh(mut self) -> Self {
        self.transfer_ms_per_mib = transfer_ms_per_mib(self.bandwidth_mbps);
        self
    }
}

/// Deterministic transfer-cost estimator.
///
/// Token-transfer dominates for inter-node generations; model/prompt transfer
/// is occasional (provisioning, KV relocation). We express everything as a
/// per-MiB transfer time in milliseconds so the planner can sum it linearly.
///
/// `bandwidth_mbps` is megabits/sec; MiB is 8 * 1.048576 Mbit. We ignore the
/// constant and return ms/MiB = (8 * 1024 * 1024 * 8 / bps) * 1000 → simplify
/// to ms/MiB = (8.39e6 * 8) / (mbps * 1e6) * 1000 ≈ 67_108.864 / mbps.
pub fn transfer_ms_per_mib(bandwidth_mbps: u32) -> u32 {
    if bandwidth_mbps == 0 || bandwidth_mbps == u32::MAX {
        bandwidth_mbps
    } else {
        ((67_108.864_f64 / bandwidth_mbps as f64).round()) as u32
    }
}

/// Estimated wall-clock cost of moving `data_mib` to a peer, in ms.
pub fn estimated_transfer_ms(data_mib: u64, link: &LinkMetrics) -> u32 {
    (data_mib as u32).saturating_mul(link.transfer_ms_per_mib)
}

/// The coordinator's view of the inter-node graph: for each peer, its link
/// back to this coordinator. Directed (coordinator-centric) because the
/// coordinator is the sole planner in the hub-and-spoke model.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkGraph {
    /// peer key (PeerId string) → measured link from this coordinator.
    by_peer: BTreeMap<String, LinkMetrics>,
}

impl NetworkGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records (or replaces) the link to a peer.
    pub fn set(&mut self, peer: &str, link: LinkMetrics) {
        self.by_peer.insert(peer.to_string(), link);
    }

    /// The current link to a peer, or a fresh `Lan` prior if unknown.
    pub fn get(&self, peer: &str) -> LinkMetrics {
        self.by_peer
            .get(peer)
            .copied()
            .unwrap_or_else(|| LinkMetrics::prior(Locality::Lan, None))
    }

    pub fn peers(&self) -> impl Iterator<Item = (&String, &LinkMetrics)> {
        self.by_peer.iter()
    }

    /// Number of peers with a recorded (measured) link.
    pub fn measured_len(&self) -> usize {
        self.by_peer.len()
    }

    /// Combined transport cost of reaching a peer: a small fixed per-request
    /// RTT term plus the transfer time for `prompt_and_model_mib`.
    pub fn reach_cost_ms(&self, peer: &str, prompt_and_model_mib: u64) -> u32 {
        let link = self.get(peer);
        link.rtt_us / 1000 + estimated_transfer_ms(prompt_and_model_mib, &link)
    }

    /// A deterministic total ordering (best link first) for tie-breaking when
    /// two workers score equally. Prefers low RTT, then high bandwidth.
    pub fn sort_peers(&self, peers: Vec<String>) -> Vec<String> {
        let mut with_link: Vec<(String, LinkMetrics)> = peers
            .into_iter()
            .map(|p| (p.clone(), self.get(&p)))
            .collect();
        with_link.sort_by(|a, b| {
            a.1.rtt_us
                .cmp(&b.1.rtt_us)
                .then_with(|| b.1.bandwidth_mbps.cmp(&a.1.bandwidth_mbps))
                .then_with(|| a.0.cmp(&b.0))
        });
        with_link.into_iter().map(|(p, _)| p).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_estimate_is_deterministic() {
        // 1 Gbps → ~0.067 ms/MiB → 0. Transfer tiny payloads are ~free.
        assert_eq!(transfer_ms_per_mib(1_000), 67);
        assert_eq!(estimated_transfer_ms(0, &LinkMetrics::prior(Locality::Lan, None)), 0);
    }

    #[test]
    fn reach_cost_combines_rtt_and_transfer() {
        let mut g = NetworkGraph::new();
        // 10ms RTT, 10 Mbps → 6710 ms/MiB.
        g.set(
            "far",
            LinkMetrics::prior(Locality::Remote, Some(10_000)),
        );
        // 1ms RTT, 1000 Mbps.
        g.set(
            "near",
            LinkMetrics::prior(Locality::Lan, Some(1_000)),
        );
        let cost_far = g.reach_cost_ms("far", 2);
        let cost_near = g.reach_cost_ms("near", 2);
        assert!(cost_far > cost_near, "far node must cost more to reach");
    }

    #[test]
    fn sorting_prefers_low_rtt_then_high_bandwidth() {
        let mut g = NetworkGraph::new();
        g.set("b", LinkMetrics::prior(Locality::Lan, Some(5_000))); // 5ms
        g.set("a", LinkMetrics::prior(Locality::Lan, Some(1_000))); // 1ms
        g.set("c", LinkMetrics::prior(Locality::Lan, Some(1_000))); // 1ms
        let order = g.sort_peers(vec!["a".into(), "b".into(), "c".into()]);
        assert_eq!(order, vec!["a", "c", "b"]);
    }

    #[test]
    fn unknown_peer_starts_at_lan_prior() {
        let g = NetworkGraph::new();
        let link = g.get("some-peer");
        assert_eq!(link.locality, Locality::Lan);
    }
}