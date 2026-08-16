//! Coordinator-side KV-cache / session accounting (M20).
//!
//! The coordinator must make KV-aware placement decisions *honestly*: it
//! should not invent engine telemetry that llama-server does not expose. The
//! real engine (`llama-server`) reports its KV *capacity* (`n_ctx`) but not
//! its live *occupied* slots in a form we consume. So this module keeps
//! coordinator-owned accounting of which worker holds which conversation's
//! KV prefix, derived from the real requests we route and their real
//! `tokens_used`:
//!
//! - **Session residency**: a `session_id` maps to the worker + model that
//!   already holds that session's KV prefix. A continuation request carrying
//!   the same `session_id` should be steered back to that worker (cache
//!   locality) — cold prefill elsewhere would waste the resident prefix.
//! - **KV headroom per worker**: for a model with real capacity
//!   (`ServedModel.context_tokens`, i.e. `n_ctx`), the coordinator sums the
//!   tracked `tokens_used` of every resident session on that worker to derive
//!   an honest `(used, capacity)` pair → [`KVCacheState::Partial`], which the
//!   fabric planner reads as headroom.
//!
//! Engines that advertise no context capacity (`context_tokens == 0`) present
//! [`KVCacheState::Empty`] (unbounded) and the planner falls back to
//! context-length-only routing — correct but less informed. Nothing here is
//! fabricated from the engine; every number comes from real requests we
//! routed and a real `n_ctx` a worker advertised.

use libp2p::PeerId;
use std::collections::BTreeMap;

/// Accounted KV-cache state for one routed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionKv {
    /// Worker that currently holds this session's KV prefix.
    pub worker: PeerId,
    /// Model the prefix belongs to (affinity is model-scoped).
    pub model_hash: String,
    /// KV slots this session occupies (real `tokens_used` from the last
    /// routed request in this session).
    pub tokens_used: u32,
    /// The worker's context capacity for this model (`n_ctx`), `0` if unknown.
    pub capacity: u32,
}

/// Coordinator-side, synchronous account of session->worker residency.
#[derive(Debug, Default)]
pub struct SessionAccount {
    by_session: BTreeMap<String, SessionKv>,
}

impl SessionAccount {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records (or refreshes) where a session's KV prefix lives after a
    /// routed request completed. `tokens_used` is the real input+output
    /// tokens reported by the worker for this request.
    pub fn record(
        &mut self,
        session_id: &str,
        worker: PeerId,
        model_hash: &str,
        tokens_used: u32,
        capacity: u32,
    ) {
        self.by_session.insert(
            session_id.to_string(),
            SessionKv {
                worker,
                model_hash: model_hash.to_string(),
                tokens_used,
                capacity,
            },
        );
    }

    /// Whether a session is known and resident on a worker.
    pub fn lookup(&self, session_id: &str) -> Option<&SessionKv> {
        self.by_session.get(session_id)
    }

    /// The worker that holds a session's prefix, if any.
    pub fn residency(&self, session_id: &str) -> Option<PeerId> {
        self.lookup(session_id).map(|s| s.worker)
    }

    /// Honest per-worker KV occupancy for a model: the sum of `tokens_used`
    /// over every resident session on `worker` for `model_hash` with a known
    /// capacity. Returns `(used, capacity)`, or `None` when the worker
    /// advertises no context capacity (`n_ctx == 0` → treat as unbounded).
    pub fn worker_kv_used(&self, worker: &PeerId, model_hash: &str) -> Option<(u32, u32)> {
        let mut cap = 0u32;
        let mut used = 0u32;
        for s in self.by_session.values() {
            if &s.worker == worker && s.model_hash == model_hash {
                cap = cap.max(s.capacity);
                used = used.saturating_add(s.tokens_used);
            }
        }
        if cap == 0 {
            // No advertised capacity: cannot derive a headroom figure.
            None
        } else {
            Some((used, cap))
        }
    }

    /// Number of distinct tracked sessions.
    pub fn len(&self) -> usize {
        self.by_session.len()
    }

    /// Snapshot of every tracked session (sorted by session id via BTreeMap),
    /// for observability. Each entry is real accounted state.
    pub fn snapshot(&self) -> Vec<(String, SessionKv)> {
        self.by_session.iter().map(|(id, kv)| (id.clone(), kv.clone())).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.by_session.is_empty()
    }

    /// Drops sessions that are no longer relevant — e.g. after a worker is
    /// evicted/offline — so stale residency never steers routing to a dead
    /// worker. Returns the number of removed sessions.
    pub fn drop_worker(&mut self, worker: &PeerId) -> usize {
        let before = self.by_session.len();
        self.by_session.retain(|_, s| &s.worker != worker);
        before - self.by_session.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn peer_a() -> PeerId {
        PeerId::from_str("12D3KooWDpJ7As7BWAwRMfu1VU2WCqNjvq387JEYKDBj4kx6nXTN").expect("peer id")
    }
    fn peer_b() -> PeerId {
        PeerId::from_str("12D3KooWJryUnTdcqKPPRajShWkLGXsKbN1H5nHNbv5EgMYXR4d4").expect("peer id")
    }

    #[test]
    fn records_and_looks_up_residency() {
        let mut acc = SessionAccount::new();
        acc.record("s1", peer_a(), "m1", 512, 2048);
        assert_eq!(acc.residency("s1"), Some(peer_a()));
        assert_eq!(acc.lookup("s1").unwrap().tokens_used, 512);
        assert_eq!(acc.residency("unknown"), None);
    }

    #[test]
    fn derives_per_worker_kv_used_sums_sessions() {
        let mut acc = SessionAccount::new();
        acc.record("s1", peer_a(), "m1", 300, 2048);
        acc.record("s2", peer_a(), "m1", 200, 2048);
        acc.record("s3", peer_b(), "m1", 900, 2048);
        let (used, cap) = acc.worker_kv_used(&peer_a(), "m1").unwrap();
        assert_eq!(used, 500);
        assert_eq!(cap, 2048);
        // Different worker unaffected.
        assert_eq!(acc.worker_kv_used(&peer_b(), "m1").unwrap().0, 900);
    }

    #[test]
    fn no_capacity_means_unbounded() {
        let mut acc = SessionAccount::new();
        acc.record("s1", peer_a(), "m1", 500, 0);
        assert_eq!(acc.worker_kv_used(&peer_a(), "m1"), None);
    }

    #[test]
    fn dropping_a_worker_clears_its_sessions() {
        let mut acc = SessionAccount::new();
        acc.record("s1", peer_a(), "m1", 100, 1024);
        acc.record("s2", peer_b(), "m1", 100, 1024);
        assert_eq!(acc.drop_worker(&peer_a()), 1);
        assert_eq!(acc.residency("s1"), None);
        assert_eq!(acc.residency("s2"), Some(peer_b()));
    }
}
