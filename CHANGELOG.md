# Changelog

All notable changes to DecentraAI. Adheres loosely to
[Keep a Changelog](https://keepachangelog.com/). The workspace ships as a
single version (`1.0.0`) shared by every crate.

## [Unreleased] — Multi-node fabric identity

### Multi-node fabric experience (real Desktop ↔ Laptop identity)
- **`allow_remote_inference` is now real, not dead config**: the setting is
  enforced as a worker-side inbound gate (remote `InferRequest`s to a
  node that did not opt in are rejected with a terminal, non-retryable
  error), advertised in every `ComputeAdvertisement`
  (`accepts_remote_inference`, `#[serde(default)]` → old peers deserialize
  to `false`, the conservative choice) and enforced coordinator-side by the
  `CapabilityMatcher` (a worker that did not opt in is `NotAcceptingRemote`
  and never selected for remote work; the local peer is always eligible).
- **Per-node identity reaches the API**: `/v1/network` now returns the
  real LAN addresses of every connected peer (`addresses`) plus the node's
  own listen addresses (`local_addresses`) from a new p2p `Peers` snapshot;
  `/v1/compute` worker rows carry real static identity + resources
  (CPU cores, total RAM, GPU name/VRAM, engine, served models with KV
  context, `last_seen_secs`, `accepts_remote_inference`) sourced from the
  advertised capability — never invented.
- **The dashboard shows the distributed fabric from the node's own
  perspective**: a "Fabric nodes" strip renders the local node plus every
  discovered worker as identity cards (name, peer, LAN address, engine,
  load/RAM/VRAM bars, served-model chips) with a live trust chain
  DISCOVERED → UNTRUSTED → APPROVED → CONNECTED → WORKER READY.
- **Discovery events are real**: a client-side diff of the worker registry
  surfaces discovered / offline / reconnected events (with a canvas pulse
  on the affected node) — no fake animations, only real transitions.
- **Workers view is identity-first**: the registry table became per-node
  cards with the same identity/resources/trust-chain rendering plus the
  real trust action buttons (master-gated Approve/Revoke).
- The fabric stage and pipeline now label nodes honestly: `● connected` /
  `○ not connected`, `REMOTE-OK` / `local-only` (from the advertised opt-in),
  real LAN addresses under each node, a RAM-free resource ring, and the
  WORKER pipeline stage names the executing node (`local` / `remote`).
- All identity data is read-only from `/status`, `/v1/compute`,
  `/v1/network`; the page still never mutates state except on explicit
  intent.

## [Unreleased] — Living Fabric UI

### Living Fabric (Overview redesign, "DecentraAI = a living distributed AI fabric")
- The Overview is no longer primarily a grid of statistics: the primary
  element is a live **Canvas 2D fabric stage** (single-binary constraint
  kept — no external assets, no CDN, `requestAnimationFrame` engine).
- The stage renders the fabric as living entities: the local node at the
  center, every advertised worker (Laptop/Desktop/GPU) as an entity with
  real status color, load arc, trust and live labels, connected by
  measured P2P links (M19 RTT) drawn as beziers.
- **Execution visibly travels**: particles flow only along genuinely live
  links; the pipeline strip (USER → REQUEST → PLANNER → RESERVATION →
  FABRIC → WORKER → ENGINE → STREAM → RESULT) lights up from real queue /
  recent-request / decision data. Idle is calm and atmospheric; a real
  request activates the planner, lights the selected worker, shows the
  reservation, streams tokens and returns to calm on completion.
- **The M23 planner has a visible identity**: a planner chip shows
  idle/classifying/routing/recovering state, and a decision strip renders
  safe operational facts only — CLASSIFYING → N CANDIDATES → NETWORK COST →
  KV AFFINITY → ENGINE CAPABILITY → SELECTED WORKER → EXECUTING — with no
  chain-of-thought exposed.
- **Recovery is part of the story**: when real M24 recovery events exist
  (restart/recover/evict/replan), the mode flips to `recovering`, the
  affected worker changes color, the planner chip reacts and a recovery
  pulse ring appears on the stage.
- Topology view uses the same canvas engine on a larger stage (SVG diagram
  replaced). All other views (Chat, Decisions, Execution, Workers, Network,
  Models, Observability, Recovery, Diag, Security, Settings) are untouched
  functionally and remain fed by real runtime state.
- Invariant preserved: the page still only polls read-only control
  endpoints; a chat POST or admin action happens exclusively on explicit
  user intent.

## [Unreleased] — Command Deck UI

### Command Deck (embedded control plane, rewritten)
- The embedded dashboard is rewritten as a full **Command Deck** in a new
  `crates/runtime/src/dashboard.rs` module (single-binary constraint kept:
  pure HTML+CSS+JS served by the node, no external assets or build step).
- 13 views on a sidebar rail: Overview, Chat, Topology (live fabric),
  Autonomous decisions (M23), Execution, Workers, Network, Models,
  Observability, Recovery, Diag, Security, Settings — all fed by real
  runtime state (`/status`, `/v1/peers`, `/v1/compute`, `/v1/network`,
  `/v1/execution`) with no mock data.
- **Topology** renders the live fabric as SVG: local node at the center,
  advertised workers around it, edges colored by measured RTT (M19),
  worker rings shaded by health/load, trusted badges.
- **Autonomous decisions** renders the M23 decision ring: workload class,
  candidate score breakdowns (tps/latency/load/queue/headroom/net/kv),
  constraint breaches, KV affinity, expected mode, reasoning and the
  safe-reasons trace.
- **Observability** adds latency/tok-per-s sparklines; **Recovery** surfaces
  engine respawns, KV sessions and resilience events; **Security** keeps the
  token create/list/revoke actions plus the audit event stream.
- **Settings** now renders real generation defaults (temperature/top_p/
  top_k/repeat_penalty/system prompt) and the subscription tier policies —
  `/status` gains `generation` and `tiers` fields (tiers null when
  unconfigured), covered by two new tests.
- Command palette (Ctrl+K) jumps to any view; quick chat keeps SSE streaming
  with abort/retry; advanced views stay behind the opt-in advanced block
  (the dashboard never polls proxied inference endpoints, so watching the
  page still cannot inflate counters or reset the idle clock).
- The old inline `DASHBOARD_HTML` / `JS_TEMPLATE` constants in `api.rs` are
  removed; the module is wired as `crate::dashboard`.

### Fixes
- `decentraai-fabric`: `ExecutionPhase` used an invalid serde rule
  (`rename_all = "uppercase"` → `"UPPERCASE"`), which broke the workspace
  build; fixed.
- `decentraai-fabric`: `observe_appends_events_to_the_decision_trace` test
  called `evaluate` with five arguments after the planner parameter was
  added; now passes `&ExecutionPlanner::default()`.

## [Unreleased] — M23 Increment B (decision core into the live fabric)

### M23 Full Autonomy, Increment B (live decision + lifecycle observability)
- `decentraai-fabric::decision` is now wired into the actual coordinator path,
  not just an exported module: `ComputeManager::record_decision` builds an
  explainable `ExecutionDecision` (workload class, candidates, hard constraints,
  per-candidate score + network cost, selected worker, KV affinity, engine
  capability, expected mode) per routed request, using the **same live
  planner/network/KV state** as `plan_and_reserve`.
- `ComputeManager::finalize_decision` correlates each decision with its real
  `reservation_id`, `plan_id` and observed `outcome`, and appends the
  Reserved → Executing → Completed/Failed → Released lifecycle trace.
- `decentraai_fabric::decision::adapt` (OBSERVE → ADAPT) now drives the real
  retry/replan decision in `route_request` with the live remaining
  eligible-worker count (`ComputeManager::eligible_worker_count`), preserving
  M24 idempotency-safe retry semantics (never after output, never on definitive
  rejection/cancellation).
- Control plane: `/v1/execution` now also returns the bounded (cap 64)
  `decisions` ring; dashboard adds an "Autonomous decisions (M23 lifecycle)"
  view rendering workload, selected worker, priority, network cost, KV affinity,
  reservation, outcome and the safe-reasons trace.
- Honest scope: this is the decision core *integrated into the live execution
  lifecycle*. Self-healing, mid-request multi-objective re-planning and
  proactive rebalancing (true M23 Full Autonomy) remain not-claimed.
- Full honest status (what is operational vs foundation): see
  [`docs/autonomous-execution.md`](docs/autonomous-execution.md).

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
- M21 (distributed MoE) / M22 (multi-engine) remain **foundation-only**:
  engineered with safe, gated increments (expert-split guard, engine-kind
  selection + capability probe) but not claimed production-verified — no engine
  advertises the gating capabilities yet.
- M23 (autonomous planner) is **partial**: the decision core is now integrated
  into the live execution lifecycle (Increment B), but true full autonomy
  (self-healing, mid-request multi-objective re-planning, proactive
  rebalancing) is **not** claimed.

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