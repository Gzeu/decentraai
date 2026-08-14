# M11 Adaptive Compute Fabric

## Mission

Make DecentraAI select the most efficient execution plan for each request and hardware topology. The system must increase useful tokens per second and reduce time-to-first-token without assuming that every worker has the same GPU, memory, engine or network quality.

## Core principle

Do not connect every model or GPU together by default. Choose the least expensive execution mode that satisfies model fit, latency, throughput, privacy and reliability requirements.

```text
model fits on one worker
  -> replica/data parallelism
model needs multiple GPUs in one trusted node
  -> tensor parallelism
model needs multiple trusted low-latency nodes
  -> tensor + pipeline parallelism
large repeated context
  -> prefix/KV-cache-aware routing
latency-sensitive generation
  -> speculative decoding
high concurrent load
  -> continuous batching
```

## Execution modes

### SingleWorker

Use when one worker can serve the model within memory and latency limits. This is the default and safest mode.

### DataParallelReplica

Run multiple independent replicas of the same model and route requests to the best eligible replica. Use for throughput, availability and heterogeneous decentralized networks.

### TensorPipelineParallel

Split one model across multiple GPUs/workers. Permit only inside a trusted low-latency cluster with an explicit interconnect policy. Never use arbitrary public P2P peers for token-synchronous tensor traffic.

### Speculative

Use a compatible draft model to propose tokens and a target model to verify them. Enable only after a benchmark proves a positive acceptance rate and lower latency.

### PrefillDecodeDisaggregated

Separate prompt prefill from token decode. Use only when traffic patterns and network transfer costs justify the complexity.

## Contracts

### ComputeCapabilities

Every worker announcement should expose normalized capabilities:

```rust
pub struct ComputeCapabilities {
    pub engines: Vec<InferenceEngine>,
    pub gpu_vendor: Option<String>,
    pub gpu_model: Option<String>,
    pub gpu_count: u32,
    pub vram_bytes: u64,
    pub system_memory_bytes: u64,
    pub quantizations: Vec<Quantization>,
    pub max_context_tokens: u32,
    pub max_batch_tokens: u32,
    pub supports_continuous_batching: bool,
    pub supports_prefix_cache: bool,
    pub supports_speculative_decoding: bool,
    pub supports_tensor_parallel: bool,
    pub supports_pipeline_parallel: bool,
    pub supports_kv_offload: bool,
    pub interconnect: InterconnectType,
    pub measured_tps: f32,
    pub measured_ttft_ms: f32,
}
```

Capabilities must be measured or verified, not trusted solely from a self-reported announcement. The scheduler should distinguish `advertised`, `verified` and `expired` capability states.

### ExecutionPlan

```rust
pub struct ExecutionPlan {
    pub request_id: Uuid,
    pub model_hash: String,
    pub mode: ExecutionMode,
    pub engine: InferenceEngine,
    pub worker_ids: Vec<PeerId>,
    pub tensor_parallel_size: u32,
    pub pipeline_parallel_size: u32,
    pub max_batch_tokens: u32,
    pub max_context_tokens: u32,
    pub prefix_cache_key: Option<String>,
    pub draft_model_hash: Option<String>,
    pub deadline_ms: u64,
}
```

A plan is immutable after execution starts. Replanning may happen only before assignment or through an explicit safe migration protocol.

### PerformanceProfile

Track rolling measurements per worker/model/engine pair:

- time to first token;
- inter-token latency;
- tokens per second;
- queue wait time;
- prompt processing time;
- decode time;
- p50/p95/p99 latency;
- error and timeout rates;
- prefix-cache hit rate;
- GPU utilization and memory pressure;
- energy/cost estimate when available.

## Planner algorithm

1. Validate request limits and required model capabilities.
2. Resolve model hash, quantization and context requirements.
3. Filter workers by trust, readiness, model availability and capability.
4. Detect topology: same process, same host, trusted cluster or public network.
5. Generate feasible execution plans.
6. Reject plans that violate memory, deadline, trust or interconnect policies.
7. Score remaining plans using measured data.
8. Reserve queue/batch capacity atomically.
9. Execute and emit plan plus outcome metrics.
10. Update the performance profile and circuit breaker.

A starting score can be:

```text
score =
  0.30 * normalized_throughput
+ 0.25 * cache_affinity
+ 0.20 * capacity_headroom
- 0.15 * predicted_latency
- 0.10 * failure_risk
```

The weights must be configuration, not hard-coded forever. Never allow score to override hard eligibility constraints.

## Model and engine strategy

### llama.cpp

Default edge backend for CPU, consumer GPUs and local quantized models. Good baseline for decentralized workers and deterministic local deployment.

### vLLM

Preferred GPU backend for high-throughput serving, continuous batching and trusted multi-GPU deployments.

### SGLang

Optional backend for workloads benefiting from prefix caching, structured generation and agentic execution patterns.

All engines must implement the same internal adapter contract:

```text
health()
model_info()
capacity()
complete(request)
stream(request)
cancel(request_id)
metrics()
```

Provider-specific JSON must not leak into the P2P protocol or frontend.

## Optimization layers

### Layer 1: model fit

Use GGUF/quantized variants and select the smallest model meeting the quality target. Reject a plan before loading a model that exceeds verified memory limits.

### Layer 2: batching

Use continuous batching with token-budget limits. Prefer length-aware admission to avoid long prompts starving short requests. Track both request count and total active tokens.

### Layer 3: caching

Use model-aware prefix cache keys. Route repeated system prompts and long prefixes to workers with cache affinity. Never include secrets or raw private user data in shared cache keys.

### Layer 4: speculative decoding

Enable only for compatible draft/target pairs. Measure acceptance rate, verifier overhead and end-to-end TTFT/ITL. Automatically disable when it regresses the configured SLO.

### Layer 5: parallelism

Use data-parallel replicas by default. Enable tensor/pipeline parallelism only for trusted clusters with measured interconnect bandwidth and latency.

### Layer 6: memory management

Support KV-cache limits, eviction policy, optional CPU/SSD offload and admission control. Prefer rejecting early over triggering uncontrolled host swapping.

## Network and trust policy

- Public/heterogeneous peers: only complete replica execution.
- Trusted same-region peers: replica execution and optional prefill/decode split.
- Trusted low-latency cluster: tensor/pipeline parallelism after handshake and benchmark verification.
- Never allow a remote peer to request arbitrary code execution or arbitrary model loading.
- Parallel execution groups require explicit membership, signed capabilities and revocation.
- Keep cluster traffic on authenticated private channels and separate it from public worker discovery.

## Rollout phases

### M11.1: capabilities and replicas

Add capability schema, hardware probes, measured profiles, replica routing and continuous batching.

### M11.2: cache and performance control

Add prefix-cache-aware routing, token budgets, admission control and adaptive scoring.

### M11.3: multi-GPU execution

Add trusted cluster registration, tensor/pipeline execution plans and interconnect verification.

### M11.4: speculative and disaggregated execution

Add draft-model planning and prefill/decode separation only after benchmark gates pass.

### M11.5: self-optimizing scheduler

Use observed performance profiles to re-plan workers, batch sizes and engine selection. Keep decisions explainable and auditable.

## Safety and failure behavior

- If capability data is stale, fall back to replica mode or reject.
- If cache transfer fails, continue without cache rather than failing inference.
- If a parallel worker disappears, fail the plan and use a complete replica fallback when idempotency allows it.
- Never duplicate a request after a completed response is externally visible.
- If measured performance degrades, open a circuit breaker and downgrade the plan.
- Preserve user-visible request IDs across fallback.

## Definition of done

M11 is complete only when benchmarks show a measurable improvement over the M10 baseline, the scheduler explains each selected plan, unsafe topologies are rejected, capability claims are verified, and fallback remains correct under worker failure.
