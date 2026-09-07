# ROADMAP HISTORY — detailed milestone record (M9–M24, P0–P11, Q4)

> This file preserves the full historical roadmap detail that used to live
> in AGENTS.md §7. The current forward-looking roadmap lives at the bottom of
> AGENTS.md ("Where we are heading"). Nothing below has been deleted; it is
> the archaeological record of how the fabric was built.

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
  `Abstained` on missing gold/empty output) with honest gates — the headline
  `comparison()` is **paired over tasks graded in BOTH single and collective**
  (a global per-mode aggregate is contaminated when modes see different
  tasks), and a `collective_beats_single` verdict needs **MIN_SAMPLES=5 shared
  graded tasks and a MIN_MARGIN=0.05 accuracy delta**, otherwise "not enough
  samples". `crates/distributed/src/benchmark_manager.rs` (runtime) runs tasks
  through the live inference executor via the `BenchmarkInference` trait
  (collective = N generations, plurality vote on grades, ties → Abstained)
  and feeds every run into the Evidence RAG as `EvidenceFamily::Benchmark`
  (facts only). API: `GET /v1/bench` (paired + global) + `POST /v1/bench/run`
  (operator+; real tokens); the dashboard has a Bench view with paired
  headline KPIs. The verdict is a hypothesis about this fabric on this
  hardware, never a universal claim.
  **Dataset adapter (F1)**: `scripts/bench-browsecomp-plus.py` downloads and
  de-obfuscates BrowseComp-Plus (MIT, fixed 100K-doc corpus, 830 reasoning
  queries) into `bench/browsecomp_plus.jsonl`; `crates/distributed/src/
  benchmark_datasets.rs` reads it into tasks (deduped/truncated evidence
  passages — the corpus averages 32K chars/doc). CLI: `decentraai bench run`
  and `decentraai bench dataset --file … --limit N --mode both --agents N`
  (both = single + collective per task, the only honest pairing), which runs
  a batch through the live node and prints the paired comparison. First live
  data (1B model, 5 shared tasks): single 0% vs collective 20% (+20pp,
  honest verdict); on 3 tasks both 0% → no meaningful margin. RAG with real
  evidence docs graded 67% vs 0% without retrieval — evidence suggests
  retrieval, not agent count, is the decisive factor on deep-research tasks.
  Benchmark lessons (`bench/*_accuracy`, `bench/median_latency_ms`) are
  derived in the Evidence RAG's `lessons()`.

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

## Primordial Mind → World initiative → first economic cycles (2026-09)

Agent Proposal & Experiment Protocol v0.1 landed first (#84, DECOUPLED
FIRST: pure cognitive core, deny-by-default economics), then grew in
vertical slices, each proven live before the next: v0.2 bounded testnet
lane (replay-first executor, cumulative budget) → v0.3 autonomous
selection (agent scores and picks; verdicts inferred, never declared) →
v0.4 experiment CONSTRUCTION from signals/deltas → v0.5 persistent
research journal (longitudinal families) → v0.6 multi-lens consensus
(generative/conservative/skeptic, +1000bp agreement uplift).

World bridge (#89, vertical slice): `GET /v1/world` becomes a typed
observation, the selected research becomes a REAL World mission
(`POST /v1/world/mission`, 409-keep, never a parallel sim).

Autonomous loop v0.7 (#90, `84a83af`, 1806 tests): incremental World
consumption (`WorldCursor` + information-gain gate + SKIP on steady),
persistent agent activity (Exploring/Working/Trading/Resting), lenses
mapped onto REAL entity ids (no fakes), append-only research graph with
stable ids, economy feedback (refuted ≠ failure), cheapest-service
recording, atomic persists on every boundary (cursor advances even on
deny — live lesson world-003/004).

M15 pressure-trigger (#91, `79ff393`): the trigger lives INSIDE the node
(tick-hook → pure deterministic detector in `proposal::pressure` →
existing loop as bounded child). The trigger path can NEVER pass
`--enable-live-testnet` (asserted in tests); token travels via env, never
argv. Live on prod: baseline tick 8, quiet skips, tick 13 FIRED
(new-events + tick-drift) → `cycle:trigger-13` ran itself → evidence
Supported → mission-adopted skips after. Trigger left ENABLED on prod
(budget 100 wei = read-only-only children, cooldown 20). Live lesson:
the service boots via `node_start`, not `serve_common` — attach lives on
both paths (`d9785aa`).

First testnet economic cycles (wallet ops #92, `fbd41df`): fresh wallet
`erd18ju7q8fluce5ns4y8tze7k4au4yrj47csfkph9lgpyqg0rnzculqxl2j0x` (seed
0600, existing wallet kept separate), faucet/drip funded. Cycle 1:
100 wei self-transfer, TX `f7825a44…03da`, success, independently
verified (sender = receiver, value 100, fee 0.00005, nonce 0→1).
Cycle 2: the agent once honestly chose read-only despite the armed flag
(budget is a ceiling, not an order), then probe-250, TX `051793bb…75c6`,
success, 250 wei. Lane disarmed after (per-invocation flag; trigger
cannot arm it).

Hardening sprint (#93, `28e8737`, 1835 tests): F1 typed `tx_status`
errors (sweep matches the 404 VARIANT, never text) + F2 resubmit cap
(`OnChainProof::resubmit_count`, MAX 3, terminal Failed + sweep routing,
single lock scope); wallet hardening (OsRng parity + seed zeroize);
`backup-node.sh` / `restore-node.sh` (manifest + automated secret-shape
audit — caught raw `dca_` keys in `consumer_keys.json` live; restore
refuses on live node, proven on scratch); findings doc adopted from PR
#83 (closed superseded — branch predated the fmt toolchain).

External audit PR #94 (L3/L4, OPEN, not merged): reviewed on-record —
patches `crates/tokens/src/tokenomics.rs`, which is NOT compiled
(not declared in `lib.rs`; 0 tests execute), so the fix is a no-op and
its tests never run in CI; the report marks 7 items remediated with 1
file patched and mixes 12 vs 16 numbering. SEC-15's math is sound;
SEC-01 (spawn race, low severity, guards exist) and SEC-02
(`unsigned_abs` nit, unreachable) concern live code and were assessed
honestly. `mergeable:false` observed once was transient (main moved
under the PR base); status at review: CLEAN. Left to its author.

Standing profile after this chapter: 1835 tests green, clippy + fmt
clean, CI green; prod node on systemd with research trigger on; World
tick 30+, mission `task-0022`; trigger day-0 baseline saved
(`experiments/trigger-baseline-2026-09-06.json`, 7-day report due
2026-09-13).

### Post-M17 — Collective Memory, N-of-M Consensus, P2P Durability (DONE 2026-09-07)

All 13 planned items delivered in a single session. Workspace at ~2000+
tests, 82 suites, clippy clean.

#### Collective Memory — Sync, Bridge, Conflict Resolution

- **Config-based sync gating:** `MemorySyncSection` in YAML config
  (`memory_sync.enabled`, `interval_secs`, `on_write`, `max_peers`).
  Legacy `DECENTRAAI_MEMORY_PROPAGATE=1` env fallback preserved.
  `Arc<AtomicBool>` write-trigger flag for immediate propagation on
  eligible writes.
- **MCP tools:** `memory_list_scopes`, `memory_read_entries`,
  `memory_write_entry` — consumer-key gated via `memory` scope. Added to
  consumer permission check, dispatch chain, and `discover_capabilities`
  scope mapping.
- **E2E tests:** `mcp_collective_memory_list_write_read_flow` (full CRUD
  flow: list → write → read → dup rejection → bad scope),
  `memory_write_triggers_propagation_flag`.
- **Bridge personal ↔ collective:** `memory_bridge.rs` module with
  `BridgeMapping` (category → collective scope config), `mirror_write()`
  (personal write → collective scope), `read_bridged()` (collective
  entries → augmented snapshot), `ensure_bridged_scopes()` (startup).
  Default mapping: lessons→`agent.lessons`, decisions→`agent.decisions`,
  experiences→`agent.experiences`. Wired into consumer handler write path
  (mirror after success) and read path (augment with `bridged_collective`).
- **Conflict resolution MCP tools:** `memory_list_conflicts` (groups
  entries by `subject_key`, ranks claims by status strength/confidence/
  timestamp, read-only), `memory_resolve_conflict` (marks losing claims
  as `Obsolete` via `transition_status`, destructive annotation). Both
  gated by `memory` scope.

#### N-of-M Consensus — Per-Replica State Machine

- **Store extensions:** `ReplicaAssignment`, `ReplicaResult`,
  `ConsensusOutcome` structs; `AssignmentStore` extended with
  `replica_items`, `replica_results`, `consensus_outcomes` HashMaps.
  Methods: `init_replicas()`, `claim_replica()`, `submit_replica()`,
  `try_consensus()` (quorum-aware, cached), `replicas()`, `consensus()`.
- **MCP wiring:** `orchestration_propose_resp` auto-calls
  `init_replicas()` when `replicas > 1`; MCP `orchestrate_status` shows
  `replica_details[]` + `consensus` object per stage.
- **Tests:** 4 unit tests — `replica_lifecycle_init_claim_submit_consensus`,
  `replica_disagreement_rejected`, `replica_uninitialized_errors`,
  `init_replicas_rejects_duplicate`.

#### P2P Durability — Persistence, Partition Detection, Adaptive Reconnect

- **known_addresses persistence:** `load_known_addresses()` /
  `save_known_addresses()` (JSON, atomic tmp+rename). `NetworkConfig.
  data_dir: Option<PathBuf>`. Event loop loads at startup, saves on every
  insert (mDNS, disconnect, Kademlia). node-cli wired with
  `data_dir: Some(data_dir.clone())`.
- **Transfer resume:** bitmap persistence at `staging/<manifest_id>.done`
  (already existed).
- **Partition detection:** `partition_detected_at: Option<Instant>` in
  event loop. Set when 0 connected peers + all reconnect budgets exhausted.
  Cleared on any new `ConnectionEstablished`. Exposed as
  `partition_detected: bool` in `PeersSnapshot`. E2E test:
  `partition_detection_e2e`.
- **Adaptive reconnect (LAN vs WAN):** LAN detection via
  `is_lan_address()` (private IPs, loopback, link-local). LAN:
  `RECONNECT_MAX_ATTEMPTS_LAN=3`, `RECONNECT_BASE_BACKOFF_LAN_MS=200`.
  WAN: `RECONNECT_MAX_ATTEMPTS=5`, `RECONNECT_BASE_BACKOFF_MS=500`. Unit
  tests for `is_lan_address` and `known_addresses_roundtrip_persistence`.

### M19 — Memory Bridge Durability (DONE 2026-09-07)

Multi-node sync for personal→collective bridge entries.

- **Config:** `bridge_sync: bool` in `MemorySyncSection` (default: false).
  When enabled, bridge scopes are propagation-eligible and mirrored
  entries are auto-promoted to Verified for cross-node sync.
- **Scope creation:** `ensure_bridged_scopes` creates scopes with
  `level=Network` + `allow_remote_write=true` when bridge_sync=true
  (propagation-eligible); `level=Node` when false (local-only). Both
  modes require `allow_remote_write=true` because the bridge writes as
  the agent (not scope owner).
- **Entry promotion:** `mirror_write` sets `MemoryStatus::Verified` on
  mirrored entries when bridge_sync=true, making them travel-worthy for
  the memory propagator.
- **Node-cli wiring:** reads `config.memory_sync.bridge_sync`, creates
  default bridge mapping, calls `state.attach_memory_bridge(mapping,
  bridge_sync)`.
- **Tests:** 7 unit tests (scope level, entry status), 1 E2E test
  (`bridge_synced_entries_travel_to_remote_peer` — two real P2P nodes,
  bridge entry propagates and lands as Candidate on receiver).
- **Standing:** 82 suites, 0 failures, clippy clean.

### M21 — Tensor Parallelism (DONE 2026-09-07)

Multi-GPU tensor parallelism for vLLM/Sglang with automatic GPU detection.

- **EngineCapabilities:** `tensor_parallel: bool` → `tensor_parallel: Option<u8>`
  across all crates (fabric, inference-adapter). `None` = no support;
  `Some(n)` = n-way TP. Backward-compatible deserializer accepts legacy
  `bool` format (`false` → `None`, `true` → `Some(1)`).
- **GPU detection:** `detect_gpu_count()` in runtime: checks
  `CUDA_VISIBLE_DEVICES` (comma-separated device IDs → count), then
  `nvidia-smi --query-gpu=index --format=csv,noheader`, falls back to
  `None`. `resolve_tensor_parallel_degree(config_override)` applies the
  operator override or auto-detects.
- **Config:** `tensor_parallel_degree: Option<u8>` in `InferenceSection`.
  Explicit `Some(0)` disables TP; `Some(n)` forces n-way; `None` = auto.
- **Engine launch:** vLLM gets `-tp=N`, Sglang gets `--tp=N` injected
  into `RuntimeConfig.extra_args` when TP degree ≥ 2 and engine matches.
  Both local `serve start` and `decentraai-worker` paths wired.
- **Planner:** `meets_capabilities` compares `Option<u8>` TP degree
  (works via `PartialOrd` on `Option`); `CollaborativeModel` requires
  `tensor_parallel: Some(1)`.
- **Probe:** `probe_capabilities` for vLLM/Sglang returns `Some(1)`
  (conservative sentinel for "supports TP, degree unknown until launched
  with explicit `-tp`").
- **Tests:** 3 new — `resolve_tensor_parallel_degree_explicit_override`,
  `server_args_injects_tensor_parallel_flag`, `server_args_sglang_tp_format`.
  Golden corpus backward-compat verified (1916 passed, 0 failed).
- **Landing page:** updated to show M21 tag instead of ROADMAP placeholder.
- **Standing:** 82 suites, 1916 passed, 0 failed, clippy clean.

