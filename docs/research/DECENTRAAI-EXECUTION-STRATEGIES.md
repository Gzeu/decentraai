# DecentraAI Execution Strategies

Status: INFERRED (strategy abstraction informed by VERIFIED features in vLLM, SGLang, LMCache, disaggregated serving, speculative decoding, decentralized routing, and DecentraAI’s existing planner).[cite:103][cite:110][cite:118][cite:121][cite:133][cite:139][cite:147][cite:98]

This document proposes an ExecutionStrategy abstraction for DecentraAI’s next-generation fabric.

## 1. Motivation

DecentraAI should not be "just another inference server".
Its strength is an adaptive control plane that decides **how** a task executes across heterogeneous resources.

Rather than hard-coding a single execution mode, DecentraAI should model a set of **ExecutionStrategies** and choose among them per request.

## 2. ExecutionStrategy Abstraction (INFERRED)

Define:

```text
ExecutionStrategy {
  id: StrategyId,
  kind: StrategyKind,
  workers: Vec<WorkerId>,
  stages: Vec<Stage>,
  kv_state: KvState,
  network_path: NetworkPathFacts,
  expected_ttft: Estimate,
  expected_throughput: Estimate,
  expected_latency_p50: Estimate,
  expected_latency_p99: Estimate,
  expected_energy_cost: Estimate,
  communication_cost_bytes: Estimate,
  confidence: f64,
  provenance: EvidenceClass,
}
```

Where `StrategyKind` includes (INFERRED):

- `SingleWorker`.
- `FanOut` (independent-request distribution).
- `CollaborativeModel` (model-parallel cooperative inference).
- `DisaggregatedPrefillDecode`.
- `SpeculativeDraftVerify`.
- `Pipeline` (multi-model heterogeneous pipeline).
- `CacheAwareRoute`.
- `RemoteExecution`.
- `LocalPrivateExecution`.

Each strategy consists of **stages** (e.g., prefill, decode, draft, verify, vision-encode, rerank) with assigned workers.

## 3. CAN_RUN vs CAN_COLLABORATE (INFERRED)

Introduce two top-level predicates per worker and per request:

- `CAN_RUN(model, request, worker)`:
  - Worker has enough resources (VRAM, RAM, CPU) and capabilities to run the model alone.

- `CAN_COLLABORATE(model, request, worker)`:
  - Worker can participate positively in a multi-worker strategy (e.g., draft-only, prefill-only, pipeline stage, KV cache provider).

Planner uses these to form **candidate strategies**:

- SingleWorker strategies: require CAN_RUN.
- Collaborative strategies: require combinations of CAN_RUN and CAN_COLLABORATE across workers.

## 4. Strategy Ranking (INFERRED)

For each candidate strategy, compute scores along:

- Performance potential (TTFT, throughput).
- Network requirements (RTT, bandwidth, jitter).
- Implementation complexity (internal ranking; some strategies are more fragile).
- Heterogeneous hardware fit (how well stages match worker capabilities).
- Fault tolerance (resilience to worker failure).
- DecentraAI fit (alignment with fabric design and trust/policy).

Planner chooses the top-ranked strategy subject to policy constraints.

## 5. Example Strategy Rankings (INFERRED)

Given five heterogeneous nodes, possible strategies include:

- SingleWorker on strongest GPU.
- FanOut for batch workloads.
- DisaggregatedPrefillDecode between Desktop and Laptop.
- SpeculativeDraftVerify with CPU draft and GPU target.
- Pipeline for multimodal tasks (vision + LLM + rerank).
- CacheAwareRoute using distributed prefix caches.

Each strategy is scored and ranked; planner may prefer, for example, `DisaggregatedPrefillDecode` or `SpeculativeDraftVerify` over naïve model-parallel splitting.

## 6. Recommendations (GO / EXPERIMENT / WAIT)

- **GO NOW**:
  - Implement `ExecutionStrategy` and `StrategyKind` as planner abstractions.
  - Add `CAN_RUN` and `CAN_COLLABORATE` predicates.

- **EXPERIMENT FIRST**:
  - Implement a subset of strategies (SingleWorker, FanOut, DisaggregatedPrefillDecode, SpeculativeDraftVerify, Pipeline) and evaluate them on real workloads.

- **WAIT**:
  - Ultra-complex strategies involving many stages and workers until simpler strategies are proven valuable.

ExecutionStrategy as a first-class abstraction is central to DecentraAI’s "unfair advantage" as an adaptive fabric controller.
