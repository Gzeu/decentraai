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
- [x] P2: chat UI in the single embedded dashboard (non-streaming `/v1/chat/completions`, real worker routing); tier-filtered model selector folded into the single-dashboard approach — chat uses the node's active model and shows what it is
- [x] P3: admin actions in the single embedded dashboard (`/admin` + `/api/admin/token/*` create/revoke tokens, usage per token)
- [x] P6: dashboard views — Models, Settings, Diagnostics, Execution, Workers, Network (advanced) + normal-user view (Model, Inference, Chat, Queue, Recent, System), all from real runtime state, no mock data
- [x] P7: Command Deck UI — the embedded dashboard rewritten as a 13-view
  control plane (Overview, Chat, Topology, Autonomous decisions, Execution,
  Workers, Network, Models, Observability, Recovery, Diag, Security,
  Settings) with a sidebar rail, command palette (Ctrl+K), live fabric
  topology SVG, M23 decision traces, and Settings rendering the real
  generation defaults + tier policies from `/status`
- [x] P8: Living Fabric UI — the Overview becomes the visual identity of
  DecentraAI: a Canvas 2D fabric stage renders the local node and every
  advertised worker as living entities with real P2P links (M19 RTT),
  execution visibly travels through a USER→REQUEST→PLANNER→RESERVATION→
  FABRIC→WORKER→ENGINE→STREAM→RESULT pipeline (real queue/decision data
  only), the M23 planner has a visible identity with a safe-facts decision
  strip (classifying/candidates/network cost/KV affinity/engine/selected/
  executing, no chain-of-thought), and M24 recovery becomes part of the
  story (affected worker changes state, replan pulse). Metrics/tables stay
  but are secondary; same single-binary embedded dashboard, same 13 views.
- [x] P9: Multi-node fabric identity — `inference.allow_remote_inference`
  enforced end-to-end (worker-side inbound gate + advertised
  `accepts_remote_inference` + coordinator `NotAcceptingRemote` matcher;
  local peer always eligible); real LAN addresses per connected peer plus
  own listen addresses surfaced via a new p2p `Peers` snapshot
  (`/v1/network.addresses` + `local_addresses`); `/v1/compute` workers
  carry real static identity/resources (CPU, RAM, GPU, engine, served
  models with KV context, last seen, remote opt-in); the dashboard shows
  the fabric from the node's own perspective — Fabric nodes identity cards
  with a live trust chain (DISCOVERED→UNTRUSTED→APPROVED→CONNECTED→WORKER
  READY), a real discovery event feed (discovered/offline/reconnected), a
  named WORKER pipeline stage (`local`/`remote`), and an identity-first
  Workers view (per-node cards, master-gated Approve/Revoke). All data
  read-only from `/status`, `/v1/compute`, `/v1/network` — nothing faked.
- [x] P10: Node identity = compact ID — a fresh node's default name is its
  own `dca-xxxxxx` indicator (derived from the identity at `setup` time, no
  manual naming); every node advertises that stable id (`node_id` in the
  advertisement, `/v1/compute` and `/status`; dashboard shows it on canvas
nodes, Fabric nodes cards and Workers cards, with a client-side fallback
   for pre-`node_id` peers). `setup --name` stays as an optional semantic
   label.
- [x] P11: Fabric chat routing — the dashboard `/v1/chat/completions` proxy
  routes a chat request to a *trusted remote worker* that advertises the
  requested model (P2P `InferRequest`, SSE + non-streaming), and tags every
  inference response with `X-Decentra-Origin`/`X-Decentra-Worker`/
  `X-Decentra-Node`. The chat shows a "served by `dca-xxxx` · remote" badge
  and the model selector gains a "Remote workers" group from live
  `/v1/compute` data. Local models always win; remote routing never holds a
  local queue slot.
- [x] P12: Model picker for the whole fabric — the chat selector is rebuilt
  as `Auto (best available)` (default), `Local models` and `Remote workers`
  (every advertised remote model, labelled with its node, even when a local
  copy exists). Auto picks the largest model actually *served* anywhere in
  the fabric (honest, deterministic: size desc, local wins ties, node id
  asc), rewriting the local body when it lands locally; a manual remote
  choice sends `worker_hint: <node_id>` and the proxy routes to exactly that
  node (400 with a clear message if it is not trusted / not accepting remote
  inference / does not serve the model).
  - Regression fix `6e92ffe` (2026-08-18): the selector silently showed only
    local models — `populateChatNodes`/`populateChatModels` ran with `c=null`
    before `/v1/compute` was fetched in `refresh()`, so the `Remote workers`
    optgroup never appeared. Calls moved after the compute fetch; pinned by
    `dashboard_populates_chat_models_after_compute_fetch`.
- [x] P13: chat can speak — local text-to-speech behind the
  dashboard chat. The node runs a managed Python `tts_server.py` subprocess
  (external engine, never FFI — same invariant as llama.cpp) driven from
  `<data_dir>/tts/` (venv + voice files, set up by `scripts/setup-tts.sh`).
  Config section `tts: {enabled, voice, speed}`; absent section = TTS off and
  the chat hides the speak control. `/v1/tts` proxies Bearer-authenticated
  requests to the loopback subprocess and returns 16-bit mono WAV (4096-char
  cap, tier rate limit, 401 without a token, 404 when disabled). `/status`
  reports `tts {enabled, healthy, voice, speed}`; the v2 chat adds a 🔊 speak
  button per assistant message with stop-on-reclick.
  - Engine v1 (commit `f07d0a6`): **Kokoro-82M ONNX** — excellent English,
    but Romanian is unsupported, producing broken-sounding speech.
  - Engine v2 (commit `…`): **Piper VITS** — Romanian native with correct
    diacritics (ă â î ș ț), non-autoregressive (zero hallucinations), CPU
    real-time (RTF ~1.2x). Voices: `ro_RO-raluca-high` (female, WER 2.2%,
    default), `ro_RO-lili-high` (female), `ro_RO-mihai-medium` (male).
    `piper-tts` 1.4+ is a single wheel with espeak-ng embedded — no sudo.
    Live-validated on `dca-GriBWu`: 200 audio/wav with the master token, 401
    without; 12/12 v2 polling contracts unchanged; 974 tests green.
- [x] P4: contribution-based tier suggestions from catalog + reputation
- [x] P5: invites (`decentraai invite` prints a copy-pastable
  `<reachable-multiaddr>/p2p/<libp2p-peer-id> <guest-token>` string;
  `decentraai join "<invite>"` parses it, auto-provisions identity + config,
  stores the Tier-1 Guest token as the node's credential
  (`runtime/invite.token`, 0600) and verifies the coordinating peer is
  reachable over the verified P2P path). The invite uses the **libp2p peer id**
  (base58 `12D3KooW…`) derived from the node key, never the raw identity hex —
  a libp2p multiaddr cannot parse the hex id (fix `b644278`). Live-validated
  end-to-end on the LAN fabric: fresh data dir → `join` → "connected to the
  coordinating peer"; regression-pinned by `invite_peer_id_is_libp2p_not_identity_hex`.

## 8. Operations and scale (in progress)
- [x] Q1: generation defaults (sampling + system prompt merged into
  requests), interactive model picker with memory-fit verdicts,
  dashboard lists every indexed model
- [x] Q2: fair FIFO queue for inference requests — one request at a
  time reaches the backend with full resources, 503/504 on full/timeout,
  Queue card on the dashboard shows serving + waiting live
- [x] Q3: remote backend (`serve start --backend http://host:port`) —
  a weaker station keeps auth/tiers/queue/dashboard while a stronger machine
  runs the model. No local llama-server is spawned or probed; the proxy falls
  back to `state.backend_url` (the remote) when the manager is unloaded, and
  the local node surfaces 503 for an unreachable backend (`remote_backend_started`
  audit event)
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
- [x] M9-9: Reputation-based compensation for workers — **live ledger**
  (`decentraai-compute::compensation`: `reward_tokens` pure policy +
  `CompensationLedger`): a deterministic, synthetic **contribution-credits**
  ledger (not a payment platform) = `verified_requests × rate`, scaled by
  contribution quality and a reputation term (clean-service ratio
  `verified/(verified+failed)`). Zero verified work or a complete-failure
  record earns 0. The ledger is **wired into the live coordinator**:
  `ComputeManager::record_credited_contribution` credits it (idempotent by
  `ref_id`, exactly once per execution, profile frozen at credit time so each
  credit is explainable) on every verified completion; failures and replays
  earn nothing. Surfaced via the `get_compensation` MCP tool, the
  `ContributionRow.compensation_earned` column in `/v1/compute`, and the
  dashboard Workers view. A worker whose verified-transfer reputation is bad
  is already banned and never routed work, so its earnings are zero
  regardless — the reputation axis rewards how *cleanly* served work
  completed.

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

## 11. HuggingFace Hub catalog + on-demand download (DONE)

The swarm is no longer limited to models that already exist on a peer:
operators can search the HuggingFace Hub and pull verified GGUF artifacts
straight into the local registry.

- [x] `decentraai-hub` crate (no external engine deps): `HfRef` reference
  parsing (`hf:org/repo` or `hf:org/repo:file.gguf`), `HubCatalog` search
  (filter=gguf) + per-repo GGUF tree listing (sizes, LFS SHA-256), and
  verified download (`download_model`/`download_verified`: SHA-256 enforced
  from the Hub tree API, staged `.part` write + atomic rename, so no
  partial/corrupted artifact ever enters the registry)
- [x] `decentraai model search <query>` — list GGUF models from the Hub with
  their pipeline category/tool (`--category` filter, `--categories` to
  discover what tool types exist)
- [x] `decentraai model pull hf:org/repo[:file.gguf]` — download into
  `~/.decentraai/models` and refresh the registry; an unpinned repo auto-picks
  the largest GGUF (deterministic "best quantization" default, matching the
  fabric picker's size heuristic)
- [x] CLI parse tests for `model search`/`model pull`

Verified live against the production Hub API (search, tree, resolve/CDN
redirect) and an end-to-end Qwen2.5-0.5B download (auto-pick + pinned file,
sha256 verified, registry refreshed).

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
- [x] Node starts with `decentraai init` / `decentraai setup` and validated
  configuration (`config validate` + Q4 wizard + auto-provisioning `node`)
- [~] Worker is paired via token and approved (`decentraai invite`/`join`
  provides a least-privilege token seat, P5; `decentraai trust add` approves a
  worker for the capability scheduler — physical QR pairing is not implemented)
- [x] Worker publishes models, capacity and real-time status
- [x] Client sends prompt (CLI `distributed --prompt`, `decentraai-p2p-invoke`)
- [x] Router selects eligible worker
- [x] Request transmitted via authenticated P2P
- [x] Worker calls real llama-server (not mock handler)
- [x] Streaming response to client, with cancellation via `InferCancel`
- [x] Timeout, retry and fallback work correctly
- [x] Queue depth, latency, P50/P95/P99 and success rate in the dashboard —
  `/status` now exposes `latency_ms.{p50,p95,p99}`, `success_rate_percent` and
  `requests_failed` (from a ring buffer of real request durations + a live
  success/failure counter), rendered in the normal-user Inference card
- [x] Each routed request produces an audit event (`inference_completed` /
  `inference_failed`) with request ID, worker ID, model hash, trace id, session
  and status — written best-effort from `DistributedInference` into
  `logs/audit.jsonl` when the node sets a logs dir
- [x] Offline worker detected and excluded from routing (M24 reaper:
  stale-heartbeat flip-offline via `mark_offline`, reservation pruning, and
  worker eviction with audit after a grace window)
- [x] E2E test can start two local nodes and reproduce full flow
  (`crates/distributed/tests/compute_e2e.rs`: real libp2p nodes on loopback —
  advertisement propagation → selection → reservation → streamed inference →
  release, plus fallback and fake-worker re-provisioning)

### Implementation Phases

#### Phase 1: Common Contracts
- [x] Stabilize NodeConfig, WorkerAnnouncement, InferRequest, InferResponse — all
  four are stable serde contracts: NodeConfig is `deny_unknown_fields` with
  strict validation (config:16, validate ~282); WorkerAnnouncement/InferRequest/
  InferResponse are fully defined serde structs (infer_protocol.rs)
- [x] Stabilize WorkerStatus, TaskPlacement — WorkerStatus is wire-serializable;
  TaskPlacement is now serializable too (derive + round-trip test)
- [~] Define error codes and retry semantics — `DistributedError::is_retryable()`
  + bounded backoff is defined/tested (only transport retries; rejections/cancels
  never resend). Machine-readable numeric error *codes* are not present (only
  string + boolean `retryable`)
- [~] Request lifecycle received→queued→assigned→running→completed/failed/timeout —
  the pipeline is fully wired (accepted→queued→dequeue→stream→terminal); there is
  no distinct observable "assigned" transition and queued-timeout cleanup is
  requester-side only (`cleanup_timed_out` is defined but not swept on the worker)
- [~] Mandatory request_id, trace_id, created_at, deadline_at, model_hash,
  sender_peer_id, assigned_worker_id — 6 of 7 are mandatory fields on
  InferRequest; `assigned_worker_id` is absent (routing uses
  `TaskPlacement.selected_worker` instead), and `new()` seeds sender with a
  placeholder

#### Phase 2: Real Data Plane
- [x] Fix queue manager: shared state, not clone — `RequestQueueManager` is
  `Arc<Mutex<HashMap<PeerId, Arc<Mutex<WorkerRequestQueue>>>>>`, shared, not
  cloned-by-value (queue.rs:185)
- [x] Install effective inference handler — served via `P2PNode.on_infer`
  (`register_worker_backend`) rather than inside `ChainedHandler`; the chain
  dispatches announcements/compute, inference bypasses it (lib.rs:518-730)
- [x] Adapter for llama-server/OpenAI-compatible API — `OpenAiCompatibleBackend`
  (complete/stream against /v1/chat/completions, /v1/models, /health)
- [x] Streaming incremental tokens — `stream()` with `"stream":true` + SSE
  parse into `StreamChunk`; worker relays as `InferProgress`
- [~] Backpressure and bounded queues — per-worker queue depth is capped and
  queue-full is answered terminal, but the ingress channel is unbounded
  (`mpsc::unbounded_channel`) with no producer suspension
- [x] Retry only for transient errors — `is_retryable()` returns true only for
  P2PError/RequestTimeout; bounded backoff; tested (lib.rs:222,915)
- [x] Circuit breaker for unstable workers (P5: per-worker breaker trips after
  consecutive retryable failures; open workers are omitted from the planner feed
  and re-admitted after a cooldown)
- [x] Idempotency for resent requests — `ReplayGuard` (per-auth-peer nonce + TTL)
  rejects a resent request terminal before admission/queue/backend (replay.rs)
- [x] Server-side limits for timeout, tokens, prompt size — enforced in
  `inference-adapter::validate()` on both paths; the local /v1 proxy now also
  caps prompt bytes + max_tokens and applies a 300s HTTP timeout (runtime proxy
  hardening), and idle-clock/counters only fire on real inference POSTs

#### Phase 3: Trust and Security
- [~] Verify WorkerAnnouncement / compute advertisement signature — P3 signs
  compute advertisements, verified on receipt (the legacy unsigned path remains
  as a fallback). (WorkerAnnouncement legacy frame itself is not signed.)
- [x] Compare announcement.peer_id with transport peer ID — the on_infer path
  (P2) verifies against the authenticated connected peer; signed advertisements
  are keyed to the signer's public key mapping to the claimed peer (P3)
- [x] Pairing with expiration and revocation — `decentraai invite --ttl <min>`
  issues a Guest token that stops working after the TTL (tokens registry now
  stores an optional `expires_at`, `lookup`/`is_active` reject expired tokens);
  revoke was already supported
- [x] Replay protection via nonce/sequence number — P4: per-sender nonce +
  bounded TTL ReplayGuard on the worker; outbound monotonic nonces
- [x] Capability-based authorization: which models each worker can serve — the
  fabric planner only selects workers that serve (or can provision) the
  requested model (`fabric::planner` filters `trusted && healthy &&
  serves_model`, M13/M14/M18) and the worker independently refuses requests for
  models it does not hold; per-tier client model allowlists gate who may request
  each model (P1/H4)
- [~] Rate limiting per token and per peer — per-token (per-tier) sliding-window
  already existed (`crates/runtime`); **per-peer** added on the P2P worker path
  (`distributed::rate_limit::PeerRateLimiter`, keyed by the authenticated peer,
  `peer_rate_limited` audit)
- [x] Limit prompt size and output size — enforced in `inference-adapter`
  `validate()` (`max_prompt_bytes`/`max_output_tokens`) on both `generate` and
  `stream`, surfaced as a clear terminal `InferFailed` (`PromptTooLarge` /
  `OutputLimitExceeded`) by the worker
- [x] Secret management without tokens in config or logs (token registry stores
  hashes; signing keys live only in the node's identity; audit never logs them)
- [x] Audit for login, pairing, revoke, routing and inference — per-request
  `inference_completed`/`inference_failed` (M10) and `replay_rejected` (P4)
- [x] Role separation: admin, operator, client — tokens carry a `role`
  (client/operator; the master token is admin). Client tokens may only run
  inference within tier limits; operator tokens get read-only operational views
  (`/v1/compute`, `/v1/network`, `/v1/execution`) but not token management;
  the master token has everything. CLI `token create --role`, admin API `role`
  field, and enforced API gates (H4)

#### Phase 4: Control Plane and Frontend
- [x] Onboarding: create node + generate identity + health check —
  `decentraai setup` (data dirs, identity, validated config, READY) and
  `decentraai doctor [--online]` (admission + live reachability);
  pairing is token-based (`decentraai invite`/`join`, P5) — QR is intentionally
  not used
- [x] Chat: conversations, streaming, stop generation, retry, model selection —
  SSE streaming, in-page history (localStorage), plus a live model selector,
  a client-side Stop (AbortController) and Retry
- [x] Workers: approve/revoke, status, models, capacity, latency — the Workers
  view (real `/v1/compute`) shows status/load/queue/tok-s/latency plus reachable
  and connection_errors, and Approve/Revoke buttons hit master-gated
  `/api/admin/worker/{trust,revoke}`
- [~] Models: registry, hash, quantization, context size, availability —
  context/RAM/VRAM/availability shown; per-model hash and quantization are not
  surfaced in the view
- [x] Network: peers, trust, latency, connection errors — peers view (trusted/
  banned/scores), links view (RTT/BW/locality), connected list, and worker
  reachable/connection_errors
- [~] Observability: logs, metrics, traces, alerts — audit/security-events view,
  `/metrics` (Prometheus), latency percentiles/success-rate; no distributed
  traces and no alert channel
- [x] Admin: tokens, roles, quotas, audit events — token create includes a role
  selector, token list shows role, `/api/admin/events` shows recent audit
  events inline; quotas are not part of the (free) subscription model
- [~] Settings: node config, inference defaults, limits, retention — the
  Settings view shows node/resource config and limits read-only; it does not
  edit live settings and there is no data retention knob

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

## 16. M20: KV-Aware Inference Fabric — DONE

KV-aware placement builds on the M19 network-aware scheduler. The coordinator
now makes honest, KV-aware placement decisions from real advertised `n_ctx` and
tracked request/session state — no invented engine telemetry. Committed at
`caf9121`.

Scope:
- [x] KV locality / continuation affinity — coordinator-side session→worker
      residency (`SessionAccount`); a continuation with a known session is
      steered to the worker holding the KV prefix, with deterministic fallback
      when residency is unknown/stale. Wired into `RequestFacts`
      (`is_continuation`, `prefix_resident_on`).
- [x] context / KV state — `ServedModel.context_tokens` advertises a worker's
      real `n_ctx`; `fabric_facts` now reports each worker's `KVCacheState`
      (was hardcoded `Unknown`) from real capacity + accounted usage.
- [x] KV headroom — `KvPlanner` + `KVCacheState` headroom consumed in the
      planner; honest coordinator-side accounting (`record_session_usage`)
      derived from real `tokens_used` and advertised `n_ctx` — no invented
      telemetry.
- [x] long-context placement — per-worker KV occupancy feeds the planner's
      long-context handling; actual cross-worker steering depends on
      multi-worker contention and a worker advertising a real `n_ctx`.
- [~] prefill / decode considerations — gated behind
      `EngineCapabilities::prefill_decode_separation`, which no real engine
      advertises; llama-server stays conservative. **Not claimed as
      implemented** — the capability gate exists, the split does not run.

**Notes (not claimed):** live llama-server KV *occupancy* telemetry is not
exposed/consumed; the coordinator-side accounting described above is the
honest model. Cross-worker non-empty KV-headroom steering has not been observed
on the live link (the current worker advertises unknown capacity and there is
no multi-worker contention) — the logic is unit/integration-proven.

## 17. WIRED — M21: Distributed MoE / Expert Fabric (abstraction + planner integration)

`decentraai-fabric::expert` provides the honest abstraction: an
`ExpertRegistry` (which workers hold which experts), the pure `ExpertRouter`
(whole-model fallback vs expert split), and the `expert_routing` capability
gate. Every expert-aware decision is gated behind an engine advertising
`EngineCapabilities::expert_routing`. **No engine DecentraAI runs advertises
it** — so the router returns exactly the whole-model result (the single
correct answer for a monolithic model) and an `ExpertSplit` is never produced
in production. The planner wiring is live and regression-tested:
`ExecutionPlanner::build_stage` passes **all eligible candidates** to
`ExpertRouter.route` (fix `ae42e0a`), so a split is sound and reachable the
moment an engine advertises the capability; `expert_capable_worker_routes_to_expert_split`
pins the wiring and `non_expert_engine_keeps_honest_whole_model_reasoning`
guards the honest fallback. **Not marked done**: distributed MoE is not
production-verified on real hardware (no capable engine yet); this is
preparedness, but the routing path is reachable and tested, not dead.

## 18. WIRED — M22: Multi-Engine Runtime Abstraction (contract + live classification)

`decentraai-fabric::engine` defines `EngineKind` (llama-server / vLLM / SGLang /
Ollama / generic OpenAI-compatible) and the `EngineCapabilities` contract
(streaming, KV reporting, prefill/decode separation, expert routing, tensor
parallel). DecentraAI never embeds an engine; it drives an external process's
OpenAI-compatible HTTP API. Capabilities are conservative by default and
narrowed by a live probe. The classification is wired into the live fabric:
`ComputeManager::fabric_facts` parses each worker advertisement's engine string
via `EngineKind::parse` and feeds `WorkerFacts.engine` + `advertised_capabilities()`
to the execution planner, so scoring reflects the engine's real surface
(regression-pinned by `engine_kind_capabilities_drive_worker_facts`). **Only
llama-server is actually spawned today**, and `prefill_decode_separation` stays
`false` for it (that split is a parked idea, not a running feature). **Not
production-verified** for any second engine; this is the abstraction + live
classification layer only.

## 19. PARTIAL — M23: Autonomous Execution Planner (foundation + live decision core)

`decentraai-fabric::planner::ExecutionPlanner` folds real state into worker
scoring: throughput, latency, load, queue depth, RAM/VRAM headroom, network
reach cost (M19) and KV headroom (M20), ranked deterministically. It is deeply
integrated (the live node routes through `plan_and_reserve`).

**M23 Increment B/C** turns the planner's single-worker selection into an
explicit, observable decision in the real execution path, without replacing the
M18/M19/M20/M24 planner it extends (a cooperative base with the live decision
core, decision correlation, event-driven adapt and control-plane surfacing):

- `decentraai-fabric::decision` (`WorkloadClass`/`classify`, `ConstraintKind`/
  `ConstraintResult`, `CandidateOutcome`, `ExecutionDecision`, `ExecutionEvent`,
  `adapt`) is now **wired into the live coordinator**, not just an exported API.
- The coordinator records an explainable decision per routed request via
  `ComputeManager::record_decision` (DISCOVER → CLASSIFY → CANDIDATES →
  CONSTRAINTS → SCORE → SELECT), built with the **same live planner** the routing
  path uses — so scores and per-candidate network cost reflect the real measured
  network graph (M19) and KV state (M20), never a cold-default planner.
- Each decision is **correlated** with `reservation_id`, `plan_id` and the
  observed `outcome` via `ComputeManager::finalize_decision`, and carries a
  lifecycle trace (Reserved → Executing → Completed/Failed → Released). Safe
  operational metadata only — no chain-of-thought, no request content.
- `adapt()` (OBSERVE → ADAPT/RECOVER/REPLAN) now feeds the real retry path in
  `route_request` with the **actual** remaining eligible-worker count from the
  live registry (never a fabricated count), preserving the M24 idempotency-safe
  retry semantics.
- The control plane (`/v1/execution`) exposes the bounded, concurrency-safe
  decision ring (`ComputeManager::decisions`, newest-first, cap 64), rendered by
  the dashboard "Autonomous decisions" view (workload, selected worker, mode,
  priority, network cost, KV affinity, reservation, outcome, trace, safe reasons).

Fully autonomous goals — self-healing, multi-objective re-planning mid-request,
proactive rebalancing — are **not** claimed; the planner remains a capable
single-worker selector, not an autonomous orchestrator. This is `decentraai-fabric::decision`
integrated into the live execution lifecycle, not M23 Full Autonomy.

## 20. M24: Resilient Distributed Fabric — DONE

- [x] worker health monitoring + stale detection (coordinator reaper, heartbeat
      staleness, worker eviction with audit)
- [x] reservation timeout / expiry and release on completion/failure
- [x] graceful shutdown, mDNS discovery/recovery, startup recovery (systemd
      restart + identity/config auto-provision)
- [x] **false-ready prevention** — node admission: the compute broadcaster now
      gates worker advertisement on live engine health (TCP probe), so a node
      with a crashed engine never advertises a ready worker (7b22dbf)
- [x] **engine crash recovery** — the universal node's runtime auto-restarts a
      crashed llama-server from a stored restart spec via a 5s supervisor
      loop (`ServeManager::ensure_healthy`)
- [x] **bounded, idempotency-safe request-level retry** in the fabric route
      path — `route_request` retries transport-level failures (P2P/timeout) on
      a fresh planner-chosen worker up to `config.max_retries` with exponential
      backoff via `FallbackHandler`, releasing and re-planning per attempt;
      `DistributedError::is_retryable()` never re-sends a definitive worker
      rejection or a cancelled request (no duplicated non-idempotent work). The
      streaming path stays single-attempt + legacy fallback to avoid
      duplicating partial output to the client.
- [x] **explicit bounded P2P reconnect loop** — on `ConnectionClosed` the swarm
      re-dials a known-peers' last address with exponential backoff capped at
      `RECONNECT_MAX_ATTEMPTS`, then relies on mDNS re-discovery; addresses are
      captured at mDNS discovery and on dialer connect.

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

## 23. Next-Gen Phase 1 — Capability Requirements & Provenance Matching

The Hub already classifies models into a capability taxonomy with VERIFIED /
INFERRED provenance. What was missing was a way to *ask the Hub* "which models
can do X?" with an explainable, provenance-aware verdict instead of manually
naming a model. This is the foundation of the Next-Gen capability fabric,
intent planning, and the agent control plane's "which models can run this?"

- [x] `CapabilityRequirement` — a required capability plus a minimum evidence
  level (`EvidenceLevel::Verified` / `EvidenceLevel::Any`).
- [x] `match_requirements` / `satisfies_any` — pure, deterministic matching of a
  model's capabilities against a requirement set, returning a per-requirement
  checklist (`Satisfied` / `InsufficientProvenance` / `Missing`) with a human
  reason for each. No opaque score.
- [x] Honesty guarantees: an INFERRED claim never satisfies a VERIFIED
  requirement (reported as `InsufficientProvenance`); an absent capability is
  `Missing` (UNKNOWN), never assumed satisfied; a model's unrequested
  capabilities are surfaced separately as `extra`.
- [x] `CapabilityKind::from_str` — parse the snake_case serialized form so API /
  MCP can express capability filters.
- [x] Hub search capability filter — `GET /api/admin/hub/search?capability=ocr`
  returns only models whose real metadata supports that capability (with
  `matched`/`total` counts), dropping models that cannot back the claim.
- [x] Tests for every honesty rule + the API filter (pure, no hardware).

## 24. Next-Gen Phase 7 — MCP Capability Search

Extends the read-only MCP control plane so an external agent can ask "which
models can do X?" without knowing model names. Reuses the existing auth and the
pure capability matcher — MCP remains a thin translation layer (no new
identity/registry/privilege, ADR-004/ADR-005).

- [x] `search_models_by_capability` MCP tool — takes a snake_case capability
  (e.g. `ocr`, `vision`, `coding`, `summarization`) plus optional `query`/
  `limit`, returns only Hub models whose real metadata backs the claim.
- [x] Honest by construction: unknown/invalid capabilities return an explicit
  error or empty result, never a fabricated positive; Hub failures surface as
  an error object rather than a false "no models".
- [x] Architecture preserved: the async HTTP handler precomputes the Hub lookup
  into `McpContext.capability_search`; the MCP module stays I/O-free and pure
  (`capability_search_request` tells the handler whether a lookup is needed).
- [x] Tests for the tool dispatch + request-arg extraction (pure, no network).

## 25. Next-Gen Phase 1 — Capability Fit on the Model Card

Puts the provenance-aware capability matcher in front of the operator on the
Model Hub card, so "can this model do X?" is answered visibly and honestly.

- [x] `GET /api/admin/hub/model/{repo}?requires=ocr` — the model card now
  includes a `capabilities.fit` verdict: whether the model satisfies the
  requested capability at VERIFIED evidence, with a per-requirement checklist
  (`satisfied` / `insufficient_provenance` / `missing`) and a human reason.
- [x] Dashboard "Capability fit" block on the model card: pick a capability
  from the known taxonomy and see the honest verdict (VERIFIED vs INFERRED vs
  MISSING) and why.
- [x] Tests for the fit verdict covering the honesty rules (inferred never
  satisfies verified; absent capability is missing; no `requires` → null).

## 26. Next-Gen Phase 1/7 — Capability Fit on Model Comparison + Planner Plumbing

Two additive steps that extend the capability fabric end-to-end (operator UI and
execution decision observability), both honest by construction.

**Comparison capability fit**
- [x] `GET /api/admin/hub/compare?repos=...&requires=ocr` now adds a
  `capabilities.fit` verdict per compared model (same provenance-aware engine
  and honesty rules as the single-model card), plus a top-level `requires`.
- [x] Dashboard Model Comparison gains a "Capability fit" selector + per-model
  verdict checklist, reusing the model-card style.
- [x] Tests for the compare fit verdict covering the honesty rules.

**Planner capability-requirement plumbing (Phase L foundation)**
- [x] `RequestFacts.required_capability: Option<String>` — an optional required
  capability on the planner input.
- [x] `PlannerRationale.capability_requirement: Option<CapabilityRequirementView>`
  — records an honest verdict (`satisfied: false`, `evidence: "UNKNOWN"`) plus a
  reasoning note when a requirement is present. The engine-neutral fabric never
  claims satisfaction without real evidence; a coordinator with real
  `ModelCapabilities` may later overwrite it.
- [x] The two `RequestFacts` construction sites in `compute.rs` updated
  (requirement `None` today — routing/protocol untouched).
- [x] Tests: no requirement → None verdict; requirement → honest UNKNOWN;
  serde round-trip.

## 27. Next-Gen Phase L — Capability Requirement Reaches the Execution Decision

The planner plumbing (section 26) recorded the requirement in
`PlannerRationale`, but it stopped there — `ExecutionDecision` (the record
surfaced to agents via MCP/`/v1/compute#decisions` and the dashboard) did not
carry it. This closes that gap so an agent/operator can see what capability an
execution was asked to satisfy.

- [x] `ExecutionDecision.capability_requirement: Option<CapabilityRequirementView>`
  — populated in `evaluate()` from the planner rationale, so the decision
  carries the same honest verdict (`satisfied=false`, `evidence="UNKNOWN"` when
  the fabric has no `ModelCapabilities`).
- [x] Dashboard "Autonomous decisions" cards now show a `cap ✓/✗` badge with
  the required capability + evidence.
- [x] Test: a request with a required capability surfaces the honest UNKNOWN
  verdict on the decision and mentions it in reasoning; a request without one
  carries `None`.

## 28. Next-Gen Phase L — Real Capability Data Persisted + Resolved

The capability requirement plumbing was honest-but-UNKNOWN because the fabric
had no real per-model capability data. This wires in the authoritative source:
capabilities are classified from the Hub at pull time, persisted in the local
registry, exposed on the fabric model list, and the fabric can resolve a real
evidence-backed verdict from supplied claims.

- [x] `CapabilityClaimRecord { capability, provenance }` on `ModelRecord`
  (hub-agnostic projection of the hub taxonomy; `#[serde(default)]` keeps older
  registries valid; survives rescans and save/load). `ModelRegistry::record` +
  `set_capability_claims`.
- [x] Pull-time persistence: `admin_hub_pull_handler` fetches `model_detail`,
  classifies, and persists the claims for the pulled file (best-effort — a
  failure to persist never breaks the pull).
- [x] Fabric model list exposes `capability_claims` for local models (absent
  when the registry has none = UNKNOWN; remote workers untouched).
- [x] `resolve_capability_requirement` — pure, public fabric function that maps
  supplied claims to an honest verdict (VERIFIED satisfies; INFERRED never
  satisfies a verified requirement; MISSING otherwise). `capability_view`
  still yields UNKNOWN when no claims are supplied (existing behavior unchanged).
- [x] Tests: registry claims persistence/rescan/save-load; pull mapping helpers
  (snake_case, relative path, suffix match); resolver honesty rules (verified /
  inferred / missing / case-insensitive / pure).

## 29. Next-Gen Phase L — Coordinator Resolves Real Capability Verdict

The persisted claims + resolver existed, but the coordinator still recorded
UNKNOWN because a caller-provided requirement and the registry claims never
reached the planner. This wires the full loop end-to-end.

- [x] `WorkloadRequirements.required_capability: Option<String>` — a
  protocol-agnostic, additive carrier for an optional required capability (does
  not touch the signed P2P `InferRequest` frame).
- [x] `RequestFacts.capability_claims: Vec<(String, String)>` — the planner now
  resolves a REAL verdict from supplied claims instead of always UNKNOWN.
- [x] `ComputeManager.set_registry_path` + `capability_claims_for_model` — the
  coordinator maps model hash → file name (via the local advertisement) → the
  registry's persisted claims, and passes them into both `record_decision` and
  `plan_and_reserve`.
- [x] `admin_hub_pull_handler` sets the registry path on the compute manager so
  a pulled model's claims are immediately resolvable.
- [x] Tests: fabric resolver real-verdict path (verified/inferred/missing/
  unknown); distributed coordinator resolves a verified OCR claim → decision
  records `satisfied=true, evidence="VERIFIED"`.

## 30. Next-Gen Phase 7 — MCP Local Capability Search

The fabric model list already exposes persisted `capability_claims`. This adds
an agent-facing tool that answers "which of MY models can do X?" from that real
data — no Hub round-trip, no privilege change.

- [x] `find_local_models_by_capability` MCP tool — filters THIS node's models
  by a required capability (snake_case) with optional `evidence` (`any` default,
  or `verified`-only). Returns only models with real persisted claims; a model
  with no claim is never included (honest: absent = UNKNOWN).
- [x] Architecture preserved: the async HTTP handler precomputes the filter into
  `McpContext.local_capability_search`; the MCP module stays pure
  (`local_capability_search_request` extracts args).
- [x] Pure `filter_local_models_by_capability` over the fabric model list
  (case-insensitive, provenance-honest).
- [x] Tests: tool dispatch, arg extraction (evidence defaults to `any`),
  and the pure filter (any/verified/case-insensitive/no-claim honesty).

## 31. Next-Gen Phase 7 — MCP get_worker_capability

The North Star question "which workers in MY fabric can run this model for this
capability?" as a read-only MCP tool — a thin projection over existing fabric
state, no execution/reservations/model start.

- [x] `get_worker_capability { model, capability, evidence }` tool: per worker
  returns real identity (peer_id/node_id/node_name kept separate), model
  availability (served/on-disk/unavailable), trust, engine compatibility,
  RAM/VRAM fit, capability provenance, and an explainable
  CAN_RUN / CANNOT_RUN / UNKNOWN verdict (no opaque score).
- [x] Pure `worker_capability_verdict` reuses the existing capability resolver
  (`resolve_capability_requirement`), the resource-fit vocabulary, and the
  authoritative `ComputeAdvertisement` — no duplicate scoring/estimator/matcher.
- [x] Honest by construction: empty claims → UNKNOWN (never success/failure);
  UNKNOWN capability/provenance/resource telemetry is never converted into a
  verdict; remote workers with no data resolve to UNKNOWN, not a fabricated
  pass; no workers → explicit UNKNOWN/no compatible worker.
- [x] Tests cover all ten required cases: verified+compatible → CAN_RUN,
  insufficient RAM/VRAM → CANNOT_RUN, inferred+verified-evidence → CANNOT_RUN,
  missing claim → UNKNOWN, untrusted → CANNOT_RUN, missing telemetry → UNKNOWN,
  no workers → UNKNOWN, and identity separation.

## 32. Next-Gen — Unified "CAN I RUN THIS?" Fabric Fit

Combines capability fit + resource fit + worker fit + model availability into a
single explainable fabric-wide answer — the foundation for the Fabric Digital
Twin and the Intent → Capability → Model → Worker → Execution flow. Pure
aggregation over the existing per-worker projection (no new scoring/estimator).

- [x] `aggregate_can_i_run(&[WorkerCapResult])` — pure fabric-wide verdict
  (CAN_RUN if any worker can; CANNOT_RUN if workers exist but none can; else
  UNKNOWN, including no-workers). Chosen worker = first CAN_RUN; explainable
  aggregated reasons from the real per-worker checks.
- [x] `get_worker_capability` MCP tool now returns a `fit` block (verdict +
  counts + chosen_worker + reasons) alongside the per-worker projections.
- [x] `GET /v1/can_run?model=&capability=&evidence=` — the same unified view as
  plain JSON (operator/admin), reusing `mcp_worker_capability`.
- [x] Dashboard Models view gains a "CAN I RUN THIS?" card (model + capability +
  evidence inputs) rendering the real verdict, counts, reasons and per-worker
  blockers.
- [x] Tests: aggregate over real verdicts (any-can-run, none-can-run, all
  unknown, no-workers-not-invented, end-to-end good+bad worker).

## 33. Next-Gen Phase D — Variant-Aware CAN I RUN THIS?

Model Hub 2.0 foundation: a repository is a MODEL, each GGUF file is a
deployable VARIANT. The unified fabric fit now carries honest variant metadata.

- [x] `variant_quantization_from_file_name` — pure, INFERRED-only quantization
  classifier (Q8/Q6/Q5/Q4/Q3/Q2/FP16) from the GGUF file name; no marker →
  `None` (UNKNOWN), never guessed or presented as VERIFIED.
- [x] Per-worker `WorkerCapResult.quantization` (derived from the worker's
  matched file name) surfaced in the `get_worker_capability` / `/v1/can_run`
  per-worker JSON.
- [x] `/v1/can_run` and the MCP tool now return a `model_info` block
  (`quantization` + `available_workers` = real count holding the model).
- [x] Dashboard Model card gains a "CAN I RUN THIS? (fabric)" button that calls
  the LOCAL `/v1/can_run` (no Hub round-trip), sharing the `renderCanIRun`
  helper with the Models view. The decisions `capability_requirement` badge
  confirmed already rendered.
- [x] Tests: quantization classifier (10 cases incl. case-insensitivity and
  no-marker → None) + verdict carries inferred quantization (null path + Q4).

## 34. Next-Gen Phase D/L — Variant Fit, Intent Resolver, Registry Capability Query

Three parallel, additive capabilities completing the fabric-fit + intent
foundations (GitHub-safe, no execution).

**Per-variant fabric fit**
- [x] `registry_variants_for_model` — enumerates the REAL on-disk GGUF variants
  from the local registry (never invented), sorted deterministically.
- [x] `/v1/can_run` + MCP `get_worker_capability` return a `variants` array:
  per variant `file`, `quantization` (INFERRED), `size_bytes`, and a `fit`
  block (same per-worker pipeline → aggregate). Existing fields unchanged.

**Intent resolver (Phase L first step)**
- [x] `crates/hub/src/intent.rs` — pure, deterministic intent→capability mapping
  (keyword lexicon, INFERRED, dedup, no opaque scoring). `capabilities_for_intent`
  and `intent_requirements`.
- [x] MCP `resolve_intent` read-only tool + pure `resolve_intent(ctx, intent,
  evidence)` that cross-references the fabric model list's persisted claims:
  matched capabilities w/ models, unmatched capabilities, honest `note`. Wired
  into the HTTP handler.
- [x] Tests: hub intent mapping (all required cases), MCP arg parsing + matched/
  unmatched/unknown behavior.

**Registry capability query**
- [x] `ModelRegistry::models_with_capability(capability, require_verified)` and
  `models_with_any_claim` — pure, authoritative local queries (provenance
  preserved, case-insensitive, deterministic sort; no-claim models never
  returned). Tests across verified gating / no-match / case-insensitivity /
  sorting.

## 35. Next-Gen Phase D/L — Extended Intent, Registry Summary, Digital Twin Overview

Three parallel, additive capabilities closing the intent + Digital Twin loop
(GitHub-safe, no execution).

**Extended intent lexicon**
- [x] `LEXICON` extended: Retrieval, Reranking, Reasoning, StructuredOutput,
  Agents, TextToSpeech, Audio, Multimodal, DocumentUnderstanding, Video —
  all mapping to existing `CapabilityKind` variants (no new kinds).
- [x] `capability_label` (presentation-only delegate) and
  `intent_capabilities_with_labels` (capability+label pairs). Tests for the
  new mappings, dedup, and labels.

**Registry capability summary**
- [x] `ModelRegistry::capability_summary() -> Vec<(capability, verified_count,
  inferred_count)>` — authoritative local overview (distinct models per
  provenance bucket, deduped, deterministic sort). Tests for bucketing, dedup,
  empty registry, sort.

**Digital Twin overview**
- [x] `GET /v1/capabilities` — operator/admin endpoint returning the local
  on-disk capability summary (verified/inferred model counts).
- [x] Dashboard Models view gains a "Capability overview" card rendering the
  real local summary (no fabricated counts); Model card gains a
  "CAN I RUN THIS? — on-disk variants (fabric)" block that calls `/v1/can_run`
  and renders the real `variants` array (file, quantization, size, per-variant
  fit) with per-worker blockers.

## 36. Next-Gen Phase L/D — Intent→Fit Closed Loop, Variant Comparison, Verdict UI

Three parallel additions closing the Intent Planner loop and completing the
variant/verdict UI (GitHub-safe, no execution).

**Intent → capability → fabric fit (Intent Planner loop closed)**
- [x] `resolve_intent_with_fit` MCP tool + pure extractor `intent_fit_request`
  and dispatch. HTTP layer precomputes it via `mcp_intent_with_fit`: resolves
  the intent deterministically to capabilities, finds a real local model with a
  persisted claim per capability, and evaluates it against the fabric through
  the SAME per-worker verdict + aggregate pipeline. A capability with no local
  model reports honest UNKNOWN with an explicit reason.
- [x] Tests: composed flow (OCR → real model → UNKNOWN fit with no workers;
  coding → no local model → UNKNOWN reason; unknown intent → empty), tool def +
  extractor + dispatch.

**Variant comparison (Phase D)**
- [x] Dashboard Models view gains a "Variant comparison" card: input + capability
  select + button that renders `/v1/can_run` `variants` side-by-side, sorted
  deterministically (CAN_RUN → CANNOT_RUN → UNKNOWN, then by file). Empty →
  honest empty message; error → error message.

**Execution verdict UI**
- [x] `renderDecisions` confirmed already renders the `capability_requirement`
  badge (cap ✓/✗ + required capability + evidence) consistently on every
  decision card that carries one.

## 37. Next-Gen Phase 8 — OTel GenAI Projection + Best-Variant

**OpenTelemetry GenAI semantic conventions (Phase 8)**
- [x] `/metrics` now also emits `gen_ai.*` metric families (request.count,
  token.input/output, request.duration) with `gen_ai.request.model`,
  `gen_ai.operation.name`, `gen_ai.provider.name` labels — ADDITIVE to the
  DecentraAI-specific provenance; derived from real node state; never prompts
  or outputs. `prometheus_escape` guards label values.
- [x] Remote fabric routes (`/v1/execute`, streamed + non-streamed remote
  chat) record **real input-token estimates** via `prompt_token_estimate`
  instead of hardcoded `prompt_tokens:0` — the remote worker never echoes
  usage through the P2P stream, so `gen_ai.server.token.input` would read 0
  forever for distributed execution without the local estimate (fix `267a65d`;
  live-verified: non-streamed remote 54 prompt tokens, streamed 43).
- [x] Tests: metrics endpoint exposes the `gen_ai.*` families; escape helper
  handles quotes/backslashes/newlines.

**Best-variant selection (Phase D)**
- [x] `/v1/can_run` / MCP `get_worker_capability` `model_info` now carries
  `best_variant` — the first on-disk variant whose fit is CAN_RUN (deterministic
  file-name order), else null (honest: no variant confirmed runnable). Directly
  answers "which variant should I deploy on THIS fabric?".

## 38. Next-Gen Phase N — Historical Execution Statistics

Deterministic historical intelligence from real measured execution history
(no ML, no synthetic benchmarks).

- [x] `execution_statistics(&[ExecutedPlan])` — pure aggregate deriving records,
  outcome counts, measured throughput/latency (only from records with real
  `tokens_used` + `processing_time_ms`; `None` measurements excluded, never
  treated as 0), retries, and per-model / per-worker outcomes (deterministic
  order). Re-exported from `decentraai_distributed`.
- [x] `GET /v1/stats` — operator/admin endpoint returning the deterministic
  history statistics; honest 0-record response when no compute manager attached.
- [x] Tests: deterministic aggregates (measured-only, retries, per-model,
  per-worker, empty history) + endpoint auth/JSON.

## 39. Next-Gen Phase B/H/N — Resource Intelligence, Perf Provenance, Recovery Timeline

Three product-level capabilities (GitHub-safe, no execution), all additive and
reusing authoritative structures.

**Phase B — Unified Resource Intelligence operator view**
- [x] `GET /v1/resources` — unified operator view over CPU / RAM / VRAM / DISK /
  KV / QUEUE / LATENCY, each with TOTAL/AVAILABLE/RESERVED/IN_USE/HEADROOM where
  the dimension supports it and explicit provenance (MEASURED/ESTIMATED/RESERVED/
  ACTUAL/UNKNOWN). RAM and VRAM strictly separate; UNKNOWN is never a fabricated
  zero; every value comes from `SystemSnapshot`, `probe_gpu`, the compute
  manager's real advertisements + reservation ledgers + session count, and the
  node queue. VRAM reservations honestly omitted (no fabricated 0).
- [x] Dashboard Diagnostics gains a Resources card rendering the real node +
  per-worker state with provenance badges.

**Phase N → planner — Perf provenance (MEASURED vs ESTIMATED)**
- [x] `WorkerFacts.perf_measured` + `CandidateScore.perf_measured` — additive
  provenance marker (real measured completions fed the EWMA vs estimated/0).
  Never affects the score formula. Coordinator fills it honestly:
  `tokens_per_second>0 || latency_ms>0`.

**Phase H — Recovery timeline (self-healing visualization)**
- [x] `recovery_timeline(&ExecutionDecision) -> Value` — pure projection of the
  lifecycle trace: outcome, final phase, phases_seen, recoveries count,
  last OrchestrationAction, order-preserving event timeline, human summary.
  Reuses the existing event/phase/action vocabulary (no new recovery engine).
- [x] `ExecutionDecision.last_orchestration` (serde-tagged) + `/v1/execution`
  attaches a `recovery` timeline per decision; Dashboard decisions cards render
  a "self-healed ×N / no recovery" badge + phase list.

## 40. Next-Gen Phase H/N — Perf Provenance + Recovery in the Agent Control Plane

Surface the perf-provenance and recovery-timeline work through the MCP / agent
control plane (read-only).

- [x] `WorkerMetricRow.perf_measured` — honest MEASURED/ESTIMATED perf marker
  on every worker row in `metrics_report`, flowing into MCP `list_workers` and
  `/v1/compute`. Never affects scheduling.
- [x] MCP `list_executions` now attaches a `recovery` timeline per execution
  (projected from the real decisions keyed by request_id) so agents can see the
  self-healing loop (recoveries, phase, adaptation, ordered event trace).

## 41. Next-Gen Phase C — Fabric Graph / Digital Twin endpoint

A projection of the conceptual fabric graph from authoritative live state
(NODE → WORKER → ENGINE → MODEL → CAPABILITY → EXECUTION), the Digital Twin
foundation. No fake nodes, no hardcoded names/IPs; future nodes work
automatically.

- [x] `GET /v1/fabric` (operator/admin): `nodes` (peer_id/node_id/node_name kept
  separate, trust, engine, health, served/available models), `models`
  (distinct, deduped, with INFERRED quantization + persisted capability claims +
  holding nodes), `capabilities` (from real claims, with models + nodes),
  `executions` (decisions with recovery timeline), `network` (measured links),
  `kv` (sessions). Absent data = empty array (honest).
- [x] Pure `fabric_graph_aggregate` helper (unit-testable), async handler does
  only I/O. Reuses `variant_quantization_from_file_name` and
  `claims_for_file_name`.
- [x] Dashboard Fabric view gains a "Fabric graph · digital twin" card (counts +
  compact node/capability lists, real state only).
- [x] Tests: endpoint auth/JSON shape + pure aggregation (model dedup, identity
  separation, capabilities from real claims only).

## 42. Next-Gen Phase C — Fabric Graph in the Agent Control Plane

Expose the Digital Twin fabric graph to external agents as a read-only MCP tool.

- [x] `get_fabric_graph` MCP tool (read-only, no args): returns the same real
  projection as `GET /v1/fabric` (nodes with identity kept separate, models with
  INFERRED quantization + persisted claims, capabilities, executions with
  recovery timeline, network, kv). Precomputed by the HTTP layer via
  `mcp_fabric_graph` (reuses `fabric_graph_aggregate`).
- [x] Pure `fabric_graph_request` extractor + dispatch.
- [x] Tests: tool listed, extractor matches only the tool, precomputed snapshot
  returned unchanged.

## 43. Next-Gen Phase M — Policy Gate in CAN I RUN THIS?

The unified fabric fit now honors the remote-sharing policy, reusing the
existing `accepts_remote_inference` opt-in (no new permission system).

- [x] `worker_capability_verdict_with_policy` — adds an explicit `policy` check:
  a remote worker that has not opted into remote inference is a definitive
  CANNOT_RUN (never a fabricated pass); the LOCAL node is always allowed its own
  work. Threaded through `mcp_worker_capability` (model + variants) and
  `mcp_intent_with_fit` using the real local peer + advertisement flag.
- [x] `worker_capability_verdict` kept as a test-only convenience wrapper
  (policy on) so existing tests are unchanged.
- [x] Test: remote-no-opt-in → CANNOT_RUN via policy check; local always allowed.

## 44. Next-Gen Phase 1+2 — Unified Fabric Decision + Historical Comparison

Turn the independent capabilities into ONE coherent read-only fabric decision.

**Phase 1 — Unified Fabric Decision**
- [x] `unified_fabric_decision(state, intent, evidence, explicit_model)` — ONE
  coherent projection: request → capabilities → model options (per model +
  variant with CAN_RUN/CANNOT_RUN/UNKNOWN) → per-variant fabric fit →
  chosen decision (first CAN_RUN, deterministic) → why (real per-worker passing
  checks). Reuses the capability resolver, `worker_capability_verdict_with_policy`
  (trust/policy/engine/ram/vram), `aggregate_can_i_run`, registry claims,
  quantization. NOT a new planner/scoring system.
- [x] `GET /v1/decision?intent=&evidence=&model=` (operator/admin, read-only).

**Phase 2 — Historical comparison in the decision**
- [x] The decision carries a `historical` block from the real
  `execution_statistics` (measured throughput/latency/outcomes/retries per
  model/worker); UNKNOWN when insufficient — never fabricated averages.

**Tests (focused)**
- [x] `/v1/decision` auth + coherent structure (request/capabilities/decision/
  why/historical; decision null honestly when no workers).

## 45. Next-Gen Phase 4 — `decide` Agent Workflow Tool

The agent control plane gains ONE coherent workflow tool that encapsulates the
full decision flow (reduces the need to chain many tools).

- [x] MCP `decide { intent, evidence?, model? }` — precomputes
  `unified_fabric_decision` (intent → capabilities → model options → per-variant
  fabric fit → decision → why → historical). Read-only, no execution. Pure
  `decision_request` extractor + dispatch.
- [x] Tests: tool listed, arg parsing (evidence default + model optional),
  precomputed snapshot returned unchanged.

## 46. Next-Gen Phase 3 — Digital Twin Decision View

The dashboard gains a Decision card (progressive disclosure) rendering the ONE
coherent fabric decision from `/v1/decision`.

- [x] "Decision" card in the Models view: intent input + evidence selector +
  "decide" button → `decideNow()`.
- [x] Progressive disclosure: (1) decision banner + why, (2) capabilities →
  model options (quantization, verdict badge, can-run workers), (3) historical
  (measured; UNKNOWN when insufficient). Empty → honest empty/UNKNOWN, never
  fabricated.

## 47. Next-Gen Phase 5 — Recovery in the Unified Decision

The unified decision now also answers "what happened when something failed?" by
projecting the recent recovery timeline from the real decisions (reusing the
existing `recovery_timeline` vocabulary — no second recovery engine).

- [x] `/v1/decision` and MCP `decide` include a `recent_recovery` array: the
  last few decisions' recovery timeline (outcome, recoveries, adaptation,
  ordered event trace), each tagged with its request_id. Advisory-only — never
  claims an action the runtime did not take.

## 48. Next-Gen — decide → reserve → execute (mutation path)

The read-only fabric OS gains its first confirmed mutation: run a real inference
for an intent through the existing decide → plan_and_reserve → route_request
path. Reuses authoritative systems; no new planner/ledger/engine.

- [x] `POST /v1/execute` (master-gated, MUTATING): `{ intent, prompt, max_tokens,
  stream?, model?, evidence?, confirm: true }`. Refuses without `confirm: true`
  (mutation safety). Flow: `unified_fabric_decision` → chosen model →
  `resolve_model_hash` (from real advertisements) → `DistributedInference::route_request`
  (reserve + route + audit + recovery). Returns the decision + real inference
  result (output, tokens, worker) or a clear honest error.
- [x] `resolve_model_hash(file_name)` — resolves a real BLAKE3 hash from fabric
  advertisements; `None` honestly when the fabric does not hold the model.
- [x] Shared `run_execute_decision` core (handler + MCP both call it; the core
  enforces `confirm: true` so no caller bypasses it).
- [x] MCP `execute_decision { intent, prompt, max_tokens, ..., confirm: true }`
  — precomputes the execution into the context (status/ok/body), mutation
  safety enforced by the same core. Reuses existing master auth.
- [x] Tests (focused): refuse without confirm; honest 422 when no runnable
  decision (never a fabricated run); MCP tool listed, arg parsing, snapshot.

## 49. Next-Gen — STREAM + REPLAN steps

Complete the decide→confirm→reserve→execute→stream→measure→history→recovery→replan
loop with the two missing steps.

- [x] **STREAM**: `POST /v1/execute` with `stream: true` now emits SSE from the
  fabric router (`route_request_streamed`), reusing the chat proxy's streaming
  pattern — real incremental output + trailing usage, mutation-safety `confirm`
  enforced. Non-streaming JSON path unchanged (used by MCP).
- [x] **REPLAN (advisory)**: on execution failure the response carries a
  `replan` advisory derived from the existing `adapt()` vocabulary (retryable +
  eligible alternatives → REPLAN_AVAILABLE; else NO_ALTERNATIVE/ABORT).
  Advisory-only — never claims an action the router did not take.

## 50. Next-Gen — Dashboard Execute (confirm + streaming)

The operator can now run a decided intent on the fabric from the dashboard.

- [x] Decision card gains Execute controls: optional model, max_tokens, stream
  checkbox, prompt textarea, and an "Execute (confirm)" button. A real UI
  `confirm()` dialog (matching the backend `confirm: true`) precedes the run.
- [x] `executeDecision()`: non-streaming path renders the real `executed`
  (model/worker/tokens/output); streaming path consumes SSE via
  `getReader`/`TextDecoder`, appends `delta.content` incrementally, records the
  trailing `usage`, shows `[DONE]` (or an honest "stream closed" warning). Errors
  render the real message + replan advisory. No fabricated output/worker/tokens.

## 51. Next-Gen — session/continuation in execute (KV locality)

- [x] `/v1/execute` (stream + non-stream) and MCP `execute_decision` accept an
  optional `session_id` → `InferRequest.with_session`, so a run links to an
  earlier one and the fabric router steers back to the worker holding the
  session's KV prefix (continuation / cache locality, reusing M20).

## 52. Next-Gen — Phase M: mutation master-gating via MCP

- [x] Policy fix: MCP `execute_decision` (a mutation — runs real inference +
  reserves a worker) now requires the MASTER token, not just an operator role.
  The MCP handler gates read-only tools at operator/admin, but a mutating tool
  must be admin-only (an operator may decide; only admin may execute).

## 53. Next-Gen — MEASURE + HISTORY feedback in execute

- [x] `/v1/execute` success (non-streaming) now returns a `measure` block
  (real tokens_used, latency_ms, derived tokens_per_sec, provenance MEASURED —
  from the actual router response) and the `historical` stats updated after the
  run (UNKNOWN when no compute manager). Completes the
  execute → measure → history loop with real, not fabricated, numbers.

## 54. Next-Gen — dry-run for execute (mutation preview)

Safety preview before a real mutation: show exactly what would be reserved/routed
without executing.

- [x] `ComputeManager::plan_preview(model_hash, prompt_tokens, session_id,
  priority)` — builds the same `ExecutionPlan` the coordinator would use via
  `fabric_facts` + the fabric planner, WITHOUT reserving or sending anything.
  Returns `(plan, chosen_worker, estimated_ms)` or `None` (no eligible worker).
  Read-only; `in_flight` stays 0.
- [x] `/v1/execute` with `dry_run: true` (still requires `confirm: true`)
  returns the dry-run preview (`would_execute`: model/worker/estimated_ms/plan)
  instead of routing; never sends a request or holds a reservation. Honest 422
  when no eligible worker.
- [x] MCP `execute_decision` documents + accepts `dry_run` (flows through the
  same core).
- [x] Tests: `plan_preview` plans without reserving (in_flight 0) + honest None
  for unknown model; `/v1/execute` dry-run 422 without a fabric model.

## 55. Next-Gen — Dashboard dry-run preview button

- [x] Decision card gains a "Preview (dry-run)" button (`previewDecision()`):
  reuses the same intent/prompt/max_tokens/evidence/model inputs, POSTs
  `/v1/execute` with `dry_run: true` (+ `confirm: true` for the mutation gate),
  renders the real `would_execute` (model/worker/estimated_ms/stages/plan_id)
  with a "no request sent · no reservation held" note; no real-run confirm
  dialog (a preview is read-only). Errors render the real message + honest
  "no eligible worker" case. Nothing fabricated.

## 56. Next-Gen — Sessions endpoint (KV locality observability)

- [x] `SessionAccount::snapshot()` + `ComputeManager::sessions()` — expose every
  coordinator-tracked KV/session (session_id → worker residency + model +
  accounted tokens + capacity), real accounted state only.
- [x] `GET /v1/sessions` (operator/admin): the session snapshot; honest empty
  when no compute manager.
- [x] Test: auth + honest empty (never fabricated residency).

## 57. Next-Gen — Dashboard Sessions card (KV locality)

- [x] Execution view gains a "Sessions (KV locality)" card rendering the real
  `/v1/sessions` snapshot: session/worker/model (short), tokens_used/capacity
  (— when unknown), and a KV-headroom badge (ok / near-capacity warn / UNKNOWN
  faint when null). Empty → "no active sessions"; wired into the 3s refresh.
  Nothing fabricated.

## 58. Next-Gen — MCP list_sessions tool

- [x] `list_sessions` read-only MCP tool: exposes the coordinator-tracked
  KV/session residency (worker + model + tokens + capacity + KV headroom) to
  external agents, so they can see why a continuation would be steered to a
  specific worker. Precomputed by the HTTP layer (same `sessions()` snapshot as
  `/v1/sessions`); pure `sessions_request` extractor + dispatch.
- [x] Tests: tool listed, extractor matches only the tool, precomputed snapshot
  returned unchanged.

## 59. Next-Gen — Continue-from-sessions (KV locality in the UI)

- [x] Decision card gains a `session_id` input; `executeDecision()` and
  `previewDecision()` send it ONLY when non-empty (continuation / KV locality
  via the existing `/v1/execute` path).
- [x] Sessions card rows gain a "continue" button: `continueSession(sid)`
  pre-populates the session_id + intent/prompt defaults (never overwrites an
  existing prompt), switches to the Models view, and toasts — the real run still
  flows through the confirmed Execute path. No fabricated output.

## 60. Next-Gen — execute by capability directly (no intent parsing)

- [x] `/v1/execute` and MCP `execute_decision` accept `capability` as an
  alternative to `intent` (either one drives the unified decision; a snake_case
  capability name is itself resolvable by the intent lexicon). `prompt` +
  `confirm: true` still required. Honest 422 without a fabric model.
- [x] MCP tool schema documents intent OR capability; `execution_request`
  enforces "at least one". Tests: capability-only accepted, neither → rejected;
  endpoint capability-only proceeds past the intent gate and honestly 422s.

## 61. Next-Gen — Dashboard capability-only execute

- [x] Decision card gains a `capability` input (`#dec-cap`); `executeDecision()`
  and `previewDecision()` send `capability` only when intent is absent (intent
  preferred when both filled). Real run still goes through the confirmed
  Execute path. Nothing fabricated.

## 62. Next-Gen — "execute this" on CAN_RUN model options

- [x] In the Decision card, each CAN_RUN model option gains an "execute"
  button: `useModelOption(cap, model)` pre-populates `#dec-cap` (and clears
  intent so the exact capability is sent) + `#dec-model`, prefills the prompt
  only if empty, and toasts — execution still requires the confirmed Execute
  path (no auto-run, no bypass). CANNOT_RUN/UNKNOWN options show a muted "—".

## 63. Next-Gen — Phase M: LIMITS on mutations (execute rate limit)

- [x] `/v1/execute` (master-gated mutation) is rate-limited per token name
  (`master`) to `EXECUTE_RATE_LIMIT_PER_MINUTE = 10` via a separate sliding
  window (never interacts with tier inference limits). A limited call returns
  429 and audits `execute_rate_limited`. Read-only calls are never limited here.
- [x] Test: limit+1 call → 429 + audit (each call consumes a slot before body/
  decision handling, so honest 422s count).

## 64. Next-Gen — Device-class projection (mobile/lightweight worker foundation)

First step toward "a fabric from ALL your devices": classify each fabric node's
device class from its REAL advertised capability (GPU/RAM/cores), exposed in the
Digital Twin.

- [x] `device_class(&ComputeCapability)` — pure INFERRED classification
  (server/desktop/laptop/mobile/edge) from real hardware advertisement; never
  fabricated, never changes scheduling.
- [x] `/v1/fabric` nodes now carry `device_class`, so an operator can see at a
  glance which fabric members are lightweight/mobile.
- [x] Test: classification across server/desktop/laptop/mobile/edge from real
  capability inputs.

## 65. Next-Gen — Adaptive fan-out / load-balance projection

Second step toward "a fabric from ALL your devices": show how much each CAN_RUN
worker could contribute (request-level load balancing), advisory only.

- [x] `load_balance_for_workers(...)` — pure, INFERRED, advisory: per CAN_RUN
  worker a `suggested_share_pct` from real advertised `tokens_per_second` ×
  idle headroom (100-load), normalized to ~100%. Includes `device_class`.
  Never changes scheduling; no eligible → empty.
- [x] `unified_fabric_decision` model_options now carry a `load_balance` array.
- [x] Test: faster/more-idle worker gets a larger share, shares sum ~100,
  no-eligible → empty.

## 66. Next-Gen — Mobile-worker foundation in the UI

- [x] Fabric graph card shows each node's `device_class` badge (mobile/desktop/
  server/laptop/edge) when present.
- [x] Decision card model options show the advisory `load_balance` fan-out
  shares (per CAN_RUN worker: short id + name + share %) under a "fan-out
  advisory:" note. Empty/absent → nothing rendered. Real state only.

## 67. Next-Gen — Node version advertisement (update readiness)

Helps the "update all nodes" workflow: each node now advertises its DecentraAI
build version, so a coordinator/operator can see which fabric members are stale.

- [x] `ComputeAdvertisement.node_version` (`#[serde(default)]`, backward-compat;
  empty = UNKNOWN for older peers). Set from `CARGO_PKG_VERSION` in
  `build_advertisement` (all production advertisements), so the local node
  reports its real build.
- [x] `/v1/fabric` nodes carry `node_version` → visible next to each node's
  device_class, so you can tell desktop (old) vs laptop (new) at a glance.

## 68. Next-Gen — Node version badge in the UI

- [x] Fabric graph card now shows each node's `v<node_version>` badge next to
  its `device_class`, so you can see at a glance which fabric members are stale
  (e.g. desktop on an old build vs laptop on the new one). Empty/absent version
  → nothing rendered (honest).

## 69. Next-Gen — Node Lifecycle & version consistency

Builds on node_version: the coordinator now classifies each fabric peer's
version and lifecycle from real evidence, so operators can see which nodes need
update.

- [x] `ComputeManager.node_version()` — coordinator's own build version.
- [x] `version_status(coordinator, remote)` — pure: CURRENT / OUTDATED /
  UNKNOWN (a different known version is OUTDATED; empty is UNKNOWN; never
  fabricates). `node_lifecycle(trusted, healthy, status)` — pure projection of
  DISCOVERED → TRUSTED → ONLINE (+ *_OUTDATED) using only real evidence;
  UPDATING/VERIFIED are NOT emitted (no real remote-update mechanism yet).
- [x] `/v1/fabric` now returns a `coordinator.version` block, and each node
  carries `version_status`, `outdated`, and `lifecycle` (plus the existing
  `node_version`). Mismatch is observable here (no new event system).
- [x] Focused tests: version_status honesty; node_lifecycle only emits
  evidence-backed states.

## 70. Next-Gen — Version-consistency UI

- [x] Fabric graph card now shows per-node `version_status` (current/outdated/
  unknown) and `lifecycle` badges from real backend fields, plus a coordinator
  `v<version>` line and an honest "N node(s) need update" count (only nodes with
  `outdated === true`; UNKNOWN nodes never counted).

## 71. Next-Gen — Node Lifecycle & Upgrade documentation

- [x] `docs/deployment.md` gains a "Node Lifecycle & Upgrade" section: the
  evidence-backed lifecycle (DISCOVERED → TRUSTED → ONLINE → OUTDATED;
  UPDATING/VERIFIED explicitly future, not produced), honest version semantics
  (CURRENT/OUTDATED/UNKNOWN; never claims an update for UNKNOWN), a safe
  out-of-band upgrade workflow (no remote shell, reuses trust; re-classified
  after restart), and a platform-agnostic architecture note (Linux/Windows/ARM/
  mobile each have their own packaging; the fabric only observes node_version).

## 72. Next-Gen — Standalone lightweight worker (`decentraai-worker`)

Separates the WORKER PLANE from the CONTROL PLANE: a worker joins the fabric,
advertises capability/resources/engine/model, accepts authorized remote
inference, executes on a local llama-server, and reports real measurements —
WITHOUT running the planner, model hub, registry scan, dashboard, MCP, tokens,
decisions, or orchestration.

- [x] New `[[bin]] decentraai-worker` in `crates/node-cli` (`src/bin/decentraai-worker.rs`),
      reusing 100% of the existing `decentraai-distributed` worker path
      (`ComputeManager` worker-side methods + `DistributedInference::register_worker_backend`)
      plus identity/config/system-probe/engine. NO duplicate identity, trust,
      capability matcher, resource estimator, auth, or signed P2P protocol.
- [x] Worker-only sequence: load identity → load config → spawn llama-server →
      `ComputeManager` (advertise + serve) → `DistributedP2PHandler`/`P2PNode` →
      `DistributedInference` (`register_as_worker` + `register_worker_backend`) →
      immediate advertise + periodic broadcaster. Skips all coordinator-only
      calls (no `set_signing_identity` for outbound, no router, no network probe,
      no reaper, no contributions, no metrics server).
- [x] Lifecycle (evidence-backed): DISCOVERED (signed ad + mDNS) → TRUSTED
      (coordinator trusts peer id) → CONNECTED/READY (listening + advertising) →
      BUSY (serving) → OFFLINE (heartbeat lapses). UPDATING/VERIFIED never
      emitted (no remote update mechanism). CLI usage:
      `decentraai-worker --name <n> --model <file.gguf> [--binary <llama-server>]`.
- [x] Platform-neutral worker path (libp2p TCP/Noise/mDNS; llama-server spawn is
      cross-platform; GPU probe degrades cleanly). Reuses the same signed
      advertisement + inbound signature verification as the full node.

## 73. Next-Gen — Worker architecture documentation

- [x] `docs/WORKER_ARCHITECTURE.md`: control plane vs worker plane, worker
  dependency boundary, worker contract/advertisement (today vs future fields),
  evidence-backed lifecycle, platform abstraction, mobile readiness & adaptive
  contribution (explicitly CONTRACT/PLAN, not implemented), and the distributed
  inference boundary (experimental). Honest: no fabricated mobile telemetry,
  remote update, or distributed inference.

## 74. Next-Gen — Real GPU thermal/utilization telemetry

First step toward thermal-aware adaptive contribution, using REAL measured data
(no fabricated telemetry).

- [x] `ComputeAvailability.gpu_temperature_celsius` + `gpu_utilization_percent`
  (`#[serde(default)]`, backward-compat): populated in `build_advertisement`
  from the real nvidia-smi probe; `None` when no GPU / no measurement (UNKNOWN).
- [x] `/v1/fabric` nodes carry a `gpu` block (temperature_celsius,
  utilization_percent) — real measured values, or null (UNKNOWN).
- [x] This is the honest foundation for the future mobile/thermal-pressure
  scheduler signals; nothing fabricated.

## 75. Next-Gen — Productize the standalone worker

- [x] `scripts/install-app.sh` documents the installed standalone worker
  (`decentraai-worker --name <n> --model <file.gguf>`) in its messaging/summary;
  `cargo install --path crates/node-cli` already installs both binaries.
- [x] `scripts/uninstall-app.sh` removes both `decentraai` and
  `decentraai-worker` binaries (exact `rm -f`, safe). bash -n validated.

## 76. Next-Gen — Worker join / status / doctor

Make a standalone worker easy to join an existing fabric, reusing ALL existing
mechanisms (identity, config, guest-token credential, verified P2P dial) — no
new identity/token/trust/auth/discovery system.

- [x] `decentraai-worker --join '<multiaddr> <dsk_ token>'` — parses the invite
  (same format + `dsk_` prefix as `decentraai join`), loads/generates identity
  in the shared store, stores the guest credential 0600, dials the coordinating
  peer over the verified P2P path, audits `worker_joined`. Secrets never logged.
- [x] `decentraai-worker --status` — real local state (PeerId, identity, join
  credential, config validity, lifecycle, CPU/RAM/GPU from a live probe). UNKNOWN
  stays UNKNOWN.
- [x] `decentraai-worker --doctor` — read-only diagnostics reporting REAL,
  useful problems (identity missing, not joined, bad config, llama-server not
  found, no model). Never fabricates.
- [x] Tests: `parse_invite` accepts multiaddr+dsk_, rejects non-dsk_/missing
  parts. Config/credential persist across restart (same data dir); perms 0600.

## 77. Next-Gen — Worker health / lifecycle in status

- [x] `decentraai-worker --status` reports an evidence-backed worker-side
  lifecycle: READY (identity + credential + valid config + engine + model) vs
  DISCOVERED (joined, needs a model/engine) vs UNKNOWN (not joined). Engine +
  model availability are probed; Trust and BUSY are clearly marked as
  coordinator-side (the coordinator decides trust and observes busy/queued).
  UPDATING/VERIFIED never emitted.

## 78. Next-Gen — Dashboard "Add a lightweight worker" instructions

- [x] Fabric graph card gains a static, instruction-only "Add a lightweight
  worker" block: how to run `decentraai invite` on the coordinator, then
  `decentraai-worker --join "<multiaddr> <dsk_ token>"` + `--model <file.gguf>`
  on the new machine, then `decentraai trust add --peer <peer-id>`.
  Instructions only — no new backend/mutation, and the real multiaddr/token are
  never fabricated (they come from `decentraai invite` on demand).

## 79. Next-Gen — Cross-platform worker packaging boundary

- [x] `docs/WORKER_ARCHITECTURE.md` gains a practical packaging table: Linux
  (systemd), Windows (console/scheduled task), ARM (container/CPU-only), and
  Android/mobile marked FUTURE — not supported. The worker contract is identical
  across platforms; only probes + engine adapters differ; no single update
  mechanism assumed.

## 80. Next-Gen — Adaptive-contribution capacity state (FULL/LIMITED/UNAVAILABLE)

- [x] `ComputeAvailability::capacity_state()` — pure, evidence-backed
  classification from real health/load/queue: FULL (healthy + headroom),
  LIMITED (healthy but load>=80 or queue>=6), UNAVAILABLE (unhealthy). Never
  fabricated. Foundation for adaptive contribution (desktop high / laptop
  medium / phone limited).
- [x] Exposed in `/v1/fabric` nodes (`capacity`) and `/v1/resources` fabric rows
  (`capacity`).
- [x] Test: evidence-backed FULL/LIMITED/UNAVAILABLE from real availability.
  No planner-scoring change (no authoritative worker capacity model yet).

## 81. Next-Gen — Capacity badge in the UI

- [x] Fabric graph card shows each node's `capacity` badge (FULL ok / LIMITED
  warn / UNAVAILABLE bad) from the real `capacity` field, plus a small
  "capacity: FULL / LIMITED / UNAVAILABLE" legend when any node reports it.
  Absent → nothing rendered. Real state only.

## 82. Next-Gen — Contribution accounting foundation (deduped measured work)

First GitHub-safe primitive of the Compute Contribution & Quota roadmap: a
credit ledger that prevents double-counting, using REAL measured work (never
advertised hardware). No economics/conversion invented.

- [x] `MeasuredContribution { credited_executions, total_tokens,
  total_processing_ms, verified_completions }` — real measured work that earned
  credit; `None`/absent measurements stay UNKNOWN (never fabricated).
- [x] `ComputeManager::record_credited_contribution(peer, request_id, verified,
  tokens_used, processing_time_ms)` — DEDUPS by `request_id` (returns false on
  duplicate/replay at the credit layer, so the same execution is never counted
  twice); credits real tokens + processing time only on verified completions.
  Reuses the existing peer identity + `ExecutedPlan` measured fields; no second
  telemetry system.
- [x] `measured_contribution()` snapshot + re-export.
- [x] Test: duplicate request_id credits once (tokens/time not double-counted);
  failed executions earn no measured credit.

## 83. Next-Gen — Compute contribution & quota: live wiring + quota ledger

Continues the [Compute Contribution & Quota roadmap](docs/roadmap/COMPUTE-CONTRIBUTION-AND-QUOTA.md)
Q1 + Q3: wire the credited, deduped measured-work accounting into the live
execution paths AND add a deterministic quota ledger that converts it into
spendable quota. No economics invented (versioned placeholder policy), no
consumer API keys yet, no dashboard quota UI yet.

- [x] **Q1 live wiring** — `record_credited_contribution` is now called in the
  authoritative `route_request` (non-streamed) and `route_request_streamed`
  paths on every **verified completion with measured usage** (tokens +
  processing time), keyed by `request_id`. Failures, timeouts, transport errors
  and retries earn nothing; the same execution is credited exactly once (both
  the credit layer and the quota layer dedup by `request_id`, and the streamed
  path is single-attempt so no mid-stream retry can double-credit).
- [x] **Q3 pure quota ledger** (`crates/compute/src/quota.rs`) — deterministic,
  no I/O/no async, serde-serializable:
  - `QuotaLedger::credit/reserve/settle/release` with explicit
    `EARNED → AVAILABLE → RESERVED → CONSUMED` lifecycle; `release` returns
    unused reservation to the pool.
  - Idempotent by an explicit `ref_id` (existing execution/request/reservation
    id): duplicate credit / reserve / settle / release are no-ops; double-settle
    is refused; overdraw is refused (`InsufficientQuota`).
  - `ContributionPolicy { version, units_per_token, units_per_processing_ms }`
    — versioned, replaceable, inspectable; default is a documented placeholder
    (1 token→1 unit, 1 ms→1 unit), NOT a fair market price. Historical credits
    retain the policy version that produced them; `set_policy` swaps in place.
  - UNKNOWN measurements (None) earn nothing — never fabricated; an unmeasured
    execution creates no account record.
  - Append-only, bounded audit trail (provenance) with `policy_version`.
- [x] **ComputeManager integration** — `set_contribution_policy`,
  `contribution_policy`, `quota_account`, `quota_accounts`, `quota_events`;
  credited work earns quota keyed by the worker peer id (existing identity).
- [x] **Observability** — `/v1/compute` surfaces `quota` (per-account
  earned/available/reserved/consumed, totals, active policy version) alongside
  workers/contributions. Real measured state only.
- [x] **MCP `get_quota` tool** — read-only projection of the quota ledger for
  external agents (per-account balances, totals, policy version). Same
  master-gated boundary as the other MCP tools; no quota mutation surface.
- [x] Tests: quota ledger lifecycle (reserve/settle/release/partial/duplicate/
  insufficient/audit/policy-version), ComputeManager wiring (credited work earns
  quota, duplicates don't, failures don't, policy replaceable + versioned), and
  MCP `get_quota` exposure.

## 84. Next-Gen — Consumer API keys (`dca_`) with quota authorization (Q2)

Account-scoped consumer API keys for the [Compute Contribution & Quota
roadmap](docs/roadmap/COMPUTE-CONTRIBUTION-AND-QUOTA.md) Q2: an access
credential + quota ceiling that lets agents/applications consume fabric
compute against the authoritative quota ledger. Reuses the existing `dsk_`
auth architecture (hash-only storage, atomic persistence, revoke-by-id, roles)
— no second authentication system, no invented economics.

- [x] **ConsumerKeyStore** (`crates/tokens/src/consumer.rs`): `dca_` keys with
  `owner_account`, `quota_ceiling`, `rate_limit_per_minute`, `scopes`.
  Plaintext shown once; only BLAKE3 hash + short display prefix stored. Create/
  revoke-by-id/list; corrupt registry starts fresh.
- [x] **Account** — the consumer account is the existing `QuotaLedger`
  `AccountId` (no parallel identity system). The ledger is `Arc`-shared between
  `ComputeManager` (worker credits) and `ApiState` (consumer reserve/settle), so
  provider contribution and consumer consumption are ONE authoritative balance.
- [x] **Auth** — `Auth::Consumer` resolved in `classify` from the `dca_`
  prefix; strictly an inference credential. `require_master` /
  `require_operator_or_admin` reject it (a consumer key is never admin).
- [x] **Quota authorization** — before routing, reserve
  `min(account.available, quota_ceiling)`; settle against real measured
  completion tokens on success; release on any other exit via a RAII guard
  (no leak, no double-settle, no overdraw). Insufficient quota denies the
  request (403) and is audited `consumer_quota_denied`.
- [x] **Rate limit** — independent per-key sliding window
  (`rate_limit_per_minute`), separate from tier/execute limits; audited
  `consumer_rate_limited`.
- [x] **Admin API** — `/api/admin/consumer-key/create|revoke|list`
  (master-gated). List shows metadata + live usage + owner account balance;
  never the secret.
- [x] **CLI** — `decentraai consumer-key create|list|revoke`.
- [x] **MCP** — `list_consumer_keys` read-only metadata tool (never the
  secret); `get_quota` already covers the ledger.
- [x] **Dashboard** — admin page Consumer API Keys card (create/list/revoke,
  usage + account quota).
- [x] **Main dashboard Quota card** (Q4 observability) — per-account
  earned/available/reserved/consumed, totals, active policy version, plus a
  "recent quota events" provenance trail (credit/reserve/settle/release with
  policy version), from `/v1/compute`. Real measured state only.
- [x] **MCP consumption flow** — a consumer `dca_` key may call the inference
  tools `decide` (read-only) and `execute_decision` (quota-bounded mutation)
  through `/mcp`: per-key rate limit → reserve `min(account.available,
  ceiling)` → execute through the existing fabric → settle the real measured
  tokens → release unused reservation (RAII guard, incl. on failure). All
  other tools (workers, network, executions, sessions, quota, consumer keys)
  are denied to consumers; a consumer never gains admin/operator privileges.
- [x] Tests: create/auth/invalid/revoked/permission/ceiling/rate-limit/
  reserve-settle/release-on-failure; secret never in metadata; quota
  provenance exposed after a settled consumer request; MCP consumer can
  `decide`, executes with quota, is denied operational tools, and a failed
  MCP execution releases its reservation. Workspace tests, clippy `-D
  warnings`, release build green.

## 85. Next-Gen — Adaptive worker contribution

First step of the mobile/lightweight-worker adaptive-contribution direction:
a worker's effective capacity is reduced by real, measured pressure so the
planner sends stressed workers less work. Real signals only — nothing
fabricated, UNKNOWN stays neutral.

- [x] `ComputeAvailability.battery_percent` (Option<u8>) — real battery charge
  when probed (mobile/laptop); `None` on desktop/UNKNOWN. Backward-compatible.
- [x] **Battery probe** (`system-probe::probe_battery`) — reads the real Linux
  battery charge from `/sys/class/power_supply/*/capacity`, skipping
  AC/charger entries and reporting the conservative min across cells. `None`
  on desktop / no battery (honest UNKNOWN). Wired into `SystemSnapshot` and
  `build_advertisement`, so a worker actually advertises `battery_percent`.
- [x] `ComputeAvailability::adaptive_contribution_factor()` (0.0..1.0) — pure
  product of real GPU thermal, GPU utilization, CPU load and battery terms:
  thermal ≥95°C → ×0.1, ≥90 → ×0.25, ≥80 → ×0.5, ≥70 → ×0.8; full GPU util
  → ×0.3; full CPU load → ×0.3; battery ≤10% → ×0.1, ≤20 → ×0.25, ≤50 → ×0.6.
  UNKNOWN signals are neutral (never invented); result clamped to (0, 1] so a
  worker always remains eligible for a small share.
- [x] **Scheduler scoring** — `ComputeScheduler::score` multiplies the base
  fit score by the adaptive factor, so a stressed worker is ranked below an
  otherwise-identical healthy worker (verified by test).
- [x] **Observability** — `/v1/fabric` nodes + `/v1/resources` fabric rows
  expose `adaptive_contribution` (+ `battery_percent`); dashboard fabric row
  shows `capacity` + `adaptive` factor, fabric graph node shows battery level
  + adaptive badge.
- [x] Tests: factor is neutral with all signals UNKNOWN, reduces under
  thermal/battery/GPU-util stress, stays positive under combined worst case;
  scheduler picks the healthy worker over the thermally-stressed identical
  one; battery probe reads real capacity, skips chargers, reports the
  conservative min, and returns None without a battery. Workspace tests,
  clippy `-D warnings`, release build green.

## 86. Next-Gen — Mobile worker contract & Android feasibility (honest)

Documentation + the concrete adaptive-contribution foundation for the
mobile/lightweight-worker direction. No fabricated mobile telemetry; `None`
stays UNKNOWN.

- [x] `docs/WORKER_ARCHITECTURE.md` §5/§6 updated: the "Today vs Future"
  table now marks GPU thermal, battery state and adaptive contribution as
  **implemented** (real values), with foreground/background, network quality
  and CPU/SoC thermal still 🔒 plan. The adaptive-contribution section
  describes the real scheduler behavior (stressed worker ranked lower).
- [x] **Android feasibility (honest)** — the worker contract is
  Android-portable (Rust + libp2p cross-compiles via the NDK), but a real
  Android worker is blocked on a maintained llama-server Android build + the
  Android process/service engine adapter. Battery probe is the prototype for
  the mobile battery path; SoC thermal/NPU/foreground are not implemented.
  Documented as a feasibility note, not a supported platform.

## 87. Next-Gen — Isolated llama.cpp RPC tensor-split experiment (harness)

The llama.cpp RPC path is classified EXPERIMENT (per
`docs/DISTRIBUTED_INFERENCE.md`): a real two-node measurement must precede any
enablement. This lands the honest, isolated harness — it produces REAL
measured latency/throughput when the operator has the RPC binaries, and never
fabricates a result.

- [x] `scripts/rpc-experiment.sh` — isolated experiment that spawns its own
  `ggml-rpc-server` + throwaway `llama-server` (`--rpc` + `--tensor-split`),
  runs a fixed prompt set, and reports real measured latency (ms) + tokens per
  run. JSON report (avg latency, total tokens, per-sample latencies,
  provenance `REAL_MEASURED`). Exits non-zero if prerequisites are missing or
  all runs fail. Does not touch the live node/quota/fabric; LAN-only; never
  runs by default.
- [x] `docs/DISTRIBUTED_INFERENCE.md` — measurement-protocol note documenting
  how to run the harness and what the report contains.
- [x] Honest boundary: no `ggml-rpc-server` / llama-server present in this
  environment, so **no fabricated measurements are reported here**; the
  harness exists and produces real results when run on real hardware.
  LOCAL-BLOCKED on hardware + RPC binaries for an actual dataset.

## 88. Next-Gen — Adaptive fan-out / load-balance real

The roadmap's adaptive fan-out / load-balancing real: distribute **independent
requests** across workers in proportion to each worker's real, currently-useful
capacity — NOT by splitting a single model across devices (that stays gated
behind `supports_staging()`, parked). A phone 30% / laptop 20% / desktop 50% for
a batch of independent requests.

- [x] `decentraai_compute::adaptive_load_shares` (pure, deterministic,
  I/O-free): per-worker share `(0,1]` summing to 1.0, derived from throughput ×
  idle headroom × adaptive contribution factor (thermal/battery/GPU-util
  pressure). Unhealthy workers excluded; ties broken by peer id; shares sorted
  share desc / peer id asc. Never splits a single generation — advisory only.
- [x] `load_balance_for_workers` (runtime fabric decision) now uses the
  authoritative pure distribution and exposes `adaptive_contribution` per
  worker, so the dashboard/decision shows the real adaptive shares.
- [x] Tests: equal workers → equal shares; faster worker → larger share;
  thermally-stressed worker → smaller share (factor recorded); unhealthy worker
  excluded; empty/all-unhealthy → empty; deterministic regardless of input
  order. Workspace tests, clippy `-D warnings`, release build green.

## 89. Next-Gen — Batch allocation in the real planner / execution path

Makes the adaptive fan-out **operational** for batches of independent requests:
a deterministic request → worker allocation from the pure `adaptive_load_shares`
distribution, integrated into the fabric planner and the distributed execution
boundary. Never splits a single generation/model (stays gated behind
`supports_staging()`, parked).

- [x] `decentraai_fabric::allocate_batch` (pure, deterministic): assigns a set
  of independent requests to workers using `adaptive_load_shares` over the
  fabric facts, honoring the same invariants as the single-request planner —
  never an unhealthy/untrusted/incompatible worker; a **continuation** is
  pinned to its KV-prefix worker; weighted-interleaved so shares spread evenly
  (not one worker first); deterministic regardless of input order (request-id
  asc + peer-id asc tie-breaks). `BatchAssignment`/`BatchAllocation` carry
  provenance (request id, worker, share, kv_pinned, eligible).
- [x] `DistributedInference::plan_batch` — operational planner boundary: builds
  `RequestFacts` + `WorkerFacts` from the LIVE compute manager (real capacity,
  load, KV residency) and returns the deterministic allocation for a
  same-model batch. Empty/no-compute → honest empty/None.
- [x] `DistributedInference::route_batch` — operational dispatch: each
  independent request runs through the existing authoritative `route_request`
  path (capacity, reservation, retry, quota, KV affinity, recovery, audit).
  Returns `BatchRequestOutcome` (request id, chosen worker, result) preserving
  per-request provenance. Never auto-retries a failed request (idempotency).
- [x] **LOCAL-BLOCKED (honest boundary)**: pinning each request to its exact
  allocated worker inside the single-request reserve/retry loop is NOT wired —
  `route_batch` executes each request via the existing safe path and reports
  the actually-chosen worker. This preserves all safety invariants without a
  larger runtime change; the allocation is authoritative for planning, and
  dispatch reuses the proven per-request path.
- [x] **Exact worker pinning (wired)** — `ComputeManager::plan_and_reserve_on`
  reserves the exact preferred worker when it is still eligible (trusted +
  healthy + serves the model + headroom, via the existing `reserve_worker`),
  building a single-stage plan targeting it; falls back to normal planning if
  it is no longer eligible (dropped / unhealthy / full / untrusted / local
  node). `DistributedInference::route_request_on` pins the first attempt to the
  allocated worker; retries re-plan freely. `route_batch` now pins each
  independent request to its allocated worker via `route_request_on` (falling
  back when the worker is no longer eligible), so the batch allocation is
  actually honored. Quota / KV affinity / capability / trust / idempotency /
  retry / recovery / provenance are preserved by the shared single-request
  path.
- [x] Tests: 2 equal workers balance; faster worker gets more; LIMITED worker
  gets less than idle; unhealthy worker never assigned; incompatible worker
  never serves; batch covers every request exactly once; KV continuation pinned;
  deterministic regardless of input order; provenance preserved. Workspace
  tests, clippy `-D warnings`, release build green.

## 90. Next-Gen — Dashboard workload distribution

Operator visibility into the adaptive fan-out: how a batch of independent
requests would be spread across the fabric, from real adaptive-contribution
factors. Advisory; real values only.

- [x] Fabric graph card gains a "Workload distribution" bar: each eligible
  node's adaptive share (normalized from its real `adaptive_contribution`),
  sorted largest-first, with a percentage bar. Absent adaptive_contribution →
  nothing rendered (never fabricated). Marked advisory.

## 91. Next-Gen — Live two-node fabric validation (Laptop i5 ↔ Desktop i7)

Mobile/Android stays roadmap-only. Primary target: a rock-solid real two-node
fabric. Live validation performed against the actual environment (see
`docs/TWO_NODE_VALIDATION.md`).

- [x] **Exact worker pinning wired** (`plan_and_reserve_on`, `route_request_on`,
  `route_batch` pinning) — see §89; the LOCAL-BLOCKED gap is closed.
- [x] **Live discovery verified** — this node restarted with the current binary;
  both workers appear in `/v1/compute` and the Desktop peer is `connected` in
  `/v1/network` (real LAN P2P connection, 192.168.1.132).
- [x] **Live local inference verified** — `/v1/chat/completions` returns a real
  completion.
- [x] **Live version-mismatch scenario proven** — the Desktop node
  (`dca-NGE65Z`) advertises `accepts_remote_inference: false` because it runs an
  older binary (built 2026-08-15, predating the advertisement field); it
  deserializes to the conservative default `false`. The coordinator honestly
  rejects routing to it ("does not accept remote inference") rather than
  sending work to a worker that did not opt in.
- [x] **LOCAL-BLOCKED — live remote execution** (documented honestly): full
  end-to-end remote execution across the two nodes was not completed this
  session because the Desktop needs a current binary (manual/remote step) and
  live requests were served locally (local models win). Distributed execution
  is otherwise covered by the loopback E2E two-node tests. Required to close:
  upgrade the Desktop binary + a model served by only one node / a forced
  remote route.

## 92. Next-Gen — Desktop upgrade procedure + remote execution validation gate

The live two-node finding is that the Desktop (`dca-NGE65Z`, 192.168.1.129)
runs an older binary and conservatively advertises `accepts_remote_inference:
false`. The Desktop's control plane (API on loopback by design, SSH closed) is
not reachable from the laptop, so the upgrade must run on the Desktop by its
operator.

- [x] `scripts/upgrade-node.sh` — exact, idempotent in-place upgrade: build
  current HEAD, stop the node service (ETXTBSY guard), swap the binary with a
  timestamped backup, restart the systemd user service, and verify the node
  now advertises `accepts_remote_inference: true`. Never touches data/config/
  identity.
- [x] `docs/TWO_NODE_VALIDATION.md` — documents the exact Desktop upgrade
  steps + the post-upgrade trust + forced-remote route commands to prove
  Laptop → Desktop remote execution.
- [x] **LOCAL-BLOCKED — live remote execution validation**: cannot be
  completed from this laptop because the Desktop's API/SSH are unreachable
  (loopback API + no SSH). The upgrade + re-validation must run on the
  Desktop. Two-node P2P connectivity IS verified live (this laptop dials the
  Desktop's P2P port 38231; `/v1/network` shows it connected), which is the
  precondition for remote execution.

## 93. Collective Intelligence — P0+P1 agent substrate (DONE)

Direction (agreed with George, `docs/COLLECTIVE_INTELLIGENCE.md`): DecentraAI
evolves from a distributed inference fabric into a collective-intelligence
infrastructure in which many specialized agents collaborate as one distributed
system. **An agent is a logical execution context on a node — not a new
process.** This milestone lays the P0+P1 foundation: the agent model, the
unified capability matcher, and the signed wire advertisement.

### P0 — agent model (done, commit: `decentraai-agents` crate)
- [x] `crates/agents` (pure, no I/O): `AgentRecord` (identity/capabilities/
  role/policies/memory_scopes/lifecycle), `AgentRegistry` (deterministic
  local registry), `AgentCapability` (unified semantic+execution view),
  `ToolDescriptor` (extensible tool kinds: mcp/builtin/http/custom),
  `AgentTask` (generic task contract with schemas + verification — the shape
  P3 delegation will route), `AgentAdvertisement` (wire shape).
- [x] **Unified capability language**: `CapabilityKind`/`Provenance`/
  `CapabilityClaim`/`CapabilityRequirement` + hub requirement types now derive
  `Deserialize` (additive), making the semantic taxonomy wire-compatible.
- [x] **Unified matcher** (`AgentMatcher::match_agent`): ONE compositional
  verdict — hub provenance-aware semantic gate + agent model allowlist gate +
  compute physical gate (trust/health/RAM/VRAM/reservations) — replacing the
  two unrelated matchers a caller previously had to reconcile by hand.

### P1 — signed discovery (done)
- [x] `SignedAgentAdvertisement` in `decentraai-protocol` (opaque bytes +
  Ed25519 signature, anti-spoof: signer key must map to the claiming peer).
- [x] `AgentManager` in `decentraai-distributed`: local agent registry +
  remote advertisement registry + stale eviction + signed wire bytes +
  flattened dashboard view.
- [x] `DistributedP2PHandler` branch: verifies signed agent advertisements
  before updating the agent view (rejected forgery is dropped, never trusted).
- [x] Node wiring (`decentraai node` + `distributed`): registers the node's
  default logical agents (generalist + model-tied executor, honest INFERRED
  provenance), broadcasts them on the advertisement heartbeat, prunes stale
  remote views.
- [x] CLI `decentraai agent list` — read-only view of the node's advertised
  agents.
- [x] Dashboard AGENTS view (`/v1/agents` + nav) — local + remote logical
  agents with capability chips (provenance marks), tools, models, policies,
  sandbox mode; real state only.
- [x] E2E: two real libp2p nodes exchange signed agent advertisements
  bidirectionally; a forged advertisement is rejected at the signature gate;
  the unified matcher answers a semantic match against a remote agent.

### Honesty notes
- Default agents claim LLM capabilities as INFERRED (the node cannot back
  VERIFIED claims without Hub metadata — never claimed stronger than real).
- Agent tasks are defined and tested but NOT routed yet (P3 = delegation).
- Remote agents are visible in the dashboard but cannot receive work yet
  (P2/P3 = messaging + delegation).

## 94. Collective Intelligence — P2–P7 agent fabric (DONE)

Continues §93. The pure agent-fabric milestones landed in one coherent push
(commit `b2a4701`), each as a module in `crates/agents` (pure, no I/O) with
unit tests + E2E where a wire path exists.

### P2 — agent messaging (DONE)
- [x] `message.rs`: `AgentMessage` (ask/delegate/reply/verify/ping; opaque
  JSON payload; nonce; created_at), `AgentInbox` (bounded per-recipient FIFO,
  overflow dropped never grown), `validate_message`.
- [x] `AgentMessenger` in `decentraai-distributed`: sends `AgentMessage` over
  the existing libp2p request/response channel (Noise-authenticated), lands
  inbound frames in the right recipient's inbox.
- [x] `DistributedP2PHandler` branch delivers inbound messages to the inbox.
- [x] E2E: two real nodes exchange a `Delegate` message over the transport.

### P3 — delegation DAG (DONE)
- [x] `delegation.rs`: `DelegationPlan` (DAG of `DelegationStage`s with
  depends-on edges + per-stage verification), `DelegationPlanner::plan_task`
  (one stage per required capability, deterministic first-capable-agent
  routing, then a synthesis stage), `execute_plan` (topological execution
  with an injected executor, per-hop schema verification on the VALUE, honest
  Partial verdicts).
- [x] Honesty: an unroutable capability rejects the plan (never invents an
  executor); per-hop verification runs when the stage demands it.

### P4 — result verification / consensus (DONE)
- [x] `verification.rs`: `VerificationReport`/`VerificationCheck`/`CheckKind`,
  `check_output_schema` (honest JSON structural check), `ConsensusPolicy` +
  `evaluate_consensus`, `DisagreementResolution` + `resolve_disagreement`,
  `VerificationLedger` (bounded, immutable per task_id).

### P5 — collective memory (DONE)
- [x] `memory.rs`: `MemoryLevel`/`MemoryAccess`/`MemoryPolicy`/`MemoryEntry`/
  `MemoryScope`, `can_read`/`can_write` (ownership + access + trust +
  provenance), `enforce_retention`, `MemoryRegistry` (bounded scopes with
  expiry pruning + access enforcement). Runtime SQLite persistence is a later
  concern; the model the runtime will enforce is settled here.

### P6 — agent reputation (DONE)
- [x] `reputation.rs`: per-(agent, capability) `AgentReputation` with factors
  (reliability/quality/latency/uptime/safety/provenance), EMA `ReputationStore`
  `observe`, deterministic `best_for_capability`, `safety_penalty` (only
  policy/crypto violations — network errors never touch safety), unknown
  reputation = 0.0 NOT a penalty.

### P7 — policy engine (DONE)
- [x] `policy.rs`: `Permission`/`PolicyDecision`/`PolicyEngine` — explicit
  Allow/Deny with reasons for tools, models, peers, resource budgets,
  network egress; Controlled-Exploration boundary (Normal/Exploration/
  Experimental → `ExplorationLimit`). **Agent Power ≠ Permission** enforced.

### Honesty notes
- Per-hop verification checks the JSON *value* (not its serialization, which
  would always parse), catching a stage that promised an object and returned
  a string.
- `check_output_schema` is deliberately shallow (structural, not a full
  JSON-Schema validator) and never claims more validation than it does.
- Messaging is wired end-to-end (E2E-verified) but not yet driven by the
  product UI; P3/P9 orchestration will use it.

## 95. Collective Intelligence — P8–P11 + live orchestrator (DONE)

Continues §93/§94. The remaining pure fabric milestones landed (commit
`b1d5a0f`) plus the live orchestrator that binds them together.

### P8 — talent tree (DONE, `crates/agents/src/talent_tree.rs`)
- [x] Dynamic capability graph: `TalentNode` (capability + prerequisites +
  resource estimate + provenance + confidence + experimental), `TalentTree`
  (`can_unlock`, `resolve_path`, `available_capabilities`, `reachable`),
  `seed_talent_tree()`. No fixed levels, no hardcoded end; new capabilities
  are added at the graph level without code changes. Composite nodes are
  mapped to the closest existing `CapabilityKind` (documented honestly).

### P9 — collective workflows (DONE, `crates/agents/src/workflow.rs`)
- [x] `WorkflowTemplate`/`WorkflowStep`/`run_workflow`/`WorkflowOutcome`:
  named reusable DAG templates instantiated into concrete `DelegationPlan`s
  and executed with the delegation executor.
- [x] `research_report_template()`: the architecture-doc example
  (Research → Financial → Documents → Synthesis, with a Critic verification).

### P10 — self-optimization (DONE, `crates/agents/src/selfopt.rs`)
- [x] `SelfOptimizer`: weighted observations per `OptimizationDimension` →
  Increase/Decrease/Rebalance suggestions scored under hard-ceiling
  (Reliability/Security/Privacy) and soft-target (Quality/Cost/Latency)
  constraints; `suggest_compute`. Pure policy loop — the runtime applies it.

### P11 — agent economy (DONE, `crates/agents/src/economy.rs`)
- [x] `CapabilityOffer` (quality/reliability/price-per-unit/SLA/concurrency),
  `BookingRequest`/`negotiate` (explicit Booked/Rejected verdicts),
  `EconomyLedger` (bounded, cheapest-first selection). Non-monetary
  (synthetic credits), modular — later wired to Quota/Compensation.

### P3.5 — live orchestrator (DONE, `crates/distributed/src/agent_orchestrator.rs`)
- [x] `AgentOrchestrator` binds the pure fabric to the live P2P channel:
  **plan** (`DelegationPlanner`) → **select** (reputation-ranked executor from
  the local+remote agent view, local first) → **delegate** (sends
  `AgentMessage::Delegate` over the messenger, awaits the `Reply` with a
  re-dialing send + reply timeout) → **verify** (per-hop value check) →
  **collect** (topological, honest Partial on any failure).
- [x] `AgentMessenger::set_transport` (interior-mutable P2P) resolves the
  circular messenger/handler/node construction.
- [x] E2E: a real coordinator delegates a 2-stage workflow to a remote agent
  on another node over the transport, receives the replies, verifies the
  object output and completes — the first real agent-to-agent delegated work.

### Honesty notes
- Orchestrator selection ranks by reputation but a tie never penalises a
  capable agent (unknown reputation = 0.0, not a penalty).
- The remote "agent runtime" in the E2E is a test stub that answers
  `Delegate`; a production agent runtime (executing real tasks on the remote
  side) is the next step, built on this orchestrator.

## 96. Collective Intelligence — production agent runtime + memory + observability + CLI (DONE)

Continues §93–§95. Commit `e28f5b3`. The fabric's runtime halves + product
surfaces.

### Production agent runtime (DONE, `crates/distributed/src/agent_runtime.rs`)
- [x] `AgentRuntime`: the remote-side executor. Drains an agent's inbox, runs
  each `AgentMessage::Delegate` through an injected `AgentExecutor` (a
  `for<'a,'b> Fn(&AgentTask, &Value) -> Result<Value>` seam to a real engine),
  replies to the delegating peer with `AgentMessage::Reply` (error-shaped on
  failure so the orchestrator marks the stage failed instead of hanging).
  `run_forever` drain loop; honest `ExecutedMessage` outcomes.
- [x] `AgentMessage.from_peer` (additive, backward-compatible): the
  orchestrator stamps its peer on Delegates so the remote runtime knows where
  to reply.
- [x] The orchestrator E2E now uses a real `AgentRuntime` (not a test stub) on
  the remote node.

### Collective memory persistence (DONE, `crates/distributed/src/agent_memory.rs`)
- [x] `MemoryStore`: SQLite-backed persistent store enforcing the pure P5
  policies — `register_scope`/`write`/`read`/`search`/`unregister_scope`/
  `list_scopes` with access control (owner/trust/remote-opt-in), expiry
  pruning, `max_entries` (newest kept), JSON policy round-trip, and real
  cross-reopen persistence (proved by test).

### Collective-graph observability (DONE, dashboard)
- [x] The AGENTS view now renders a Collective Graph: aggregate metrics
  (total/local/remote agents, capability claims, tools, models, per-role
  breakdown) and a capability-coverage table (agents per capability +
  verified-provenance badge) — all from the real `/v1/agents` payload, no
  mock data, inside the advanced container.

### CLI (DONE, `decentraai agent`)
- [x] `decentraai agent show --agent <id>` — full local record.
- [x] `decentraai agent workflow [--template research_report]` — template steps.
- [x] `decentraai agent reputation --agent <id>` — synthetic-sample reputation
  profile (honestly labeled) demonstrating the P6 model.
- [x] `decentraai agent talent-tree --have <caps> --budget_mb N [--target X]` —
  P8 capability-graph availability/resolve-path.

### Honesty notes
- The `AgentRuntime`'s executor is injected; a production *inference*
  executor (calling the local llama-server / routing a request) is the next
  integration step — the runtime never pretends to execute without one.
- CLI `reputation` uses deterministic synthetic samples, clearly labeled —
  never presented as real measurements.

## 97. Collective Intelligence — production inference executor + node agent host (DONE)

Continues §96. Commit `2e2c8e3`.

- [x] `AgentExecutor` is now **async** (takes `AgentTask` + `Value` by value,
  returns a boxed future) so a real inference backend can be awaited.
- [x] `InferenceAgentExecutor` (in `agent_runtime.rs`): an executor that runs
  a delegated LLM task through the fabric's real path
  (`DistributedInference::route_request`). Input is a prompt string or
  `{ "prompt", "model_hash?" }`; model resolution is task-workload > input >
  default; output is `{ text, model_hash, tokens }`. Pure
  `infer_request_from` maps input→request (unit-tested, no network).
- [x] **Node daemon is now a live agent host**: `decentraai node` wires the
  agent messenger (placeholder → `set_transport` on the real P2P node), spawns
  a production `AgentRuntime` with the `InferenceAgentExecutor` (when a model
  is served) so it can execute delegated LLM tasks, and opens the SQLite
  `MemoryStore` — all best-effort, never disturbing the existing flow.

### Honesty notes
- The agent runtime answers `Delegate` for the node's own agent via the
  orchestrator's message protocol; full remote-orchestration on LAN between
  two live `decentraai node` hosts is the next live validation.

## 98. Node — explicit model selection (`node.model`) (DONE)

Commit `1c4f9bb`. Pivot on local GGUF models.

- [x] `NodeSection.model: Option<String>` (additive, `deny_unknown_fields`-safe;
  absent = auto-detect as before).
- [x] `resolve_model_name`: an explicit `node.model` wins over auto-detection
  and a missing file is a HARD startup error (the operator notices a typo, not
  a silent fallback); blank = no model; None = first sorted `.gguf`.
- [x] `decentraai node` serves the resolved model through llama-server.
- [x] Docs + example config.

Switch between models by editing `node.model` (e.g. Llama-3.2-1B for speed,
Mistral-7B for quality) and restarting the node — no re-detection needed.

## 99. Collective workflows live over the fabric (DONE)

Commit `1d3a9a5`. The P9 `research_report_template` now runs end-to-end over
the live fabric, not just as a pure unit.

- [x] `AgentOrchestrator::orchestrate_plan(&DelegationPlan)` — execute an
  already-instantiated plan (e.g. from `WorkflowTemplate::instantiate`) by
  delegating each stage to a chosen executor, verifying per hop. Shared
  `run_plan` loop behind both `orchestrate` (plan+run) and `orchestrate_plan`.
- [x] `select_executor` fix: a stage with NO capability requirements (e.g. a
  synthesis stage) is eligible on any agent — the orchestrator never invents
  an executor, but also does not block unconstrained stages on a capability
  match (this is why the synthesis stage of a workflow whose master task
  declares no capabilities previously failed as "no capable agent").
- [x] E2E: the research-report workflow (Research → Finance → Documents →
  Synthesis) is instantiated from the P9 template and executed on a real
  remote node's `AgentRuntime` — all four stages delegate, per-hop
  verification passes, and a final output is produced.

## 100. Collective workflows in the API + dashboard (DONE)

Commit `b7c2a18`. A user can now trigger a real collective workflow from the
dashboard/API on a single node, executing through the node's local agents.

- [x] `AgentOrchestrator` is shared into `ApiState` and wired in `decentraai
  node`; a runtime is spawned **per local agent** (the orchestrator selects
  these as executors), each answering delegated stages through the inference
  executor.
- [x] `POST /v1/agents/orchestrate` — body `{ prompt, template? }`; instantiates
  the named `WorkflowTemplate` (research_report), runs it via
  `orchestrate_plan` with the prompt as the seed, returns verdict + per-stage
  results + final output.
- [x] `orchestrate_plan(plan, seed)` / `run_plan` now merge a `seed` (e.g. the
  user prompt) into every stage's inputs, so the original prompt stays
  available to each stage (executor reads `inputs.prompt`).
- [x] Dashboard AGENTS view: a "Collective workflow" runner — prompt input +
  template select + Run button, rendering per-stage verdicts and the final
  output. Real state only.

### Honesty notes
- On a single node the workflow runs through the node's own agents; with
  remote peers trusted, the same orchestrator delegates to them (LAN
  validation is the next live step).

## 101. Collective workflow verified LIVE on a real node (DONE)

Commit `e0a9d3c`. The research-report workflow now runs end-to-end on a real
`decentraai node` with a real local model, generating actual text.

Three production bugs fixed in cascade, each with the root cause:

- **libp2p refuses self-dial**: a single-node orchestrator delegating to its
  own agent over P2P never got a reply. `AgentMessenger::send` now detects
  self-delivery (`p2p.local_peer_id() == peer`) and pushes straight into the
  local inbox instead of round-tripping over libp2p (single-node workflows
  must not depend on a P2P loopback).
- **missing capability**: the local generalist agent did not advertise
  `DocumentUnderstanding`, so the workflow's `documents` stage was
  unroutable. `default_local_agents` now also claims (INFERRED) Document
  Understanding, Summarization, Classification, StructuredOutput — honest for
  a generalist LLM.
- **stale backend URL**: the single-node inference executor captured a static
  backend URL, but llama-server respawns on a new port (M24), so the captured
  port went stale. `InferenceAgentExecutor` now takes the node's LIVE engine
  URL cache (`live_engine_url`) and re-reads it per call, and executes
  delegated tasks directly against the local backend over HTTP (distributed
  `route_request` cannot self-route), falling back to `route_request` when no
  local backend is configured.

Verified live: `POST /v1/agents/orchestrate` (research_report) on this node
with Llama-3.2-1B returns `verdict: completed`, all four stages verified, and
a real generated final report text.

933 workspace tests green; clippy clean.

## 102. Two-node LAN validation tooling (READY — pending Desktop upgrade)

Commit `pending`. The Desktop node (dca-NGE65Z) runs an older binary, so it is
not yet visible as a remote worker/agent (old builds omit agent
advertisements and `accepts_remote_inference`). Trust is already set on this
laptop.

- [x] `scripts/validate-lan.sh` — from the coordinating laptop: checks the
  API, requires at least one remote worker, reports trusted/remote_ok per
  remote worker, picks a model served only by the remote node (so routing is
  forced remote), routes a real chat request to it, and reports the reply —
  proving two-node remote inference end-to-end. Exits non-zero on any failure.
- [x] **Desktop upgraded (2026-08-19)**: `git pull && bash scripts/upgrade-node.sh`
  on the Desktop (now at `979acbf`, advertises agents + `accepts_remote_inference`),
  then `bash scripts/validate-lan.sh` on this laptop → reply `REMOTE`.

## 103. Two-node LAN validation VERIFIED (DONE)

Commit `80829be` (fix) + `..` (script). Two-node remote inference is verified
end-to-end on real hardware (Laptop i5 ↔ Desktop i7).

- [x] **Root-cause bug fixed**: `SignedAgentAdvertisement` and
  `SignedComputeAdvertisement` share the same wire envelope; the handler tried
  the agent branch first and dropped the compute advertisement (its inner
  payload is a ComputeAdvertisement, so the agent verify failed with "missing
  field protocol_version" and returned). The remote worker therefore never
  appeared in `/v1/compute`, even though the same node's agent advertisement
  arrived. Fix: decode the inner payload as an `AgentAdvertisement` first;
  only treat it as an agent ad when that succeeds, otherwise fall through to
  the compute branch. Regression test added.
- [x] **Verified live**: after the fix, the Desktop (i7) appears in the
  Laptop's `/v1/compute` as a **trusted, remote_ok remote worker**; a real
  chat request to the shared Llama model routes remote and returns a reply.
- [x] `scripts/validate-lan.sh` now works without a remote-only model (both
  nodes share the tiny Llama) — it verifies the remote worker is trusted +
  remote_ok, routes a real request, and reports the reply.
- [x] E2E isolation: test P2P nodes disable mDNS (parallel loopback tests no
  longer discover each other); the forged-ad test asserts the specific peer is
  rejected (deterministic under parallel runs).

## 104. Local GGUF model set + dataset/skill layer (P8 dataset)

Commit `pending`. Two parts:

### Model set (downloaded, verified SHA-256) on the Laptop (~30 GiB RAM)
- `Llama-3.2-1B-Instruct` (tiny, existing), `Mistral-7B` (general, existing)
- `qwen2.5-3b-instruct-q4_k_m` (small/rapid, 2.1 GiB)
- `qwen2.5-coder-7b-instruct-q4_k_m` (coding, 4.7 GiB)
- `nomic-embed-text-v1.5.Q4_K_M` (embeddings/RAG, 84 MiB)
- Registry updated to 5 models (`decentraai registry list`). The Desktop keeps
  the tiny model (8 GiB RAM).

### Dataset/skill layer (`crates/agents/src/dataset.rs`, P8 dataset)
The mechanism that lets the Talent Tree evolve — the chain
`Hardware → Models → Tools → Datasets → Capabilities → Talents → Agent Power`:
- `DatasetDescriptor` (develops capabilities, source, kind, quality,
  provenance, license), `SkillDescriptor` (binds a dataset to a model with a
  base capability + prerequisites, unlocking capabilities), `SkillRegistry`,
  and `build_agent_capabilities` (model base caps + applicable skills →
  unlocked caps, feeding `TalentTree::available_capabilities`).
- `decentraai agent skill` demonstrates the chain with a seeded
  code-finetune dataset + code-agent skill: a coding model unlocks
  `tool calling`. Honest provenance — a dataset claims only what it develops.

## 105. P8 dataset/skill audit + integrity fix (DONE)

Commit `pending`. Following the dataset-layer audit (docs/DATASET_AUDIT.md),
the provenance-laundering hole is closed:

- **Invariant enforced**: a skill can only unlock capabilities its dataset
  actually develops (`SkillDevelopsNotInDataset` on `add_skill`). A Verified
  dataset can no longer lend Verified provenance to capabilities it never
  trained for.
- **`build_agent_capabilities` unlocks `dataset.develops`** (the evidence
  source), never a skill's own declaration — so unlocked capabilities are
  exactly those with dataset evidence, carrying the dataset's provenance.
- Added tests: skill.develops outside dataset.develops is rejected; build
  unlocks dataset-developed capabilities with dataset provenance.
- `docs/DATASET_AUDIT.md` documents the A–K findings: dataset=evidence,
  skill=application gate, quality/confidence currently inert (TalentTree
  ignores provenance), talent/execution runtime wiring is the next milestone.
- Ran `cargo fmt --all` (formatting-only; also fixed a pre-existing
  inference-adapter formatting diff).

944 workspace tests green; clippy clean.

## 106. RAG retrieval foundation (DONE)

Commit `b32e09c`. The RAG direction (nomic-embed-text-v1.5 downloaded) now
has a pure retrieval index in `crates/agents/src/retrieval.rs`:

- `IndexedDocument` (id, text, optional capability, embedding `Vec<f32>`, tags).
- `RetrievalIndex`: add/remove/get/len/ids + `search(query_embedding, top_k)`
  ranking by cosine similarity desc, tie-break doc id asc (deterministic);
  empty-vector/orthogonal queries match nothing (honest).
- `cosine_similarity` pure function.

Next wiring step (documented): a runtime `/v1/embeddings` path that serves
`nomic-embed-text-v1.5` and feeds this index, exposing a `Retrieval` capability
to agents.

## 107. RAG embeddings endpoint (DONE)

Commit `pending`. The RAG retrieval path now has a live embeddings endpoint:

- `EmbeddingClient` (distributed): a thin HTTP client to an OpenAI-compatible
  embeddings backend (`/v1/embeddings`), not managing the backend process.
- `POST /v1/embeddings` on the node: `{ "input": "..." }` →
  `{ "embedding": [...], "dim": N }`, wired when
  `inference.embeddings_backend_url` is set (a llama-server launched with
  `--embedding`, e.g. on `nomic-embed-text-v1.5`).
- Verified live: nomic-embed-text-v1.5 via llama-server --embedding returns a
  768-dim vector through the node endpoint.

The RetrievalIndex (retrieval.rs) is ready to be populated from these vectors;
a query/index endpoint is the next wiring step.

## 108. RAG index + query (DONE)

Commit `pending`. The RAG path is now fully functional end-to-end.

- `RetrievalManager` (distributed): holds a `RetrievalIndex` fed by the
  embeddings backend — `index(doc_id, text, capability)` and
  `query(text, k)` (embed → cosine search).
- `POST /v1/rag/index` — `{ doc_id, text, capability? }` → embeds + indexes.
- `POST /v1/rag/query` — `{ text, k? }` → top-k similar documents.
- Verified live: index a document, query "collective intelligence network"
  returns it with score 0.76 (real nomic-embed vectors, deterministic cosine).

This completes Dataset → indexed knowledge → embeddings → retrieval
capability foundation. Exposing a `Retrieval` capability to agents (so a
workflow can call retrieval) is the next step.

## 109. Retrieval capability on the agent (DONE)

Commit `pending`. A node with a configured embeddings backend now claims the
`Retrieval` capability (INFERRED) on its generalist agent — the RAG path is
advertised on the fabric, so a workflow/stage that requires `Retrieval` can be
routed to a node that can actually perform semantic retrieval.

Verified live: the agent advertises `retrieval` alongside its other
capabilities (chat, coding, tool_calling, document_understanding, ...).

## 110. Reputation from real results + UI (DONE)

Commit `pending`. The orchestrator now feeds the `ReputationStore` from real
verified executions: each delegated stage records Reliability/Quality
(success) and Latency (normalised, faster-is-better, never punished to zero)
per (agent, capability). A `reputation_snapshot()` exposes the measured
history; `POST /v1/reputation` + a dashboard Reputation view render it.

This is real, measured history — not the synthetic samples the CLI demo used.
Empty until workflows run.

## 111. Retrieval tool in execution (DONE)

Commit `pending`. The inference executor now performs RAG at runtime: when a
delegated task's inputs carry a `retrieve` string, `InferenceAgentExecutor`
queries the RetrievalIndex (via nomic-embed) and augments the prompt with the
top-k retrieved context before generating. Best-effort — retrieval failure
degrades to a plain generation, never a hard error. The output records which
docs were retrieved (`retrieved_docs`).

Wiring: EmbeddingClient + RetrievalManager are created once and shared by the
inference executor (retrieval tool) and the API (/v1/embeddings, /v1/rag).

## 112. Model Fabric — provider control plane, P1–P3 (DONE, `crates/providers`)

Commit `249c55a`. New workspace crate `decentraai-providers` (pure domain,
no I/O) + `decentraai-inference-adapter` reuse:

- **P1 — provider domain**: `Provider` (kind, base_url, credential_ref,
  health, circuit), `ConnectedModel` (upstream_model, symbolic hash,
  display_name, capabilities, context_window, pricing, budget, health,
  circuit, usage, sharing policy default **OFF**), `ProviderKind`
  (OpenRouter/OpenAi/Groq/Together/Fireworks), `ProviderSummary`
  (masked credential fingerprint — never the secret or its key id).
- **P2 — credential store**: `CredentialStore` is **in-memory only**; the
  persisted record carries only a key reference (`dcrypt_{hex}`), never the
  raw secret. Tested: the secret does not appear in `db/providers.json`.
- **P3 — symbolic hash + wire handles**: `prov-` + SHA-256(provider_id +
  upstream_model) (24 hex chars, total 29). `ModelHandle` wire form
  `provider:{provider_id}:{model_id}`; raw upstream names are matched as
  well. Canonical signing stays anchored in the manifest; providers carry
  **no** signatures by design.

## 113. Model Fabric — health, circuit breaker, manager, adapter (DONE)

Commits `265230d`, `58ab406`, `5d3e151`. `ProviderHealth`/`ModelHealth`
(Unknown/Healthy/Degraded/Offline/Disabled), `CircuitState`
(Healthy/Degraded/Open/HalfOpen), `Pricing` (input/output per 1M +
provenance), `ModelBudget`, `ModelUsage`. `ProviderManager` owns CRUD,
persistence (tmp+sync+rename), catalog (local + fabric + provider views),
health probes (provider-level, model-level, latency), and the credential
store. `ModelAdapter` wraps the backend-neutral `OpenAiCompatibleBackend`
for complete/stream/health with error classification.

## 114. Model Fabric — provider plane wired into the node (DONE, commit `33fa52d`)

`ApiState.providers: Option<Arc<tokio::sync::Mutex<ProviderManager>>>` +
`attach_providers()`; admin routes (master-gated) in `providers_api.rs`:
`POST /api/admin/providers`, `POST /api/admin/providers/{id}/test`,
`POST /api/admin/providers/{id}/discover`, `POST /api/admin/providers/{id}/models`,
`DELETE /api/admin/providers/{id}/models/{model_id}`,
`POST /api/admin/providers/{id}/models/{model_id}/enable`,
`POST /api/admin/providers/{id}/models/{model_id}/sharing`,
`DELETE /api/admin/providers/{id}`; `GET /v1/providers` (operator_or_admin)
returns `{ providers: [{ summary, models }] }`. Audit events:
`provider_created`, `provider_tested`, `model_connected`, `model_deleted`,
`model_enabled`, `sharing_updated`, `provider_deleted`.

Provider-backed chat routing (`resolve_provider_model`): a request for a
provider model (symbolic hash, handle, or upstream name) is served directly
by the adapter — no local engine slot, no fabric worker. Buffered + SSE
streamed, OpenAI-compatible. **Security invariant**: secrets stay in the
in-memory credential store only; the API key never lands in code, logs,
commits, or docs.

## 115. Model Fabric — dashboard Providers view (DONE, commit `956ed85`)

Dashboard v2 gains a `Providers` view (nav button ◈): live provider cards
(kind, base_url, masked fingerprint, circuit, latency, failure count, shared
count, connected models ENABLED/DISABLED/shared + symbolic hash + latency)
plus a master-gated "Add provider" form. Data comes from the real
`/v1/providers` payload; watching the page never touches the backend.

## 116. Model Fabric — cost-aware auto routing + agent model powers (DONE, commits `f034afb`, `dfcc9e3`, `3fd622c`)

**P7 — cost-aware auto selection**: `best_provider_model()` (pure decision)
picks the best enabled provider model for `auto`/`__auto__`: provider+model
enabled, neither circuit-OPEN, health rank (Healthy > Degraded > Unknown >
Offline), cheaper total cost wins when both report pricing, deterministic
tie-break (provider_id asc, model_id asc). `resolve_provider_model` handles
`auto`; fabric routing keeps priority for `auto` — the provider is the
fallback only when no fabric/local model is runnable (or when the fabric
plane is absent entirely). Explicit provider handles still win
unconditionally before fabric routing.

**P9 — Agent Model Powers**: an agent may pin a provider model for a task
by naming its symbolic hash, provider handle, or raw upstream name.
`is_provider_model_ref()` detects provider refs; `InferenceAgentExecutor`
bails with a clear error when a task requests a provider model but the node
has no local backend (provider models require the local OpenAI-compatible
proxy — the fabric `route_request` path has no provider knowledge).

**P10 — end-to-end test**: `resolve_provider_model` with `model=auto`
serves the connected provider model over a loopback OpenAI-compatible mock,
pinning the resolver→adapter wiring.

Tests: 1018 workspace tests green; clippy `-D warnings` clean; fmt clean.

## 117. ExecutionStrategy foundation (P1, research roadmap — DONE, commits `996b73a`, `004680c`)

The research branch `research/distributed-inference-multi-worker` documented a
multi-phase execution-strategy roadmap (`docs/research/EXECUTION-STRATEGY-ROADMAP.md`).
**P1 — ExecutionStrategy foundation** is now implemented in `crates/fabric`,
without changing any existing behavior:

- `StrategyKind` — `SingleWorker`, `BatchFanOut`, plus 4 **gated experimental**
  kinds (`SpeculativeDraftVerify`, `DisaggregatedPrefillDecode`,
  `CacheAwareRoute`, `CollaborativeModel`) that the planner never emits today.
- `EvidenceProvenance` — MEASURED / ESTIMATED / INFERRED / EXPERIMENTAL /
  UNKNOWN (no fabricated measurements; missing data is UNKNOWN).
- `StrategyRationale` + `RejectedStrategy` — why a strategy was chosen and why
  alternatives were rejected.
- `ExecutionStrategy` + `CanRunReport` — every `PlanResult` and
  `ExecutionDecision` now carries the strategy and a per-worker
  CAN_RUN/CAN_COLLABORATE snapshot (serde-defaulted so pre-P1 persisted
  decisions deserialize cleanly).
- `CAN_RUN` mirrors the existing eligibility projection (trusted + healthy +
  serves the model); `CAN_COLLABORATE` is **deliberately conservative** —
  `false` for every worker today, because no engine DecentraAI runs advertises
  speculative / disaggregated / collaborative capabilities. Claiming
  collaboration the fabric cannot execute would be a lie.
- A real fan-out decision (engine-advertised staging + `allow_fanout` + ≥2
  ranked workers) carries `BatchFanOut`; everything else stays `SingleWorker`.

**P1 follow-up — Model-Fabric Execution Spec (`8c3df13`)**: the research branch
added `docs/research/MODEL-FABRIC-EXECUTION-SPEC.md` tying M11 Adaptive Compute
Fabric to ExecutionStrategy. Implemented as pure foundation (no behavior
change): `ExecutionMode` enum + `StrategyKind::execution_mode()` mapping (§1.3),
`MultiModelPipeline` as the 7th strategy kind, `TrustTier`
(`public`/`trusted-remote`/`trusted-cluster`) with per-tier strategy + mode
filtering (§4; planner must filter candidates by tier before scoring, KV/cache
migration across tiers disallowed), and `PlannerConfig` scoring profiles
(`latency_profile`/`throughput_profile`/`cost_profile`, §3.2 — hard constraints
are never overridden by scores). All pure; the planner still emits only
SingleWorker/BatchFanOut and no code path consults TrustTier today.

Tests: 1034 workspace tests green; clippy `-D warnings` clean; fmt clean.

**P1 follow-up — spec §2–§3 (`750cd6b`)**: `EngineCapabilities` extended with the
§2.1 flags (`continuous_batching`, `speculative_decoding`, `kv_offload`,
`prefix_cache`, `pipeline_parallel`, all serde-defaulted) + vLLM/SGLang probe
baselines; `StrategyKind::required_capabilities()`/`meets_capabilities()`;
`PerformanceProfile` (§2.2, all-optional metrics, missing = UNKNOWN,
`measured_count()`/`has_core_evidence()`); `base_score()` + `NormalizedMetrics`
+ `ScoringWeights` (§3.1, spec's base formula with latency/throughput presets).
All pure — the planner still emits only SingleWorker/BatchFanOut and no code
path consults the new primitives yet.

Tests: 1042 workspace tests green; clippy `-D warnings` clean; fmt clean.

**P1 follow-up — NetworkFacts (`cf27cb4`)**: `LinkMetrics` gains `jitter_us` +
`packet_loss_percent` (Option, serde back-compat) + `stability()` fold
(unmeasured = UNKNOWN = 0, conservative); `sort_peers` tie-breaks on
stability; `NetworkFacts` aggregates link + reach cost + stability per worker
(P2 shape, no planner consumer yet — waits for live jitter/loss measurement).

**P1 follow-up — Promotion gates (`931cbb9`)**: `PromotionEvidence` encodes the
§6 hard gates for moving a strategy from EXPERIMENTAL to BETA/PRODUCTION:
capabilities verified, net benefit proven, tiers enforced, threat model
reviewed, rollback tested. `promotable()`/`unmet()` make the decision explicit
and auditable.

Remaining research phases (P3 speculative, P4 prefill/decode, P5 cache-aware,
P6 collaborative/RPC) stay **experimental** — they require live LAN
measurements (Laptop ↔ Desktop) before the planner may select them.

## 118. Live LAN session 2026-08-19 — ops, P2 wiring, memory fix (DONE)

A full two-node session on real hardware, all verified live:

- **Ops — self-upgrade on schedule** (`1f0de5a`, `639398c`, `1ba6d30`):
  `decentraai upgrade check|apply|auto` + `node --auto-upgrade`; the systemd
  unit starts the node with `--auto-upgrade` and `WorkingDirectory` pinned to
  the repo; `scripts/upgrade-node.sh` patches the unit idempotently
  (`ENABLE_AUTO_UPGRADE=0` opt-out) so the Desktop got self-upgrade on its next
  local upgrade. Both nodes verified on `--auto-upgrade` (6h watcher).
- **P2 — NetworkFacts wired into the planner** (`b4b5602`):
  `ExecutionPlanner::network_score` folds measured jitter/packet-loss stability
  into the network term ONLY when measured — `(None, None)` stays neutral so
  links that only carry RTT keep the exact pre-P2 score; a measured flaky link
  loses up to 30% of its network score. Regression test pins the neutral case.
- **Bug fix — collective memory stopped growing after the first workflow run**
  (`979acbf`): `write_workflow_to_memory` derived `entry_id` from `plan_id` +
  `task_id`, both fixed across template instantiations (`research_report` →
  `workflow-run`), so the second run hit the `memory_entries` PRIMARY KEY and
  its writes were swallowed by `let _ =`. Fix: fold the run timestamp into
  every entry id. Verified live: `workflow_results` grew 6 → 11 entries on a
  real second run. Regression test
  `repeated_workflow_runs_each_write_memory_entries`.
- **Live validation** (all on real LAN): `validate-lan.sh` → reply `REMOTE`
  (forced remote to the Desktop-only model); chat streaming remote token-by-
  token; execution view shows P1 `single_worker` decisions with rationale;
  M19 network probe `rtt_ms: 174`; research_report workflow `verdict:
  completed`; P6 reputation fed from real executions (3 entries, score ~0.83,
  reasons reliability/quality/latency with sample counts).

Tests: 1056 workspace tests green; clippy `-D warnings` clean.

## 119. Tool Runtime — OCR + STT subprocesses (DONE)

Local AI tool subprocesses beyond TTS, same pattern (never FFI): an embedded
Python server written to `<data_dir>/tools/<tool>/server.py` at start, spawned
as a child on an ephemeral loopback port, health-probed, and proxied through
an authenticated `/v1/<tool>` endpoint. Missing setup (venv/model absent) never
fails startup — the node serves without the tool and logs a warning.

- **OCR** (`/v1/ocr`, `scripts/setup-ocr.sh`): RapidOCR (PP-OCRv4 on
  onnxruntime, CPU-friendly, models bundled in the wheel). Request body:
  `{"image_b64": "...", "lang": "en"}`; response: `{text, lines, boxes}`.
  50 MiB body cap; per-token rate limit shared with inference.
- **STT** (`/v1/stt`, `scripts/setup-stt.sh`): faster-whisper (CTranslate2,
  CPU int8). Request body: `{"audio_b64": "...", "lang": "ro"}`; response:
  `{text, language, duration_s}`. 100 MiB body cap; model `tiny|base|small|
  medium|large-v3`, HF cache pinned under the data dir via HF_HOME.
- **Generic `ToolServer`** (`crates/runtime/src/tools.rs`): shared subprocess
  lifecycle (write script → spawn venv python → wait `/health` → stop+kill on
  drop) used by both tools; TTS keeps its own manager untouched.
- **Config**: `[ocr]` and `[stt]` sections in `node.yaml`, both off by default
  (absent = disabled), `deny_unknown_fields` like every other section; example
  config documents all three tool sections.
- **Status**: `/status` reports `ocr.enabled/healthy` and `stt.enabled/
  healthy/model` for the dashboard.

Tests: 1061 workspace tests green (was 1056); clippy `-D warnings` clean.
New tests: config parse + deny-unknown-fields for both sections, status
defaults, TTS handler untouched.

## 120. Tool Runtime — HF Skills (transformers pipelines, DONE)

Small CPU-friendly HuggingFace pipelines as local tools, same never-FFI
subprocess pattern: one embedded Python server (`hf_skill_server.py`) hosts
all enabled skills, pipelines load lazily on first call, proxied through the
authenticated `/v1/skills/<id>` endpoint.

- **Skills**: `sentiment` (distilbert SST-2), `ner` (dslim/bert-base-NER),
  `summarize` (sshleifer/distilbart-cnn-12-6), `translate_ro_en` and
  `translate_en_ro` (Helsinki-NLP opus-mt). Models download on first use into
  `<data_dir>/tools/skills/models` via HF_HOME (setup script
  `scripts/setup-skills.sh`).
- **Config**: `[skills]` section (off by default, `deny_unknown_fields`); an
  unknown skill id in `skills.list` is a config-time error when enabled — a
  typo can never silently no-op. Disabled sections may keep unknown ids.
- **runtime_evidence (P8)**: the Skills view now reports per-skill
  `runtime_evidence: true` only when this node actually executes that skill
  id (the HF-skills subprocess runs it); the top-level flag flips when any
  skill runs. Declarations alone never count as evidence.
- **Status**: `/status` reports `skills.enabled/healthy/list`; the dashboard
  Tools card shows the HF Skills row.

Tests: 1063 workspace tests green; clippy `-D warnings` clean. New tests:
config parse + unknown-skill rejection, status defaults, `/v1/skills/<id>`
404 when disabled.
