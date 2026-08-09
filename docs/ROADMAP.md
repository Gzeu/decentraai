# Roadmap

## Phase 0 — Local verified core
- Rust workspace, configuration schema, structured logging.
- Ed25519 identity and secure local key storage abstraction.
- System probe for CPU, RAM, disk, GPU/VRAM where available.
- Streaming chunking, BLAKE3 hashes, Merkle root, signed manifest.
- SQLite state database; staging, verified cache, and quarantine directories.
- Unit tests for manifest validation, chunk corruption, atomic writes, and interrupted transfers.

Exit: a local GGUF file can be chunked, published, reassembled, and verified without reading the full file into memory.

## Phase 1 — LAN swarm
- libp2p transport, mDNS discovery, authenticated handshake.
- Manifest discovery and chunk exchange between two nodes.
- Concurrent transfer scheduler, resume support, per-peer score, and invalid-chunk penalties.
- CLI: init, doctor, scan, publish, search, download, verify, status.

Exit: two LAN nodes transfer a model, resume an interrupted download, and reject a deliberately corrupted chunk.

## Phase 2 — Local inference
- Isolated llama-server adapter and process supervisor.
- Safe local API bound to localhost, OpenAI-compatible routes, streaming and cancellation.
- Model-load policy based on detected resources.
- Metrics for TTFT, tokens/s, VRAM, RAM, queue depth, and errors.

## Phase 3 — Private remote inference
- Signed capability announcements.
- Private swarm allowlist and explicit worker consent.
- Authenticated requests, queue limits, cancellation, streaming, worker selection, local reputation.

## Phase 4 — Internet hardening
- Configurable bootstrap nodes, relay support, NAT diagnostics, optional DHT.
- Adversarial tests for replay, flood, invalid manifests/signatures, peer churn, and recovery.

## Phase 5 — Research
- Pipeline parallelism only in low-latency trusted clusters.
- Placement optimizer based on latency, VRAM, availability, and throughput.
- Off-chain credits only after abuse-resistant measurement exists.
