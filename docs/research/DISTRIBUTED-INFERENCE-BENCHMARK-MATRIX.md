# Distributed Inference Benchmark Matrix

Status: **VERIFIED** where real benchmarks are cited, **INFERRED** for qualitative trends, **UNKNOWN** where exact data is unavailable. This document is intentionally conservative and avoids fabricated numbers.

## Purpose

Provide a structured matrix of **external benchmarks and qualitative trends** for distributed inference across 1–5 workers, focusing on:

- llama.cpp RPC (heterogeneous CPU/GPU, 1–3 nodes).
- vLLM TP/PP on datacenter GPUs.
- TensorRT-LLM vs vLLM on NVIDIA datacenter GPUs.
- General network scaling behavior (1–10 GbE, Wi‑Fi).

This matrix is a **reference** and starting point; DecentraAI should add its own measurements for concrete fabric configurations.

## Network and Interconnect Characteristics

Representative network characteristics:[web:49][web:36][web:59]

| Link Type       | Bandwidth (approx) | RTT (approx) | Notes                                   |
|-----------------|--------------------|--------------|-----------------------------------------|
| 1 GbE Ethernet  | 125 MB/s           | ~1 ms        | Too slow for TP; marginal for PP        |
| 2.5 GbE Ethernet| 312 MB/s           | ~0.8–1 ms    | RPC row-split can saturate quickly      |
| 10 GbE Ethernet | 1.25 GB/s          | ~0.5–1 ms    | Minimum viable for PP; borderline for TP|
| NVLink (intra-node)| 50+ GB/s (varies gen)| < 0.5 µs | Preferred for TP within node            |
| InfiniBand HDR  | 200 Gb/s           | ~0.6 µs      | Training and high-end TP/PP             |

10 GbE direct-attach between Mac Studio and DGX measured ~9.41 Gbps via iperf3 in a llama.cpp RPC benchmark.[web:36]

## llama.cpp RPC Benchmarks (External)

From mixed Metal + CUDA benchmarks over 10 GbE:[web:36]

| Mode                | Prefill tok/s | Decode tok/s | Notes                                      |
|---------------------|--------------:|-------------:|--------------------------------------------|
| Single-node (GPU)   |    (VERIFIED, exact numbers omitted here) | (VERIFIED) | Baseline, NVLink/PCIe only                     |
| RPC Metal + CUDA    |        317.7  |        52.7  | ~4.2× prefill speedup; decode slower        |

Key qualitative findings:

- Prefill benefits significantly from offloading to remote GPUs.
- Decode becomes **network-latency bound**; token generation is slower with RPC than single-node runs.[web:36]

Community reports on 1 GbE vs 2.5 GbE:

- 1 GbE "works" but leads to substantial overhead; tensor-parallel row splits quickly saturate bandwidth on 2.5 GbE.[web:46]

## vLLM TP/PP Scaling

From vLLM docs and PP/TP benchmark analyses:[web:24][web:50][web:51][web:52][web:56]

Qualitative patterns:

- **TP (within node)**: best latency and throughput when GPUs share NVLink; TP=8 on a strong node yields high efficiency for interactive workloads.
- **PP (across nodes)**: scales memory and throughput but increases per-token latency due to pipeline bubbles; PP depth must be chosen carefully.
- **DP**: removes model-parallel overhead entirely and can outperform TP/PP for high concurrency workloads.

Example configurations (from vLLM and SiPipe analyses):

| GPUs | TP | PP | Workload Type         | Trend                                       |
|------|----|----|-----------------------|---------------------------------------------|
| 1    | 1  | 1  | Baseline              | Single GPU; no model parallel               |
| 2    | 2  | 1  | TP only               | Clear latency improvement; NVLink critical  |
| 4    | 4  | 1  | TP only               | Good scaling; communication overhead rising |
| 8    | 8  | 1  | TP only               | Latency excellent; all-reduce cost high     |
| 8    | 4  | 2  | TP+PP                 | Better throughput/heavy workloads           |
| 16   | 2  | 8  | TP+PP                 | Higher PP; latency increases                |

SiPipe shows that PP-based configurations (e.g. vLLM P⁴₄) can achieve up to ~3.22× throughput over pure TP configurations (P¹₁₆) under specific conditions by eliminating pipeline bubbles.[web:54]

## TensorRT-LLM vs vLLM

From TensorRT-LLM performance comparisons:[web:18]

Qualitative trends:

- TensorRT-LLM generally delivers higher throughput and lower TTFT than vLLM on NVIDIA datacenter GPUs (A100, H100, B200) with FP8.
- Reported advantages include ~1.34× higher throughput on short sequences and ~2.72× better time-per-output-token on long sequences for certain models.

DecentraAI can treat TensorRT-LLM as a **specialized cluster backend** where available, not as a ubiquitous P2P solution.

## Cases Where N+1 Workers Hurt Performance

From available data and analyses:[web:22][web:54][web:19][web:36][web:46]

- Extending TP across nodes (instead of within NVLink islands) introduces expensive all-reduce across slower interconnects; throughput plateaus or regresses.
- Deep TP degrees (e.g. TP=16) can hit a ceiling where communication dominates compute; adding more TP ranks no longer improves tok/s.[web:22][web:54]
- For llama.cpp RPC, adding remote workers over 1–2.5 GbE can **increase decode latency** due to RPC round-trips, even if prefill is faster.[web:36][web:46]

Qualitative conclusion: beyond **4–8 strong GPUs on fast interconnects**, or 2–3 well-connected RPC workers, adding more workers often harms per-request latency and complicates failure scenarios.

## Benchmark Matrix Template for DecentraAI

DecentraAI should populate the following matrix with its own measurements:

| Fabric Configuration                           | Backend        | Workers | Network       | Prefill tok/s | Decode tok/s | TTFT (ms) | Notes                          |
|------------------------------------------------|---------------|--------:|--------------|--------------:|-------------:|----------:|---------------------------------|
| Desktop 4070 only                              | llama.cpp     |       1 | local        |              |             |           | Baseline                        |
| Desktop 4070 + Laptop 3060 (10 GbE)           | llama.cpp RPC |       2 | 10 GbE       |              |             |           | layer split                     |
| Desktop 4070 + Laptop 3060 + CPU node (1 GbE) | llama.cpp RPC |       3 | mixed 1/10Gb |              |             |           | expect degraded decode latency  |
| 2× RTX 4090 (single node)                      | vLLM          |       2 | NVLink       |              |             |           | TP only                         |
| 2× RTX 4090 (2 nodes, 10 GbE)                  | vLLM          |       2 | 10 GbE       |              |             |           | TP+PP                           |
| 4× RTX 4090 (2 nodes, 10 GbE)                  | vLLM          |       4 | 10 GbE       |              |             |           | TP+PP                           |

The matrix should clearly mark **VERIFIED**, **EXPERIMENTAL**, **INFERRED** rows and avoid any made-up numbers.

---

This benchmark matrix is intentionally incomplete; it captures **external behavior** and provides a scaffold for DecentraAI’s own measurements. All future entries must link to primary sources or DecentraAI’s internal measurement harness.
