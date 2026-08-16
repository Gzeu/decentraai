# Multi-Worker Selection for Distributed Inference

Status: **VERIFIED** for algorithms and patterns directly from research papers and system docs, **INFERRED** for DecentraAI-specific adaptations, **UNKNOWN** where data is missing.

## Problem Statement

Given a pool of heterogeneous workers (desktop GPUs, laptop GPUs/CPUs, additional nodes over varied networks), determine an **optimal subset** of workers to participate in distributed inference for a single request so that:

- End-to-end latency and throughput are maximized.
- Network and synchronization overhead are accounted for.
- Worker load, thermal state, and reliability are considered.

This explicitly rejects naive "use all workers" strategies; adding a worker is only acceptable if it yields a **positive net benefit**.

## Observed Scaling Patterns

Research and engineering reports show that **N+1 workers can make performance worse**:

- Tensor parallelism scales well within a node but experiences diminishing returns and eventual regression when extended across nodes due to high all-reduce communication cost.[web:22][web:21][web:30]
- Pipeline parallelism scales nearly linearly up to a point, after which pipeline bubbles (idle stages) increase latency; deeper pipelines incur longer per-token paths.[web:54][web:50]
- Distributed inference engines such as vLLM recommend modest TP/PP degrees (e.g. TP=8, PP=2) and warn against overly large TP across slow interconnects.[web:24][web:51][web:56]
- Heterogeneous cluster studies (Hetis, Parallax) explicitly drop low-end GPUs from dense compute groups because their inclusion increases synchronization and communication overhead more than the compute they contribute.[web:60][web:69]

For llama.cpp RPC in particular, community benchmarks show that adding remote workers over slow links can **hurt decode latency**, even when prefill speeds improve.[web:36][web:46][web:49]

## Metrics for Worker Evaluation

From heterogeneous inference research and cluster schedulers:[web:60][web:62][web:64][web:65]

Each worker should be characterized by:

- **Compute capacity**: approximate TFLOPs, single-token latency under a standard micro-benchmark.
- **Memory capacity**: VRAM/RAM available for weights and KV cache.
- **Network characteristics**: bandwidth and RTT to other workers (and to primary node), interconnect type (NVLink, PCIe, Ethernet, Wi‑Fi).
- **Current load and thermal state**: utilization, queued jobs, temperature, throttling.
- **Reliability**: historical failure rate, disconnection patterns.

DecentraAI already has WorkerFacts and resource intelligence; distributed inference selection extends this with **per-model** and **per-topology** metrics.

## Selection Algorithms from Literature

### Hetis Primary and Attention Worker Selection

Hetis serves LLMs in heterogeneous GPU clusters by:

- Selecting a subset of GPUs as **Primary workers** to handle dense modules (MLPs, projections) using DP/TP/PP, chosen to minimize dense computation cost.[web:60]
- Reserving remaining GPUs as **Attention workers**, dynamically pooled to compute attention heads and move KV caches between devices.[web:60]

The key pruning rule (VERIFIED): remove GPU \(\kappa\) from the candidate primary set if:

\[ C_p(\sigma - \kappa, M, \mathcal{R}) / C_p(\sigma, M, \mathcal{R}) \leq 1 + \Delta \] [1]

with \(\Delta\) ≈ 0.05. Intuitively: if removing a low-end GPU increases dense compute cost by ≤ 5%, it is not worth keeping.

### Dynamic Graph Partitioning for Heterogeneous Workers

Kitzberger’s thesis on distributed ONNX-based LLMs proposes:[web:62]

- Collecting worker metrics (memory, bandwidth, latency, execution speed).
- Running a dynamic programming algorithm over layers and workers (O(n²m) for n layers, m workers) to find a partition that minimizes total execution cost under constraints.
- Accepting a new assignment only if its cost **improves** the current one by a factor \(\tau\), i.e. \(C_{total}(A_{cand}) < \tau \cdot C_{exec}(A_{cur})\).

This formalizes the idea that minor improvements may not justify repartitioning overhead.

### Heterogeneity-Aware Schedulers

Schedulers such as SCHEDTUNE examine GPU nodes and exclude those that cannot fit job memory, then pick nodes by shortest queuing delay to minimize job completion time.[web:64] Workload-aware schedulers in ML inference prioritize GPU-like workers first, falling back to CPU workers only when GPUs are saturated.[web:65]

## DecentraAI Selection Strategy (INFERRED Design)

DecentraAI can adapt these ideas into a **DistributedExecutionCandidate** abstraction with the following fields:

- `workers`: ordered list of selected workers with per-worker metrics.
- `partition_strategy`: description of TP/PP/RPC split (e.g. llama.cpp RPC layer split across 3 devices).
- `memory_allocation`: weights and KV cache placement across workers.
- `network_topology`: logical graph of links (bandwidth, RTT) connecting workers.
- `expected_throughput`: predicted tok/s for prefill and decode.
- `expected_latency`: predicted TTFT and per-token latency distribution.
- `communication_cost`: estimated bytes transferred and time spent in inter-device communication per token.[web:56][web:50]
- `confidence`: statistical or heuristic confidence score based on historical measurements.
- `provenance`: description of how the candidate was constructed (e.g. from Hetis-style pruning, historical benchmarks, or operator hints).

### Candidate Construction and Pruning

For each request and eligible worker pool:

1. **Filter** workers that cannot hold required weights and KV cache.
2. **Generate** candidate subsets using heuristics:
   - Start from fastest and largest-memory workers, add additional workers in descending capability order.
   - Consider small subsets first (2–3 workers) to limit search space.
3. **Estimate cost** for each subset using simple performance models (e.g. TP communication cost formula from vLLM tuning notes, PP bubbles from SiPipe).[web:54][web:56]
4. **Prune** subsets where adding a worker increases total cost or yields marginal improvement below threshold \(\Delta\) (e.g. 5%).[web:60][web:62]

### Selection Rule

Given best single-worker plan \(P_1\) and best collaborative plan \(P_k\), DecentraAI should:

- Prefer \(P_k\) only if `expected_latency(P_k)` < `expected_latency(P_1)` by a configurable margin and reliability is acceptable.
- Otherwise, **fall back to single-worker inference** or simple fan-out.

This is directly aligned with the design principle:

- Option A: Desktop only → 30 tok/s.
- Option B: Desktop + Laptop → 45 tok/s.
- Option C: Desktop + Laptop + GPU3 → 51 tok/s.
- Option D: Desktop + Laptop + GPU3 + CPU node → 38 tok/s.

DecentraAI must choose B or C (whichever meets objectives), not D.

## Dynamic / Adaptive Fabric Considerations

Research on hybrid CPU-GPU inference and heterogeneous clusters (APEX, Hetis, Parallax) suggests:[web:60][web:69][web:74]

- Dynamic load-aware scheduling is feasible **between requests**, adjusting worker roles as throughput and latency requirements change.
- Within a single in-flight request, changing worker subsets or partitions is expensive due to the need to move weights and KV caches; most systems avoid mid-request repartitioning.

DecentraAI’s selection mechanism should therefore:

- Allow **adaptive reassessment** between requests in a long-running session.
- Avoid mid-decode repartitioning unless a worker fails, in which case the planner can restart inference from a KV checkpoint or from the prompt using a reduced worker set.

## Security and Trust in Multi-Worker Selection

Worker selection affects security posture:

- Including low-trust workers in a cooperative partition exposes them to weight shards, activations, and possibly KV contents.[web:61][web:63][web:68]
- Distributed inference should respect trust boundaries; high-sensitivity requests may restrict collaboration to workers with strong authentication and hardening.

DecentraAI can integrate trust levels into selection:

- Tag workers with trust scores and capabilities.
- Require certain trust thresholds for roles (e.g. primary dense compute vs auxiliary attention vs KV cache hosting).

Selection then becomes multi-objective: optimize performance subject to **trust constraints**.

---

This document describes algorithms and design patterns for **selecting an optimal subset of workers** for distributed inference in DecentraAI. Concrete implementation choices, integration with Planner and WorkerFacts, and model-specific heuristics are developed further in the DecentraAI design document.
