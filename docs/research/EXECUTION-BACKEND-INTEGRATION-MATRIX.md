# DecentraAI Execution Backend Integration Matrix

This document maps DecentraAI execution strategies to practical, **existing** inference backends/runtimes.
DecentraAI remains the **control plane / planner / policy / provenance**, and backends remain the **inference engines**; workers are adapters that bridge DecentraAI to these engines.

## Legend

- Complexity: LOW / MEDIUM / HIGH
- DecentraAI fit: HIGH / MEDIUM / LOW
- Evidence: VERIFIED (documented + widely used), EXPERIMENTAL (papers / early adopters), UNKNOWN (no clear data)

Backends considered:
- **llama.cpp** (llama-server, RPC)
- **vLLM** (OpenAI-compatible server, tensor/pipeline parallelism)[cite:183][cite:195]
- **SGLang** (OpenAI-compatible server with advanced scheduling, RadixAttention, etc.)[cite:184][cite:192]
- **LMCache** (KV cache layer for vLLM/SGLang, prefill–decode disaggregation)[cite:185][cite:186][cite:190][cite:189]
- Other orchestration frameworks (Ray, etc.) as supporting infrastructure.[cite:183][cite:188][cite:57]

---

## 1. SINGLE_WORKER

**Recommended backend:** vLLM or SGLang, with llama.cpp kept as a minimal/CPU-friendly option.

- **Exact project:**
  - vLLM (`vllm-openai` server)[cite:195]
  - SGLang (`sglang serve`)[cite:184][cite:192]
  - llama.cpp (`llama-server`)
- **Current stable/relevant version:**
  - vLLM ≥ 0.15.x / 0.8.x docs (fast-evolving; use latest stable, e.g. 0.15.1 from recent distributed tutorials)[cite:188]
  - SGLang ≥ 0.5.x (e.g. v0.5.9 tag in quickstart)[cite:192]
  - llama.cpp latest master (no formal versioning; pinned commit per DecentraAI release)
- **Official documentation:**
  - vLLM: distributed serving & `vllm serve` CLI docs[cite:183][cite:194]
  - SGLang: main site + GitHub README / launch_server docs[cite:184][cite:192]
  - llama.cpp: README / docs for `llama-server`
- **Required runtime:**
  - Python 3.10+ environment, CUDA-capable PyTorch / FlashInfer for vLLM/SGLang.[cite:183][cite:192]
  - Native C++ binary and GGUF models for llama.cpp.
- **Supported platforms:**
  - vLLM: Linux x86_64 with CUDA; some TPU/Ray modes.[cite:183][cite:194]
  - SGLang: Linux x86_64 with CUDA; Docker images; OpenAI-compatible API.[cite:184][cite:192]
  - llama.cpp: Linux/macOS/Windows; CPU and GPU builds.
- **GPU/CPU requirements:**
  - vLLM/SGLang: GPU strongly recommended; can run small models on CPU.[cite:195][cite:184]
  - llama.cpp: flexible; good for CPU-only laptop/desktop.
- **Network requirements:**
  - SingleWorker: local loopback or LAN HTTP; no multi-node parallelism needed.
- **Heterogeneous workers supported:**
  - Yes at fabric level (DecentraAI chooses per worker); each backend instance is homogeneous per node.
- **Same-model multi-worker execution:**
  - Not needed; SingleWorker runs on one worker.
- **Production-ready vs experimental:**
  - vLLM and SGLang SingleWorker serving are **production-ready** in many deployments.[cite:183][cite:184]
  - llama.cpp local serving is mature; RPC remains experimental.
- **API/CLI/interface:**
  - OpenAI-compatible HTTP: `/v1/chat/completions` via vLLM/SGLang.[cite:183][cite:184][cite:192]
  - llama.cpp: HTTP/JSON or RPC depending on mode.
- **DecentraAI worker changes:**
  - Worker becomes a generic HTTP client to the backend: connect to OpenAI-compatible API instead of only llama-server.
- **Adapter viability:**
  - YES: DecentraAI workers can adapt to vLLM/SGLang/llama.cpp via an OpenAI-compatible adapter.
- **Measurements before enabling:**
  - Throughput, latency, TTFT per worker; CPU/GPU utilization per backend.
- **Fallback:**
  - SingleWorker on local llama-server; if backend fails, DecentraAI falls back to its existing single-node path.
- **Security implications:**
  - Backend must run on trusted nodes; TLS/identity between DecentraAI node and backend; prompts stay on worker.
- **Licensing implications:**
  - vLLM (Apache 2.0), SGLang (open-source, permissive), llama.cpp (MIT/BSD-like); all acceptable for DecentraAI.
- **Complexity:** LOW–MEDIUM.
- **DecentraAI fit:** HIGH.
- **Evidence:** VERIFIED.

---

## 2. BATCH_FAN_OUT

**Recommended backend:** vLLM or SGLang, plus DecentraAI’s existing batch allocation (`adaptive_load_shares` / `allocate_batch`).

- **Exact project:** vLLM, SGLang.[cite:183][cite:184]
- **Version:** same as SingleWorker.
- **Documentation:** distributed serving and scheduling docs (vLLM), server docs (SGLang).[cite:183][cite:195][cite:192]
- **Runtime & platforms:** as above.
- **GPU/CPU:** GPU recommended; CPU workers possible for small models.
- **Network:** LAN; no inter-backend coordination beyond multiple HTTP endpoints.
- **Heterogeneous workers:**
  - Supported at DecentraAI level by routing independent requests to different workers based on capacity/perf; backends themselves are homogeneous per instance.
- **Same-model multi-worker execution:**
  - Not required; each request is independent.
- **Production vs experimental:**
  - Production-ready; this is DecentraAI’s current adaptive batch fan-out extended to multiple backend types.
- **API/CLI:** OpenAI-compatible per backend; DecentraAI uses multiple HTTP endpoints.
- **Worker changes:**
  - None to core planner; workers already participate in batch routing; backend types expand.
- **Adapter viability:**
  - YES: adapter simply dispatches batch requests to workers using existing `allocate_batch` and perf metrics.
- **Measurements:**
  - Per-worker tokens/s, latency, queue depth; EWMA metrics already available in DecentraAI.[cite:166]
- **Fallback:**
  - If batch fan-out degrades performance, planner falls back to SingleWorker per request.
- **Security:**
  - Same as SingleWorker; ensure per-request isolation.
- **Licensing:** as above.
- **Complexity:** MEDIUM (multi-worker scheduling), but fabric already implements most of it.
- **DecentraAI fit:** HIGH.
- **Evidence:** VERIFIED.

---

## 3. SPECULATIVE_DRAFT_VERIFY

**Recommended backend:** SGLang (speculative decoding, RadixAttention), vLLM with candidate/draft support, plus LMCache for KV reuse.[cite:184][cite:192][cite:185][cite:186][cite:189]

- **Exact project:**
  - SGLang (speculative decoding; multi-head scheduling).
  - vLLM (experimental speculative features in some branches; documentation sparse).
  - LMCache (KV layer for cross-engine reuse).[cite:185][cite:186][cite:190][cite:189]
- **Version:**
  - SGLang ≥ v0.5.x for advanced features.[cite:192]
  - LMCache ≥ v0.4.6.[cite:138]
- **Runtime & platforms:** same Python/CUDA stack as above + LMCache deployed as a sidecar or library.[cite:190]
- **Heterogeneous workers:**
  - SGLang and vLLM support multi-GPU/multi-node; LMCache supports KV movement across engines/nodes.[cite:185][cite:190][cite:189]
- **Same-model multi-worker:**
  - Yes in speculative decoding: small draft model on Laptop, full model on Desktop, with LMCache transporting KV/prefixes.[cite:185][cite:190]
- **Production vs experimental:**
  - LMCache is described as widely adopted and production-ready KV cache layer.[cite:185][cite:186][cite:193][cite:189]
  - Speculative decoding across heterogeneous nodes remains EXPERIMENTAL.
- **API/CLI:**
  - SGLang/vLLM: OpenAI-compatible server.
  - LMCache: controller API for KV operations (lookup, move, pin/unpin).[cite:190]
- **DecentraAI worker changes:**
  - Workers must integrate with LMCache clients and coordinate draft/verify loops.
- **Adapter viability:**
  - YES, in principle: DecentraAI can treat LMCache + SGLang/vLLM as an external KV-aware speculative engine, driving draft on Laptop and verify on Desktop via control API.
- **Measurements:**
  - Draft vs baseline tok/s, TTFT, decode latency, acceptance rate, network transfer, CPU/GPU utilization, energy/thermal.
- **Fallback:**
  - If speculative path underperforms, planner falls back to SingleWorker on strongest node.
- **Security:**
  - KV caches contain prompt/model state; LMCache must be deployed within trust boundary and use encrypted transport for remote KV movement.[cite:190]
- **Licensing:** LMCache and vLLM/SGLang are permissive; integration acceptable.[cite:183][cite:184][cite:138][cite:193]
- **Complexity:** HIGH.
- **DecentraAI fit:** MEDIUM (requires careful adapter and measurement).
- **Evidence:** EXPERIMENTAL (for heterogeneous draft/verify; LMCache core is VERIFIED).

---

## 4. DISAGGREGATED_PREFILL_DECODE

**Recommended backend:** LMCache + vLLM/SGLang (Transport Mode prefill–decode disaggregation).[cite:185][cite:190][cite:189]

- **Exact project:** LMCache (KV cache transport), vLLM/SGLang as engines.[cite:185][cite:190]
- **Runtime & platforms:**
  - LMCache deployed with Python backends and storage (GPU, CPU DRAM, local NVMe, remote KV store).[cite:190]
- **Heterogeneous workers:**
  - Supported; LMCache moves KV across devices/nodes and engines.[cite:190][cite:185]
- **Same-model multi-worker:**
  - Yes: prefill on Desktop, decode on Laptop or vice versa.[cite:190]
- **Production vs experimental:**
  - LMCache’s Transport Mode is described as making PD disaggregation practical at enterprise scale; however, DecentraAI-specific integration would be EXPERIMENTAL.[cite:189][cite:190]
- **API/CLI:** LMCache controller API for KV lookup, move, clear, pin/unpin.[cite:190]
- **DecentraAI worker changes:**
  - Workers must understand LMCache KV identifiers and coordinate prefill/decode sessions.
- **Adapter viability:**
  - YES: DecentraAI can orchestrate which worker does prefill and which does decode, with LMCache carrying KV; control plane remains DecentraAI; engine is vLLM/SGLang.
- **Measurements:**
  - TTFT improvements, KV transfer cost, network bandwidth usage, error rates.
- **Fallback:** SingleWorker without PD disaggregation.
- **Security:** KV caches crossing nodes require strict policy; LMCache must honor encryption and access control; KV contains prompt contents.[cite:190]
- **Licensing:** LMCache + vLLM/SGLang are permissive.
- **Complexity:** HIGH.
- **DecentraAI fit:** MEDIUM.
- **Evidence:** EXPERIMENTAL (integration; LMCache core is VERIFIED).

---

## 5. CACHE_AWARE_ROUTE

**Recommended backend:** LMCache as KV layer; vLLM APC / SGLang RadixAttention; DecentraAI uses LMCache metadata to steer cache-aware routing.[cite:185][cite:186][cite:190][cite:189]

- **Exact project:** LMCache + vLLM/SGLang.
- **Runtime & platforms:** as above.
- **Heterogeneous workers:**
  - Supported; LMCache manages KV across engines/nodes.[cite:190]
- **Same-model multi-worker:**
  - Cache-aware routing is primarily about multiple engines sharing prefixes; multi-worker decode remains experimental.
- **Production vs experimental:**
  - LMCache cache offloading/offline storage is production-ready; full fabric‑wide cache-aware routing guided by DecentraAI is EXPERIMENTAL.[cite:189][cite:193]
- **API/CLI:** LMCache controller API; vLLM/SGLang OpenAI API.
- **DecentraAI worker changes:**
  - Workers must query LMCache for prefix hit/miss and KV locality and feed this into planner decisions.
- **Adapter viability:**
  - YES: DecentraAI can use LMCache as a service; workers call LMCache to check KV state and decide whether to stay or migrate.
- **Measurements:**
  - Cache hit ratios, TTFT, throughput, network overhead.
- **Fallback:** SingleWorker with local KV only.
- **Security:**
  - KV state is sensitive; LMCache must enforce per‑tenant isolation.
- **Licensing:** acceptable.
- **Complexity:** HIGH.
- **DecentraAI fit:** MEDIUM.
- **Evidence:** EXPERIMENTAL (routing; LMCache core VERIFIED).

---

## 6. COLLABORATIVE_MODEL (tensor/pipeline parallel)

**Recommended backend:** vLLM tensor/pipeline parallelism, possibly backed by Ray on multi-node clusters; llama.cpp RPC for experimental GGML tensor split; TensorRT-LLM / DeepSpeed for advanced TP/PP on NVIDIA GPUs.[cite:183][cite:188][cite:57]

- **Exact project:**
  - vLLM distributed serving (`--tensor-parallel-size`, `--pipeline-parallel-size`, `--distributed-executor-backend`).[cite:183][cite:194]
  - Ray (multi-node executor backend).[cite:183][cite:188]
  - llama.cpp RPC (`ggml-rpc-server` + `llama-server --rpc --tensor-split`) — experimental.
  - TensorRT-LLM, DeepSpeed — heavier, TP/PP training/inference frameworks.
- **Runtime & platforms:**
  - vLLM: Python/CUDA, Ray cluster via Docker; multi-node cluster with matched environments.[cite:183][cite:188]
- **Heterogeneous workers:**
  - vLLM recommends hiding host heterogeneity via uniform Docker images; cluster expects consistent GPU types for predictable performance.[cite:183]
- **Same-model multi-worker:**
  - Yes; this is the main point: shards model across GPUs/nodes via TP/PP.[cite:183][cite:57]
- **Production vs experimental:**
  - vLLM TP/PP is production-ready for homogeneous clusters.[cite:183][cite:57]
  - Heterogeneous consumer fabric (mixed laptop/desktop GPUs) is EXPERIMENTAL.
- **API/CLI:** vLLM `serve` CLI with `--tensor-parallel-size`, `--pipeline-parallel-size`, and `--distributed-executor-backend` (mp/ray).[cite:194][cite:183]
- **DecentraAI worker changes:**
  - Workers become frontends to a vLLM cluster; DecentraAI sees the cluster as a single logical engine.
- **Adapter viability:**
  - YES: DecentraAI can treat the vLLM TP/PP cluster as one worker; collaborative model splitting happens inside vLLM.
- **Measurements:**
  - Throughput, latency, network overhead, resource utilization across GPUs/nodes.
- **Fallback:** SingleWorker without TP/PP (e.g. local llama-server or single-node vLLM).
- **Security:** cluster must be within trust boundary; Ray/vLLM communications must be secured.
- **Licensing:** vLLM (Apache 2.0), Ray (Apache 2.0), TensorRT/DeepSpeed have their own licenses.
- **Complexity:** HIGH.
- **DecentraAI fit:** MEDIUM (cluster is opaque logical worker; fabric decides whether to use it).
- **Evidence:** VERIFIED for homogeneous clusters; EXPERIMENTAL for heterogeneous consumer devices.

---

## 7. MULTI_MODEL_PIPELINE

**Recommended backend:** vLLM/SGLang staged flows with DecentraAI orchestrating multi-model pipelines; external workflow engines (Ray DAGs, etc.) for complex pipelines.

- **Exact project:** vLLM, SGLang, Ray.
- **Runtime & platforms:** as above.
- **Heterogeneous workers:**
  - Supported at fabric level: different workers run different pipeline stages (e.g. OCR → summarization → code generation), each backed by its own model/backend.
- **Same-model multi-worker:**
  - Not required; pipeline uses different models per stage.
- **Production vs experimental:**
  - Single-stage vLLM/SGLang serving is production-ready; DecentraAI multi-model pipeline orchestration remains EXPERIMENTAL.
- **API/CLI:** OpenAI-compatible endpoints for each model; Ray or internal orchestration may manage stage-to-stage flows.
- **DecentraAI worker changes:**
  - None; fabric already supports intent→capability→model matching.[cite:166]
- **Adapter viability:**
  - YES: DecentraAI can orchestrate stages, each backed by vLLM/SGLang/llama.cpp via adapters; pipeline remains in control plane.
- **Measurements:**
  - End-to-end latency, per-stage latency, error propagation.
- **Fallback:** SingleWorker or simpler pipeline (fewer stages).
- **Security:** data flow across stages must obey trust/policy boundaries.
- **Licensing:** same as underlying engines.
- **Complexity:** MEDIUM–HIGH (depending on pipeline depth).
- **DecentraAI fit:** HIGH (fabric already has capability and model matching).[cite:166]
- **Evidence:** EXPERIMENTAL (for multi-stage orchestration in DecentraAI).

---

## 8. REMOTE_EXECUTION

**Recommended backend:** existing DecentraAI distributed path (P2P + llama-server) plus vLLM/SGLang running on remote nodes.

- **Exact project:** DecentraAI’s own `DistributedInference` + remote llama-server / vLLM / SGLang.
- **Runtime & platforms:** laptop i5 + desktop i7; remote GPU node with chosen backend.[cite:170]
- **Heterogeneous workers:**
  - Supported: different nodes can have different backends (llama.cpp on laptop, vLLM on desktop).
- **Same-model multi-worker:**
  - Not required; remote execution simply runs the whole request on a remote worker.
- **Production vs experimental:**
  - Remote execution with llama-server is already part of distributed inference; remote execution via vLLM/SGLang is an extension.
- **API/CLI:** P2P `InferRequest` → worker → backend HTTP.[cite:166]
- **DecentraAI worker changes:**
  - Workers gain backend type configuration (llama-server vs vLLM/SGLang).
- **Adapter viability:**
  - YES: remote execution is DecentraAI’s existing model; backends remain engines.
- **Measurements:**
  - RTT, throughput, latency across LAN; worker capacity metrics.
- **Fallback:** SingleWorker local.
- **Security:** remote workers must be trusted and opt into remote inference; identity and policy enforced.[cite:166]
- **Licensing:** as per chosen backend.
- **Complexity:** MEDIUM.
- **DecentraAI fit:** HIGH.
- **Evidence:** VERIFIED (for current llama-server route); EXPERIMENTAL (for vLLM/SGLang on remote nodes).

---

## 9. LOCAL_PRIVATE_EXECUTION

**Recommended backend:** llama.cpp or vLLM/SGLang running strictly on local node; DecentraAI treats this node as private, never routing to remote workers.

- **Exact project:** llama.cpp, vLLM, SGLang.
- **Runtime & platforms:** local machine (Laptop or Desktop), local HTTP loopback.
- **Heterogeneous workers:**
  - Not applicable; only local worker participates.
- **Same-model multi-worker:**
  - Not applicable; SingleWorker local.
- **Production vs experimental:**
  - Production-ready; this is the safest starting point.
- **API/CLI:** local OpenAI-compatible HTTP, no P2P.
- **DecentraAI worker changes:**
  - None beyond configuration; planner enforces `LocalPrivateExecution` policy.
- **Adapter viability:**
  - YES: DecentraAI runs local-only engine via existing adapters.
- **Measurements:**
  - Local throughput and latency.
- **Fallback:** not needed; this is already the fallback.
- **Security:** strong privacy; prompts never leave local machine.
- **Licensing:** as per backend.
- **Complexity:** LOW.
- **DecentraAI fit:** HIGH.
- **Evidence:** VERIFIED.

---

## Backend fit questions

### 1. Best backend for current i5 laptop + i7 desktop

- **Best fit:** llama.cpp (CPU-friendly, GGUF, flexible), plus **vLLM or SGLang on the i7 desktop** for heavier models.[cite:195][cite:184][cite:192]
- **Why:** laptop can run small GGUF models or act as control plane; desktop can host GPU-heavy vLLM/SGLang servers.

### 2. Best backend for future 3–5 heterogeneous consumer nodes

- **Best fit:** vLLM cluster with Ray backend (for homogeneous GPU pool) plus LMCache and SGLang for KV-aware and speculative experiments.[cite:183][cite:188][cite:185][cite:190]
- **Why:** vLLM TP/PP is well-documented for multi-node clusters; LMCache & SGLang extend capabilities across engines.

### 3. Easiest backend to experiment with

- **Easiest:** vLLM and SGLang: pip/uv install, simple `serve` / `launch_server` commands, OpenAI-compatible API.[cite:183][cite:184][cite:192]

### 4. Safest backend to integrate first

- **Safest:** vLLM or SGLang for SingleWorker + BatchFanOut on a single GPU node; llama.cpp remains the baseline.[cite:195][cite:184]
- **Reason:** open-source, widely used, clear OpenAI-compatible interface; easy to confine within one node.

### 5. Backend to avoid for now

- **Avoid/Delay:** full TensorRT-LLM/DeepSpeed multi-node TP/PP integration and heavily heterogeneous vLLM clusters; complexity and operational overhead are HIGH, and DecentraAI’s fabric is not yet tuned for these.[cite:57][cite:183]

---

## Top 3 backend integrations for DecentraAI

### 1. vLLM SingleWorker + BatchFanOut

- **Why:**
  - OpenAI-compatible API server (`vllm serve`) with strong performance.[cite:183][cite:195]
  - Tensor/pipeline parallelism available later without changing DecentraAI’s control plane.[cite:183][cite:194]
- **Requirements:**
  - Python 3.10+, CUDA-enabled GPU, stable vLLM version (e.g. 0.15.1).[cite:188]
- **First experiment:**
  - Replace local llama-server with vLLM on Desktop as backend for SingleWorker, then integrate BatchFanOut to multiple vLLM workers.
- **Success criteria:**
  - Equal or better TTFT and tok/s vs llama.cpp; stable latency; no regressions in failure/recovery behavior.
- **Fallback:**
  - Switch back to local llama-server SingleWorker.

### 2. SGLang SingleWorker + Speculative/Advanced Scheduling (experimental)

- **Why:**
  - High-performance serving with OpenAI-compatible endpoints; advanced scheduling and speculative decoding designed for distributed clusters.[cite:184][cite:192]
- **Requirements:**
  - Python 3.10+, CUDA GPUs, SGLang + kernel installed.
- **First experiment:**
  - Integrate DecentraAI worker adapter to SGLang’s OpenAI API as SingleWorker; later enable SGLang-specific speculative features on a Desktop node.
- **Success criteria:**
  - Improved throughput/latency on desktop; ability to experiment with speculative draft/verify while keeping DecentraAI as control plane.
- **Fallback:**
  - Use vLLM/llama.cpp backends; disable speculative path.

### 3. LMCache + vLLM/SGLang (KV-aware experiments)

- **Why:**
  - LMCache provides KV cache offloading, reuse, and prefill–decode disaggregation across vLLM/SGLang engines with documented production deployments.[cite:185][cite:186][cite:190][cite:189][cite:193]
- **Requirements:**
  - LMCache installed with vLLM/SGLang; storage backends for KV (CPU, disk, remote).
- **First experiment:**
  - Deploy LMCache with vLLM on a single node; measure TTFT improvements and KV reuse for long-context workloads.
- **Success criteria:**
  - Verified TTFT reduction, cache hit ratios, stable behavior; no data leaks across tenants.
- **Fallback:**
  - Use vLLM/SGLang without LMCache; revert to DecentraAI’s own KV-aware single-node logic.

Each of these integrations preserves DecentraAI as the **control plane & planner** while treating backends as opaque engines and workers as adapters. Multi‑worker strategies (speculative, PD disaggregation, collaborative model) are layered on top of these backends and remain **explicitly experimental** until measurements on the DecentraAI fabric prove net benefit.
