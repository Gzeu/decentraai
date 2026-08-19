# DecentraAI Execution Strategy Roadmap

## 1. Current architecture

DecentraAI already runs as a universal node that is both coordinator and worker on each machine, with a fabric-aware control plane, capability registry, and a distributed inference router that drives a local llama-server or remote backend via an OpenAI-compatible API.[cite:166]
The Next-Gen fabric work has introduced `decentraai-fabric` for execution planning, `ComputeManager` for worker capability/availability, network-aware scheduling (M19), KV-aware placement (M20), autonomous decision tracing (M23), and a resilient fabric with request-level retries and recovery (M24).[cite:166]

## 2. ExecutionStrategy abstraction

### 2.1 StrategyKind

Execution strategies describe *how* a single logical request is executed:
- `SingleWorker` — one worker runs the entire request.
- `BatchFanOut` — multiple workers each run independent requests from a batch.
- `SpeculativeDraftVerify` — weak worker drafts, strong worker verifies.
- `DisaggregatedPrefillDecode` — one worker does prefill, another decode.
- `CacheAwareRoute` — route/migrate based on KV/cache state.
- `CollaborativeModel` — tensor/pipeline-parallel model execution across workers.

Each strategy has explicit capabilities, prerequisites, and provenance flags.

### 2.2 CAN_RUN vs CAN_COLLABORATE

For a given `WorkloadRequirements` + `WorkerFacts` set, the planner must answer:
- `CAN_RUN(worker, model, capability)` — the worker can run the request alone (capacity, engine, trust, policy, capability fit).
- `CAN_COLLABORATE(worker, model, capability, strategy)` — the worker can safely participate in a *multi-worker* strategy (network, KV, engine features, concurrent load).

`CAN_RUN` reuses the existing `get_worker_capability` and `aggregate_can_i_run` projections.[cite:166]
`CAN_COLLABORATE` extends it with fabric-level constraints: network reach cost, KV locality, adaptive contribution factor, and experimental engine features (e.g., prefill/decode separation, tensor parallel support).[cite:166]

### 2.3 Decision provenance

Every planner decision must preserve:
- `MEASURED` — directly observed metrics (tokens/s, latency, RTT, throughput).
- `ESTIMATED` — derived from conservative estimators (transfer cost, dry-run). 
- `INFERRED` — logical conclusions from architecture and configuration.
- `EXPERIMENTAL` — gated strategies that are under measurement.
- `UNKNOWN` — missing data; never fabricated.

The existing `ExecutionDecision`/`ExecutionEvent` and historical statistics endpoints (`/v1/stats`) already carry part of this provenance; the roadmap extends them to strategy selection and explanation.[cite:166]

## 3. Strategy selection flow

### 3.1 Inputs

The planner takes:
- Request facts: intent/capability, model, prompt size, max tokens, session_id.
- Worker facts: trust, health, capacity, KV state, engine capabilities.
- Network facts: RTT, bandwidth, jitter, packet loss, locality.
- Historical performance: measured throughput/latency per worker/model.
- Policy/trust: remote inference opt-in, roles, tiers, quota, capacity state.

### 3.2 Flow

1. Classify workload (interactive vs batch; continuation vs fresh; critical vs best-effort).
2. Enumerate candidate strategies (SingleWorker, BatchFanOut, plus gated experimental ones).
3. For each strategy, compute:
   - eligible workers (`CAN_RUN`/`CAN_COLLABORATE`).
   - estimated execution cost (compute + communication + coordination).
   - reliability score (health, failure rate, recovery history).
4. Compare against `SingleWorker` baseline and reject any strategy whose net benefit is not positive or whose provenance is `UNKNOWN`/`EXPERIMENTAL` without measurements.
5. Select the best strategy and worker subset; record rationale, including `WHY selected`, `WHY rejected`, `WHAT evidence`, `WHAT unknown`.

If evidence is insufficient, the decision is marked `UNKNOWN/EXPERIMENTAL`, and the router safely falls back to `SingleWorker`.

## 4. NetworkFacts

Network facts must become first-class inputs:
- RTT per link via `InferPing`/`InferPong` (already implemented as part of M19).
- Bandwidth estimates and locality (LAN vs remote, same host vs loopback).[cite:166]
- Jitter and packet loss where measurable (extension to existing probes).
- Connection stability (heartbeat staleness, reconnect history).

These feed a `NetworkFacts` structure per worker and per link, used to estimate transfer/synchronization cost and to determine when distributed strategies are network-bound.

## 5. KV state model

KV/session state is already tracked via `SessionAccount`, `KVCacheState`, and `ComputeManager::sessions()`.[cite:166]
The roadmap extends this to classify KV state per worker/session as:
- `LOCAL` — KV lives only on a single worker.
- `REPLICATED` — KV is safely replicated to multiple workers.
- `TRANSFERABLE` — KV can be transferred/migrated at acceptable cost.
- `REMOTE` — KV is only on a remote worker.
- `UNKNOWN` — no KV information.

Execution strategies use KV state to decide whether to stay, replicate, or migrate, always preserving trust/privacy boundaries.

## 6. Historical performance integration

Historical performance (`/v1/stats`) already aggregates measured throughput, latency, retries and outcome distributions per worker/model.[cite:166]
ExecutionStrategy selection must consume this data to:
- Prefer workers with consistent high throughput and low latency for SingleWorker.
- Detect cases where adding workers (e.g., N+1) historically made performance worse.
- Adjust strategy scoring when workers are chronically overloaded or unreliable.

The planner treats missing historical data as `UNKNOWN` rather than assuming performance.

## 7. Provenance rules

All strategy decisions and fabric projections obey:
- No fabricated measurements: missing metrics are `UNKNOWN`.
- INFERRED vs VERIFIED clearly distinguished (e.g., quantization from file names is INFERRED).[cite:166]
- Policy gates (trust, remote opt-in) are treated as hard constraints — violations become CANNOT_RUN.
- Experimental strategies are opt-in and clearly labelled.

## 8. Fallback behavior

When a strategy cannot be executed safely (insufficient data, network degradation, worker failure), the planner:
- Marks the decision as `UNKNOWN` or `EXPERIMENTAL_FAILED`.
- Falls back to `SingleWorker` or conservative `BatchFanOut`.
- Preserves idempotency and avoids duplicate work by reusing existing replay guards and retry semantics.[cite:166]

No strategy may silently downgrade security (trust, privacy) or correctness; failure paths must be explicit and auditable.

## 9. Failure handling

Failure handling reuses the existing M24 resilient fabric:
- Worker health detection and eviction for stale heartbeats.
- Reservation timeout and release.
- Engine crash recovery via supervisor loops.
- Bounded retries for transport-level errors only (no duplicate non-idempotent work).

ExecutionStrategy adds:
- Per-strategy failure classification (draft rejected vs verify failure vs network-bound).
- Strategy-level recovery advisories (replan vs abort) surfaced through `/v1/execute`.

## 10. Implementation phases

### Phase P0 — Finish current fabric

Goal: live validation of the existing two-node fabric (Laptop ↔ Desktop) with exact worker pinning and remote execution.

Tasks:
- Complete live Laptop→Desktop remote execution validation.
- Validate `plan_and_reserve_on` / `route_request_on` pinning on remote workers.
- Verify quota accounting, provenance, recovery, batch routing, and dashboard views for remote execution.

Acceptance: a documented two-node validation report with all invariants confirmed.

### Phase P1 — ExecutionStrategy foundation

Goal: introduce ExecutionStrategy abstractions without changing existing behavior.

Tasks:
- Define `ExecutionStrategy` and `StrategyKind` and integrate them into the planner.
- Implement `CAN_RUN`/`CAN_COLLABORATE` predicates using existing WorkerFacts/ComputeAdvertisement.
- Extend decisions with per-strategy rationale and provenance flags.

Acceptance: SingleWorker and existing BatchFanOut operate via ExecutionStrategy with identical observable behavior.

### Phase P2 — NetworkFacts

Goal: turn network measurements into first-class inputs.

Tasks:
- Extend existing RTT probes with jitter/packet-loss estimation.
- Populate a dedicated `NetworkFacts` structure per worker/link.
- Integrate NetworkFacts into strategy scoring (e.g., disallow multi-worker when reach cost dominates decode).

Acceptance: planner can show when a multi-worker strategy would be network-bound and correctly prefers SingleWorker.

### Phase P3 — Speculative draft/verify (experimental)

Goal: implement Laptop+Desktop speculative draft/verify as the first multi-worker single-task experiment.

Tasks:
- Implement `SpeculativeDraftVerify` strategy behind an explicit experimental gate.
- Configure Laptop with a small draft model, Desktop with the main model.
- Instrument baseline vs speculative runs with tok/s, TTFT, decode latency, acceptance, network, utilization, failures, energy/thermal.

Acceptance: speculative strategy remains disabled by default; only considered when MEASURED metrics show net benefit vs SingleWorker.

### Phase P4 — Disaggregated prefill/decode (experimental)

Goal: evaluate prefill/decode split via an existing backend (vLLM/SGLang/LMCache/NIXL) instead of custom KV engines.[cite:166]

Tasks:
- Integrate one mature backend that supports prefill/decode separation.
- Implement `DisaggregatedPrefillDecode` as a gated strategy using that backend.
- Run Desktop↔Laptop experiments with full metrics similar to P3.

Acceptance: no custom KV engine introduced; strategy stays experimental until proven beneficial.

### Phase P5 — Cache-aware execution (experimental)

Goal: make KV/cache awareness a first-class decision input while reusing existing backends.

Tasks:
- Classify KV state per session as LOCAL/REPLICATED/TRANSFERABLE/REMOTE/UNKNOWN.
- Integrate LMCache or equivalent prefix-caching in trusted clusters.
- Implement `CacheAwareRoute` to prefer staying or migrating based on measured benefit vs transfer cost.

Acceptance: cache-aware routing preserves privacy/trust and stays gated until measured benefit; KV migration never crosses trust boundaries.

### Phase P6 — Collaborative model / llama.cpp RPC (experimental/wait)

Goal: keep collaborative/tensor-parallel execution strictly experimental and off by default.

Tasks:
- Implement an isolated collaborative-model harness reusing the existing llama.cpp RPC script and design.[cite:168]
- Benchmark Desktop-only, Laptop-only, and Desktop+Laptop collaborative configurations.
- Compare prefill/decode metrics, TTFT, latency, bandwidth, VRAM/RAM/CPU/GPU utilization, and failure behavior.

Acceptance: collaborative model execution is enabled only behind explicit experimental flags and only if measurements demonstrate net benefit; 3–5 node tensor-parallel remains WAIT.

## 11. Benchmark requirements

For each experimental strategy (P3–P6), benchmarks must:
- Use real hardware (Laptop i5 + Desktop i7 + GPUs) on the LAN.[cite:166]
- Record baseline SingleWorker metrics and strategy-specific metrics.
- Avoid fabricated data; mark unmeasured fields as UNKNOWN.
- Include heterogeneous worker behavior (GPU generations, CPU/RAM/VRAM differences).

Benchmark reports live in `docs/research/DISTRIBUTED-INFERENCE-BENCHMARK-MATRIX.md`.

## 12. Acceptance criteria

The planner must eventually support decisions like:
- Desktop only → 30 tok/s.
- Desktop + Laptop → 45 tok/s.
- Desktop + Laptop + GPU3 → 51 tok/s.
- Desktop + Laptop + GPU3 + CPU node → 38 tok/s.

And select the strategy/worker subset that produces positive net benefit (e.g., B or C, not D), falling back to SingleWorker when evidence is insufficient or net benefit is negative.

Production execution remains conservative until real measurements prove benefit; all multi-worker strategies stay opt-in and explicitly labelled as EXPERIMENTAL.
