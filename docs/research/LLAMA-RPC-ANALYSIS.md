# llama.cpp RPC Backend Analysis

Status: **VERIFIED** where backed by upstream llama.cpp docs and source, **EXPERIMENTAL** for PoC behavior, **INFERRED** for implications, **UNKNOWN** where data could not be located.

## Overview and Maturity

The llama.cpp RPC backend exposes ggml devices on remote hosts via a `rpc-server` binary and allows `llama-cli` / `llama-server` to offload tensor operations to one or more remote servers over TCP.[web:4][web:11][web:13][web:80]

Upstream documentation calls the RPC backend a **fragile and insecure proof-of-concept**, explicitly warning against running it on open networks or in sensitive environments.[web:4][web:9] The implementation focuses on technical feasibility, not production-grade security or multi-tenant robustness.

## RPC Architecture

Primary components (VERIFIED from source and docs):

- **ggml RPC backend** (`ggml_backend_rpc_*`): implements a backend that serializes ggml tensor operations and sends them over TCP to a remote server hosting a real backend (CPU, CUDA, Metal, etc.).[web:80][web:78]
- **rpc-server** (`tools/rpc/rpc-server.cpp`): exposes local devices (CUDA, Metal, CPU, etc.) as RPC endpoints; built with `-DGGML_RPC=ON` and appropriate accelerator flags (e.g. `-DGGML_CUDA=ON`).[web:4][web:10][web:13]
- **llama-cli / llama-server**: standard llama.cpp clients that can be built with RPC support and passed a comma-separated list of RPC servers via `--rpc host:port,...`.[web:4][web:11][web:77]
- **RPC protocol**: fixed binary framing with `rpc_cmd` and size-prefixed payloads; max 16 servers (`GGML_RPC_MAX_SERVERS`).[web:81][web:86]

The `rpc-server` is currently single-client, single-thread per connection; server-to-server communication is work-in-progress and not ready.[web:86]

## Device and Model Split Capabilities

### Local Multi-GPU Split Modes

llama.cpp supports several split modes for multi-GPU runs:[web:82]

- `layer` (default): pipeline parallelism; each GPU holds a contiguous slice of layers, and the KV cache for layer *l* lives on the GPU that owns layer *l*.
- `row` (deprecated): older tensor-parallel path splitting dense weights by rows.
- `tensor` (experimental): tensor parallelism that splits both weights and KV across participating GPUs via a meta-device abstraction.

The `--tensor-split` option specifies proportions per device (comma-separated, in the order of `--device`), and applies both to local GPUs and to RPC devices once registered.[web:79][web:76][web:82]

### RPC Devices and Tensor-Split

Remote RPC servers appear as additional devices in the same device list used for multi-GPU splits; the **same `--tensor-split` proportions apply across local and remote devices**.[web:76][web:84]

Example (VERIFIED):

```bash
llama-server \ 
  -m model.gguf \ 
  --rpc 192.168.1.10:50052,192.168.1.11:50052 \ 
  --tensor-split 4,3,3
```

Here, the main host’s local GPU gets 4/10 of the model, each remote RPC device gets 3/10, with layers or tensors distributed according to `--split-mode`.[web:76]

This means **one inference request can be executed cooperatively across multiple heterogeneous workers** (CPU/GPU) via the RPC backend, with the model partitioned across them.

## Model Parallelism Semantics

### Layer (Pipeline) Parallelism over RPC

With `split-mode=layer`, layers are partitioned across devices; tokens flow sequentially through layers, crossing device boundaries when needed.[web:82][web:76]

Over RPC, this is effectively **pipeline parallelism over TCP**:

- Weights for layer ranges reside on different devices (local GPU, remote GPU, remote CPU).
- Activations for boundary layers are serialized and sent over the network between devices.[web:13][web:80]

This mode works reasonably over modest interconnects (1–10 GbE), but decode latency accumulates per-token due to per-step RPC calls.[web:36][web:49]

### Tensor Parallelism over RPC

Experimental `split-mode=tensor` splits both weights and KV across GPUs using a meta-device abstraction; this is designed primarily for multiple NVIDIA GPUs with CUDA backend and high-speed interconnects.[web:82]

Using this over networked RPC devices is **EXPERIMENTAL** and not well-characterized; community reports suggest that row/tensor splits become strongly network-bound even on 2.5 GbE, quickly maxing bandwidth and hurting latency.[web:46][web:76]

### KV Cache and Memory Distribution

KV cache behavior for multi-GPU and RPC:

- In `layer` mode, KV entries for a given layer live on the device that owns that layer, so KV is naturally partitioned along layers.[web:82]
- In `tensor` mode, KV is sharded across GPUs along tensor dimensions.[web:82]
- llama.cpp server supports unified KV (`--kv-unified`) and various KV cache data types (f16, bf16, q4/q5/q8 variants), plus offload and defragmentation options, but these primarily concern single-node multi-GPU behavior.[web:45]

RPC-specific KV behavior is not heavily documented; however, distributed inference guides note that **KV cache and weights are both distributed across local and remote devices**, with capacity constrained by the memory `-m` advertised by each rpc-server.[web:4][web:8][web:13]

## Multi-Node and Heterogeneous Hardware Support

llama.cpp RPC explicitly supports:

- **Multiple RPC servers** specified via `--rpc host:port,...`; each server exposes one or more devices (GPU or CPU).[web:4][web:10][web:11]
- **Heterogeneous backends**: CUDA on Linux, Metal on macOS, and CPU-only workers can all participate in a single inference graph.[web:4][web:6][web:17]

Examples from upstream docs and community:

- Arm-based CPU cluster: master node hosts model; worker nodes run `rpc-server`; distributed inference across CPU-only machines when model does not fit on a single node.[web:8]
- Mixed NVIDIA + Apple Silicon: Mac Studio running `llama-server` with RPC to DGX Spark; layers offloaded to remote CUDA GPUs over 10 GbE.[web:17][web:36]

These examples confirm that **one inference request can be served by a heterogeneous multi-node fabric** with CPU and GPU workers, so long as all machines run compatible llama.cpp builds with `GGML_RPC=ON`.[web:4][web:6][web:13]

## Network Requirements and Performance Behavior

Real benchmarks and reports:

- 10 GbE direct-attach between Mac Studio and DGX: measured 9.41 Gbps with iperf3; prefill throughput improvement (≈ 4.2×) and decode slower (≈ 2× for 7B, ≈ 47% slower for 72B) due to per-token RPC round-trips (~0.17 ms each).[web:36]
- CPU/GPU mixed clusters: guidance that 1 GbE "works" but is strongly bottlenecked; 10 GbE considered minimum for distributed inference, with InfiniBand recommended for high throughput.[web:49]
- Community tests: 1 GbE performing adequately for simple layer splits, but tensor-parallel row mode saturates bandwidth even on 2.5 GbE.[web:46]

Conclusion (VERIFIED/INFERRED):

- Prefill (processing the prompt) can see significant speedups when offloading large portions of the model to remote GPUs.
- Decode (token generation) is **network-latency bound**; each token requires RPC invocations across devices in the partition graph, and slower links degrade throughput.

In heterogeneous, consumer-grade networks (mixed Wi‑Fi, 1 GbE, 2.5 GbE), llama.cpp RPC is likely to be **practical for fitting larger models**, but not for maximizing per-request latency.

## Synchronization and KV Behavior

The RPC backend follows ggml’s graph execution model:

- RPC client (`llama-server`) constructs computation graphs and sends tensor ops over RPC; RPC servers execute ops on their backends and return results.[web:78][web:75]
- Synchronization occurs at graph boundaries and during all-reduce style operations (especially for tensor-style splits), but implementation details are mostly in ggml RPC source rather than high-level docs.[web:13][web:81]

KV cache behavior across RPC devices is largely implicit: each device stores KV entries for layers/tensors it owns, and the client orchestrates attention computation accordingly.[web:82][web:37][web:40]

KV-related features such as slots, cache reuse, and persistent KV caches are implemented in `llama-server` and are mostly **orthogonal** to RPC; they can reduce prefill cost but do not fundamentally alter per-token cross-device coordination.[web:34][web:39][web:42]

## Failure Behavior and Limitations

Known limitations and behaviors:

- The RPC backend is documented as **fragile and insecure**, not intended for exposed production use; it lacks built-in authentication, encryption, and multi-tenant isolation.[web:4][web:9]
- `rpc-server` is single-client and single-thread per connection; concurrent multi-client workloads require external coordination or multiple server instances.[web:86]
- Community reports note memory leaks and OOM conditions in long-running RPC deployments; killing the primary node does not release memory on RPC servers, requiring manual `rpc-server` restart.[web:73]
- There is no documented mechanism for **automatic re-partitioning or failover** during an in-flight inference; worker failure is expected to abort the request.

In practice, distributed inference via RPC is **best-effort**:

- If any RPC server fails during compute, the primary process will encounter errors and the request fails.
- Recovery is via retry, potentially with different worker set or reduced partition, reloading weights and KV.

## Practical Worker Counts and Selection

Based on upstream docs and real-world write-ups:

- The RPC backend supports up to 16 servers (`GGML_RPC_MAX_SERVERS`).[web:81]
- Most documented deployments use **2–3 workers**: one primary GPU host plus one remote GPU, or one primary CPU node plus one or two workers.[web:8][web:10][web:17][web:49]
- Community guidance suggests that beyond a small number of well-connected workers, RPC overhead and coordination complexity outweigh latency benefits; using more workers is primarily for **fitting larger models**, not for latency gains.[web:49][web:73]

Implication (INFERRED): For DecentraAI’s multi-worker distributed inference, llama.cpp RPC is most practical with **2–3 strong workers** on decent Ethernet (≥ 10 GbE) and CPU-only or weaker devices participating only when needed for memory.

## Suitability for DecentraAI

Strengths (VERIFIED/INFERRED):

- Enables DecentraAI to **fit larger GGUF models** across multiple consumer devices (desktop GPU + laptop GPU/CPU + additional nodes) without changing model format.
- Works with **heterogeneous hardware** (NVIDIA CUDA, Metal, CPU), fitting DecentraAI’s P2P network vision.[web:4][web:6][web:17]
- Integrates naturally with existing llama.cpp-based worker harness; DecentraAI already experiments with this backend.

Weaknesses:

- **PoC maturity**: no built-in auth, fragile protocol, single-client server; unsuitable for untrusted or open networks without additional security layers.[web:4][web:9][web:83]
- **Network bound** decode performance: more workers and slower links can **hurt latency**, even if prefill speeds improve.[web:36][web:46][web:49]
- Limited failure handling: mid-inference worker failure aborts request; recovery requires higher-level orchestration and possibly KV checkpointing.[web:73][web:42]

Conclusion (INFERRED): llama.cpp RPC is a **good experimental substrate** for DecentraAI’s multi-worker distributed inference R&D, especially for exploring heterogeneous fabrics and partition strategies. It should be sandboxed inside trusted segments, fronted by DecentraAI’s planner and trust/policy engine, and treated as an **experimental collaboration mode** rather than production default.
