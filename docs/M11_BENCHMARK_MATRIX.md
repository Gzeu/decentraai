# M11 Benchmark Matrix

## Purpose

Measure useful end-to-end performance, not isolated GPU utilization. Every optimization must be compared against the M10 single-worker baseline using the same model, prompt set, quantization, hardware and request distribution.

## Required metrics

### Latency

- TTFT: time to first token.
- ITL: inter-token latency.
- end-to-end latency.
- queue wait time.
- p50, p95 and p99.

### Throughput

- output tokens per second;
- requests per second;
- aggregate tokens per second;
- active sequences;
- batch utilization.

### Resource efficiency

- GPU utilization;
- VRAM used and peak VRAM;
- CPU utilization;
- system memory and KV-cache usage;
- network bytes and transfer latency;
- estimated energy/cost per million tokens.

### Quality and correctness

- exact output contract validity;
- timeout/error rate;
- cancellation success;
- speculative acceptance rate;
- cache hit rate;
- fallback success rate;
- no duplicate externally visible completions.

## Test dimensions

| Dimension | Values |
|---|---|
| Engine | llama.cpp, vLLM, SGLang when supported |
| Quantization | FP16/BF16, INT8, INT4/GGUF |
| Execution | single, replica, TP, PP, speculative |
| Context | short, medium, long, repeated-prefix |
| Load | 1, 4, 16, 64 concurrent requests |
| Output | 32, 128, 512, 2048 tokens |
| Network | local, same-region, constrained-latency |
| Failure | timeout, worker loss, cache miss, backend error |

## Baselines

### B0: single worker

One worker, one engine, no prefix cache, no speculative decoding, fixed batch policy.

### B1: replicas

Two or more complete replicas with queue-aware routing.

### B2: optimized replicas

Replicas plus continuous batching, token budgets and prefix-cache-aware routing.

### B3: trusted multi-GPU

Tensor/pipeline parallelism inside a verified low-latency cluster.

### B4: speculative

Draft/target pair compared with the same target model without speculation.

## Acceptance gates

An optimization may merge only if:

- it improves the target metric by at least 10% in its intended workload; or
- it reduces memory by at least 15% without violating quality/latency SLOs; or
- it improves availability/fallback behavior with no correctness regression.

Additionally:

- p99 latency must not regress by more than 5% for baseline workloads;
- error rate must not increase;
- cancellation and timeout tests must pass;
- no secret or prompt leakage may appear in logs/cache keys;
- the selected execution plan must be recorded for reproducibility.

## Benchmark procedure

1. Pin model hash, engine version, quantization and configuration.
2. Warm the backend and separately record cold-start metrics.
3. Run a calibration pass and discard it.
4. Run each scenario for a fixed duration or fixed request count.
5. Repeat each scenario at least three times.
6. Record raw events with request ID, plan ID and worker ID.
7. Compute confidence intervals and p50/p95/p99.
8. Compare against the appropriate baseline.
9. Run failure scenarios separately from throughput scenarios.
10. Store a summary artifact and the exact command/configuration.

## Scenario suite

### S1: single request

Measures cold/warm TTFT and output correctness.

### S2: concurrent short requests

Measures batching efficiency and replica routing.

### S3: long repeated prefixes

Measures prefix-cache affinity and cache eviction behavior.

### S4: long context saturation

Measures KV-cache pressure, admission control and degradation.

### S5: worker failure

Stops the selected worker during execution and verifies safe fallback or typed failure.

### S6: cancellation

Cancels during queueing, prefill and decode; verifies backend cancellation and capacity release.

### S7: speculative decoding

Compares acceptance rate and end-to-end latency against the target-only baseline.

### S8: TP/PP cluster

Measures communication overhead, scaling efficiency and failure behavior in a trusted cluster only.

## Required artifact format

Each run must produce:

```json
{
  "run_id": "uuid",
  "commit": "git-sha",
  "model_hash": "...",
  "engine": "...",
  "quantization": "...",
  "execution_mode": "...",
  "workers": ["peer-id"],
  "concurrency": 16,
  "prompt_tokens": 512,
  "output_tokens": 256,
  "ttft_ms": {"p50": 0, "p95": 0, "p99": 0},
  "itl_ms": {"p50": 0, "p95": 0, "p99": 0},
  "tokens_per_second": 0.0,
  "cache_hit_rate": 0.0,
  "error_rate": 0.0,
  "fallback_success_rate": 0.0
}
```

## Implementation order

1. Add metrics fields and plan IDs to the existing request lifecycle.
2. Build a repeatable single-worker benchmark.
3. Add replica routing and compare B0/B1.
4. Add batching and cache affinity and compare B1/B2.
5. Add trusted-cluster TP/PP only after topology verification.
6. Add speculative decoding as an opt-in experiment.
7. Add a scheduler policy report explaining why each plan was selected.
