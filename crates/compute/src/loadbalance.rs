//! Adaptive load-balance distribution (Next-Gen).
//!
//! The roadmap's adaptive fan-out / load-balancing real: distribute
//! independent requests across workers in proportion to each worker's real,
//! currently-useful capacity — NOT by splitting a single model across
//! devices (that is the parked `supports_staging()` tensor-split path). A
//! phone might get 30%, a laptop 20%, a desktop 50% for a batch of
//! independent requests.
//!
//! The share is a pure, deterministic function of the real availability
//! signals each worker already advertises:
//!
//! - **Throughput** (`tokens_per_second`) — how fast it actually serves.
//! - **Idle headroom** (`100 - load_percent`) — how much spare capacity.
//! - **Adaptive contribution factor** ([`ComputeAvailability::adaptive_contribution_factor`])
//!   — thermal/battery/GPU-util pressure, so a hot or low-battery worker gets
//!   a smaller share automatically.
//!
//! The result is a normalized share in `(0, 1]` per eligible worker, summing
//! to 1.0. It is **advisory only**: it tells the coordinator how to spread an
//! independent-request batch; it never splits a single generation. Workers
//! with no useful capacity get a zero (effectively excluded) share; the
//! remaining shares renormalize.
//!
//! Deterministic regardless of input order (ties broken by peer id), so tests
//! and operators get the same answer every time.

use serde::{Deserialize, Serialize};

use crate::availability::ComputeAvailability;

/// One worker's adaptive load share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadShare {
    /// Worker peer id.
    pub peer_id: String,
    /// Node id (human-readable, when present).
    pub node_id: String,
    /// Share of the independent-request batch this worker should receive,
    /// normalized in `(0, 1]` (sums to 1.0 across the returned set).
    pub share: f64,
    /// The raw (unnormalized) capacity weight this share was derived from, for
    /// transparency.
    pub weight: f64,
    /// The adaptive contribution factor (0..1) that scaled this worker's
    /// weight — 1.0 when healthy, lower under thermal/battery pressure.
    pub adaptive_factor: f32,
}

/// Computes the adaptive load-balance shares for a set of eligible workers.
///
/// `workers` is `(peer_id, node_id, availability)`. Only healthy workers are
/// considered. Returns shares sorted by (share desc, peer_id asc) so the
/// largest-share worker is first and the ordering is deterministic. Empty when
/// there are no healthy workers.
pub fn adaptive_load_shares(
    workers: &[(String, String, ComputeAvailability)],
) -> Vec<LoadShare> {
    // Healthy workers only (an unhealthy worker takes no share).
    let eligible: Vec<&(String, String, ComputeAvailability)> =
        workers.iter().filter(|(_, _, a)| a.healthy()).collect();
    if eligible.is_empty() {
        return Vec::new();
    }

    let mut weights: Vec<(String, String, f64, f32)> = Vec::new();
    for (peer, node, a) in &eligible {
        let tps = f64::from(a.tokens_per_second.max(1));
        let idle = f64::from(100_u8.saturating_sub(a.load_percent.min(100))).max(1.0) / 100.0;
        let adaptive = a.adaptive_contribution_factor();
        // Capacity weight = throughput × idle headroom × adaptive factor.
        // A hot / low-battery / loaded worker is scaled down automatically.
        let weight = tps * idle * f64::from(adaptive);
        weights.push((peer.clone(), node.clone(), weight, adaptive));
    }

    let total: f64 = weights.iter().map(|(_, _, w, _)| w).sum();
    if total <= 0.0 {
        return Vec::new();
    }

    let mut shares: Vec<LoadShare> = weights
        .into_iter()
        .map(|(peer, node, weight, adaptive)| LoadShare {
            share: weight / total,
            peer_id: peer,
            node_id: node,
            weight,
            adaptive_factor: adaptive,
        })
        .collect();
    // Deterministic: share desc, then peer id asc.
    shares.sort_by(|a, b| {
        b.share
            .partial_cmp(&a.share)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.peer_id.cmp(&b.peer_id))
    });
    shares
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::availability::WorkerHealth;

    fn avail(load: u8, tps: u32) -> ComputeAvailability {
        ComputeAvailability {
            available_ram_mb: 4096,
            available_vram_mb: None,
            load_percent: load,
            queue_depth: 0,
            tokens_per_second: tps,
            current_latency_ms: 10,
            status: WorkerHealth::Ready,
            gpu_temperature_celsius: None,
            gpu_utilization_percent: None,
            battery_percent: None,
        }
    }

    #[test]
    fn equal_workers_get_equal_shares() {
        let ws = vec![
            ("a".to_string(), "dca-a".to_string(), avail(10, 100)),
            ("b".to_string(), "dca-b".to_string(), avail(10, 100)),
        ];
        let shares = adaptive_load_shares(&ws);
        assert_eq!(shares.len(), 2);
        // Equal capacity -> ~0.5 each (within float rounding).
        let total: f64 = shares.iter().map(|s| s.share).sum();
        assert!((total - 1.0).abs() < 1e-6, "shares must sum to 1, got {total}");
        for s in &shares {
            assert!((s.share - 0.5).abs() < 1e-6, "equal workers -> 0.5 each, got {}", s.share);
        }
    }

    #[test]
    fn faster_worker_gets_a_larger_share() {
        let ws = vec![
            ("fast".to_string(), "dca-f".to_string(), avail(10, 300)),
            ("slow".to_string(), "dca-s".to_string(), avail(10, 100)),
        ];
        let shares = adaptive_load_shares(&ws);
        assert_eq!(shares[0].peer_id, "fast", "the faster worker is listed first");
        assert!(
            shares[0].share > shares[1].share,
            "faster worker must get a larger share ({} vs {})",
            shares[0].share,
            shares[1].share
        );
    }

    #[test]
    fn thermally_stressed_worker_gets_a_smaller_share() {
        let mut hot = avail(10, 200);
        hot.gpu_temperature_celsius = Some(95); // heavy pressure
        let ws = vec![
            ("healthy".to_string(), "dca-h".to_string(), avail(10, 200)),
            ("hot".to_string(), "dca-hot".to_string(), hot),
        ];
        let shares = adaptive_load_shares(&ws);
        assert_eq!(shares[0].peer_id, "healthy", "healthy worker wins the larger share");
        let hot_share = shares.iter().find(|s| s.peer_id == "hot").unwrap();
        assert!(
            hot_share.share < shares[0].share,
            "thermally-stressed worker gets less ({} vs {})",
            hot_share.share,
            shares[0].share
        );
        assert!(
            hot_share.adaptive_factor < 1.0,
            "adaptive factor records the pressure"
        );
    }

    #[test]
    fn unhealthy_worker_is_excluded() {
        let mut down = avail(10, 200);
        down.status = crate::availability::WorkerHealth::Unhealthy;
        let ws = vec![
            ("a".to_string(), "dca-a".to_string(), avail(10, 100)),
            ("down".to_string(), "dca-down".to_string(), down),
        ];
        let shares = adaptive_load_shares(&ws);
        assert_eq!(shares.len(), 1, "unhealthy worker takes no share");
        assert_eq!(shares[0].peer_id, "a");
        assert!((shares[0].share - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_or_all_unhealthy_yields_empty() {
        assert!(adaptive_load_shares(&[]).is_empty());
        let mut down = avail(10, 200);
        down.status = crate::availability::WorkerHealth::Unhealthy;
        let ws = vec![("a".to_string(), "dca-a".to_string(), down)];
        assert!(adaptive_load_shares(&ws).is_empty());
    }

    #[test]
    fn shares_are_deterministic_regardless_of_input_order() {
        let ws_a = vec![
            ("a".to_string(), "dca-a".to_string(), avail(20, 100)),
            ("b".to_string(), "dca-b".to_string(), avail(40, 200)),
            ("c".to_string(), "dca-c".to_string(), avail(60, 300)),
        ];
        let ws_b = {
            let mut v = ws_a.clone();
            v.reverse();
            v
        };
        let s1 = adaptive_load_shares(&ws_a);
        let s2 = adaptive_load_shares(&ws_b);
        let key = |s: &[LoadShare]| -> Vec<(String, u64)> {
            s.iter()
                .map(|x| (x.peer_id.clone(), (x.share * 1e6) as u64))
                .collect()
        };
        assert_eq!(key(&s1), key(&s2), "ordering and shares must be deterministic");
    }
}
