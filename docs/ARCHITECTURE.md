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

### Initial module boundaries
```text
crates/config             configuration and schema validation
crates/identity           Ed25519 keys and peer identity
crates/system-probe       CPU, RAM, GPU, VRAM, disk, network probing
crates/policy-engine      profile calculation and resource governor
crates/protocol           message schemas, versioning, validation
crates/p2p                libp2p, mDNS, private bootstrap, transport
crates/manifest           canonical manifest, signature, Merkle verification
crates/chunk-store        staging, verified cache, quarantine, atomic writes
crates/transfer-engine    scheduling, resume, retries, peer selection
crates/registry           local model and transfer state
crates/inference-runtime  runtime supervisor and llama-server adapter
crates/inference-router   request queue, streaming, auth, cancellation
crates/reputation         local observations and temporary bans
crates/api                localhost HTTP/WebSocket API
```
