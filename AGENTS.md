# AGENTS.md — master prompt for DecentraAI development

You are continuing the development of **DecentraAI**, a decentralized
P2P network for distributing AI model artifacts and serving verifiable
inference. Read this file fully before writing any code.

## 1. What the project is (and is not)

DecentraAI lets trusted peers on a LAN share GGUF models with
cryptographic verification (BLAKE3 chunks + Merkle root + Ed25519
identity), and serves inference through a managed llama.cpp
`llama-server` subprocess behind an OpenAI-compatible local API with a
live web dashboard.

It is **not**: a public internet network (LAN/private swarm first), a
payment platform, a model training framework, or a wrapper around
llama.cpp internals (the engine is always an external process, never
FFI).

Current state: ROADMAP.md is fully done (M0–M8). The next roadmap
(subscription tiers, chat UI, admin dashboard) is in section 7 below.

## 2. Repository layout (9 workspace crates)

- `crates/config` — typed YAML config with strict validation (ports,
  loopback-only API, ranges). Tests cover every rule.
- `crates/identity` — Ed25519 keypairs, 0600 persistence, PeerId
  derivation. The libp2p keypair is derived from the node key.
- `crates/protocol` — message schemas (`deny_unknown_fields`, size caps,
  base64 binary fields), canonical signing (`sign_manifest` /
  `verify_manifest_signature`), catalog messages. Manifest/chunk
  responses carry NO signatures by design: integrity is anchored in the
  signed manifest's `chunk_hashes` + Merkle root, enforced per chunk at
  assembly.
- `crates/manifest` — GGUF magic check, 4 MiB chunks, BLAKE3,
  deterministic Merkle root over raw digests, atomic JSON writes.
- `crates/p2p` — libp2p actor (commands over a channel, never blocks the
  event loop), request/response codec, `transfer.rs` (per-chunk
  verification, `.part` staging + `.done` resume bitmap, Merkle gate,
  atomic rename, quarantine on corruption), `reputation.rs` (only
  cryptographic failures count toward bans; deterministic ranking score
  desc / PeerId asc), `RegistryServer` (catalog + manifests + chunks).
- `crates/registry` — local model registry with path safety (no
  symlink escape, no paths outside root).
- `crates/runtime` — llama-server process manager (health-probed,
  killed on drop), admission gate (RAM reserve, GPU policy, temperature),
  `api.rs` (thin axum proxy + Bearer auth + inference metrics + web
  dashboard; the dashboard NEVER polls the proxy — only `/status` and
  `/v1/peers` — so watching the page cannot reset the idle clock).
  `tools.rs` is the Tool Runtime: generic `ToolServer` subprocess lifecycle
  (embedded Python server → ephemeral loopback port → health probe →
  authenticated `/v1/<tool>` proxy) shared by OCR (`/v1/ocr`, RapidOCR),
  STT (`/v1/stt`, faster-whisper) and HF skills (`/v1/skills/<id>`, small
  transformers pipelines: sentiment/NER/summarize/translate ro↔en — the
  per-skill `runtime_evidence` flag in the Skills view is true only when this
  node actually executes the skill). Missing venv/models never fails startup —
  the node serves without the tool and logs a warning.
  `tool_calling.rs` (distributed) is the real tool-calling protocol: the
  agent executor exposes the spawned tools to the model as bindings, parses a
  fenced `[TOOL_CALL]` JSON block, executes the tool over loopback HTTP and
  re-asks with `[TOOL_RESULT]` injected (bounded rounds, malformed calls stop
  the loop).
- `crates/audit` — append-only JSON-lines security log
  (`logs/audit.jsonl`): peer bans, chunk verification failures,
  admission rejections, inference starts. Prompts and outputs are never
  audit material.
- `crates/compute` — pure compute-sharing domain (M11): `ComputeCapability`,
  `ComputeAvailability`/`ComputeAdvertisement`, `WorkloadRequirements`,
  `ResourceReservation`/`ReservationLedger`, `CapabilityMatcher`,
  `ComputeRegistry`, `ComputeScheduler`. No I/O, no async; all types
  serde-serializable for P2P transport. The scheduler answers "which node
  executes this workload?"; reservations are coordinator-side and TTL-bounded.
- `crates/system-probe` — hardware snapshots and admission decisions
  (RAM reserve is a hard floor, GPU temperature is a hard stop).
- `crates/node-cli` — the `decentraai` binary: `init`, `doctor`,
  `config validate`, `registry scan|list`, `swarm start`, `pull`,
  `serve start`.

## 3. Non-negotiable invariants

1. **Verify before use.** No artifact is used before hash + manifest +
   policy verification. Per-chunk BLAKE3, final full-file hash + Merkle
   root, atomic rename into `models/`.
2. **Only cryptographic failures punish peers.** Network errors never
   touch reputation scores. Corrupted chunks count toward a temporary
   ban AND quarantine the staging artifact with metadata.
3. **Determinism.** Canonical serialization for signing; scheduler
   ranking is score desc, PeerId asc; persistence is tmp+sync+rename.
4. **Secrets stay local.** `identity/key.pem` and `runtime/api.token`
   are mode 0600, never logged, never committed, never sent anywhere.
   The API binds to loopback (config validation rejects public binds).
5. **Prompts and outputs are never logged.** Audit records security
   events only, with best-effort writes that never break the main flow.
6. **The inference engine is a subprocess.** llama.cpp runs as
   `llama-server` with health probes and kill-on-drop; upgrades are
   binary swaps.

## 4. Coding conventions

- Rust 2024 edition, rust 1.85+. No `unsafe`. No new dependencies
  without justification in the commit message.
- Docs comments explain *why*, especially invariants and threat model.
- Every function that is a pure decision (budget derivation, admission,
  ranking, arg building) is separated from I/O so tests can drive it
  with synthetic inputs.
- Errors: `anyhow` with `.context()` at boundaries; `bail!` with a
  message a user can act on. Never `unwrap()` outside tests.
- Async: tokio. The p2p swarm is an actor; requests go through
  oneshot replies; handlers are `Arc<dyn RequestHandler>`.
- Naming mirrors the domain: manifest, chunk, catalog, reputation,
  admission, quarantine, audit.

## 5. Quality gates (must pass before every push)

```bash
git pull --rebase
git log --oneline -1   # confirm the expected commit is checked out
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Test suite baseline: 106+ tests, all green. E2E tests in
`crates/p2p/tests/e2e_transfer.rs` spin up real libp2p nodes on
loopback — keep them fast (<20s total) and deterministic (retry loops
only for connection settling, never for logic).

Every feature lands with tests: unit tests for pure logic, E2E for
protocol changes. A milestone is not done until its ROADMAP line is
checked AND the tests proving it are green.

## 6. Workflow that produced this repo

1. Discuss the milestone in chat; agree scope before code.
2. Push coherent file sets with a descriptive commit message (the
   "why", not the "what"). Update ROADMAP.md and README.md in the same
   push as the feature.
3. The user verifies locally with the gates above and reports the
   output; fix-forward, never amend published history.
4. When a user-reported bug appears (e.g. dashboard self-poll inflating
   counters), fix it AND add the test that would have caught it.

## 7. Next roadmap (agreed direction)

Subscription model: **everything is free; your tier reflects your
contribution**. Admin-only token issuance from a dashboard.

- **P1 — Token registry + tiered auth**: `db/tokens.json` stores
  BLAKE3-hashed tokens → {name, tier, created, revoked}; CLI
  `decentraai token create|list|revoke` (admin token only); proxy
  resolves token → tier → per-tier model allowlist + in-memory rate
  limiting; audit `token_created`, `token_revoked`, `rate_limited`.
- **P2 — Chat UI**: `/chat` page in the dashboard; model selector
  filtered by the caller's tier; token stored in localStorage;
  streamed chat (SSE) is done.
- **P3 — Admin dashboard**: `/admin` behind the master token; create /
  revoke tokens, set tiers, usage per token, peer catalogs; everything
  audited.
- **P4 — Contribution-based tiers (done)**: `decentraai tier apply`
  writes M17's measured contribution suggestions into `db/tokens.json`,
  pairing each token to the worker of the same name. Dry-run by default,
  `--yes` to apply; each change records a `tier_changed` audit event.
  `tier suggest` stays read-only.
- **P5 — Invites & join**: admin generates an invite (bootstrap
  multiaddr + Tier-1 token); `decentraai join <invite>` bootstraps a
  fresh node. Live-validated end-to-end on LAN (fix `b644278`: invite must
  carry the libp2p peer id, not the identity hex).
- **M9 / M18 — Distributed inference**: real P2P routing of inference between
  universal nodes (see the M18 foundation above). Reputation-based
  compensation for workers (M9-9) is wired as a live contribution-credits
  ledger (fix `3b6fe90`): `CompensationLedger` credits verified work
  idempotently per execution, exposed via `get_compensation` MCP + the
  `compensation_earned` column; synthetic bookkeeping, never money.

### Execution Fabric — M18 + M19 + M20 (verified-DONE); M21–M24 NEXT

`decentraai-fabric` (pure, no I/O): the engine-neutral execution planner.
`ExecutionPlan` (single/sequential/fan-out) + fallback; `reserve_worker`
(planner owns *who*, scheduler enforces capacity); `plan_and_reserve`
integrated into `route_request`/`route_request_streamed`. The **M18 foundation
is verified on real LAN hardware (Desktop ↔ Laptop universal nodes):** trusted
admission → fabric planner → reservation → P2P `InferRequest` → remote worker's
local llama-server (loopback, kept alive — never idle-unloaded) → streamed
response → reservation release. Worker reuse, concurrent requests and
bidirectional execution all proven; the loopback backend URL is never
advertised as a remote endpoint.

The crate also holds building blocks that were once **parked as NEXT
milestones (M21–M23)**; they are now **wired into the live planner and
regression-tested** (do not mark *done* — no engine DecentraAI runs
advertises expert routing, so the split path is reachable but not
production-verified):
- **M21** `ExpertRegistry`/`ExpertRouter` behind `expert_routing`: wired in
  `ExecutionPlanner::build_stage`, which passes **all eligible candidates** to
  the router (fix `ae42e0a` — a single-candidate call could never produce a
  split). No engine advertises the capability, so the honest whole-model
  fallback is what runs; `expert_capable_worker_routes_to_expert_split` +
  `non_expert_engine_keeps_honest_whole_model_reasoning` pin both sides.
- **M22** `EngineKind` + capability probe: `ComputeManager::fabric_facts`
  parses each advertisement's engine string and feeds `WorkerFacts.engine` +
  `advertised_capabilities()` to the planner (`engine_kind_capabilities_drive_worker_facts`).
- **M23** `ExecutionPlanner`: the live single-worker selector behind
  `plan_and_reserve`; see ROADMAP §19 for the exact scope (not full autonomy).
M24 (resilience) has landed its remaining production gaps
(see below) and is now considered wired: the coordinator reaper
`reap_unhealthy`, reservation TTLs, stale/offline worker eviction with audit,
graceful + startup recovery, mDNS recovery, false-ready prevention, engine
crash auto-recovery, **bounded idempotency-safe request retry**, and an
**explicit bounded P2P reconnect loop** all exist.

**M24 (resilience) is wired:**
- Coordinator reaper, reservation TTLs, stale/offline eviction with audit,
  graceful + startup recovery, mDNS recovery.
- **False-ready prevention** (`7b22dbf`): the compute broadcaster gates worker
  advertisement on live engine health.
- **Engine crash auto-recovery** (`a4aa762`): `ServeManager::ensure_healthy`
  respawns a crashed llama-server from a stored restart spec via a periodic
  supervisor.
- **Bounded, idempotency-safe request retry**: `route_request` retries
  transport-level failures (P2P connection / timeout) on a fresh planner-chosen
  worker up to `config.max_retries`, with exponential backoff via
  `FallbackHandler`, releasing each attempt's reservation and re-planning per
  attempt. `DistributedError::is_retryable()` encodes the policy: a definitive
  worker rejection or a cancelled request is **never** re-sent — so
  non-idempotent work (re-generation, double token/KV accounting) is never
  duplicated. The streaming path intentionally stays single-attempt + legacy
  fallback (retrying mid-stream would duplicate partial output to the client).
- **Explicit bounded P2P reconnect loop**: on `ConnectionClosed` the swarm
  re-dials a peer whose last address is known, with exponential backoff capped
  at `RECONNECT_MAX_ATTEMPTS` (then it relies on mDNS re-discovery). Addresses
  are captured at mDNS discovery and on dialer connect.

**M20 (KV-aware inference fabric) is verified-DONE** (commit `caf9121`):
coordinator-side KV/session accounting (`SessionAccount`), continuation
affinity, and KV-aware planner inputs (`ServedModel.context_tokens` →
`KVCacheState`, `is_continuation` / `prefix_resident_on`) are implemented and
wired into the real `plan_and_reserve` route path; live Desktop → Laptop
requests confirm the planner consumes real KV state, with
reservations/streaming/release intact. KV occupancy is accounted
coordinator-side from real `tokens_used` + advertised `n_ctx` — live
llama-server KV occupancy telemetry is **not** claimed, and prefill/decode
split stays gated behind `prefill_decode_separation` (not run by any engine).

**M19 (network-aware scheduler) is verified-DONE** on real Desktop ↔ Laptop
LAN hardware: the `NetworkGraph` + `InferPing/Pong` RTT probe measures live
round-trip time to the remote worker every 5s and folds measured reach cost
into `ExecutionPlanner::score` (`net_score`), steering worker selection on the
real link. Completion: `5bc0c17`. M19 is the source of the network term in
planner scoring that M20's KV placement will combine with.

Q4: `decentraai setup` — detect hardware → identity → auto-select model →
write validated config → READY. Idempotent; verified end-to-end on Ubuntu.

### Collective Intelligence — P0–P11 fabric + orchestrator + workflows (DONE)

Direction (agreed 2026-08-17, see `docs/COLLECTIVE_INTELLIGENCE.md`): DecentraAI
evolves from a distributed inference fabric into a collective-intelligence
infrastructure. **An agent is a logical execution context on a node — not a new
process.** The full fabric is implemented in `crates/agents` (pure, no I/O)
plus the runtime half in `decentraai-distributed`:

- **P0 — agent substrate**: `AgentRecord`/`AgentRegistry`/`AgentTask`/
  `ToolDescriptor`/`AgentAdvertisement`; the **unified capability matcher**
  (one compositional verdict: hub provenance-aware semantic gate + agent model
  allowlist + compute physical gate — the two capability languages are
  cross-wired). `SignedAgentAdvertisement` (anti-spoof), `AgentManager`,
  `/v1/agents` + dashboard AGENTS view.
- **P1 — signed discovery**: `SignedAgentAdvertisement` in protocol; agents
  advertised over the P2P heartbeat; E2E two-node signed advertisement
  exchange.
- **P2 — messaging**: `AgentMessage` (ask/delegate/reply/verify/ping) +
  bounded `AgentInbox`; `AgentMessenger` bridges to the libp2p request/response
  channel with **self-delivery** (a single-node workflow must not depend on
  libp2p self-dial).
- **P3 — delegation DAG**: `DelegationPlan`/`DelegationPlanner`/`execute_plan`;
  per-hop verification on the JSON *value* (not its serialization). An
  unroutable capability rejects the plan — never invents an executor.
- **P4 — verification/consensus**: `VerificationReport`/`CheckKind`, honest
  structural `check_output_schema`, `ConsensusPolicy`/`evaluate_consensus`,
  `DisagreementResolution`, bounded immutable `VerificationLedger`.
- **P5 — collective memory**: `MemoryLevel`/`MemoryAccess`/`MemoryPolicy`/
  `MemoryEntry`/`MemoryScope`, `can_read`/`can_write` (ownership + access +
  trust + provenance), `enforce_retention`, `MemoryRegistry`; SQLite
  `MemoryStore` in distributed (persistent, access-enforcing).
- **P6 — reputation**: per-(agent, capability) `AgentReputation` with factors
  (reliability/quality/latency/uptime/safety/provenance), EMA `ReputationStore`,
  deterministic `best_for_capability`, `safety_penalty` (policy/crypto only —
  network errors never touch safety; unknown reputation = 0, not a penalty).
- **P7 — policy**: `PolicyEngine` — explicit Allow/Deny for tools/models/peers/
  budgets/egress + the Controlled-Exploration boundary (Normal/Exploration/
  Experimental). **Agent Power ≠ Permission.**
- **P8 — talent tree**: dynamic capability graph (`TalentNode`/`TalentTree`/
  `can_unlock`/`resolve_path`/`available_capabilities`), no fixed levels.
- **P9 — collective workflows**: `WorkflowTemplate`/`WorkflowStep`/
  `run_workflow`/`WorkflowOutcome`; `research_report_template`
  (Research → Finance → Documents → Synthesis).
- **P10 — self-optimization**: `SelfOptimizer` (weighted observations →
  Increase/Decrease/Rebalance under hard/soft constraints).
- **P11 — economy**: `CapabilityOffer`/`BookingRequest`/`negotiate`/
  `EconomyLedger` — non-monetary, modular.
- **P12 — collective knowledge & decisions v1 (DONE)**: the closed evidence
  loop `KnowledgeObject → CollectiveDecision → memory feedback →
  VerifiedComputeReceipt → CompensationLedger → evidence → KnowledgeObject`.
  `crates/agents` holds the pure fabric (`knowledge.rs`, `decision.rs`,
  `receipt.rs`): knowledge confidence is **derived from evidence, never
  declared** (no evidence → 0.0); decisions delegate the vote to the single
  `evaluate_consensus` language; receipts are idempotent per execution id and
  credit compensation for verified work only. `crates/distributed` holds
  `knowledge_runtime.rs`, which shares the authoritative compensation ledger
  with the compute manager, persists feedback into the `collective.knowledge`
  memory scope, and seeds per-worker contribution profiles at wiring (never
  from an HTTP body — unknown workers earn 0 honestly). API: `GET
  /v1/knowledge` + `POST /v1/knowledge/receipt` + `POST
  /v1/knowledge/decide` (operator+); the dashboard has a Knowledge view.
- **Evidence RAG (experimental memory, DONE)** — "what have we learned?":
  `crates/agents/src/evidence.rs` (pure) is the deterministic index over five
  evidence families (benchmark/execution/receipt/memory/consensus) with two
  honest query paths — structural (keyword/tag, always available) and semantic
  (cosine over real embeddings only, never a fake score) — plus derived
  `lessons()` (success rate, median duration/RTT, verified-work rate, adoption
  rate; zero evidence in, zero lessons out). `crates/distributed/src/
  evidence_manager.rs` syncs idempotently from live sources (`ComputeManager`
  executions, `KnowledgeRuntime` receipts/decisions, `MemoryStore` collective
  scopes). API: `GET /v1/evidence` + `POST /v1/evidence/query` (operator+,
  lazy sync at request time); the dashboard has an Evidence view. Evidence
  carries **facts, never prompts/outputs**.
- **Benchmark Lab (DONE)** — "does the collective beat a single agent?":
  `crates/agents/src/benchmark.rs` (pure) is the deterministic task/run
  registry (Single/RAG/Collective modes, `grade_answer` on normalized gold,
  `Abstained` on missing gold/empty output) with honest gates — a
  `collective_beats_single` verdict needs **MIN_SAMPLES=5 graded runs per mode
  and a MIN_MARGIN=0.05 accuracy delta**, otherwise "not enough samples".
  `crates/distributed/src/benchmark_manager.rs` (runtime) runs tasks through
  the live inference executor via the `BenchmarkInference` trait (collective
  = N generations, plurality vote on grades, ties → Abstained) and feeds every
  run into the Evidence RAG as `EvidenceFamily::Benchmark` (facts only).
  API: `GET /v1/bench` + `POST /v1/bench/run` (operator+; real tokens);
  the dashboard has a Bench view. The verdict is a hypothesis about this
  fabric on this hardware, never a universal claim.

**Runtime half (`decentraai-distributed`)**:
- `AgentOrchestrator`: binds the pure fabric to the live P2P channel —
  plan (`DelegationPlanner`) → reputation-ranked executor selection (local
  first; a stage with no capability requirements is eligible on any agent) →
  delegate via `AgentMessage::Delegate` → per-hop verify → collect.
  `orchestrate_plan(plan, seed)` runs an instantiated workflow with the user
  prompt injected into every stage.
- `AgentRuntime`: remote-side executor — drains an agent's inbox, runs each
  `Delegate` through an injected async `AgentExecutor` and replies.
  `InferenceAgentExecutor` runs delegated LLM tasks either against the node's
  **live local backend over HTTP** (single-node; distributed `route_request`
  cannot self-route) or via `route_request` (multi-node).
- **Node daemon is a live agent host**: `decentraai node` wires the messenger,
  spawns a production `AgentRuntime` per local agent with the inference
  executor, opens the SQLite `MemoryStore`, and shares the orchestrator into
  the API.
- **`POST /v1/agents/orchestrate`** (`{ prompt, template? }`) runs a collective
  workflow on the node's own agents; the dashboard AGENTS view has a
  Collective-workflow runner.

**Verified live**: `research_report` on a real node with Llama-3.2-1B returns
`verdict: completed`, all four stages verified, real generated report text.
`node.model` selects the served GGUF model explicitly (hard error on a typo).

Next (documented): two-node LAN validation — Desktop must
`git pull && bash scripts/upgrade-node.sh` (old builds omit agent
advertisements + `accepts_remote_inference`), then `bash scripts/validate-lan.sh`
on the coordinating laptop. Collective memory written from workflows and
reputation fed from real results are also open follow-ups.

Productization (installable app): `decentraai node` is the one-process
background daemon (auto-provision identity/config, LAN/P2P discovery +
verified auto-share, auto-select + serve model, dashboard/API bound
immediately — control plane up even while the model loads or faults).
`decentraai open` launches the dashboard. `deploy/decentraai-node.service`
(systemd *user* unit, auto-start + restart) + `scripts/install-app.sh` /
`scripts/uninstall-app.sh` + `deploy/decentraai.desktop` install a normal-user
app. Run/stop: `systemctl --user {start,stop,restart,status} decentraai-node`;
logs: `journalctl --user -u decentraai-node -f`.

User interface (ONE app): the product has a single user-facing UI — the embedded
dashboard served by the node from `crates/runtime/src/api.rs`. It is the one
control plane; there is no separate frontend project (the old SvelteKit
`frontend/` was obsolete — never served/built by the node — and was removed).
The dashboard renders Overview, Chat (`/v1/chat/completions`), Workers, Network,
Execution (planner decisions with network/KV reasons), Models, Settings and
Diagnostics in the embedded HTML, all from real runtime state surfaced by
`/status`, `/v1/compute`, `/v1/network`, `/v1/execution` and `/v1/peers` — no
mock data, watching the page never touches the backend. Chat history is kept
in-page for the session (server-side conversation persistence is not wired).
Chat streams by default: the proxy detects `stream:true` and forwards
llama-server's SSE body chunk-by-chunk (a channel that drops early on client
disconnect), and the page's JS reads the stream incrementally and surfaces
latency + tokens from the trailing `usage` event; the `stream` checkbox offers
a non-streaming fallback. The dashboard is split into a normal-user view
(Model, Inference, Chat, Queue, Recent inference, System) and an opt-in "Show
advanced" block (reputation, Workers, Network, Execution, Models, Settings,
Diagnostics, audit events, share guide), so distributed-compute complexity is
hidden unless the operator wants it.

Admin (token create/list/revoke, `/admin`) is gated on the master API token
via `ApiState::require_master` — subscriber tokens and unauthenticated callers
are rejected (401/403), so the security event that the admin page previously
omitted (it classified but discarded the auth result) is now enforced.

Tier semantics: Tier 1 Guest (invited, small/public models, tight rate
limit), Tier 2 Contributor (shares ≥1 verified model), Tier 3 Core
(shares large/multiple models, clean reputation). Tiers are earned by
sharing, measured with the existing catalog + reputation primitives.

## 8. Pitfalls already hit (do not repeat)

- Bash treats `<PORT>` in example multiaddrs as redirection — always
  show real, copy-pastable addresses in docs.
- libp2p refuses self-dial: single-machine pull tests need a second
  data dir with a second identity.
- Dashboard JavaScript must never call proxied endpoints — it once
  inflated the request counter by ~10k and permanently reset the idle
  clock.
- `admit_inference` originally compared free RAM against the derived
  budget, which can never fail; compare against the absolute reserve.
- New cross-crate references need the dependency declared in that
  crate's Cargo.toml (compile error E0433 otherwise).
