# DecentraAI MVP Roadmap

## 1. Foundation (done)
- [x] Workspace, CI, and YAML config loader
- [x] `decentraai init` bootstrap
- [x] System probe + admission checks (`decentraai doctor`)
- [x] Model registry scan/list commands

## 2. Identity and Networking (done)
- [x] Ed25519 identity management with secure persistence
- [x] Message schema and canonical signing
- [x] Model manifest generation with Merkle root
- [x] libp2p transport with Noise + mDNS
- [x] Chunk transfer engine with per-chunk verification and resume
- [x] End-to-end test: two nodes exchange a real model

## 3. Runtime (M4 — in progress)
- [x] M4a: llama-server process manager crate (`decentraai-runtime`)
- [ ] M4b: inference admission control wired to the system probe
- [ ] M4c: local OpenAI-compatible API endpoint + `decentraai serve`

## 4. Swarm Intelligence
- [ ] Deterministic task scheduler
- [ ] Reputation and peer scoring
- [ ] Multi-provider downloads (swarm fetching)

## 5. Hardening
- [ ] Quarantine workflow for corrupted artifacts
- [ ] Audit logging and metrics
- [ ] Packaging and deployment guide
