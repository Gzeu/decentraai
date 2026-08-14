# Changelog

All notable changes to DecentraAI. Adheres loosely to
[Keep a Changelog](https://keepachangelog.com/). The workspace ships as a
single version (`1.0.0`) shared by every crate.

## [1.0.0] - Initial production release

DecentraAI is a decentralized P2P network for sharing GGUF models and serving
verifiable inference through an external llama.cpp `llama-server`, with a live
web dashboard. This release marks the M0–M24 foundation, the P1–P5
subscription/invite model, the Q1–Q4 onboarding/ops work, the full M10
security + control-plane hardening, and stable parallel test gates.

### Universal product flow
- `decentraai node` — one background process: LAN/P2P discovery, verified
  auto-share, model serving and the embedded dashboard. Every node is both
  coordinator and worker. No manual topology.
- `decentraai setup` / `decentraai init` / `decentraai open` / `doctor
  [--online]` / `config validate` / `registry scan|list` / `swarm start` /
  `serve start --backend` / `pull` / `trust` / `distributed` /
  `p2p-invoke` (CLI).
- Verified-transfer pipeline: BLAKE3 chunking, Merkle-root gate, atomic
  rename, quarantine, resume.

### Subscriptions & invites (P1–P5)
- Hashed token registry with tiers (Guest/Contributor/Core), per-tier model
  allowlists + rate limits, per-token usage and audit.
- `invite [--ttl <min>]` + `join` for least-privilege seats; expiry and
  revocation.
- `tier suggest` / `tier apply` — contribution-suggested tier promotions.

### Compute sharing & distributed inference (M11–M20, M24)
- Capability-aware scheduling with reservations (RAM/VRAM), on-demand model
  provisioning, network-aware + KV-aware planner, live `/v1/metrics`-style
  compute/network/execution views, resilient fabric (reaper, TTLs, recovery,
  false-ready prevention, bounded idempotent retry, bounded P2P reconnect).

### Security & control plane (M10, P1–P5 hardening)
- Signed/verified inference requests (anti-spoof to the authenticated peer),
  signed compute advertisements, replay protection, per-peer and per-token rate
  limiting, role separation (admin/operator/client), invite expiry.
- Per-request audit events, machine-readable error codes, `/metrics`
  (Prometheus text), OpenAPI `/openapi.json`, structured JSON logs with
  request correlation.
- Dashboard: Model, Inference, Chat (streaming + stop + retry + model
  selector), Queue, Recent, System; advanced Workers (approve/revoke) / Network
  / Execution / Models / Settings / Diagnostics / Admin (tokens + roles +
  audit) views — all from real runtime state.

### Foundations kept honest
- M21 (distributed MoE) / M22 (multi-engine) / M23 (autonomous planner) remain
  **foundation-only**: engineered with safe, gated increments (expert-split
  guard, engine-kind selection + capability probe, planner configurable
  weights + decision modules) but not claimed production-verified — no engine
  advertises the gating capabilities yet.

### How to run
```bash
docker compose -f deploy/docker-compose.yml up --build -d   # container
# or native:
decentraai node --config ~/.decentraai/node.yaml
open http://127.0.0.1:8080/
```

### Developer gates (must pass)
```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
The test suite (200+ tests incl. two-node libp2p E2E) is stable in parallel.