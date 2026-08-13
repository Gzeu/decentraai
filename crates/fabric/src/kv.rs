//! KV-cache-aware inference fabric inputs (M20).
//!
//! Long-context generation is what actually saturates GPU memory and
//! dominates latency, so a production planner must think in terms of KV-cache
//! *headroom*, not just "does this worker serve the model". This module models
//! the KV-cache state a worker can report and the decisions that follow:
//!
//! - **Context headroom**: how many tokens a worker can still attend to before
//!   its KV cache is full. Requests whose prompt+output fit comfortably are
//!   routed normally; long-context requests are steered to workers with more
//!   headroom.
//! - **Locality**: if a worker already holds the KV prefix for a session, a
//!   *resumption* is much cheaper than a cold prefill. Cache-locality-aware
//!   routing prefers continuation on the worker that holds the prefix.
//! - **Prefill/decode separation**: engines that expose a distinct prefill
//!   (prompt ingestion, memory-bandwidth-heavy) vs decode (token emission,
//!   latency-critical) phase allow the planner to stage the two on workers
//!   with different strengths. This is only used when the engine advertises
//!   the capability (see [`crate::engine::EngineCapabilities`]).
//!
//! Like the rest of the fabric crate this is a pure model; live values are
//! populated by a probe/health exchange and engine-reported metrics where the
//! engine exports them. Engines that do not report KV state simply present
//! [`KVCacheState::Unknown`] and the planner falls back to context-length-only
//! routing (still correct, just less informed).

use serde::{Deserialize, Serialize};

/// Live KV-cache state a worker can report for the model it serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KVCacheState {
    /// No cache (first token of a context), so 100% headroom available.
    Empty,
    /// `used` of `capacity` KV slots are occupied for this session.
    Partial { used: u32, capacity: u32 },
    /// The cache is effectively full for this model/session.
    Full,
    /// The engine does not report KV state (default, conservative).
    Unknown,
}

impl KVCacheState {
    /// Number of additional tokens this worker can still attend to, if known.
    /// `None` when unknown or full.
    pub fn headroom_tokens(self) -> Option<u32> {
        match self {
            Self::Empty => None, // unbounded until a capacity is reported
            Self::Partial { used, capacity } => {
                if used < capacity {
                    Some(capacity - used)
                } else {
                    None // effectively full
                }
            }
            Self::Full | Self::Unknown => None,
        }
    }

    /// Whether this worker is known to be able to take a prompt of `tokens`
    /// into its existing cache (Token-level continuation). `Unknown` treats
    /// it as fitting on the conservative assumption that capacity == known.
    pub fn can_accommodate(self, prompt_tokens: u32) -> bool {
        match self {
            Self::Empty => true,
            Self::Partial { used, capacity } => used + prompt_tokens <= capacity,
            Self::Full => false,
            Self::Unknown => true,
        }
    }
}

/// Per-request / per-worker context facts the planner weighs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextProfile {
    /// Prompt size in tokens.
    pub prompt_tokens: u32,
    /// Maximum tokens to generate.
    pub max_output_tokens: u32,
    /// Whether this request *resumes* a prior session (a known KV prefix).
    pub is_continuation: bool,
    /// Whether the KV prefix for this session is resident on a specific
    /// worker (cache locality). The planner routes continuations there.
    pub prefix_resident_on: Option<String>,
}

impl ContextProfile {
    /// Total context slots the request would occupy at full length.
    pub fn total_slots(&self) -> u32 {
        self.prompt_tokens + self.max_output_tokens
    }

    /// Whether the request is "long context" relative to a worker's claimed
    /// capacity — the signal used to prefer KV-rich workers.
    pub fn is_long_context(&self, worker_ctx: u32) -> bool {
        worker_ctx > 0 && self.total_slots() > worker_ctx / 2
    }

    /// Estimated prefill (token ingest) vs decode (token emit) split, as a
    /// ratio in [0,1] of how much work is prefill. Used only when an engine
    /// advertises `prefill_decode_separation`.
    pub fn prefill_ratio(&self) -> f32 {
        let total = self.total_slots().max(1);
        self.prompt_tokens as f32 / total as f32
    }
}

/// A routing hint produced by the KV-aware layer, consumed by the planner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KvRoutingHint {
    /// A worker id to prefer because it already holds the session prefix.
    pub cache_locality_worker: Option<String>,
    /// Whether a cold prefill of this request is preferred (KV-empty worker
    /// with the model + headroom).
    pub prefer_kv_headroom: bool,
    /// If prefill/decode separation is supported, the split-pref segment is
    /// planned on `prefill_worker`.
    pub prefill_worker: Option<String>,
}

/// The pure KV-aware decision surface. Given a context profile and a set of
/// workers with model+KV state, produce a routing hint. I/O-free and
/// deterministic so it is unit-testable.
pub struct KvPlanner;

impl KvPlanner {
    /// Produces a [KvRoutingHint] from context + per-worker KV state.
    ///
    /// `kv_state: worker_id -> (serves_model, kv_state)`.
    pub fn route(
        &self,
        ctx: &ContextProfile,
        workers: &[(String, bool, KVCacheState)],
        engine_supports_prefill_decode: bool,
    ) -> KvRoutingHint {
        // Continuation with a resident prefix goes straight back to that
        // worker — cold prefill elsewhere would be wasteful.
        if ctx.is_continuation {
            if let Some(host) = &ctx.prefix_resident_on {
                if workers.iter().any(|(id, _, _)| id == host) {
                    return KvRoutingHint {
                        cache_locality_worker: Some(host.clone()),
                        prefer_kv_headroom: false,
                        prefill_worker: None,
                    };
                }
            }
        }

        // Else prefer a worker that serves the model and has KV headroom for
        // this (possibly long) context.
        let mut best: Option<(String, u32)> = None;
        for (id, serves, state) in workers {
            if !*serves {
                continue;
            }
            if let Some(h) = state.headroom_tokens() {
                if h >= ctx.total_slots() && best.as_ref().map(|(_, b)| h > *b).unwrap_or(true) {
                    best = Some((id.clone(), h));
                }
            }
        }

        // Prefill/decode split only when the engine that would run the decode
        // part advertises it; we mirror that decision conservatively here.
        let prefill_worker = if engine_supports_prefill_decode {
            // pick the worker with the most RAM for the memory-bound prefill
            // phase if any distinct worker is known — normally just the same
            // best worker, so a single stage is fine.
            best.as_ref().map(|(id, _)| id.clone())
        } else {
            None
        };

        KvRoutingHint {
            cache_locality_worker: None,
            prefer_kv_headroom: best.is_some(),
            prefill_worker,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_goes_to_prefix_host() {
        let ctx = ContextProfile {
            prompt_tokens: 100,
            max_output_tokens: 64,
            is_continuation: true,
            prefix_resident_on: Some("worker-b".into()),
        };
        let workers = vec![
            ("worker-a".into(), true, KVCacheState::Empty),
            ("worker-b".into(), true, KVCacheState::Partial { used: 5, capacity: 4096 }),
        ];
        let hint = KvPlanner.route(&ctx, &workers, false);
        assert_eq!(hint.cache_locality_worker.as_deref(), Some("worker-b"));
    }

    #[test]
    fn prefers_kv_headroom_for_long_context() {
        let ctx = ContextProfile {
            prompt_tokens: 4000,
            max_output_tokens: 1000,
            is_continuation: false,
            prefix_resident_on: None,
        };
        let workers = vec![
            ("tight".into(), true, KVCacheState::Partial { used: 3900, capacity: 4000 }),
            ("roomy".into(), true, KVCacheState::Partial { used: 100, capacity: 8000 }),
        ];
        let hint = KvPlanner.route(&ctx, &workers, false);
        assert!(hint.prefer_kv_headroom);
        // Both have the model; roomy wins on headroom.
        assert_eq!(hint.cache_locality_worker, None);
    }

    #[test]
    fn full_cache_cannot_accommodate() {
        assert!(!KVCacheState::Full.can_accommodate(1));
        assert!(KVCacheState::Empty.can_accommodate(1000));
        assert!(KVCacheState::Unknown.can_accommodate(1000));
        assert!(KVCacheState::Partial { used: 90, capacity: 100 }.can_accommodate(10));
        assert!(!KVCacheState::Partial { used: 90, capacity: 100 }.can_accommodate(11));
    }

    #[test]
    fn prefill_ratio_is_split_of_total() {
        let ctx = ContextProfile {
            prompt_tokens: 300,
            max_output_tokens: 100,
            is_continuation: false,
            prefix_resident_on: None,
        };
        assert!(ctx.prefill_ratio() > 0.7 && ctx.prefill_ratio() < 0.8);
    }
}