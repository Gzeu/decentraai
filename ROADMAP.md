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

## 5. Hardening (done)
- [x] Quarantine workflow for corrupted artifacts (metadata + reason)
- [x] RAM admission fix (reject below the configured reserve)
- [x] Security audit log (bans, admission rejections, verification failures)
- [x] M8: packaging — `scripts/install.sh` + `docs/deployment.md`
  (systemd unit, firewall, security checklist, troubleshooting)

## 6. Sharing and UX (done)
- [x] M7a: peer catalog + `decentraai pull` (share models with one command)
- [x] M7b: web dashboard on the API port
- [x] M7c: dashboard v2 — real inference metrics (tokens, tok/s, recent
  calls, uptime, RAM/GPU), self-poll fix (watching the page no longer
  inflates the counter or blocks idle unload)

## 7. Subscriptions: free, tiered by contribution (in progress)
- [x] P1: token registry (`db/tokens.json`, hashed) + `decentraai token`
  CLI + tiered auth in the proxy (per-tier model allowlist, sliding-window
  rate limit, usage counters, audits)
- [ ] P2: chat UI in the dashboard (model selector filtered by tier)
- [ ] P3: admin dashboard (create/revoke tokens, usage per token)
- [x] P4: contribution-based tier suggestions from catalog + reputation
- [ ] P5: invites (`decentraai join <invite>`)

## 8. Operations and scale (in progress)
- [x] Q1: generation defaults (sampling + system prompt merged into
  requests), interactive model picker with memory-fit verdicts,
  dashboard lists every indexed model
- [x] Q2: fair FIFO queue for inference requests — one request at a
  time reaches the backend with full resources, 503/504 on full/timeout,
  Queue card on the dashboard shows serving + waiting live
- [ ] Q3: remote backend (`serve start --backend http://host:port`) —
  a weaker station keeps auth/tiers/queue while a stronger machine runs
  the model
- [x] Q4: onboarding wizard (`decentraai setup`) writing a validated
  config on first run

## 9. Distributed Inference (M9) - IN PROGRESS
- [x] M9-1: Design distributed inference architecture (worker discovery, request routing)
- [x] M9-2: Implement worker registration with real-time capacity reporting
- [x] M9-3: Add request routing to peer GPUs based on capacity
- [x] M9-4: Implement fallback mechanism when workers fail
- [x] M9-5: Queue management with FIFO processing and timeout support
- [x] M9-6: P2P protocol extensions for WorkerAnnouncement and InferRequest
- [x] M9-7: Inference request handler for workers (real llama-server adapter)
- [x] M9-8: Real-time capacity updates from runtime
- [ ] M9-9: Reputation-based compensation for workers

## 10. Zero-Touch Swarm Sharing (DONE)

- [x] P6-1: `sharing` config section (`mode: auto | ask | off`,
  `max_concurrent_downloads`) with validation and defaults (`auto`)
- [x] P6-2: p2p manifest-announcement callback (peer, manifest), invoked
  by the swarm event loop without blocking it
- [x] P6-3: `swarm start` auto-share worker: downloads announced models
  with full verification (per-chunk BLAKE3 + Merkle gate), registers them
  in the local registry, and re-announces them signed by the node identity
- [x] P6-4: E2E tests: announcement fires the callback; announced model
  is auto-downloaded and verified byte-for-byte

With mDNS discovery already auto-dialing LAN peers, two nodes that both
run `decentraai swarm start` now exchange models with zero manual steps.

## 12. Compute Sharing (M11–M13, DONE)

DecentraAI's core product is people sharing **compute/GPU capacity**, not
just model files. These milestones make the swarm capability-aware: nodes
advertise real hardware, the coordinator answers "which node runs this
workload?" with hardware matching + resource reservations, and model files
stay a *supporting* artifact served by nodes that already hold them.

- [x] M11: `decentraai-compute` crate (pure, no I/O, serde-serializable):
  `WorkerCapability` (GPU/VRAM/RAM/CPU/engine/served models),
  `ComputeAvailability`, `ComputeAdvertisement`, `WorkloadRequirements`,
  `ResourceReservation`/`ReservationLedger` (TTL + per-worker cap),
  `CapabilityMatcher` (trust, health, model, RAM/VRAM headroom, load,
  queue, reservation cap), `ComputeRegistry` (stale → offline),
  `ComputeScheduler` (deterministic scoring + reservation booking)
- [x] M12: `ComputeManager` in `decentraai-distributed`: builds the local
  advertisement from the real system probe (`SystemSnapshot` +
  `nvidia-smi`), processes inbound advertisements via `DistributedP2PHandler`,
  and keeps a coordinator-side registry. `decentraai distributed start`
  advertises real hardware and re-broadcasts it on the heartbeat interval
- [x] M13: capability-aware routing — `DistributedInference.route_request`
  selects through the compute scheduler (model + RAM/VRAM matching, booking
  a reservation held for the request duration, always released afterwards)
  and falls back to the legacy announcement-based router

### M11–M13 verification notes

All three milestones were verified **live** on the LAN testbed
(coordinator + worker on `192.168.1.129`):

- Real worker boots `llama-server` and logs
  `registered as distributed compute worker` with its probe-derived
  `ComputeAdvertisement` (models hash `d28cd…`, CPU-only — no GPU present)
- The coordinator trusts the worker (`decentraai trust add`) and the live
  streamed request logs
  `capability-aware scheduler selected worker … reservation_id=…`, then
  completes real inference: `--- done (tokens=8 elapsed_ms=2036 …)`
- Two-node E2E tests in `crates/distributed/tests/compute_e2e.rs` cover
  advertisement propagation → selection → reservation → release, and the
  fallback to the legacy router when the compute-selected worker is down

Issues found and fixed during verification:

- **Trust was never writable at runtime**: nothing populated `trust.db`,
  so the compute scheduler rejected every worker with `NotTrusted`. Added
  `decentraai trust add|list|remove` (`crates/node-cli`), backed by
  `TrustStore`. Also fixed a latent `TrustStore` type bug: `add_trust` bound
  numeric fields as strings while reads expected `String`, so records were
  silently dropped by SQLite's column affinity — now typed binds/reads with a
  round-trip test.
- **Fallback was dead code**: a compute-path send failure returned the error
  directly instead of falling through to the legacy router. Fixed in
  `route_request` / `route_request_streamed`; covered by the E2E fallback
  test.
- **Blocking lock inside async**: `get_stats_async` called `blocking_lock`
  accessors and would panic on a multithreaded runtime; switched to the async
  accessors (queue depth / TPS / latency now report real values).

### Next: M17 (agreed direction)
- M17: contribution-based tier recommendations driven by compute served
  (hardware × hours × verified requests)

### M17: Contribution-based tier suggestions — DONE

The subscription model ("your tier reflects your contribution") now has a
real measurement to hang on. A pure, I/O-free scoring engine in
`decentraai-compute` turns **compute served** into a suggested tier:

- `contribution.rs`: `ContributionProfile` (CPU cores, RAM, VRAM, online
  seconds, verified/failed requests) → `contribution_score()` =
  hardware × availability × verified-work, reliability-adjusted; and
  `suggest_tier()` → 1/2/3 mirroring the token crate's GUEST/CONTRIBUTOR/
  CORE. Policy thresholds are named constants; zero verified work always
  yields Guest.
- `ComputeManager` now keeps a per-worker **contribution ledger**: online
  hours accrue from the heartbeat gap between advertisements, and every
  routed request outcome is counted via `record_outcome(peer, ok)` from
  both `route_request` and `route_request_streamed`. No mocks — the ledger
  is fed by real routing traffic.
- `ComputeMetricsReport.contributions` exposes each worker's raw inputs +
  score + suggested tier, surfaced live on `/v1/compute`.
- `decentraai tier suggest` prints the persisted report (`db/
  contributions.json`, written best-effort at the advertisement interval)
  as a read-only table; it never mutates state.

Tests (all green, no mocks): pure scoring (verified-work gating, failure
demotion, tier monotonicity, empty profile) and manager-level accounting
(workings accrue and push the tier up).

### P4: Contribution-suggested tiers written to the token registry — DONE

M17 *measures* contribution and *suggests* a tier per worker; the subscription
model only fulfills its promise ("your tier reflects your contribution") when
those suggestions can become the tiers that actually gate the proxy. P4 bridges
headless measurement to enforced policy, with an explicit admin confirmation
step and a persistent audit trail.

- `decentraai-tokens::tiers`: pure, I/O-free `plan_tier_changes` maps each
  worker's `suggested_tier` to the **active token of the same name**
  (`token.name == node_name`), emitting only real changes, skipping revoked
  tokens / unknown names / out-of-range tiers, sorted deterministically so a
  dry-run byte-matches a later apply. `TokenStore::set_tier` reassigns an
  active token's tier atomically (tmp + sync + rename) and returns the previous
  tier for audit.
- `decentraai tier apply --config <c>`: connects the coordinator's persisted
  `db/contributions.json` (M17) to `db/tokens.json`. **Dry-run by default** —
  prints exactly which tokens would move (`name: tier X → Y`) without touching
  state; pass **`--yes`** to write them. Each actual reassignment records a
  `tier_changed` audit event `{name, tier}` in `logs/audit.jsonl`, and is
  idempotent (already-matching tokens are never rewritten).
- `decentraai tier suggest` remains purely read-only (the raw report); `tier
  apply` is the admin-confirm (`--dry-run`) or rule-auto-promote (`--yes`)
  step the roadmap called for.

Tests (all green): pairing by name, no-change when already at tier, skip
revoked/unknown/out-of-range, first-active-match across reissued names,
deterministic ordering, and registry persistence of `set_tier`.

### M15: Worker-side reservation enforcement — DONE

The worker enforces its own reservation ledger so that a request exceeding
the currently advertised free capacity is rejected on arrival rather than
over-committing resources. This closes the gap where the coordinator's
placement decisions and the worker's actual bookings could drift.

- `ReservationLedger` is shared worker-side; each in-flight request books
  capacity and is released on a terminal event (success, backend error, or
  cancellation).
- A request that would exceed free capacity is answered with `InferFailed`
  (`retryable = true`), so the coordinator falls back to a different worker.
- Key updates now also update the worker's advertised `queue_depth`, keeping
  advertisements honest for subsequent placement decisions.
- E2E: `worker_rejects_request_exceeding_advertised_capacity` proves the
  rejection + coordinator fallback path.

### M16: Live compute metrics — DONE

The coordinator now exposes a live, serde-friendly snapshot of the whole mesh
from *real* inference traffic, not synthetic probes. Advertisements carry
measured throughput/latency so scheduling weights real performance.

- `RuntimeMetrics` (atomics, lock-free): an EWMA of tokens/sec and latency
  smoothed from each completed request, plus live `queue_depth` and lifetime
  totals. Written by the worker's streaming task and the queue path.
- `build_advertisement` now embeds a `LivePerf` snapshot, so each periodic
  advertisement reflects measured throughput/latency/queue load; the scheduler
  can weigh those alongside raw capacity.
- `ComputeManager::metrics_report` builds a coordinator-side view: every
  worker's load, queue, tokens/sec, latency, free capacity, current in-flight
  count and reserved RAM.
- `decentraai distributed start --metrics-port <P>` serves
  `GET /v1/compute` with that JSON on `127.0.0.1` (loopback only; never
  leaks capacity over the LAN).
- Tests: EWMA throughput/latency tracking, perf-in-advertisement, and
  `metrics_report` reflecting the registry + reservations.

### M14: On-demand model provisioning — DONE

The worker auto-downloads a requested model through the existing
verified-transfer pipeline when it does not already hold it, then serves the
request. Coordinators hold the model (e.g. after a `pull`) and serve
manifests + chunks via `RegistryServer`; policy gates both sides.

- **Policy**: new `sharing.provision_models_on_demand` config (default
  `true`). `ComputeCapability` gains `can_provision`; the scheduler only
  routes to provisioning-capable workers for unserved models when
  `ComputeManager::set_allow_provisioning(true)`.
- **Worker flow**: on a model mismatch with provisioning enabled, the worker
  replies `InferAccepted` immediately, sends an empty keepalive progress
  frame (so the coordinator's request clock keeps running during the
  download), then `transfer::download` → registry index → engine load →
  streamed completion. Failures send a terminal `InferFailed`.
- **Engine lifecycle**: a `ProvisioningFactory` (`Arc<dyn Fn(PathBuf) ->
  BoxFuture<(Box<dyn Any + Send>, OpenAiCompatibleBackend)>`) loads the
  downloaded model; the engine handle stays alive in a per-node
  `ProvisionedBackends` map and is dropped with the node. In the CLI the
  factory spawns a real `llama-server` per model.
- **Fresh-node indexing fix**: a node with no registry file previously never
  indexed provisioned models (`load` failed silently). Provisioning now
  creates the registry (and its parent dir) on first download.
- **Scheduling**: `requirements_for` returns `Some` (with a default 1024 MiB
  RAM budget) whenever only provisioning-capable workers can serve, so the
  workload stays schedulable; the real footprint is re-advertised after the
  download completes.

E2E coverage (`on_demand_provisioning_downloads_verifies_and_serves`):
coordinator serves a real GGUF file via `ChainedHandler(distributed +
RegistryServer)`; the worker advertises `can_provision`, is trusted, and
provisions the requested (different) model — asserting byte-for-byte BLAKE3
of the downloaded file, registry indexing, streamed output, and reservation
release.

## 13. Complete End-to-End Flow (M10)

### Acceptance Criteria
- [ ] Node starts with `decentraai init` and validated configuration
- [ ] Worker is paired via QR or token and approved from dashboard
- [x] Worker publishes models, capacity and real-time status
- [x] Client sends prompt (CLI `distributed --prompt`, `decentraai-p2p-invoke`)
- [x] Router selects eligible worker
- [x] Request transmitted via authenticated P2P
- [x] Worker calls real llama-server (not mock handler)
- [x] Streaming response to client, with cancellation via `InferCancel`
- [x] Timeout, retry and fallback work correctly
- [ ] Queue depth, latency, P50/P95/P99 and success rate in dashboard
- [ ] Each request produces audit event with request ID, worker ID, model hash, status
- [ ] Offline worker detected and excluded from routing
- [ ] E2E test can start two local nodes and reproduce full flow

### Implementation Phases

#### Phase 1: Common Contracts
- [ ] Stabilize NodeConfig, WorkerAnnouncement, InferRequest, InferResponse
- [ ] Stabilize WorkerStatus, TaskPlacement
- [ ] Define error codes and retry semantics
- [ ] Request lifecycle: received → queued → assigned → running → completed/failed/timeout
- [ ] Add mandatory request_id, trace_id, created_at, deadline_at, model_hash, sender_peer_id, assigned_worker_id

#### Phase 2: Real Data Plane
- [ ] Fix queue manager: shared state, not clone
- [ ] Install effective inference handler in ChainedHandler
- [ ] Adapter for llama-server/OpenAI-compatible API
- [ ] Streaming incremental tokens
- [ ] Backpressure and bounded queues
- [ ] Retry only for transient errors
- [ ] Circuit breaker for unstable workers
- [ ] Idempotency for resent requests
- [ ] Server-side limits for timeout, tokens, prompt size

#### Phase 3: Trust and Security
- [ ] Verify WorkerAnnouncement signature
- [ ] Compare announcement.peer_id with transport peer ID
- [ ] Pairing with expiration and revocation
- [ ] Replay protection via nonce/sequence number
- [ ] Capability-based authorization: which models each worker can serve
- [ ] Rate limiting per token and per peer
- [ ] Limit prompt size and output size
- [ ] Secret management without tokens in config or logs
- [ ] Audit for login, pairing, revoke, routing and inference
- [ ] Role separation: admin, operator, client, worker

#### Phase 4: Control Plane and Frontend
- [ ] Onboarding: create node, generate identity, pairing QR, health check
- [ ] Chat: conversations, streaming, stop generation, retry, model selection
- [ ] Workers: approve/revoke, status, models, capacity, latency
- [ ] Models: registry, hash, quantization, context size, availability
- [ ] Network: peers, trust, latency, connection errors
- [ ] Observability: logs, metrics, traces, alerts
- [ ] Admin: tokens, roles, quotas, audit events
- [ ] Settings: node config, inference defaults, limits, retention

### Scoring Rubric (Target: 9/10)

| Domain | Target for 9/10 |
|--------|-----------------|
| Core inference | Real request to llama-server, streaming and cancellation |
| Distributed routing | Worker selection, fallback, timeout and circuit breaker |
| Security | Verified identity end-to-end, pairing and authorization |
| Reliability | Restart recovery, bounded queues, idempotency and health checks |
| API | Versioned contracts, OpenAPI, consistent errors and rate limits |
| Frontend | Chat, workers, models, logs, metrics and admin integrated |
| Observability | Metrics, structured logs, audit and correlation IDs |
| Testing | Unit, integration, adversarial and two-node E2E |
| Deployment | Docker Compose for local and reproducible deployment |
| Documentation | Real quickstart, architecture diagrams, operations runbook |
| Developer experience | CI for Rust + frontend + security + E2E |

### Recommended PR Structure

1. **fix(M9)**: shared queue state and request lifecycle
2. **feat(inference)**: real llama-server adapter with streaming
3. **feat(security)**: authenticated worker announcements
4. **feat(api)**: node control plane and SSE inference endpoint
5. **feat(frontend)**: connect chat and worker management to live API
6. **feat(reliability)**: retries, fallback, circuit breakers and recovery
7. **test(e2e)**: two-node distributed inference scenario
8. **ci**: add frontend, security, integration and E2E gates
9. **docs**: production quickstart and operations runbook
10. **release**: M10 production vertical slice

---

## 14. Execution Fabric — M18 Foundation — DONE

A new pure crate, `decentraai-fabric`, holds DecentraAI's orchestration
intelligence — the engine-neutral execution-fabric core that turns a request +
live fabric state into an `ExecutionPlan`, integrated into the real
coordinator path. It is deliberately engine-agnostic (DecentraAI is not a
marketplace, cloud, or wrapper around one model server) and never fabricates
behavior the runtime cannot provide.

**M18 — Distributed Execution Fabric Foundation — DONE**

Verified on real hardware/LAN (two physical Ubuntu machines, Desktop ↔
Laptop, both running the single universal `decentraai node`):

- [x] Multi-node universal runtime (every node is coordinator + worker)
- [x] P2P discovery / connectivity (mDNS + libp2p on the LAN)
- [x] Trusted worker admission (`decentraai trust add` → scheduler selects only
  trusted, eligible workers)
- [x] Planner-based worker selection (`plan_and_reserve` → `reserve_worker`)
- [x] Reservation lifecycle (created per request, released after completion)
- [x] Real remote inference (coordinator → P2P `InferRequest` → remote worker →
  worker's local llama-server on loopback → streamed `InferProgress` →
  terminal `InferResponse`)
- [x] Streaming responses and cooperative cancellation
- [x] Concurrent requests (separate request IDs, non-colliding reservations,
  worker capacity respected)
- [x] Worker reuse (same healthy worker selected again across requests)
- [x] Bidirectional Desktop ↔ Laptop execution
- [x] Persistent worker engine lifecycle: the worker's local llama-server stays
  bound to loopback and is **not** idle-unloaded, so the worker never
  advertises ready while its engine is dead (`5758e05`).
- [x] One `decentraai node` process — no separate `decentraai distributed`
  required for the product flow.

The fabric crate supplies the building blocks (`ExecutionPlan`,
`ComputeScheduler::reserve_worker`, an `ExecutionPlanner`, a `NetworkGraph`, a
`KvPlanner`, an `ExpertRegistry`/`ExpertRouter` capability gate). **The
single-worker reservation/streaming path is production-verified, and the
execution planner now weighs real network cost (M19) when it selects a worker.**
Model/layer splitting, KV reuse, MoE distribution and multi-worker execution
planning are still the milestones below.

## 15. M19: Network-Aware Scheduler — DONE

The execution planner no longer ranks workers in isolation: it reads a live
inter-node link graph and folds reach cost into its score. The coordinator
measures real round-trip latency by pinging every known *remote* worker over
the P2P request/response channel (`InferPing`/`InferPong`) every 5s, and writes
the measured RTT into the graph the planner reads. The local node is never
pinged (libp2p refuses self-dial).

- [x] latency — real RTT measured via `InferPing`/`InferPong` over the P2P
  request/response channel; `spawn_network_probe` times each round trip and
  feeds `ComputeManager::record_rtt` into the coordinator's `NetworkGraph`.
- [x] bandwidth — per-peer `bandwidth_mbps` in `LinkMetrics`, with soft priors
  per locality and a measured field; feeds the transfer-cost estimator.
- [x] topology / connection quality — `Locality` (Local / SameHost / Lan /
  Remote) with prior RTT + bandwidth, folded into link scoring.
- [x] transfer cost — deterministic `transfer_ms_per_mib` estimator and
  `reach_cost_ms` (RTT + transfer time) consulted by the planner.
- [x] worker load / capacity — `load_percent`, `queue_depth`, RAM/VRAM headroom
  are first-class scoring terms in the planner.
- [x] dynamic worker scoring — `ExecutionPlanner::score` combines throughput,
  latency, load, queue, headroom, network reach cost and KV headroom; ranks
  deterministically (score desc, PeerId asc) with deterministic fallbacks.
- [x] preserves trust → planner → reservation → P2P → worker path (M18);
  the loopback-backend guarantee is untouched (no llama-server LAN exposure);
  no mocks on the production path.

Related commit: `c5d2b44` (plus the probe wiring in `node-cli`).

## 16. NEXT — M20: KV-Aware Inference Fabric — NOT STARTED

KV-aware placement is the next milestone after the M19 network-aware
scheduler. The fabric crate already contains the pure KV building blocks
(`KvPlanner`, `ContextProfile`, `KVCacheState`); wiring them into the real
coordinator route path and validating on real Desktop ↔ Laptop hardware is
left to M20. **M20 implementation is not claimed complete.**

Scope (explicit, checked off as M20 lands):
- [ ] KV locality — prefer a worker that already holds the conversation's
      cache for this model (continuation affinity).
- [ ] context / KV state — the planner sees each worker's real cache
      occupancy (headroom in tokens) for the requested model + context.
- [ ] continuation affinity — route a multi-turn continuation to the worker
      that already has the prefix resident, not to a cold one.
- [ ] KV headroom — only place a request on a worker whose cache can
      accommodate the full context budget.
- [ ] long-context placement — weight placement so long prompts/decode land
      where KV capacity + bandwidth (M19) allow it.
- [ ] prefill / decode considerations — prefill-heavy (prompt) vs
      decode-heavy (generation) requests steered to suitable workers.

## 17. NEXT — M21: Distributed MoE / Expert Fabric

## 18. NEXT — M22: Multi-Engine Runtime Abstraction

## 19. NEXT — M23: Autonomous Execution Planner

## 20. NEXT — M24: Resilient Distributed Fabric

- [ ] lifecycle
- [ ] failure detection
- [ ] recovery
- [ ] worker replacement

## 21. Ubuntu UX — Q4 setup wizard — DONE

- [x] `decentraai setup` — one-command fresh-node onboarding: auto-detects
  hardware (CPU/RAM/GPU via the real system probe), generates or reuses an
  identity (0600), auto-detects a GGUF model, writes a validated config, and
  prints readiness. No manual path/worker/port/topology tuning.
- [x] Verified end-to-end: `setup` → `config validate` → `doctor` → boots a
  real libp2p distributed node off the generated config. Idempotent across
  reruns (same PeerId), model-discovery works, no-model degrades gracefully.

## 22. Installable Application (Productization) — DONE

Turn DecentraAI into an installable app; the normal user flow is
**Download → Install → Open → Ready**, hiding distributed-compute complexity.

- [x] `decentraai node` — a single background daemon that auto-provisions
  identity + config (reusing `setup`), brings up LAN/P2P discovery + verified
  auto-share, auto-detects + serves a model, and binds the dashboard/API
  immediately (control plane is up even while the model loads or faults).
  Shuts down cleanly on SIGINT/SIGTERM; the target executable for systemd.
- [x] `decentraai open` — opens the running dashboard in the default browser.
- [x] `RuntimeConfig.port` — fixed backend port so the product node can target
  the model backend deterministically before it is ready.
- [x] systemd user unit + installer + uninstaller + `.desktop` launcher +
  auto-start on boot/reboot (user lingering).
