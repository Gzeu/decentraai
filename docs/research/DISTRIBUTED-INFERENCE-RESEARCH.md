# DecentraAI Distributed Inference Research

Status: **VERIFIED** where backed by upstream docs or papers, **EXPERIMENTAL** for PoC features, **INFERRED** for design implications, **UNKNOWN** where data was not found.

## Scope and Definitions

DecentraAI is investigating **distributed inference / model parallelism** where a **single request** to a single model is executed cooperatively across multiple workers (CPUs and GPUs) and returns a **single result**, as opposed to fan-out of independent requests.[web:13][web:24]

Key parallelism patterns used by mature systems:

- **Tensor parallelism (TP)**: shards tensors inside each layer across GPUs, requiring all-reduce communication at each layer.[web:24][web:32][web:51]
- **Pipeline parallelism (PP)**: partitions the stack of layers into stages across devices, sending activations between stages.[web:21][web:22][web:24]
- **Data parallelism (DP)**: runs full model replicas on multiple devices and aggregates or routes requests for throughput.[web:51][web:53]

## Distributed Inference vs Fan-Out

Distributed inference for a single request requires that **weights and KV cache** are partitioned across devices; each token depends on previous tokens and must traverse all partitions.[web:13][web:50] This is fundamentally different from DecentraAI’s existing **adaptive fan-out**, which treats each worker as an independent serving endpoint.

Distributed inference adds:

- Cross-device coordination and synchronization per token.
- Sensitivity to **network latency and bandwidth**, especially during decode.
- Tight coupling between worker capabilities (memory, compute, interconnect).

## Mature System Landscape (High-Level)

Across vLLM, DeepSpeed, TensorRT-LLM, Megatron-LM, and Ray, distributed inference is primarily designed for **homogeneous, data-center GPUs** rather than highly heterogeneous consumer hardware.[web:24][web:18][web:28][web:31]

Common properties:

- **Single logical inference graph** partitioned across devices via TP/PP.
- Strong assumption of **fast intra-node interconnect** (NVLink, PCIe Gen4/5) and high-bandwidth inter-node links (InfiniBand, 25–100 GbE).[web:22][web:24][web:50]
- Focus on **large models (70B–400B+)** that will not fit on a single GPU.

These systems demonstrate that cooperative inference across many workers is feasible, but they operate under conditions quite different from DecentraAI’s heterogeneous, P2P fabric.

## Heterogeneous Fabric Considerations

Published work on heterogeneous GPU clusters (Hetis, Parallax, heterogeneous vLLM forks) shows that **using all available devices is not always optimal**.[web:60][web:69][web:52]

Key observations:

- Adding slower or low-memory GPUs can **increase latency** due to all-reduce or PP communication overhead.[web:54][web:56]
- Heterogeneous TP requires careful **uneven sharding** based on VRAM and compute; naive equal splits can stall fast GPUs waiting on slow ones.[web:15][web:60]
- For consumer hardware without NVLink, pipeline-style splits (`layer` mode) are often preferable to tensor-style splits (`row` / `tensor`) due to lower communication volume.[web:82][web:76]

Conclusion (INFERRED): DecentraAI should treat the worker pool as a **resource fabric** where only a subset of workers collaborate on a given distributed inference, chosen to maximize net benefit.

## Network Characteristics and When Inference Becomes Network-Bound

Representative figures:

- 1 GbE: ≈ 125 MB/s throughput, ≈ 1 ms RTT; adequate for small PP but quickly becomes a bottleneck for TP.[web:49][web:59]
- 10 GbE: ≈ 1.25 GB/s, measured ≈ 9.41 Gbps via iperf3 in llama.cpp RPC benchmarks, RTT ≈ 0.5–1 ms.[web:36][web:49]
- In distributed llama.cpp RPC Metal+CUDA tests over 10 GbE, prefill throughput improved (≈ 4.2×) but decode (token generation) became **network-latency bound**: each token incurred ~0.17 ms round-trip, resulting in up to ~2× slower decode compared to single-node runs.[web:36]

Community reports indicate that 1 GbE may "work" for RPC layer splits but leads to **significant overhead** and much slower decode; 10 GbE is often cited as the **minimum** for usable distributed inference, with InfiniBand or RoCE recommended for high-throughput clusters.[web:49][web:46][web:59]

Implication (INFERRED): In a P2P environment with mixed Wi‑Fi and Ethernet, distributed inference is likely **decode-latency dominated** unless:

- The model is large enough that prefill dominates compute.
- Only a small number of well-connected workers participate.

## Scaling Efficiency and N+1 Worker Regression

Literature and engineering reports show multiple cases where adding more workers **reduces performance**:

- Megatron-LM and related work highlight that tensor parallelism scales efficiently **within a node** but suffers when extended across nodes due to expensive all-reduce communication; PP across nodes is recommended instead.[web:22][web:21][web:30]
- vLLM guidance and SiPipe results show that higher TP degrees eventually hit a throughput ceiling because communication cost dominates, while deeper PP increases per-token latency due to pipeline bubbles.[web:24][web:54]
- DeepSpeed maintainers note that multi-node tensor parallel inference "works" but has **poor performance** because of cross-node communication.[web:19][web:29]

Typical patterns (VERIFIED from upstream docs and papers):

- **1 → 2 GPUs**: clear speedup if interconnect is fast and workload is large.
- **2 → 4 GPUs**: diminishing returns; TP often remains compute-bound.
- **Beyond 4–8 GPUs**: communication overhead (all-reduce, P2P activations) dominates; throughput gains flatten or regress depending on topology.[web:22][web:54]

For heterogeneous clusters, Hetis and similar systems **explicitly drop low-end GPUs** from the primary TP group if removing them does not increase dense-module compute cost by more than a small threshold (e.g. 5%).[web:60] This is a formalization of "N+1 workers can make things worse".

## Worker Subset Selection Concepts

Research and system designs suggest worker selection should be an optimization problem over:

- Compute capacity (TFLOPs), memory (VRAM/RAM), and KV cache headroom.[web:25][web:51]
- Network bandwidth/latency and interconnect type (NVLink vs PCIe vs Ethernet vs Wi‑Fi).[web:49][web:59]
- Current load, thermal throttling, and stability.[web:60][web:64]

Example formulations:

- Hetis selects a subset of GPUs as **Primary workers** for dense modules, pruning devices whose removal does not increase dense compute cost beyond a threshold, then uses remaining devices as Attention workers.[web:60]
- Kitzberger’s heterogeneous ONNX partitioning thesis proposes a dynamic partitioning algorithm that minimizes total execution cost subject to worker memory, bandwidth, and latency constraints, using DP over layers and workers.[web:62]

DecentraAI can adopt a simplified version: maintain a **DistributedExecutionCandidate** that describes workers, partitioning strategy, and predicted cost, and accept the candidate only if the predicted cost is strictly better than the best single-worker plan by some margin.

## Failure, Adaptation, and Recovery Themes

Mature serving systems generally:

- Treat **mid-inference worker failure** as fatal for that request; recovery is usually by re-executing from the prompt or from a saved KV cache checkpoint.[web:34][web:42][web:67]
- Adapt worker usage **between requests**, not within a single in-flight request, because repartitioning weights and KV across devices requires a new graph and state.[web:24][web:31][web:49]
- Provide limited automatic fault tolerance (e.g. restarting failed workers, health checks, Ray-based re-placement) but rarely dynamic re-sharding during decode.[web:24][web:67]

For llama.cpp, the RPC backend is explicitly documented as **fragile and insecure PoC**, with single-threaded server handling one client at a time and no robust multi-client scheduling or fault recovery.[web:4][web:86][web:9]

Implication (INFERRED): DecentraAI should:

- Treat distributed inference as **best-effort** with clear fallback to single-worker inference.
- Use persistent KV caches and prompt caches to reduce cost of retries where possible.[web:45][web:39][web:42]

## Security and Trust Themes (High-Level)

Allowing multiple workers to participate in one inference raises:

- **Prompt and model confidentiality**: remote workers see weight shards, activations, and potentially KV cache contents; if unencrypted, they can infer prompt fragments or internal state.[web:61][web:63][web:68]
- **Result integrity**: a malicious worker can return incorrect tensor results, corrupting the final output without easy detection.
- **Worker authentication and network exposure**: llama.cpp RPC currently lacks authentication and is explicitly warned against use on open networks.[web:4][web:9][web:83]

Best practices from security surveys and advisories include:

- Strict **zero-trust** boundaries between workers, encrypted transport (mTLS), and cryptographic artifact pinning for models.[web:61][web:72]
- Treat inference workers as high-privilege code, hardening them with container isolation, restricted egress, and model signing.[web:61][web:72]

DecentraAI already has trust/policy, provenance, and worker lifecycle mechanisms; distributed inference will need additional **per-tensor trust assumptions** and possibly redundancy (e.g. duplicate computation for high-security workloads).

---

This file provides the high-level research context for distributed inference and heterogeneous fabrics for DecentraAI. Detailed llama.cpp RPC analysis, multi-worker selection algorithms, DecentraAI-specific design, and benchmark matrices are captured in the companion documents in this directory.
