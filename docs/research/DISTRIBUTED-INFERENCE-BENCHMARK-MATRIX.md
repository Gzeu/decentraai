# Next-Gen Fabric Benchmark and Strategy Matrix

Status: INFERRED (structure based on VERIFIED and MEASURED behavior in referenced systems; actual benchmark values intentionally left UNKNOWN pending DecentraAI experiments).[cite:103][cite:110][cite:118][cite:133][cite:144][cite:147]

> **UPDATE (2026-08-19)**: first MEASURED values collected on the real
> two-node LAN (Laptop i5 `dca-GriBWu` 30 GiB ↔ Desktop i7 `dca-NGE65Z` 8 GiB,
> 1 Gbps LAN). See §5.

This document defines a benchmark matrix for evaluating execution strategies on a fabric of five heterogeneous DecentraAI nodes.

## 1. Strategy Dimensions

Strategies to evaluate include:

- SingleWorker.
- FanOut.
- CollaborativeModel.
- DisaggregatedPrefillDecode.
- SpeculativeDraftVerify.
- Pipeline.
- CacheAwareRoute.

Each strategy is evaluated along:

- Performance potential.
- Network requirements.
- Implementation complexity.
- Heterogeneous hardware fit.
- Fault tolerance.
- DecentraAI fit.

## 2. Benchmark Matrix Structure

The matrix is defined as a Markdown table with qualitative ratings and placeholders for future measured values.

| Strategy | Perf Potential | Network Requirements | Impl Complexity | Heterogeneous Fit | Fault Tolerance | DecentraAI Fit | Evidence |
|---------|----------------|----------------------|-----------------|-------------------|-----------------|----------------|----------|
| SingleWorker | HIGH (for single strong GPU) | LOW | LOW | MEDIUM | HIGH | HIGH | INFERRED |
| FanOut | HIGH (throughput) | LOW–MEDIUM | MEDIUM | MEDIUM | MEDIUM | HIGH | INFERRED |
| CollaborativeModel | MEDIUM | HIGH | HIGH | HIGH | LOW–MEDIUM | MEDIUM | INFERRED |
| DisaggregatedPrefillDecode | HIGH | MEDIUM–HIGH | MEDIUM–HIGH | HIGH | MEDIUM | HIGH | MIXED |
| SpeculativeDraftVerify | HIGH | MEDIUM | MEDIUM | HIGH | MEDIUM | HIGH | MIXED |
| Pipeline | HIGH | MEDIUM | MEDIUM–HIGH | HIGH | MEDIUM | HIGH | INFERRED |
| CacheAwareRoute | MEDIUM–HIGH | MEDIUM | MEDIUM | HIGH | MEDIUM | HIGH | MIXED |

All numeric benchmark cells (tokens/s, TTFT, latency) are intentionally left UNKNOWN until DecentraAI collects real measurements.

## 3. N+1 Worker Degradation Cases (VERIFIED/INFERRED)

Literature and engineering experience show cases where adding more workers hurts performance:[cite:110][cite:111][cite:133][cite:144][cite:147]

Examples:

- Too many workers in disaggregated strategies saturate network links, making KV transfer the bottleneck.
- Additional speculative draft workers increase coordination overhead without enough acceptance gain.
- Adding workers with weak hardware or poor network connectivity can slow down collaborative strategies.

The matrix will be expanded with real DecentraAI experiments that capture these N+1 degradation effects.

## 4. Next Steps

- Define benchmark scenarios (prompt types, context lengths, models).
- Implement instrumentation to collect TTFT, throughput, latency, network transfer times, cache hits, acceptance rates.
- Populate the matrix with MEASURED values in future branches.

This matrix remains a planning artefact in `research/next-gen-ai-fabric` until populated by experiments.

## 5. MEASURED values — live LAN 2026-08-19 (first evidence)

Real measurements on the two-node fabric (Laptop i5 ↔ Desktop i7, 1 Gbps
LAN, both nodes at HEAD `979acbf`). All numbers are actual runtime data —
no synthetic values. The planner still emits only `SingleWorker`/`BatchFanOut`;
these are the baselines the experimental strategies (P3–P6) must beat.

### 5.1 Per-worker throughput (SingleWorker baseline)

| Worker | Model | Hardware | Tokens | Time | Throughput |
|---|---|---|---|---|---|
| Desktop (remote) | Llama-3.2-1B-Instruct-Q4_K_M | i7, ~8 GiB | 21 tok | 1804 ms | **11.6 tok/s** (generation-only, audit `inference_completed`) |
| Laptop (local) | qwen2.5-coder-7b-instruct-q4_k_m | i5, CPU-only, 30 GiB | 14–15 tok | 5.7–6.6 s | **2.3 tok/s** (end-to-end) |

Implication: the Desktop is ~5× faster per token than the Laptop for small
generations; for a latency-sensitive request the planner's
`w_latency * priority_boost` term should strongly prefer the Desktop even with
its network cost — the measured `net_score` at ~15 ms median RTT is near 1.0,
so throughput dominates.

### 5.2 Network reality (M19 probe, 30 samples, same session)

Measured `rtt_us` probes to the Desktop over the LAN:

| Metric | Value |
|---|---|
| median | 15 ms |
| mean | 122 ms |
| min / max | 5 ms / 641 ms |
| p95 | 624 ms |
| jitter (MAD) | 7 ms |

**Key finding**: the LAN is NOT a low-jitter pipe — median 15 ms is excellent,
but p95 spikes to 624 ms (Wi-Fi/congestion bursts). This is exactly what P2
NetworkFacts stability targets: with `jitter_us` + `packet_loss_percent`
measured, the planner's `network_score` multiplies the RTT/bandwidth base by
`0.7 + 0.3 * stability()`, so a spiky link loses up to 30% of its network
score. `LinkMetrics::stability()` jitter-term measured here ≈ 0.64.

### 5.3 What this means for P3–P6 (evidence, not activation)

- **P3 speculative**: a draft/verify pair needs the draft model on the same
  node as the target to avoid cross-node per-token RTT — at median 15 ms an
  extra network hop per draft token would erase most speculative gain. Local
  draft only.
- **P4 prefill/decode**: the Desktop's 11.6 tok/s vs Laptop's 2.3 tok/s
  suggests asymmetric split (prefill on Desktop, decode on Laptop) is
  pointless — decode on the slower node dominates. Symmetric split only.
- **P5 cache-aware**: continuation affinity (M20) is the highest-value target
  — re-ingesting a long prefix costs 6+ s on the Laptop; steering a
  continuation back to the KV host saves that entirely.
- **P6 collaborative/RPC**: blocked on engine support (llama.cpp has no RPC
  server mode); no engine DecentraAI runs advertises the capability — stays
  experimental/wait per the roadmap.

### 5.4 How to reproduce

```bash
# from the coordinating Laptop, after both nodes are up:
TOKEN=$(cat ~/.decentraai/runtime/api.token)
# remote throughput (generation-only, see audit):
curl -s -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"model":"Llama-3.2-1B-Instruct-Q4_K_M.gguf","messages":[{"role":"user","content":"Say OK"}],"max_tokens":20,"stream":false}' \
  http://127.0.0.1:8080/v1/chat/completions
# local throughput (end-to-end):
curl -s -w '\nTIME_TOTAL=%{time_total}\n' -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"model":"qwen2.5-coder-7b-instruct-q4_k_m.gguf","messages":[{"role":"user","content":"Write a haiku."}],"max_tokens":60,"stream":false}' \
  http://127.0.0.1:8080/v1/chat/completions
# RTT probes: watch the node log for 'M19 network probe: measured RTT recorded'
journalctl --user -u decentraai-node -f | grep 'M19 network probe'
```
