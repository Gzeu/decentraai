# DecentraAI Distributed Inference Design

Status: **INFERRED** design informed by VERIFIED behavior in llama.cpp RPC and mature systems (vLLM, DeepSpeed, TensorRT-LLM, Megatron-LM, Ray).

## Design Goals

- Allow DecentraAI to execute **one inference request cooperatively** across multiple heterogeneous workers when beneficial.
- Preserve existing capabilities: WorkerFacts, resource intelligence, adaptive fan-out, planner, quota ledger, execution accounting, provenance, recovery, MCP-based worker lifecycle, model hub.
- Avoid duplicating mature systems; instead, integrate them as **execution backends** where appropriate.

## CAN_RUN vs CAN_COLLABORATE

Existing DecentraAI notions of **CAN_RUN** answer: "Can this worker run model M for request R by itself?" considering VRAM, RAM, CPU, policy, and trust.

Distributed inference requires a new notion: **CAN_COLLABORATE**, answering: "Can this worker participate as part of a collaborative partition for model M and request R?".

### CAN_RUN (Unchanged Core)

Per worker and model:

- Sufficient memory for weights and KV cache.
- Backend support (e.g. llama.cpp local GPU, vLLM, TensorRT-LLM).
- Trust/policy permits handling this prompt and model.

### CAN_COLLABORATE (New Dimension)

Additional requirements:

- Support for collaborative backend (e.g. `GGML_RPC=ON` llama.cpp build; vLLM with TP/PP enabled; DeepSpeed tensor-parallel inference).[web:4][web:24][web:31]
- Compatible versions and model formats across workers (GGUF, same llama.cpp tag; same vLLM build).[web:6][web:24]
- Adequate network connectivity and bandwidth to other workers (≥ 10 GbE recommended for RPC; NVLink or fast PCIe for TP).[web:36][web:49][web:22][web:50]
- Trust level sufficient for sharing weight shards, activations, and possibly KV contents.

CAN_COLLABORATE is **per-model** and **per-topology**, not just per worker.

## DistributedExecutionCandidate Abstraction

DecentraAI should introduce a **DistributedExecutionCandidate** object to represent one concrete plan for collaborative inference:

- `model_id`: model (GGUF, HF, etc.).
- `backend`: one of {llama.cpp RPC, vLLM TP/PP, DeepSpeed TP, TensorRT-LLM cluster, Megatron-style pipeline}.[web:4][web:24][web:18][web:29][web:28]
- `workers`: ordered list of participating workers, each with:
  - hardware profile (GPU type, VRAM, CPU, RAM).
  - network profile to peers (bandwidth, RTT, link type).
  - trust/policy tags and capability flags.
- `partition_strategy`: description of TP/PP/RPC layout (e.g. llama.cpp `split-mode=layer` with `--tensor-split 4,3,3`; vLLM `tensor_parallel_size=4, pipeline_parallel_size=2`).[web:76][web:24][web:50]
- `memory_allocation`: how weights and KV are placed across workers.
- `expected_throughput`: predicted prompt and decode tok/s.
- `expected_latency`: TTFT and per-token latency estimates.
- `communication_cost`: bytes and time in cross-worker communication per token.[web:22][web:54][web:56]
- `confidence`: based on historical measurements and model-based predictions.
- `provenance`: description of how the candidate was generated (Heuristic, Hetis-style optimization, operator override).

The **Planner** then compares DistributedExecutionCandidates against single-worker plans and chooses the one that best meets objectives.

## Planner Integration and Optimization Principle

For each request R and model M:

1. Gather **eligible workers** and their CAN_RUN / CAN_COLLABORATE flags.
2. Generate one or more **single-worker plans** (existing path).
3. Generate a set of **DistributedExecutionCandidates** using worker subset selection algorithms (see multi-worker selection document).[web:60][web:62]
4. For each candidate, estimate performance under current network and load.
5. Apply optimization principle:

> Only choose collaboration if the expected net benefit is positive and above a configurable threshold.

Concrete example aligning with user’s principle:

- A: Desktop only → 30 tok/s.
- B: Desktop + Laptop → 45 tok/s (CAN_COLLABORATE true; network okay).
- C: Desktop + Laptop + GPU3 → 51 tok/s.
- D: Desktop + Laptop + GPU3 + CPU node → 38 tok/s.

If latency is primary objective, Planner should select **B or C**, not D, and perhaps choose C only if added complexity and resource cost is justified.

## Backend-Specific Roles

### llama.cpp RPC (Experimental Collaboration Mode)

- Use **split-mode=layer** for pipeline-style partitioning across heterogeneous devices; avoid `tensor` mode over RPC until performance and stability improve.[web:82][web:76]
- Limit collaborative worker count to 2–3 strong workers on ≥ 10 GbE; treat additional CPU-only nodes as memory donors only when required to fit weights.[web:36][web:49]
- Keep RPC workers within **trusted network segments**, fronted by DecentraAI authentication and mTLS.[web:4][web:9][web:83][web:61][web:72]

### vLLM and TensorRT-LLM (Datacenter Backends)

For scenarios where DecentraAI targets **homogeneous clusters**:

- vLLM: use TP within node and PP across nodes; choose TP/PP degrees to balance memory and latency.[web:24][web:50][web:51]
- TensorRT-LLM: prefer for NVIDIA datacenter GPUs (A100, H100, B200) when maximum throughput is needed; treat as specialized backend, offloading entire request to cluster rather than P2P nodes.[web:18]

### DeepSpeed and Megatron-LM

- DeepSpeed: use inference-adapted tensor parallelism for multi-GPU nodes; avoid multi-node TP for production unless network is very fast.[web:31][web:29][web:19]
- Megatron-LM: primarily a training stack; inference can reuse its TP/PP layouts on clusters, but integration into DecentraAI is likely out-of-scope for consumer P2P environments.[web:22][web:28]

### Ray

Ray remains an **orchestration layer** rather than a model-parallel engine; vLLM and others use Ray as distributed executor for multi-node setups.[web:24][web:57]

DecentraAI can optionally use Ray for cluster management in datacenter contexts, but for P2P the existing MCP and worker lifecycle mechanisms are the primary orchestrators.

## Failure and Recovery Strategy

Given the limitations of collaborative backends:

- Treat distributed inference as **best-effort**; plan and implement robust fallback to single-worker inference.
- Use **KV cache persistence** and prompt caching where available to reduce cost of retries after failure.[web:34][web:39][web:42][web:45]
- On worker failure during decode:
  - Mark collaborative candidate as degraded.
  - Optionally retry from prompt on a reduced worker set or single worker, according to quota ledger and user SLA.
- Log provenance and failure events for each candidate to improve confidence estimates.

## Security and Trust Boundaries

Multi-worker collaboration introduces new trust risks:[web:61][web:63][web:68][web:71][web:72]

- **Prompt confidentiality**: activations and KV contents traversing untrusted workers can leak sensitive prompts or internal system instructions.
- **Model confidentiality**: weight shards and KV can be exfiltrated or tampered with by malicious workers.
- **Result integrity**: compromised workers can return corrupted tensor results.

DecentraAI should:

- Use **encrypted transport** (mTLS) between workers where possible.[web:61][web:72]
- Integrate **worker authentication and trust scoring** into CAN_COLLABORATE.
- Define trust boundaries per request and model (e.g. sensitive prompts only collaborate with Tier‑1 trusted workers).
- For high-sensitivity workloads, consider redundant computation (e.g. double-running critical segments on two independent workers and comparing outputs) at the cost of throughput.

## What DecentraAI Should Implement

Short term (EXPERIMENT FIRST):

- A **DistributedExecutionCandidate** abstraction and Planner logic for comparing collaborative vs single-worker plans.
- llama.cpp RPC-based experimental collaboration mode with 2–3 workers in trusted segments, using `split-mode=layer` and conservative `--tensor-split` based on VRAM and network.[web:4][web:76][web:82]
- Worker subset selection algorithms based on Hetis-style pruning and simple performance models.[web:60][web:62][web:54][web:56]
- Failure handling and fallback paths; logging and provenance for all collaborative runs.

Medium term:

- Measurement harness for real DecentraAI fabrics (desktop + laptop + extra GPU + CPU node) to populate benchmark matrix, refine performance models, and validate selection heuristics.
- Optional integration of vLLM/TensorRT-LLM in datacenter contexts as alternative backends for CAN_COLLABORATE when homogeneous clusters exist.

## What DecentraAI Should NOT Implement (Near Term)

- Full general-purpose TP/PP engine from scratch; instead rely on llama.cpp and mature frameworks.
- Mid-decode dynamic repartitioning across workers; complexity and KV movement costs outweigh benefits for most DecentraAI scenarios.
- Exposing llama.cpp RPC servers on untrusted or public networks without strong security controls.

---

This design document provides a DecentraAI-specific architecture for **multi-worker distributed inference** built on existing systems and research. The planner’s optimization logic, worker subset selection, and backend integrations will be refined through the experimental harness and benchmark matrix.
