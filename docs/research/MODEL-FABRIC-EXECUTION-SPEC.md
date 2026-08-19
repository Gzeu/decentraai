# Model-Fabric Execution Spec

This document refines the M11 Adaptive Compute Fabric for the distributed-inference research branch.
It ties together:
- M11 Adaptive Compute Fabric (execution modes, capabilities, performance profiles, planner algorithm).
- ExecutionStrategy / StrategyKind (SingleWorker, BatchFanOut, speculative, PD, cache-aware, collaborative).
- Network and trust policies.

No runtime behavior is changed by this document.

## 1. StrategyKind ↔ ExecutionMode mapping

M11 defines high-level execution modes (fabric-level plan kinds) while the ExecutionStrategy roadmap defines strategy kinds at the planner level.

### 1.1 StrategyKind

Current strategy kinds:
- `SingleWorker`
- `BatchFanOut`
- `SpeculativeDraftVerify`
- `DisaggregatedPrefillDecode`
- `CacheAwareRoute`
- `CollaborativeModel`
- `MultiModelPipeline`

### 1.2 ExecutionMode

M11 defines execution modes:
- `SingleWorker` — entire request runs on a single worker.
- `DataParallelReplica` — multiple replicas of the same model; each request runs on one replica.
- `TensorPipelineParallel` — tensor + pipeline parallelism across multiple GPUs/ workers.
- `Speculative` — draft + verify models.
- `PrefillDecodeDisaggregated` — prefill and decode split across engines.

### 1.3 Mapping

The mapping between StrategyKind (planner) and ExecutionMode (fabric) is:

- `StrategyKind::SingleWorker` → `ExecutionMode::SingleWorker`.
- `StrategyKind::BatchFanOut` → `ExecutionMode::DataParallelReplica` (for independent requests).
- `StrategyKind::SpeculativeDraftVerify` → `ExecutionMode::Speculative`.
- `StrategyKind::DisaggregatedPrefillDecode` → `ExecutionMode::PrefillDecodeDisaggregated`.
- `StrategyKind::CacheAwareRoute` → `ExecutionMode::SingleWorker` or `ExecutionMode::DataParallelReplica` with cache-aware routing; cache decisions are orthogonal to execution mode.
- `StrategyKind::CollaborativeModel` → `ExecutionMode::TensorPipelineParallel` (only in trusted clusters with validated interconnect).
- `StrategyKind::MultiModelPipeline` → sequence of `ExecutionPlan`s, each with its own `ExecutionMode` (typically SingleWorker or DataParallelReplica per stage).

The planner must always produce strategies that can be expressed as valid ExecutionPlans under these modes.

## 2. Capability and performance requirements

### 2.1 ComputeCapabilities

M11 defines a `ComputeCapabilities` struct per worker with engine list, GPU/CPU details, context and batch limits, and flags for speculative decoding, tensor/pipeline parallelism, KV offload, prefix cache, etc.

For each StrategyKind, minimum capability requirements are:

- `SingleWorker`:
  - `supports_tensor_parallel == false` acceptable.
  - `supports_speculative_decoding == false` acceptable.
  - `max_context_tokens` and `vram_bytes` verified to fit model + request.

- `BatchFanOut`:
  - Same as SingleWorker for each selected worker.
  - `supports_continuous_batching == true` preferred.

- `SpeculativeDraftVerify`:
  - `supports_speculative_decoding == true` on the verify engine or validated via backend (SGLang/vLLM).
  - Optionally, `supports_prefix_cache == true` when using KV reuse.

- `DisaggregatedPrefillDecode`:
  - `supports_kv_offload == true` or integration with a KV layer (e.g. LMCache) on both prefill and decode engines.

- `CacheAwareRoute`:
  - `supports_prefix_cache == true` and a cache layer that exposes KV locality and hit/miss statistics.

- `CollaborativeModel`:
  - `supports_tensor_parallel == true` and `supports_pipeline_parallel == true` on engines participating in the plan.
  - GPU/ interconnect capabilities verified for cluster.

Capabilities must be:
- Advertised by the worker.
- Verified via benchmarks before enabling advanced strategies.
- Marked as expired when measurements become stale.

### 2.2 PerformanceProfile

For each worker/model/engine pair, M11 defines a PerformanceProfile:
- TTFT (time to first token).
- Inter-token latency.
- Tokens per second.
- Queue wait time.
- Prompt processing time / decode time.
- p50/p95/p99 latency.
- Error / timeout rates.
- Prefix-cache hit rate.
- GPU utilization / memory pressure.
- Optional energy/cost estimates.

ExecutionStrategy uses these to:
- Establish a SingleWorker baseline per request.
- Compare net benefit of alternative strategies vs SingleWorker.
- Detect N+1 cases where adding workers decreases performance.

If PerformanceProfile fields are missing, they must be treated as UNKNOWN and cannot justify experimental strategies.

## 3. Scoring profiles

M11 proposes a generic score function combining throughput, cache affinity, capacity headroom, latency and failure risk.
This section defines scoring profiles to tune the fabric for different goals.

### 3.1 Base score

A base score can be:

```text
score =
  0.30 * normalized_throughput
+ 0.25 * cache_affinity
+ 0.20 * capacity_headroom
- 0.15 * predicted_latency
- 0.10 * failure_risk
```

### 3.2 Profiles

Profiles adjust weights for different workloads:

- `latency_profile`:
  - higher weight on predicted latency and TTFT.
  - lower weight on throughput.

- `throughput_profile`:
  - higher weight on normalized_throughput and capacity_headroom.
  - lower weight on TTFT.

- `cost_profile`:
  - includes energy/cost estimates.
  - balances cost vs latency and throughput.

The planner selects a profile based on workload classification (interactive vs batch, critical vs best-effort) and uses it to score ExecutionPlans.

Hard constraints (trust, memory, deadlines, interconnect policies) must never be overridden by scores.

## 4. Network and trust tiers

M11 defines network and trust policies:
- Public/heterogeneous peers: only complete replica execution.
- Trusted same-region peers: replica execution and optional prefill/decode split.
- Trusted low-latency clusters: tensor/pipeline parallelism after benchmark verification.

This document refines them as explicit tiers:

### 4.1 Tiers

- `public`:
  - StrategyKinds allowed: SingleWorker, BatchFanOut.
  - ExecutionModes allowed: SingleWorker, DataParallelReplica.
  - No speculative, PD, cache-aware migration or collaborative modes.

- `trusted-remote` (same-region, known operators):
  - StrategyKinds allowed: SingleWorker, BatchFanOut, limited CacheAwareRoute.
  - ExecutionModes allowed: SingleWorker, DataParallelReplica, limited PrefillDecodeDisaggregated.

- `trusted-cluster` (low-latency, controlled cluster):
  - StrategyKinds allowed: SingleWorker, BatchFanOut, SpeculativeDraftVerify, DisaggregatedPrefillDecode, CacheAwareRoute, CollaborativeModel.
  - ExecutionModes allowed: SingleWorker, DataParallelReplica, Speculative, PrefillDecodeDisaggregated, TensorPipelineParallel.

### 4.2 Enforcement

- Trust tier is derived from WorkerFacts, policy and configuration.
- ExecutionStrategy must filter candidate strategies and ExecutionModes based on tier.
- KV/cache migration across tiers is disallowed.

## 5. Model-fabric examples

### 5.1 SingleWorker on Desktop

- ExecutionMode: SingleWorker.
- StrategyKind: SingleWorker.
- Worker: Desktop vLLM instance.
- Trust tier: trusted-remote or trusted-cluster when running in LAN.
- Capabilities: verified model fit, continuous batching (optional).

### 5.2 BatchFanOut across CPU and GPU workers

- ExecutionMode: DataParallelReplica.
- StrategyKind: BatchFanOut.
- Workers: Laptop (CPU llama.cpp), Desktop (GPU vLLM).
- Trust tier: public or trusted-remote.
- Planner uses PerformanceProfile and NetworkFacts to decide per-request routing.

### 5.3 SpeculativeDraftVerify (experimental)

- ExecutionMode: Speculative.
- StrategyKind: SpeculativeDraftVerify.
- Draft worker: Laptop with small model.
- Verify worker: Desktop with full model.
- Trust tier: trusted-cluster; KV layer (LMCache) optional.
- Strategy remains gated and cannot be selected autonomously.

### 5.4 PrefillDecodeDisaggregated (experimental)

- ExecutionMode: PrefillDecodeDisaggregated.
- StrategyKind: DisaggregatedPrefillDecode.
- Prefill worker: Desktop GPU.
- Decode worker: same Desktop or trusted-cluster node.
- KV layer: LMCache or equivalent.

### 5.5 CollaborativeModel via vLLM TP/PP cluster

- ExecutionMode: TensorPipelineParallel.
- StrategyKind: CollaborativeModel.
- Backend: vLLM TP/PP cluster with Ray, treated as a single logical worker.
- Trust tier: trusted-cluster.
- Fabric sees the cluster as one worker; TP/PP happens inside backend.

## 6. Promotion and safety

A model-fabric execution mode or strategy can be promoted from EXPERIMENTAL to BETA/PRODUCTION only when:
- Capabilities required for the mode are verified and not stale.
- PerformanceProfile shows consistent net benefit vs SingleWorker.
- Network and trust tiers are correctly enforced.
- Threat model is updated and security implications are reviewed.
- Rollback/fallback paths to SingleWorker/DataParallelReplica are tested.

M11 remains the canonical reference; this spec clarifies how ExecutionStrategy and model-fabric execution modes interact in the distributed-inference research branch.
