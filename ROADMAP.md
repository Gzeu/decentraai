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

## 3. Runtime (done)
- [x] M4a: llama-server process manager crate (`decentraai-runtime`)
- [x] M4b: inference admission gate + `decentraai serve start`
- [x] M4c: OpenAI-compatible API endpoint with Bearer auth and idle unload
- [x] Fixed configurable API port + friendly root info page

## 4. Swarm Intelligence (done)
- [x] Reputation and peer scoring (M5a: bans, decay, persistence)
- [x] Manifest announcements + registry-backed serving (M5b)
- [x] Deterministic multi-provider scheduler (M5c: ranked waves + fallback)

## 5. Hardening
- [ ] Quarantine workflow for corrupted artifacts
- [ ] Audit logging and metrics
- [ ] Packaging and deployment guide
