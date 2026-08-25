# DecentraAI — Product Documentation

> **Status: Production-grade compute fabric, validated live on a 3-node deployment.**
> This document describes DecentraAI as a product: what it is, what it can do
> today (with measured results), how it recovers from failure, and how to run
> it. No features are described that are not implemented and demonstrated.

---

## 1. Product Overview

**DecentraAI is a cooperative AI compute fabric.** Independent machines — a
VPS, a desktop workstation and a laptop — pool their CPU into one shared
fabric where AI workloads run across nodes instead of on a single machine.

The problem it solves: most capable AI models need more compute than one
personal machine has, while nearby machines sit mostly idle. Cloud APIs solve
this by renting someone else's datacenter at metered prices and with no
visibility into execution. DecentraAI solves it cooperatively: the fabric
borrows capacity from its own trusted peers when local pressure demands it,
executes transparently, proves every step in an evidence chain, and pays
contributors in verified credit.

What makes it different from "distributed computing with extra steps":

- **Autonomy**: the fabric watches itself. When pressure crosses thresholds it
  borrows compute without an operator submitting anything.
- **Honesty**: failures are never hidden. An empty or incomplete result is
  reported as incomplete — never fabricated.
- **Attribution**: every unit of remote work is measured and credited through
  a ledger, so contribution becomes visible economic value.

Everything below is implemented, running on real hardware, and measured.

---

## 2. Architecture

DecentraAI is organized as layers over one invariant:

> **AI proposes → deterministic Rust decides → workers execute.**

No LLM ever selects a worker, mutates trust, issues credentials or bypasses a
reservation.

| Layer | Role |
|-------|------|
| **Core** | Node runtime: llama-server process management, OpenAI-compatible API, dashboard, CLI |
| **Protocol / DFCP** | Wire negotiation between peers: request → offer → reserve → assign → result → release, with leases |
| **P2P** | libp2p transport, verified model transfer (BLAKE3 chunks + Merkle + Ed25519), reputation (only cryptographic failures punish) |
| **Compute Fabric** | Worker discovery, capability routing, reservations, batched embeddings, chat batch, map/reduce distributed inference |
| **Model Colony** | Model selection per task: capability match + RAM fit + measured evidence |
| **Governor** | Resource-aware decision engine: LOCAL / DISTRIBUTED / QUEUE / REJECT, autonomous pressure loop |
| **Memory** | Collective memory scopes; model performance observations feed back into Model Colony |
| **EvidenceChain** | Append-only record of decisions, assignments, failures, releases, replans, completions and reduce results |
| **Economy** | Verified contribution → credit ledger → reward policy (synthetic bookkeeping; never money) |
| **Providers** | Local llama.cpp first; external providers isolated behind credentials |
| **BYOA Gateway** | External agents drive the fabric with scoped consumer keys (`dca_…`) under quota and rate limits |
| **Collective Agents** | Multi-stage workflows where each stage drives the Governor |

17 workspace crates implement these layers; the heavy lifting lives in
`runtime`, `distributed`, `compute`, `agents`, `fabric`, `protocol` and `p2p`.

---

## 3. Compute Fabric

### What it does

- **Worker discovery**: peers find each other over LAN/P2P, advertise
  capabilities (models served, RAM, embeddings backend) and connect only
  through the trust chain.
- **Reservations & leases**: capacity (CPU/RAM) is reserved before work runs;
  every lease expires; release happens on completion *and* on failure.
- **Batching**: DFCP messages carry up to ~24 embeddings vectors (or several
  chat prompts) per round-trip, amortising negotiation cost.
- **Embeddings**: each node runs a dedicated embeddings backend
  (`nomic-embed-text-v1.5`) alongside its chat model; the pool partitions
  large embedding jobs across all nodes.
- **Chat batch**: multiple independent prompts travel in one DFCP round.
- **Map/reduce inference**: a single logical workload too large for one call
  is split into shards, mapped across workers, reduced into one answer.

### Validated results (live, 3-node fabric)

| Workload | Result |
|----------|--------|
| 100,000 distributed embeddings | **42.1× speedup**, 100,000/100,000 completed, 0 failures, 31.6 emb/s |
| 1,000 distributed embeddings | 28.5× speedup |
| Chat batch (12 prompts, 3 workers) | 4.3× speedup, valid outputs |
| Map-reduce document analysis | 2.26× speedup, single coherent result |

---

## 4. Autonomous Governor (M15)

The Governor continuously evaluates **real** fabric pressure — CPU, RAM,
queue depth, latency, reachable workers — using the node's own probe, not
simulated values.

```
IDLE/LOCAL
   ↓ pressure rises above threshold (hysteresis state machine)
PRESSURE FIRED (reasons + score recorded)
   ↓
Model Colony selects the model
CPU Pool lends capacity
DFCP assigns workers
map → reduce executes
EvidenceChain records everything
   ↓ pressure falls below release threshold
RELEASE → borrowed capacity returned → LOCAL
```

- Thresholds use the existing M15 hysteresis/fairness engine (enter ≥0.35,
  exit ≤0.25, cooldown between firings). Not replaced, reused.
- Every transition (`PRESSURE_FIRED`, stage evidence, `PRESSURE_RELEASED`)
  lands in EvidenceChain.
- **No operator POST is required.** The loop runs inside the node.

Live demo: 8 concurrent chat requests raised queue+latency pressure; the
Governor fired autonomously, executed distributed map-reduce, credited remote
workers, and returned to LOCAL when pressure fell.

---

## 5. BYOA Gateway (M16)

External agents bring their own automation into the fabric with **scoped
credentials**, not admin tokens.

- Consumer API keys (`dca_…`) carry an owner account, a per-request **quota
  ceiling**, a **rate limit** and scopes. They never grant admin/operator
  privileges.
- `/v1/governor/execute` accepts master tokens *and* consumer keys:
  - rate limit is checked **before** quota is spent,
  - quota is reserved up front and settled when execution finishes,
  - the full intelligent path runs: Model Colony → resource verdict →
    distributed map-reduce → EvidenceChain → economic credit.

### Setup

```bash
# 1. Issue a scoped key for the agent's account
decentraai consumer-key create --account my-agent \
  --quota-ceiling 5000 --rate-limit-per-minute 10 --scopes inference

# 2. Fund the account's quota (master token)
curl -X POST http://node:8080/api/admin/quota/grant \
  -H "Authorization: Bearer $MASTER" -H "Content-Type: application/json" \
  -d '{"account":"my-agent","amount":50000}'

# 3. The agent drives the fabric itself
curl -X POST http://node:8080/v1/governor/execute \
  -H "Authorization: Bearer dca_…" -H "Content-Type: application/json" \
  -d '{"task_id":"job1","task_kind":"summarize","instruction":"Summarize.","content":"…"}'
```

Live demo: a consumer key drove a DISTRIBUTED map-reduce run end-to-end —
Gemma selected, reduce valid (240 chars), Desktop and Laptop credited.

---

## 6. Collective Agents (M17)

Workflows are declared as stages with dependencies:

```json
{"intent":"research",
 "stages":[
   {"stage_id":"research","capability":"chat","prompt":"Research …"},
   {"stage_id":"analyze","capability":"chat","prompt":"Analyze …",
    "depends_on":["research"]},
   {"stage_id":"verify","capability":"chat","prompt":"Verify …",
    "depends_on":["analyze"]}]}
```

`POST /v1/agents/workflow` executes the DAG in topological order, feeding each
stage the outputs of its dependencies. **Every stage drives the Governor**:
Model Colony selects the model, the resource verdict chooses LOCAL or
DISTRIBUTED, map-reduce handles oversized content, EvidenceChain records the
stage, and remote workers earn credit.

A three-stage workflow (research → analyze → verify) was demonstrated live:
three stage decisions, three model selections, three resource decisions,
concatenated results, one verified outcome.

---

## 7. Model Colony

Model selection is **evidence-based, never hardcoded**:

1. **Capability match** — the task kind maps to required capabilities
   (summarization, classification, reasoning, structured_output, chat).
2. **RAM fit** — candidates whose footprint exceeds available RAM are excluded.
3. **Verified evidence** — remaining candidates are scored by measured
   accuracy and latency, with a penalty for reasoning models (which can spend
   their whole budget thinking and return empty content).

The colony currently includes Qwen3-1.7B, Gemma-3-1B-Instruct and
Phi-4-mini-instruct (all catalogued with capabilities and RAM footprints), plus
the dedicated embeddings model. Selection reads **real performance
observations from collective Memory** (`aggregate_model`) and falls back to
seeds until observations accumulate. After every execution the run is recorded
back into Memory (`record_observation`), so the colony improves from its own
history.

Example: `summarize` selects Gemma (summarization capability, 2 GB fit);
`reason` prefers Phi over Qwen3 because Qwen3's empty-output risk penalises it.

---

## 8. Evidence & Economy

Every execution produces an append-only evidence trail:

- the Governor's decision and reasoning,
- shard assignments per worker,
- shard failures with lease release,
- replans onto alternative workers,
- completions and incompletes,
- the reduce result and its status.

The economy layer credits **verified contribution only**: a remote worker is
credited after its shard completes, measured by latency, through the existing
ledger and reward policy (synthetic bookkeeping — never money, never the token
registry). A worker that fails or dies earns nothing for that run.

Live demo: after a distributed run, the Desktop worker held a balance of
16,872 credits earned from verified contributions.

---

## 9. `execution_id` — one ID for the whole trace

Both `/v1/governor/execute` and `/v1/model-parallel` return a canonical
**`execution_id`** (`gov:{task_id}`). That same id appears in:

- every EvidenceChain entry (`gov:{id}:decision`, `…:shard-failed`,
  `…:shard-replan`, `…:completed`, `…:incomplete`, `…:reduce`),
- the economy credit reference (`gov-{id}-…`),
- the response payload (`model_selected`, verdict, timings, per-worker stats).

Given an `execution_id`, the entire story is reconstructible: **what the agent
asked, why the Governor decided, which model was chosen, what resources were
reserved, who executed, what was measured, what failed, and who was rewarded.**

---

## 10. Failure Recovery

Failure is treated as a first-class outcome, not an edge case.

**Scenario demonstrated live**: during distributed map-reduce, a worker
(Desktop) was killed mid-execution.

1. Its shard transitioned to **FAILED** — never silently dropped.
2. The lease was released (recorded in EvidenceChain as `lease-release`).
3. The shard was **replanned onto an alternative worker** with the same shard
   id (recorded as `shard-replan`).
4. Remaining shards completed; the reduce fused the completed set.
5. Economy credited **only the workers whose shards actually completed** — the
   dead worker earned nothing.

Additional guarantees:

- A shard past its attempt budget stays FAILED; if no alternative worker
  exists, the run reports **incomplete honestly** rather than fabricating a
  result.
- Reduce accepts only COMPLETED shards.
- Completed shards are never re-executed.

Known boundary: replanning requires an alternative worker to be alive. With a
single-node fabric there is nothing to borrow — the honest answer is LOCAL or
incomplete, not fiction.

---

## 11. Security Boundaries

- **Identity**: Ed25519 keypairs per node (mode 0600); PeerId derived from the
  public key; trust chain discovered → approved → connected.
- **Artifacts**: BLAKE3 chunk hashes + Merkle root verification for any model
  pulled into the registry; manifests signed by the owner.
- **Credentials**: provider API keys stay in env vars, read at call time and
  redacted on error paths; private keys never leave the node.
- **Consumer authentication**: `dca_` keys are scoped (quota ceiling, rate
  limit, scopes), resolved through the key store on every request, stored only
  as BLAKE3 hashes, revocable instantly.
- **Loopback discipline**: internal self-calls (M15/M17 Governor triggers)
  target fixed `127.0.0.1:{api_port}` — no SSRF surface; the URL port is a
  `u16`, not a user-controlled string.
- **Secrets hygiene**: the master token travels only in Authorization headers
  of internal calls and is never logged, echoed in responses or persisted in
  evidence.
- **Limits**: bounded self-call timeouts (240 s), max workflow stages (8),
  per-key rate limits and quotas, bounded DFCP message sizes.
- **Prompts and outputs are never logged**; telemetry is counters, latencies
  and statuses only.

---

## 12. Validated Capabilities

All measured on the live 3-node fabric (VPS + Desktop + Laptop):

| Capability | Measured result |
|---|---|
| Distributed embeddings @ scale | **100k embeddings, 42.1× speedup**, 0 failures |
| Distributed embeddings @ 1k | 37.8× speedup, 0 failures |
| Chat batch distribution | 4.3× speedup, valid outputs on all nodes |
| Map-reduce document analysis | Single coherent summary from 3 workers |
| Autonomous pressure trigger | Fired without operator input; released cleanly |
| BYOA external agent | Consumer key drove DISTRIBUTED execution end-to-end |
| Collective workflow | research → analyze → verify through the Governor |
| Shard failure recovery | FAILED → REPLAN → COMPLETED → VALID reduce |
| Honest failure reporting | Incomplete shards reported, never fabricated |
| Economic attribution | Desktop worker: 16,872 credits from verified contributions |
| Unified traceability | Full path reconstructed from one `execution_id` |

---

## 13. Known Limitations

Stated plainly, because honesty is a feature:

- **Qwen3 output variability**: Qwen3-1.7B spends generation budget on hidden
  reasoning; with small caps the visible content can come back empty. The
  system detects and reports this honestly (reduce retried on another worker /
  marked invalid), but the underlying model behaviour remains.
- **No GPU / no tensor parallelism**: the current stack is CPU-only.
  Tensor/pipeline parallelism requires accelerators and fast interconnects
  (NVLink-class); forcing LAN nodes into TP is not viable. It remains a
  **future, separate compute class** — the Compute Fabric does not pretend to
  be it. GGUF quantised models also do not fit safetensors-based TP flows.
- **Replan requires an alternative worker**: with one node there is nothing to
  borrow; the honest fallback is LOCAL or incomplete.
- **Reduce quality varies with the chat model**: the reduce step inherits the
  serving model's reasoning behaviour; a dedicated non-reasoning reducer is a
  future improvement.
- **Single-writer workflows**: collective orchestration executes stages in
  topological order (no parallel stage execution yet).
- **Quota funding is manual**: consumer accounts are funded via an admin grant
  endpoint; automated top-up from rewards is future work.

---

## 14. Operations / Deployment

### Topology

| Node | Hostname | Address | Role | Served model | Embeddings backend |
|------|----------|---------|------|--------------|--------------------|
| VPS | decentraai-vps | 169.58.213.145:32937 | Orchestrator + worker | Qwen3-1.7B | :7777 nomic |
| Desktop | i7 | 192.168.1.138:32937 | Worker | Qwen3-1.7B | :7777 nomic |
| Laptop | i5 | 192.168.1.132:32937 | Worker (+systemd) | qwen2.5-3b | :7777 nomic |

### Starting a node

```bash
# Build
cargo build --release -p decentraai-cli

# Run (screen for servers; systemd user service on the laptop)
decentraai node --config ~/.decentraai/node.yaml
```

**One mechanism per node.** Running both screen and the systemd service on the
same machine creates duplicate listeners on port 32937 and destabilises P2P —
this was observed and fixed in practice. Pick one supervisor and stick to it.

### Required config highlights

```yaml
inference:
  allow_remote_inference: true
  embeddings_backend_url: "http://127.0.0.1:7777"   # for distributed embeddings
sharing:
  assist:
    enabled: true          # worker side: accept DFCP assignments
autonomous_assist:         # optional: M15 autonomous trigger
  enabled: true
  tick_seconds: 5
  cooldown_seconds: 45
fabric_intelligence:
  enabled: true            # unlocks /v1/governor/* and /v1/intel/*
```

Models expected in `~/.decentraai/models/`: the chat GGUF (per config) +
`nomic-embed-text-v1.5.Q4_K_M.gguf` for the embeddings backend.

### Health checks

```bash
curl -s http://127.0.0.1:8080/status | jq .model_loaded     # engine up
curl -s http://127.0.0.1:8080/v1/peers -H "Authorization: Bearer $T"  # mesh view
curl -s http://127.0.0.1:7777/v1/embeddings -d '{"input":"x"}' ...      # embeddings alive
```

### Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Peer connects then drops every ~2 min | idle connection timeout + ping failures | raise `idle_connection_timeout`; keep exactly one node process per host |
| Two nodes listening on 32937 | duplicate supervisors (screen + systemd) | kill duplicates; keep ONE mechanism |
| `no spendable consumer quota` | consumer account unfunded | `POST /api/admin/quota/grant {account, amount}` |
| Embeddings return empty | Qwen3 spent tokens on reasoning | larger `max_tokens`, or route reduce to a non-reasoning worker |
| `empty-failed` reduce | reducer produced no visible content | see above; status is reported honestly |
| Node dies silently after deploy | old binary still running | restart the supervisor AFTER build finishes; verify binary timestamp |

### Deployment checklist

1. Pull latest `main`; verify commit hash matches expectation.
2. `cargo build --release -p decentraai-cli`.
3. Restart the node supervisor (screen or systemd) — **after** the build.
4. Confirm `/status` reports the model loaded.
5. Confirm embeddings backend answers on :7777.
6. Confirm peer count ≥ 2 on the orchestrator.

---

## 15. Roadmap (remaining, real)

Only directions already discussed and grounded — no invented milestones:

1. **Reduce quality**: prefer a non-reasoning model (Phi/qwen2.5) for the
   reduce step; hierarchical reduce for many partials. Addresses the known
   empty-output variability.
2. **Retry/replan hardening**: extend the same shard lifecycle guarantees
   proven here to the embeddings pool path at industrial scale.
3. **Automated reward settlement**: link verified contribution credit to
   automatic consumer-quota top-up (closes the economy loop mechanically).
4. **Tensor/model parallelism** — explicitly out of scope for the current
   stack (no GPUs; GGUF/safetensors mismatch; needs accelerator interconnects).
   Treated as a separate future product class, not a feature of this fabric.

---

*Document generated from live, measured deployments. If a number appears
here, a node produced it.*
