//! Deterministic batch allocation for independent requests (Next-Gen).
//!
//! The roadmap's adaptive fan-out made **operational** for batches of
//! *independent* requests: given a set of independent requests and the live
//! fabric, produce a deterministic request → worker assignment that balances
//! load by each worker's real, currently-useful capacity
//! ([`decentraai_compute::adaptive_load_shares`]), while honoring the same
//! invariants the single-request planner guarantees:
//!
//! - never assign to an unhealthy / unavailable / untrusted / incompatible
//!   worker (eligibility is exactly the planner's `trusted && healthy &&
//!   serves_model`, plus capability / KV compatibility);
//! - never break KV/session affinity: a **continuation** request is pinned to
//!   the worker holding its KV prefix;
//! - deterministic regardless of input order (tie-break by request id / peer id).
//!
//! This is **request-level** fan-out only: it assigns whole, independent
//! requests to workers. It never splits a single generation/model across
//! workers (that stays gated behind `supports_staging()`, parked).
//!
//! The allocation is advisory at this boundary: each assigned request is then
//! executed through the existing single-request reservation/retry/quota path,
//! which is the source of truth for capacity and safety. If the assigned
//! worker is no longer available at dispatch time, that request safely falls
//! back to the planner's normal choice (never routed to an unhealthy worker).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::kv::KVCacheState;
use crate::planner::{PlannerConfig, RequestFacts, WorkerFacts};

/// One deterministic request → worker assignment in a batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchAssignment {
    /// The independent request's id (the caller maps this back to the request).
    pub request_id: String,
    /// The worker this request is assigned to.
    pub worker: String,
    /// The worker's adaptive share (0..1) of the batch, for observability.
    pub share: f64,
    /// Whether this was a KV-affinity pin (continuation to its prefix host)
    /// rather than a capacity-balanced assignment.
    pub kv_pinned: bool,
    /// Whether the worker is currently eligible (trusted + healthy + serves).
    /// If `false`, the request must fall back to normal planning at dispatch.
    pub eligible: bool,
}

/// The result of allocating a batch of independent requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchAllocation {
    /// Per-request assignments, in the order the requests were given.
    pub assignments: Vec<BatchAssignment>,
    /// The adaptive share per eligible worker used for the balanced subset
    /// (capacity-based assignments), for observability.
    pub worker_shares: BTreeMap<String, f64>,
}

/// A request id for batch allocation. We keep it minimal (just the string)
/// because the runtime owns the full request; the allocator only needs an id
/// to report provenance and to make the assignment deterministic.
type RequestId = String;

/// Allocates a batch of independent requests to workers deterministically.
///
/// # Safety / determinism invariants
///
/// - Eligibility is the planner's `trusted && healthy && serves_model` (never
///   an unhealthy / untrusted / incompatible worker). Capability-model
///   compatibility is assumed to be checked by the caller / planner before it
///   builds the batch (a request that no eligible worker serves gets a
///   non-eligible assignment so the caller can fail it honestly).
/// - A continuation request (`context.is_continuation`) is **pinned** to its
///   prefix-resident worker when that worker is eligible; otherwise it is
///   treated as a cold request (falls back to the capacity-balanced set), so
///   KV affinity is never broken.
/// - The capacity-balanced subset is distributed by
///   [`decentraai_compute::adaptive_load_shares`], which is deterministic.
/// - The assignment is deterministic regardless of input order: requests are
///   processed in ascending request-id order, and worker shares are iterated
///   in ascending peer-id order.
///
/// `workers` must be the live fabric facts (same `WorkerFacts` the planner
/// uses). `_config` is reserved for policy tuning; currently unused.
pub fn allocate_batch(
    requests: &[(RequestId, RequestFacts)],
    workers: &[WorkerFacts],
    _config: &PlannerConfig,
) -> BatchAllocation {
    // Deterministic worker set (peer id asc) + eligibility map.
    let by_id: BTreeMap<String, &WorkerFacts> =
        workers.iter().map(|w| (w.peer_id.clone(), w)).collect();

    // Eligible workers: trusted + healthy + serves the model. We do not check
    // capability claims here (the caller builds a batch of requests that are
    // already known-compatible); eligibility is the planner's capacity gate.
    let eligible_peers: Vec<String> = by_id
        .iter()
        .filter(|(_, w)| w.trusted && w.healthy && w.serves_model)
        .map(|(p, _)| p.clone())
        .collect();

    // Adaptive shares for the eligible set, from real availability signals.
    let share_inputs: Vec<(String, String, decentraai_compute::ComputeAvailability)> =
        eligible_peers
            .iter()
            .filter_map(|p| by_id.get(p))
            .map(|w| {
                // Convert WorkerFacts availability to the compute availability the
                // share function consumes. We carry the signals WorkerFacts holds.
                let a = decentraai_compute::ComputeAvailability {
                    available_ram_mb: w.available_ram_mb,
                    available_vram_mb: Some(w.available_vram_mb),
                    load_percent: w.load_percent,
                    queue_depth: w.queue_depth,
                    tokens_per_second: w.tokens_per_second,
                    current_latency_ms: w.latency_ms,
                    status: if w.healthy {
                        decentraai_compute::WorkerHealth::Ready
                    } else {
                        decentraai_compute::WorkerHealth::Unhealthy
                    },
                    gpu_temperature_celsius: None,
                    gpu_utilization_percent: None,
                    battery_percent: None,
                };
                (w.peer_id.clone(), String::new(), a)
            })
            .collect();
    let shares = decentraai_compute::adaptive_load_shares(&share_inputs);
    let worker_shares: BTreeMap<String, f64> = shares
        .iter()
        .map(|s| (s.peer_id.clone(), s.share))
        .collect();
    // Workers with a zero share (excluded) still must not be assigned; the
    // allocator only uses the shares map for the balanced set below.

    // Deterministic request order: ascending request id.
    let mut ordered: Vec<&(RequestId, RequestFacts)> = requests.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(&b.0));

    // Weighted interleaving for the capacity-balanced subset: produce a
    // deterministic sequence of worker ids where each worker appears
    // proportionally to its share AND spread evenly (not all of one worker
    // first). We use a greedy largest-remainder scheme: repeatedly emit the
    // worker with the largest "unmet" proportion relative to its share.
    // Deterministic regardless of the shares-map iteration order.
    let n_requests = ordered.len();
    let mut expanded: Vec<String> = Vec::with_capacity(n_requests);
    if !worker_shares.is_empty() {
        // Target counts by largest-remainder apportionment.
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let total_share: f64 = worker_shares.values().sum();
        let mut floor_total = 0usize;
        for (peer, share) in &worker_shares {
            let exact = share / total_share * n_requests as f64;
            let floor = exact.floor() as usize;
            counts.insert(peer.clone(), floor);
            floor_total += floor;
        }
        // Distribute the remaining seats to the largest fractional remainders
        // (ties broken by peer id asc, deterministic).
        let mut remainders: Vec<(String, f64)> = worker_shares
            .iter()
            .map(|(peer, share)| {
                let exact = share / total_share * n_requests as f64;
                (peer.clone(), exact - exact.floor())
            })
            .collect();
        remainders.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        for (peer, _) in remainders {
            if floor_total >= n_requests {
                break;
            }
            if let Some(c) = counts.get_mut(&peer) {
                *c += 1;
                floor_total += 1;
            }
        }
        // Interleave the counts so shares spread evenly.
        let mut remaining: BTreeMap<String, usize> = counts;
        let total_remaining: usize = remaining.values().sum();
        let total_remaining = total_remaining.max(n_requests); // safety
        let mut emitted_total = 0usize;
        while emitted_total < n_requests {
            // Pick the worker whose emitted-so-far is furthest below its
            // target proportion (largest unmet), tie-break peer id asc.
            let mut best: Option<String> = None;
            let mut best_gap = f64::MIN;
            let mut best_key = String::new();
            for (peer, cnt) in &remaining {
                if *cnt == 0 {
                    continue;
                }
                // Gap = target proportion minus emitted proportion.
                let target = *cnt as f64 / total_remaining as f64;
                // emitted count for this peer so far:
                let emitted = expanded.iter().filter(|p| *p == peer).count();
                let gap = target - (emitted as f64 / emitted_total.max(1) as f64);
                let gap = if emitted_total == 0 { target } else { gap };
                if gap > best_gap || (gap == best_gap && peer < &best_key) {
                    best_gap = gap;
                    best_key = peer.clone();
                    best = Some(peer.clone());
                }
            }
            match best {
                Some(peer) => {
                    expanded.push(peer.clone());
                    if let Some(c) = remaining.get_mut(&peer) {
                        *c = c.saturating_sub(1);
                    }
                    emitted_total += 1;
                }
                None => break,
            }
        }
    }
    if expanded.is_empty() {
        // No capacity signals -> preserve conservative behavior: use the
        // planner's own ordering (which is deterministic) so we never invent
        // capacity. Fall back to plain round-robin over eligible workers.
        expanded = eligible_peers.clone();
    }

    let mut assignments: Vec<BatchAssignment> = Vec::new();
    let mut rr = 0usize;
    for (request_id, rfacts) in ordered {
        // KV affinity: a continuation is pinned to its prefix-resident worker.
        let mut kv_pinned = false;
        let mut assigned: Option<String> = None;
        if rfacts.context.is_continuation {
            if let Some(prefix) = &rfacts.context.prefix_resident_on {
                if by_id
                    .get(prefix)
                    .is_some_and(|w| w.trusted && w.healthy && w.serves_model)
                {
                    assigned = Some(prefix.clone());
                    kv_pinned = true;
                }
            }
        }
        let worker = assigned.unwrap_or_else(|| {
            // Capacity-balanced: weighted round-robin over the eligible set.
            if expanded.is_empty() {
                String::new()
            } else {
                let w = expanded[rr % expanded.len()].clone();
                rr += 1;
                w
            }
        });
        let eligible = by_id
            .get(&worker)
            .is_some_and(|w| w.trusted && w.healthy && w.serves_model);
        let share = worker_shares.get(&worker).copied().unwrap_or(0.0);
        assignments.push(BatchAssignment {
            request_id: request_id.clone(),
            worker,
            share,
            kv_pinned,
            eligible,
        });
    }

    // Restore the caller's original request order for the report.
    let original_order: std::collections::HashMap<String, usize> = requests
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (id.clone(), i))
        .collect();
    assignments.sort_by_key(|a| {
        original_order
            .get(&a.request_id)
            .copied()
            .unwrap_or(usize::MAX)
    });

    BatchAllocation {
        assignments,
        worker_shares,
    }
}

/// A helper to build a continuation worker facts set from a KV residency map
/// (used by tests / the runtime to construct the fabric facts for a batch).
/// `kv_residency` maps peer_id -> whether it holds the prefix (used to set the
/// request's `prefix_resident_on`); kept here so the batch allocator's KV
/// logic is testable without the runtime.
pub fn set_kv_affinity(
    workers: &[WorkerFacts],
    kv_residency: &std::collections::HashMap<String, bool>,
) -> Vec<WorkerFacts> {
    workers
        .iter()
        .map(|w| {
            let mut w2 = w.clone();
            w2.kv = if kv_residency.get(&w.peer_id).copied().unwrap_or(false) {
                KVCacheState::Partial {
                    used: 100,
                    capacity: 4096,
                }
            } else {
                KVCacheState::Empty
            };
            w2
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{EngineCapabilities, EngineKind};
    use crate::kv::{ContextProfile, KVCacheState};

    fn worker(id: &str, healthy: bool, tps: u32, load: u8) -> WorkerFacts {
        WorkerFacts {
            peer_id: id.to_string(),
            trusted: true,
            healthy,
            engine: EngineKind::LlamaServer,
            tokens_per_second: tps,
            latency_ms: 40,
            perf_measured: false,
            queue_depth: 0,
            load_percent: load,
            available_ram_mb: 4096,
            available_vram_mb: 0,
            serves_model: healthy,
            available_models: vec![],
            capabilities: EngineCapabilities::conservative(),
            kv: KVCacheState::Empty,
        }
    }

    fn req(id: &str) -> (String, RequestFacts) {
        (
            id.to_string(),
            RequestFacts {
                model_hash: "m1".into(),
                est_ram_mb: 512,
                est_vram_mb: 0,
                context: ContextProfile {
                    prompt_tokens: 100,
                    max_output_tokens: 200,
                    is_continuation: false,
                    prefix_resident_on: None,
                },
                transfer_mib: 0,
                local_peer: None,
                priority: 0,
                required_capability: None,
                capability_claims: Vec::new(),
            },
        )
    }

    fn cont_req(id: &str, prefix: &str) -> (String, RequestFacts) {
        let (id, mut r) = req(id);
        r.context.is_continuation = true;
        r.context.prefix_resident_on = Some(prefix.to_string());
        (id, r)
    }

    #[test]
    fn two_equal_workers_balance_independent_requests() {
        let ws = vec![worker("a", true, 100, 10), worker("b", true, 100, 10)];
        let reqs = vec![req("r1"), req("r2"), req("r3"), req("r4")];
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        assert_eq!(alloc.assignments.len(), 4);
        // Equal capacity -> ~50/50 split.
        let count = |w: &str| alloc.assignments.iter().filter(|a| a.worker == w).count();
        assert!(
            (count("a") as i64 - count("b") as i64).abs() <= 1,
            "equal workers balance requests (a={}, b={})",
            count("a"),
            count("b")
        );
        // Every assignment is eligible.
        assert!(alloc.assignments.iter().all(|a| a.eligible));
    }

    #[test]
    fn faster_worker_gets_more_requests() {
        let ws = vec![worker("fast", true, 300, 10), worker("slow", true, 100, 10)];
        let reqs: Vec<(String, RequestFacts)> = (0..20).map(|i| req(&format!("r{i}"))).collect();
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        let count = |w: &str| alloc.assignments.iter().filter(|a| a.worker == w).count();
        assert!(
            count("fast") > count("slow"),
            "faster worker gets more requests (fast={}, slow={})",
            count("fast"),
            count("slow")
        );
    }

    #[test]
    fn unhealthy_worker_is_never_assigned() {
        let ws = vec![worker("down", false, 300, 10), worker("ok", true, 100, 10)];
        let reqs = vec![req("r1"), req("r2")];
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        assert!(
            alloc.assignments.iter().all(|a| a.worker != "down"),
            "unhealthy worker must never receive a request"
        );
        assert!(alloc.assignments.iter().all(|a| a.worker == "ok"));
    }

    #[test]
    fn limited_worker_gets_a_smaller_share_than_an_idle_one() {
        // A LIMITED worker (healthy but heavily loaded) is scaled down by the
        // adaptive factor, so an identical idle worker wins more requests.
        let limited = worker("busy", true, 100, 90); // load 90 -> LIMITED-ish
        let idle = worker("idle", true, 100, 5);
        let ws = vec![limited, idle];
        let reqs: Vec<(String, RequestFacts)> = (0..12).map(|i| req(&format!("r{i}"))).collect();
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        let count = |w: &str| alloc.assignments.iter().filter(|a| a.worker == w).count();
        assert!(
            count("idle") > count("busy"),
            "idle worker gets more requests than a heavily-loaded one (idle={}, busy={})",
            count("idle"),
            count("busy")
        );
    }

    #[test]
    fn incompatible_worker_serves_no_requests() {
        // A worker that does not serve the model (serves_model=false) is never
        // assigned; the request fails honestly rather than routing to it.
        let mut no_model = worker("no-model", true, 300, 10);
        no_model.serves_model = false;
        let ok = worker("ok", true, 100, 10);
        let ws = vec![no_model.clone(), ok];
        let reqs = vec![req("r1"), req("r2")];
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        assert!(
            alloc.assignments.iter().all(|a| a.worker != "no-model"),
            "a worker that does not serve the model must never receive the request"
        );
        assert!(
            alloc
                .assignments
                .iter()
                .all(|a| a.worker == "ok" && a.eligible)
        );
    }

    #[test]
    fn batch_allocation_covers_every_request_exactly_once() {
        let ws = vec![worker("a", true, 200, 10), worker("b", true, 100, 10)];
        let n = 15;
        let reqs: Vec<(String, RequestFacts)> = (0..n).map(|i| req(&format!("r{i}"))).collect();
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        assert_eq!(
            alloc.assignments.len(),
            n,
            "every request gets an assignment"
        );
        // Each request id appears exactly once.
        let mut ids: std::collections::BTreeSet<String> = alloc
            .assignments
            .iter()
            .map(|a| a.request_id.clone())
            .collect();
        for (id, _) in &reqs {
            assert!(ids.remove(id), "request {id} assigned exactly once");
        }
        assert!(ids.is_empty(), "no extra assignments");
        // Every assignment is to an eligible worker.
        assert!(alloc.assignments.iter().all(|a| a.eligible));
    }

    #[test]
    fn provenance_preserves_request_id_and_worker() {
        let ws = vec![worker("a", true, 200, 10), worker("b", true, 200, 10)];
        let reqs = vec![req("alpha"), req("beta"), req("gamma")];
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        // The report restores input order; every assignment carries its
        // request id + worker + eligibility + share.
        for a in &alloc.assignments {
            assert!(a.request_id == "alpha" || a.request_id == "beta" || a.request_id == "gamma");
            assert!(a.worker == "a" || a.worker == "b");
            assert!(a.eligible);
            assert!(a.share >= 0.0 && a.share <= 1.0);
        }
    }

    #[test]
    fn continuation_is_pinned_to_its_kv_worker() {
        let mut ws = vec![
            worker("host", true, 100, 10),
            worker("other", true, 300, 10),
        ];
        let residency: std::collections::HashMap<String, bool> =
            [("host".to_string(), true)].into();
        ws = set_kv_affinity(&ws, &residency);
        let reqs = vec![cont_req("c1", "host")];
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        assert_eq!(alloc.assignments[0].worker, "host");
        assert!(alloc.assignments[0].kv_pinned);
        // The faster worker is NOT chosen despite its throughput, because the
        // KV affinity must be preserved.
    }

    #[test]
    fn allocation_is_deterministic_regardless_of_input_order() {
        let ws = vec![worker("a", true, 200, 10), worker("b", true, 100, 20)];
        let reqs = vec![
            req("r1"),
            req("r2"),
            req("r3"),
            req("r4"),
            req("r5"),
            req("r6"),
        ];
        let a1 = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        let mut rev = reqs.clone();
        rev.reverse();
        let a2 = allocate_batch(&rev, &ws, &PlannerConfig::default());
        // The report preserves each batch's input order; the REAL determinism
        // invariant is that every request id maps to the same worker
        // regardless of how the batch was ordered. Compare per-request-id.
        let key = |a: &BatchAllocation| -> std::collections::BTreeMap<String, String> {
            a.assignments
                .iter()
                .map(|x| (x.request_id.clone(), x.worker.clone()))
                .collect()
        };
        assert_eq!(
            key(&a1),
            key(&a2),
            "each request id maps to the same worker"
        );
    }

    #[test]
    fn empty_batch_or_no_eligible_workers_is_honest() {
        let ws = vec![worker("down", false, 100, 10)];
        let reqs = vec![req("r1")];
        let alloc = allocate_batch(&reqs, &ws, &PlannerConfig::default());
        // No eligible worker: the request is assigned non-eligible so the
        // runtime fails it honestly rather than routing to an unhealthy worker.
        assert_eq!(alloc.assignments[0].worker, "");
        assert!(!alloc.assignments[0].eligible);

        assert!(
            allocate_batch(&[], &ws, &PlannerConfig::default())
                .assignments
                .is_empty()
        );
    }
}
