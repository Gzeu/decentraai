# Distributed KV Cache for DecentraAI

Status: MIXED (VERIFIED for LMCache, vLLM prefix caching, SGLang RadixAttention, ShadowServe, and decentralized routing; INFERRED for DecentraAI integration).[cite:105][cite:106][cite:112][cite:115][cite:133][cite:138][cite:147]

This document explores how DecentraAI can evolve beyond per-worker KV/session affinity toward distributed KV caching, KV transfer, and cache-aware routing.

## 1. KV Cache Reuse Fundamentals (VERIFIED)

### 1.1 Prefix Caching

- KV tensors for shared token prefixes need only be computed once; subsequent requests sharing the prefix can reuse the KV cache.[cite:105]
- vLLM’s Automatic Prefix Caching (APC) and SGLang’s RadixAttention implement hash-based or tree-based prefix caching across requests.[cite:105][cite:112][cite:116]
- Prefix caching drastically reduces TTFT for long prompts (system prompts, shared documents, RAG contexts).[cite:112]

### 1.2 Cross-Replica and Distributed Prefix Caching

- Distributed prefix caching extends reuse across nodes:
  - A canonical KV cache for a popular prefix resides on one node; other nodes pull it over the network (RDMA, NVLink, Ethernet) rather than recomputing.[cite:106][cite:115][cite:144]
- vLLM and LMCache create a **global view** of KV cache blocks across pods, using indices that map token prefixes to physical KV blocks.[cite:112][cite:138]

## 2. LMCache: Distributed KV Cache Layer (VERIFIED)

### 2.1 Architecture

- LMCache is an LLM serving engine extension that:
  - Extracts KV caches from engines like vLLM and SGLang.
  - Stores them in tiered storage (GPU, CPU, NVMe, S3, Redis).
  - Shares caches across engines and queries.[cite:133][cite:138][cite:140]

### 2.2 Integration Modes

- In-process connector (`LMCacheConnectorV1`):
  - Runs inside vLLM process; supports CPU offloading and basic prefix reuse.[cite:137]
- Multi-process connector (`LMCacheMPConnector`):
  - LMCache runs as standalone server; multiple vLLM instances connect to it.
  - Enables distributed KV storage and sharing across nodes.[cite:137][cite:144]

### 2.3 Capabilities

- Cache offloading and persistence across requests and restarts.
- Distributed KV cache sharing across nodes over Ethernet/RDMA/NVLink.[cite:144]
- Cache events and affinity metrics for routing (KV cache events published via ZMQ in LMCache, consumed by schedulers).[cite:136]

## 3. Decentralized Prefix-Cache-Aware Routing (VERIFIED)

The paper "Towards Distributed Inference of LLMs on a P2P Network" proposes decentralized routing based on prefix cache overlap:[cite:147][cite:109]

- Each node maintains a local radix tree of its cached prefixes.
- Nodes exchange approximate cache summaries via periodic anti-entropy.
- Requests are routed to the node with the longest estimated prefix match.
- No KV-cache transfer is required; stale metadata causes misses, not incorrect outputs.

This approach is particularly interesting for a P2P fabric like DecentraAI, where centralized KV stores may be impractical.

## 4. KV State Classification for DecentraAI (INFERRED)

DecentraAI can classify KV state per request/session:

- **LOCAL**: KV cache exists only on the worker that computed it; continuations should prefer this worker (current behavior).
- **REPLICATED**: KV cache has been copied to multiple workers or stored in a shared store with low-latency access.
- **TRANSFERABLE**: KV cache can be moved between workers as part of an execution strategy (e.g., disaggregated prefill/decode, failure recovery).
- **REMOTE**: KV cache exists in a remote LMCache-like store; workers fetch it on demand.
- **UNKNOWN**: no KV state recorded; routing cannot exploit cache.

Each KV state would carry provenance:

- **VERIFIED**: confirmed via LMCache/vLLM/SGLang metrics.
- **MEASURED**: network transfer times and cache hit statistics collected by DecentraAI.
- **INFERRED**: estimated from context length and engine docs.
- **EXPERIMENTAL**: PoC implementations.
- **UNKNOWN**: default when no data is available.

## 5. Cache-Aware Routing Strategy (INFERRED)

Add `ExecutionStrategy::CacheAwareRoute` that:

- For new requests:
  - Queries per-worker prefix coverage scores (from LMCache or local indices).[cite:112][cite:139]
  - Selects the worker with highest prefix affinity subject to network and trust constraints.
- For continuations:
  - Prefers the worker owning the KV cache (LOCAL or REPLICATED) but may migrate KV when moving execution offers net benefit.

### 5.1 KV Migration

KV migration (TRANSFERABLE state) becomes an option when:

- The current KV owner is overloaded or degraded (high latency, thermal throttling).
- Another worker has better hardware/network characteristics.

KV migration cost = KV size × network transfer time; planner should only migrate when expected benefit (improved decode throughput, lower tail latency) outweighs transfer cost.

### 5.2 P2P vs Centralized KV

- **Centralized LMCache-like KV**:
  - Pros: simpler integration, global view, strong TTFT improvements.[cite:133][cite:144]
  - Cons: introduces central point of failure/trust; may not fit fully decentralized goals.

- **P2P prefix-aware routing**:
  - Pros: aligns with DecentraAI’s P2P vision; no central KV store.[cite:147]
  - Cons: more complex metadata dissemination; benefits sensitive to network latency and skewed prefix distributions.[cite:147]

DecentraAI can support both modes in different deployment profiles (e.g., local cluster vs wide-area P2P).

## 6. Security and Privacy Considerations (INFERRED)

KV cache encodes prompt content and intermediate model states; distributing or persisting it raises confidentiality concerns:[cite:133][cite:144][cite:142]

- KV sharing and offloading should respect trust boundaries:
  - Only share KV within trust groups or TEEs.
  - Encrypt KV at rest and in transit where possible.
- Cache-aware routing must avoid sending sensitive prompts to untrusted nodes solely for cache benefits.

DecentraAI’s trust and policy system can classify KV segments by sensitivity and control which strategies may use shared or remote KV.

## 7. Recommendations (GO / EXPERIMENT / WAIT)

- **GO NOW**:
  - Extend WorkerFacts to include cache-affinity metrics where available (from LMCache or local prefix indices).
  - Introduce KVState classification for sessions and requests.

- **EXPERIMENT FIRST**:
  - Integrate LMCache with a vLLM backend in a trusted cluster to measure TTFT and throughput wins from prefix caching and distributed KV sharing.
  - Prototype P2P prefix-cache-aware routing (without KV transfer) in a small DecentraAI swarm, using radix trees and anti-entropy gossip.

- **WAIT**:
  - Full global KV persistence and sharing across untrusted nodes; requires stronger confidentiality and verifiability mechanisms.

Distributed KV cache and cache-aware routing are key components of DecentraAI’s next-gen fabric, tightly coupled with disaggregated serving and execution strategies.
