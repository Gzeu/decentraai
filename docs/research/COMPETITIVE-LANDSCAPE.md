# Competitive Landscape: Distributed and Decentralized Inference

Status: MIXED (VERIFIED for documented projects and papers; INFERRED when comparing with DecentraAI).[cite:133][cite:137][cite:138][cite:146][cite:147][cite:148][cite:151][cite:152][cite:155][cite:157][cite:159][cite:161]

This document summarizes the most relevant projects in distributed, P2P, and community compute inference.

## 1. Engine-Centric Distributed Inference

### 1.1 vLLM, LMCache, llm-d, NVIDIA Dynamo

- Focus: production-grade serving, disaggregated prefill/decode, distributed KV cache, RDMA-aware scheduling.
- Strengths:
  - Mature integration with LMCache for prefix caching and KV offloading.[cite:133][cite:137][cite:138]
  - Disaggregated serving via PrefillRouter and NIXL.[cite:110][cite:117][cite:146]
  - Topology-aware placement for multi-host replicas.[cite:143][cite:146]
- Limitations for DecentraAI’s goals:
  - Primarily datacenter-focused; assume RDMA or high-bandwidth fabrics.
  - Less emphasis on P2P heterogeneous consumer networks.

### 1.2 SGLang + Shepherd/SMG

- Focus: high-throughput LLM serving, PD disaggregation, RadixAttention, speculative decoding.
- Strengths:
  - PD disaggregation and KV transfer via bootstrap rooms and RDMA.[cite:107][cite:111][cite:114]
  - RadixAttention-based prefix caching and cache-aware routing.[cite:116][cite:114]
  - Speculative decoding support via EAGLE and DFlash.[cite:119][cite:120][cite:121]

## 2. P2P and Community Compute Fabrics

### 2.1 CrowdLlama, HyperCluster, Mesh LLM, Meshcore

- CrowdLlama: P2P network around Ollama; DHT-based discovery; collaborative inference prototypes.[cite:148]
- HyperCluster: P2P model sharding prototype using Iroh; focuses on community AI and local collectives.[cite:151]
- Mesh LLM: auto-configured P2P inference cloud to pool spare compute; supports private models and shared resources.[cite:152]
- Meshcore: conceptual DePIN architecture for P2P LLM inference; emphasizes task-level distribution and cryptographic fairness (slashing, TEEs, ZK proofs).[cite:156]

Strengths:

- Strong P2P and community orientation.
- Exploration of DePIN and verifiable computation.

Limitations:

- Many are early-stage or conceptual.
- Often distribute tasks (prompts) rather than model layers; limited fine-grained execution strategies.

### 2.2 GPU Marketplaces and DePIN Protocols

- Lium (Bittensor subnet), Depinfer, Prime Intellect, ShareAI, Node AI, GNUS, Krako, etc., operate GPU marketplaces or decentralized compute networks.[cite:150][cite:157][cite:159][cite:161]

Strengths:

- Economic and marketplace mechanisms for GPU sharing.
- Scale across many GPUs.

Limitations:

- Focus on training and coarse-grained inference; less on heterogeneous execution strategies for single tasks.

## 3. What Competitors Do Better Than DecentraAI

- Production-ready disaggregated serving and KV caching with LMCache and RDMA.[cite:133][cite:137][cite:138][cite:110][cite:117]
- High-scale datacenter deployments and Kubernetes integration.[cite:146][cite:143]
- Proven speculative decoding integrations with detailed metrics.[cite:118][cite:121][cite:131]
- Marketplace economics and GPU resource markets.[cite:159][cite:160][cite:161]

## 4. What DecentraAI Already Does Differently

- P2P fabric with built-in trust/auth, provenance, and quota accounting.[cite:93]
- Engine-neutral control plane focused on execution strategies and heterogeneous fabrics.
- Consumer hardware orientation and MCP-based agent access.

## 5. Lessons for DecentraAI

- Learn from LMCache and disaggregated serving for KV and PD strategies.[cite:133][cite:138][cite:110][cite:117]
- Emulate prefix-cache-aware routing and cache affinity scoring.[cite:112][cite:139][cite:147]
- Adopt speculative decoding integration with acceptance metrics.[cite:118][cite:127][cite:131]

## 6. What DecentraAI Should Not Copy Yet

- Heavy RDMA-centric designs that assume datacenter fabrics.
- Blockchain-heavy core scheduling; use chain-based systems only where verifiable accounting is needed.
- Overly complex ML schedulers without clear transparency.

DecentraAI’s niche lies in being the adaptive control plane for heterogeneous, partly P2P fabrics, leveraging best-of-breed engines rather than competing as yet another single-engine inference server.
