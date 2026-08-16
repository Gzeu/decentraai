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

## 7. Subscriptions: free, tiered by contribution (done)
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
- [x] P4: contribution-based tier suggestions from catalog + reputation
- [x] P5: invites (`decentraai invite` prints a copy-pastable
  `<reachable-multiaddr>/p2p/<peer-id> <guest-token>` string; `decentraai join
  "<invite>"` parses it, auto-provisions identity + config, stores the Tier-1
  Guest token as the node's credential (`runtime/invite.token`, 0600) and
  verifies the coordinating peer is reachable over the verified P2P path)

## 8. Operations and scale (done)
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

## 9. Distributed Inference (M9) - IN PROGRESS → P0/P1/P3/P4/P5/P6

### P0 — Finish current fabric (LIVE two-node validation)
- [ ] Live Laptop→Desktop remote execution validation
- [ ] Verify exact worker pinning for remote routes
- [ ] Verify quota accounting with remote workers
- [ ] Verify provenance (audit, decisions, history) for remote executions
- [ ] Verify recovery behavior on remote worker failure
- [ ] Verify independent-request batch routing across local + remote
- [ ] Verify dashboard views for remote worker execution
- [ ] Preserve trust/security invariants end-to-end

Acceptance: all above marked [x] on a real two-node LAN fabric.

### P1 — ExecutionStrategy foundation
- [ ] Introduce `ExecutionStrategy` + `StrategyKind` in the planner
- [ ] Implement `CAN_RUN` / `CAN_COLLABORATE` predicates using WorkerFacts + capabilities
- [ ] Integrate provenance (MEASURED/ESTIMATED/INFERRED/EXPERIMENTAL/UNKNOWN) into strategies
- [ ] Add explainer for each decision (why selected vs rejected)

Acceptance: SingleWorker + existing BatchFanOut run through the new abstraction with unchanged behavior.

### P2 — NetworkFacts
- [ ] Implement RTT/bandwidth/jitter/packet-loss/stability probes between nodes
- [ ] Populate `NetworkGraph` with measured metrics (Unknown stays UNKNOWN)
- [ ] Feed NetworkFacts into strategy scoring for Single vs multi-worker

Acceptance: planner can show when a multi-worker strategy would be network-bound vs compute-bound.

### P3 — Speculative draft/verify (experimental)
- [ ] Implement `SpeculativeDraftVerify` as an experimental ExecutionStrategy behind a gate
- [ ] Configure Laptop (weak worker) with small draft model; Desktop (strong worker) as target
- [ ] Instrument baseline vs speculative runs:
      - tokens/s, TTFT, decode latency, acceptance rate, network transfer,
        CPU/GPU utilization, total latency, failures, energy/thermal where available

Acceptance: speculative path remains gated; only moves forward if MEASURED performance shows net benefit vs SingleWorker.

### P4 — Disaggregated prefill/decode (experimental)
- [ ] Evaluate integration with an existing backend (vLLM/SGLang/LMCache/NIXL)
- [ ] Implement `DisaggregatedPrefillDecode` as experimental strategy using that backend
- [ ] Run Desktop↔Laptop prefill/decode experiments with full metrics (similar to P3)

Acceptance: no custom KV engine; disaggregation remains gated until measured benefit.

### P5 — Cache-aware execution (experimental)
- [ ] Classify KV state as LOCAL/REPLICATED/TRANSFERABLE/REMOTE/UNKNOWN per session
- [ ] Integrate LMCache or prefix-caching backend in trusted clusters
- [ ] Implement `CacheAwareRoute` to stay/migrate based on measured benefit vs transfer cost

Acceptance: cache-aware routing does not violate privacy/trust; remains gated until measured benefit.

### P6 — Collaborative model / llama.cpp RPC (experimental/wait)
- [ ] Implement isolated collaborative-model harness using existing llama.cpp RPC experiment
- [ ] Benchmark Desktop only, Laptop only, Desktop+Laptop collaborative
- [ ] Compare prefill/decode metrics, TTFT, latency, bandwidth, VRAM/RAM, CPU/GPU, failure behavior

Acceptance: collaborative model execution enabled only if measurements show net benefit; 3–5 node tensor parallel remains WAIT.

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

... (remaining sections unchanged, see previous roadmap content)
