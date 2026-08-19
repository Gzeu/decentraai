# Disaggregated Prefill/Decode for DecentraAI

Status: MIXED (VERIFIED for vLLM/SGLang/NVIDIA Dynamo implementations; INFERRED for DecentraAI integration).[cite:103][cite:104][cite:110][cite:111][cite:117]

This document focuses on disaggregated prefill/decode as a core execution strategy for a heterogeneous DecentraAI fabric.

## 1. Background: Prefill vs Decode

LLM inference consists of two phases:

- **Prefill**: processing the entire input prompt (system + history + user message) to build the KV cache.
  - FLOPs-bound, dominated by matrix multiplications across the full context.[cite:108][cite:113]
- **Decode**: autoregressive token generation using the KV cache.
  - Memory-bandwidth-bound, dominated by KV reads for each new token.[cite:110][cite:111]

Running both phases on the same worker leads to interference:

- Long-context prefills can delay decode batches, increasing tail latency and degrading QoS.
- Decode-heavy workloads can suffer TTFT spikes when prefill jobs share the same GPU.

Disaggregation separates these phases onto different workers.

## 2. vLLM Disaggregated Prefilling (VERIFIED)

### Architecture

- Two vLLM instances:
  - **Prefill instance**: KV producer.
  - **Decode instance**: KV consumer.[cite:103][cite:104]
- KV transfer via `kv_transfer_params` and connectors (e.g., `NixlConnector`).
- A router or sidecar orchestrates request flow: prefill → KV transfer → decode.[cite:110][cite:117]

### KV Transfer Protocol (vLLM V1)

- Prefill request:
  - `return_token_ids: True` to obtain `prompt_token_ids`.
  - `kv_transfer_params` includes remote decode metadata (host, port, connector config).
  - `max_tokens=1` suppresses output; prefill primarily builds KV.[cite:103][cite:110]
- Prefill response:
  - Contains `prompt_token_ids` and backend-specific KV transfer parameters.
- Decode request:
  - Includes `messages` and `kv_transfer_params` with `prompt_token_ids` from prefill.
  - When `kv_transfer_params` is present, decode skips tokenization and coordinates KV transfer with the prefill instance.[cite:103][cite:110]

### Hardware Specialization

- Prefill nodes:
  - Compute-dense GPUs (H100, B200) optimized for FP8 and high FLOPs.[cite:104]
- Decode nodes:
  - Memory-dense GPUs (H200, Hopper) optimized for large KV cache and HBM bandwidth.[cite:104][cite:110]

### Network Requirements

- **KV size**: proportional to `n_layers × hidden_dim × n_heads × context_length`.
- vLLM disaggregation assumes:
  - High-bandwidth links (NVLink, InfiniBand/RDMA, or 10/25/100 GbE) for efficient KV transfer.[cite:110][cite:111]
  - Latency low enough that KV transfer completes before decode needs the cache.

On consumer fabrics, Ethernet (1/2.5/10 GbE) and Wi‑Fi impose tighter constraints.

## 3. SGLang PD Disaggregation (VERIFIED)

### Architecture

- Separate servers with `--disaggregation-mode prefill` and `--disaggregation-mode decode`.[cite:107]
- Router (`sglang_router` or Shepherd Model Gateway) handles PD disaggregation:
  - Sends requests to both prefill and decode workers.
  - Coordinates KV transfer using bootstrap rooms and RDMA.

### KV Transfer Backend

- Backends: `nixl`, `mooncake`, and others.
- Flow:[cite:111][cite:114]
  1. Decode worker receives client request.
  2. Decode forwards request to prefill, obtaining a `bootstrap_room` ID.
  3. Prefill runs prompt and writes KV cache directly into decode’s GPU memory via RDMA.
  4. Decode polls for completion; once KV is present, decode continues.

### Routing Policies

- Router can use:
  - **Cache-aware prefill selection**: choose prefill nodes with highest prefix coverage.[cite:114]
  - **Power-of-two decode selection**: randomized but balanced selection among decode nodes.[cite:114]

## 4. LMCache and Disaggregated Serving (VERIFIED)

LMCache extends disaggregated serving by externalizing KV cache:

- Extracts KV from vLLM/SGLang engines and stores it in tiered storage (GPU, CPU, disk, S3, Redis).[cite:133][cite:138][cite:144]
- Shares KV across engines and nodes via `LMCacheConnector`.[cite:133][cite:137]
- Supports both:
  - **Prefix reuse** (context caching across requests).
  - **PD disaggregation** (cross-engine/GPU cache transfer).[cite:115]

This shows that KV cache can be treated as a fabric-level resource, not just engine-local state.

## 5. DecentraAI Integration Strategy (INFERRED)

### 5.1 Execution Strategy: DISAGGREGATED_PREFILL_DECODE

Introduce an `ExecutionStrategy::DisaggregatedPrefillDecode` abstraction:

- Fields (INFERRED):
  - `prefill_workers: Vec<WorkerId>`
  - `decode_workers: Vec<WorkerId>`
  - `kv_connector: KvConnectorKind` (e.g., LMCache, NIXL, engine-native)
  - `expected_ttft: f64` (MEASURED/ESTIMATED)
  - `expected_itl: f64` (MEASURED/ESTIMATED)
  - `kv_transfer_cost_bytes: u64` (ESTIMATED from context length)
  - `network_path: NetworkPathFacts` (MEASURED)
  - `provenance: EvidenceClass`

### 5.2 Worker Selection

Planner chooses prefill/decode workers based on:

- Hardware fit:
  - Prefill: high FLOPs, sufficient VRAM.
  - Decode: high memory bandwidth, stable VRAM, good continuous batching characteristics.
- NetworkFacts:
  - RTT and bandwidth between prefill and decode workers.
  - Link classification: LAN (1/2.5/10 GbE), Wi‑Fi, WAN.
- KV state:
  - Prefill worker cache affinity (prefix coverage).
  - Decode worker capacity and existing decode load.

### 5.3 Heterogeneous Consumer Fabric Examples

**Example A:**

- Desktop GPU (RTX 4070): prefill.
- Laptop GPU (RTX 3060): decode.

Prefill runs on the compute-strong desktop; decode runs on a memory-adequate laptop if network RTT and bandwidth permit KV transfer without bottlenecking decode.[cite:104][cite:144]

**Example B:**

- Laptop GPU: prefill (small context, energy-aware selection).
- Desktop GPU: decode (long answer, high throughput).

Planner should select this only if measured network and KV transfer costs are low enough to justify energy savings or load balancing.

### 5.4 When Disaggregation is Beneficial

Disaggregated prefill/decode is beneficial when:[cite:108][cite:110][cite:111]

- Prefill workloads are long-context and interfere with ongoing decode on a single GPU.
- Hardware heterogeneity allows specialization (compute vs memory) across devices.
- KV transfer overhead is small relative to prefill cost and decode duration.

On small consumer fabrics, disaggregation may **hurt latency** if:

- Network RTT and bandwidth are insufficient (e.g., Wi‑Fi or 1 GbE for large KV blocks).
- Contexts are short; prefill cost is small compared to KV transfer cost.

Planner must therefore treat DISAGGREGATED_PREFILL_DECODE as a **conditional strategy**, not a default.

## 6. Evidence Classification

- vLLM disaggregated prefilling: **VERIFIED** (official docs and examples).[cite:103][cite:104][cite:137]
- SGLang PD disaggregation: **VERIFIED** (official docs and router configuration).[cite:107][cite:111][cite:114]
- LMCache integration for disaggregation and KV sharing: **VERIFIED** (paper + docs + GitHub).[cite:133][cite:137][cite:138][cite:140]
- Hardware specialization (prefill FLOPs-bound, decode memory-bound): **MEASURED/VERIFIED** (vendor docs and engineering blogs).[cite:104][cite:108][cite:110]
- DecentraAI ExecutionStrategy:DisaggregatedPrefillDecode abstraction: **INFERRED** based on these systems.

## 7. Recommendations for DecentraAI (GO / EXPERIMENT / WAIT)

- **GO NOW**:
  - Model DISAGGREGATED_PREFILL_DECODE as a first-class ExecutionStrategy in the planner.
  - Extend WorkerFacts and NetworkFacts to include fields required for prefill/decode selection (FLOPs, memory bandwidth estimates, RTT, bandwidth).[cite:110][cite:143]

- **EXPERIMENT FIRST**:
  - Integrate a single engine (vLLM or SGLang) with disaggregated prefilling on a two-node testbed (Desktop + Laptop) and measure TTFT/ITL improvements vs single-node and simple sharding.
  - Prototype a lightweight KV connector abstraction that can wrap LMCache and engine-native NIXL connectors.

- **WAIT**:
  - Full multi-node, RDMA-intensive PD disaggregation; DecentraAI’s consumer fabric may not justify this until RDMA is readily available.

Disaggregated prefill/decode is a **strong candidate** for one of DecentraAI’s top next-generation execution strategies, especially when combined with distributed KV caching and network-aware planning.
