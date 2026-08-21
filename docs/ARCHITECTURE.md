# Architecture

## Product boundary
A DecentraAI node is a local daemon. It can act as a client, verified model seeder, local inference host, or private remote-inference worker. The initial network is private-first: LAN discovery and explicit bootstrap peers precede any public DHT deployment.

## Planes

### Control plane
Owns configuration, Ed25519 node identity, system capabilities, policies, scheduling, model registry, local reputation, and telemetry.

### Data plane
Owns libp2p transport, peer discovery, manifest exchange, chunk transfer, request authentication, rate limiting, retries, and peer scoring.

### Inference plane
Runs separately from networking. A runtime supervisor starts and observes llama-server or llama.cpp, imposes resource limits, streams tokens, supports cancellation, and restarts crashed workers without exposing node keys.

## Node lifecycle
```text
BOOT
→ validate configuration
→ load or generate node identity
→ probe CPU/RAM/GPU/VRAM/disk/network
→ derive resource profile and policy limits
→ reconcile model cache, partial downloads, and quarantine
→ start local API and inference supervisor
→ start LAN/private P2P transport
→ publish signed capability announcement
→ READY
```

## Content-addressed model storage
1. Split a model into 4 MiB chunks by default.
2. Compute BLAKE3 for each chunk while streaming from disk.
3. Build a Merkle root over ordered chunk hashes.
4. Canonicalize and sign a manifest with an approved publisher key.
5. Store chunks only after individual hash verification.
6. Assemble the final artifact atomically only after full verification.
7. Keep incomplete or invalid artifacts in staging/quarantine, never in the verified cache.

## Remote inference
Remote inference in v1 assigns the complete model to one worker. The client sends an authenticated request; the worker enforces policy and streams tokens. Transport encryption protects data in transit, but the worker can inspect prompts while executing them. Sensitive prompts should use local inference or a private trusted swarm.

## State machines

### Model state
```text
DISCOVERED → MANIFEST_PENDING → MANIFEST_VERIFIED → DOWNLOADING
→ PARTIALLY_VERIFIED → ASSEMBLING → COMPLETE_VERIFIED → LOADED
```

Terminal or exceptional states: `QUARANTINED`, `CORRUPTED`, `EVICTED`.

### Actual module boundaries (workspace crates)
```text
crates/config             typed YAML config with strict validation
crates/identity           Ed25519 keys and peer identity
crates/system-probe       CPU, RAM, GPU, VRAM, disk, network probing
crates/compute            pure compute-sharing domain (capability, scheduler,
                          reservations, contribution, credits, placement, graphs)
crates/protocol           message schemas, versioning, validation
crates/p2p                libp2p, mDNS, private bootstrap, transport
crates/manifest           canonical manifest, signature, Merkle verification
crates/registry           local model registry with path safety
crates/fabric             pure execution planner (network graph, KV, expert routing)
crates/distributed        compute manager, router, worker, agents, knowledge
crates/runtime            llama-server process manager, API, dashboard, tools, TTS
crates/agents             pure collective-intelligence fabric (agents, memory,
                          verification, policy, knowledge, evidence, benchmark)
crates/hub                HuggingFace Hub catalog + verified download
crates/inference-adapter  engine HTTP client adapter
crates/providers          external OpenAI-compatible provider plane
crates/tokens             subscription tokens + tiers
crates/audit              append-only security log
crates/discovery          pairing codes
crates/node-cli           the `decentraai` / `decentraai-worker` binaries
```

> Note: earlier drafts named crates `policy-engine`, `chunk-store`,
> `transfer-engine`, `inference-runtime`, `inference-router`, `reputation`,
> and `api`. Those responsibilities live inside the crates above — no such
> standalone crates exist in this workspace.
