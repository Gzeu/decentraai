# Next-Gen Fabric Benchmark and Strategy Matrix

Status: INFERRED (structure based on VERIFIED and MEASURED behavior in referenced systems; actual benchmark values intentionally left UNKNOWN pending DecentraAI experiments).[cite:103][cite:110][cite:118][cite:133][cite:144][cite:147]

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
