# AGENTS.md — DecentraAI Agent Operating Contract v1.0

You are working on **DecentraAI**: a cooperative AI compute fabric where
independent, owner-controlled nodes contribute and consume **verified**
resources — models, inference, embeddings, OCR, STT, compute assist.

This file is the operating contract for any agent (human or AI) that
modifies this repository or talks to the fabric through its APIs.
Read it fully before writing code. The companion manuals live in
`.agents/skills/` and the conceptual rules in `.agents/policies/`.

> **The one invariant that governs everything:**
>
> ```text
> AI proposes  →  deterministic Rust decides  →  workers execute
> ```
>
> An LLM (local intelligence layer, external provider, agent output) may
> propose plans, parameters and recommendations. It can NEVER select a peer,
> mutate trust, issue credentials, alter hashes/reputation/config, or bypass
> reservations. Every action passes through deterministic validation.

---

## 🧠 1. Mission & Architecture

DecentraAI lets trusted peers share GGUF models with cryptographic
verification (BLAKE3 chunks + Merkle root + Ed25519 identity), serve
inference through a managed llama.cpp `llama-server` subprocess behind an
OpenAI-compatible API + MCP, and now: **autonomously assist each other with
compute** ("Sharing is Caring", DFCP v1).

It is **not**: a public internet network (LAN/private swarm first), a
payment platform, a crypto economy, a model training framework, or a
wrapper around llama.cpp internals (the engine is always an external
process, never FFI).

### Where we are (milestone tags, not slogans)

| Tag | What landed |
|---|---|
| `milestone/fabric-intelligence` | reasoning layer over the planner; intel PROPOSES validated structured plans (`POST /v1/intel/plan`) |
| `milestone/sharing-is-caring` | DFCP v1 negotiation + Compute Assist verified LIVE on 3 nodes; contribution credit only on verified success |

Forward roadmap (agreed): **M15** Autonomous Pressure Trigger +
PlacementEngine fairness · **M16** Agent Gateway (BYOA: scoped credentials
+ MCP execution tools) · **M17** Capability Sharing / Collective
Orchestration · then collective memory, adaptive execution, model
parallelism. Full historical detail: `docs/ROADMAP_HISTORY.md`.

### The agent organization

Specialized agents form an operating system over the fabric — each with a
role contract, RBAC scopes/forbidden lists, approval gates and its own
memory scope. See `docs/AGENT_ORGANIZATION.md` and the contracts in
`.agents/registry/*.md` (governor, architect, rust-engineer, api-engineer,
fabric-engineer, qa, security, vps-operator, memory-keeper, researcher,
concierge). Rule of thumb:

> **Worker = compute identity. Agent = cognitive identity.**
> AI proposes → deterministic Rust decides → workers execute.

### Repository layout (17 workspace crates)

- `crates/fabric-intelligence` — the reasoning layer: `TaskPlan`
  (closed-schema parsing of UNTRUSTED model output), mesh validation,
  provider policy (`local_first` default), secret redaction, 2 GiB artifact
  policy, telemetry. Providers: local llama.cpp backend (live URL per
  request) + OpenAI-compatible external (key read from env at call time).
- `crates/fabric` — the engine-neutral deterministic planner:
  `ExecutionPlan`, `plan_and_reserve`, expert routing scaffolding, KV-aware
  inputs (M20), network-aware scoring (M19).
- `crates/compute` — pure compute-sharing domain: capabilities, matcher,
  scheduler, reservations, credits/contribution ledgers, quota, and
  `assist.rs` (DFCP offer gates + fairness-scored selection).
- `crates/agents` — collective-intelligence fabric (pure): agent records,
  unified capability matcher, delegation DAG, verification/consensus,
  memory scopes, reputation, talent tree, workflows, benchmark, evidence RAG.
- `crates/distributed` — runtime half: `ComputeManager`, orchestrator,
  embedding/RAG clients, session KV accounting, knowledge/evidence wiring.
- `crates/protocol` — wire schemas: infer messages, DFCP v1
  (`dfcp.rs`: REQUEST→OFFER→RESERVE→ASSIGN→RESULT→RELEASE), manifest/chunk
  messages. Manifest/chunk responses carry NO signatures by design:
  integrity is anchored in the signed manifest's `chunk_hashes` + Merkle
  root, enforced per chunk at assembly.
- `crates/p2p` — libp2p actor (commands via channel, never blocks the event
  loop), request/response codec, verified transfer (`.part` staging, resume
  bitmap, quarantine), reputation (only cryptographic failures count toward
  bans), DFCP dispatch cascade, bounded reconnect loop.
- `crates/runtime` — llama-server process manager (health-probed,
  kill-on-drop, auto-respawn), admission gate, `api.rs` (axum proxy + Bearer
  auth + dashboard; the dashboard NEVER polls the proxy — only `/status` and
  `/v1/peers`), tool runtimes (OCR/STT/TTS/skills sidecars),
  `intel_assist.rs` (Sharing is Caring worker/requester), read-only MCP
  server at `/mcp` (consumer keys get decide+execute; operator/master get
  control-plane tools).
- `crates/hub` — the ONE capability taxonomy (`CapabilityKind`, 26 kinds,
  snake_case) + model classification from HF metadata.
- `crates/config` — typed YAML config, strict validation, tests per rule.
- `crates/identity`, `crates/manifest`, `crates/registry`,
  `crates/system-probe`, `crates/tokens`, `crates/providers`,
  `crates/discovery`, `crates/inference-adapter`, `crates/audit`,
  `crates/p2p-invoke` — identity/0600 persistence, chunking+Merkle, path-safe
  registry, hardware snapshots/admission, subscription tokens, external
  providers, mDNS/DHT discovery, adapter seam, append-only audit log.

---

## 🧭 2. How to reason about the Fabric

Non-negotiable invariants (violating any of these is a bug regardless of
tests passing):

1. **Verify before use.** No artifact is used before hash + manifest +
   policy verification. Per-chunk BLAKE3, full-file hash + Merkle root,
   atomic rename into `models/`.
2. **Only cryptographic failures punish peers.** Network errors never touch
   reputation. Corrupted chunks → temporary ban + quarantine.
3. **Determinism everywhere.** Canonical serialization for signing;
   ranking is score desc / PeerId asc; persistence is tmp+sync+rename;
   no randomness in scheduling decisions (tie-break by id).
4. **Secrets stay local.** Keys/tokens are mode 0600, never logged,
   committed or transmitted; external API keys are read from env AT CALL
   TIME and redacted on every error path; the API binds loopback only.
5. **Prompts and outputs are never logged.** Telemetry = counters and
   latencies only.
6. **The inference engine is a subprocess.** Health probes, kill-on-drop,
   binary-swap upgrades; ephemeral ports mean backend URLs must be resolved
   LIVE per request, never cached at boot.
7. **AI output is untrusted input.** Parse it through closed schemas
   (`deny_unknown_fields`, bounds); validate against real mesh state; treat
   malformed/hostile answers as rejections, not something to scrape around.
8. **Every lease expires; every verified contribution is auditable; every
   failed task releases resources.**

Capability-first thinking: the unit of decision is the CAPABILITY
(`decentraai_hub::capability::CapabilityKind` — the single taxonomy), not a
model name. Models are replaceable artifacts; workers are interchangeable
executors; the registry is the source of available capabilities.

---

## 🤝 3. Sharing is Caring rules

These are agent-behaviour rules, enforced by deterministic code:

```text
If the local node has safe surplus      → it MAY contribute (opt-in limits).
If the local workload is under pressure → it MAY request assistance.
Never consume remote resources without: identity + authorization +
    reservation + lease + execution evidence.
Never claim contribution without verified evidence.
Owner limits are absolute; sharing is revocable at any moment.
```

The negotiation is DFCP v1 (`crates/protocol/src/dfcp.rs`):
`RESOURCE_REQUEST → RESOURCE_OFFER → RESOURCE_RESERVE → RESOURCE_RESERVED →
ASSIST_TASK_ASSIGN → ASSIST_TASK_RESULT → RESOURCE_RELEASE`.
Offers are claims until RESERVE succeeds against the worker's ledger.
Selection scores offers deterministically: hard gates first (capability,
resource fit, freshness ≤30s, queue, recent failure), then a score where
the worker's `contribution_balance` biases ties by at most ±0.15 — fairness
is a bias, never a dictator. Ties break by peer id ascending.

Manuals: `.agents/skills/sharing.md`, `.agents/skills/dfcp.md`,
policy: `.agents/policies/resource-sharing.md`.

---

## 🛠️ 4. Skills & Capabilities

Skills (OCR, STT, embeddings, translation, …) are DECLARED capabilities
backed by datasets and executed by local sidecar runtimes. The registry is
the source of truth for what exists:

```text
CapabilityKind (hub)  +  worker advertisements  +  runtime state
        = what the fabric can actually do right now
```

Skill application flow (for any agent):

```text
Need OCR? → check local capability → unavailable?
  → query fabric capabilities (MCP list_workers/capability search)
  → deterministic route (planner/reservations)
  → reserve → execute → verify → result
```

NEVER call a remote node directly because "someone said it has OCR". Route
through the fabric so trust/reservation/evidence apply.

---

## 🔌 5. MCP / API — two doors, one fabric

```text
External agent ──┬── OpenAI-compatible API (/v1/chat/completions, /v1/models,
                 │                          /v1/embeddings)
                 └── MCP (/mcp, JSON-RPC 2.0)
                            │
                    Bearer credential (dca_ consumer key / master token)
                            ▼
                      Fabric + policy
```

- Consumer keys (`dca_…`) carry quota ceiling + rate limit + account; on
  MCP they get `decide` (read-only planning projection) and
  `execute_decision` (quota-gated mutation). Operational control-plane
  tools require operator/master.
- **MCP tools are capabilities, not authority.** A tool may request an
  action; it cannot skip policy/trust/reservation/artifact verification.
- `/v1/intel/*` endpoints expose the intelligence layer (operator+) —
  plans are advisory proposals, never commands.

---

## 🔐 6. Security & Trust

Threat model reminders for every change: malicious peer, forged
announcement, replayed task, oversized message, prompt injection into any
model in the loop, credential exfiltration, recursive agent loops.

Standing requirements: transport-authenticated identity binding (never
trust payload sender fields), bounded messages, deny-unknown-fields
schemas, replay/duplicate protection, lease expiry, evidence verification
before credit, no arbitrary execution from remote payloads, secrets never
in logs/dashboard/errors (see `.agents/policies/trust.md`).

Auth vocabulary: `dca_` = consumer key (quota-limited, non-admin);
`dsk_` = legacy subscription token (never admin UI); anything else is a
master CANDIDATE proven only by probing a master-only endpoint.

---

## 🧪 7. Testing & Verification

Quality gates before EVERY push:

```bash
git pull --rebase
git log --oneline -1     # confirm the expected commit is checked out
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Baseline: ~50 suites / ~1279 tests green (the number drifts; the gate is
the command pair above). E2E tests spin real libp2p nodes on loopback —
keep them fast (<20s) and deterministic (retry loops only for connection
settling, never logic).

Every feature lands with tests: unit tests drive pure decision functions
with synthetic input; protocol changes get E2E coverage; network/fabric
behaviour changes get a live multi-node verification before being called
done.

---

## 🚀 8. Development Workflow

Branches:

```text
main            ← stable baseline; architectural changes NEVER land directly
feature/<name>  ← branched from main or from the feature it builds upon
```

Before modifying code:

1. Inspect architecture; find the existing primitive; REUSE it.
2. Read the tests around it; identify invariants.
3. Make the smallest additive change.
4. Add unit tests (pure decisions) + integration/E2E (wire behaviour).
5. Run the gates; fix-forward, never amend published history.
6. If network/fabric behaviour changed: live multi-node verification.

Standard implementation report at the end of every task:

```text
IMPLEMENTATION REPORT
Branch / Commit:
Changed:  …
Reused:   …
Tests:    cargo test --workspace (N suites / N green), clippy -D warnings
Live verification: …
Resources: CPU/RAM/GPU notes
Security findings: …
Known limitations: …
Next recommended step: …
```

Pitfalls already hit (do NOT repeat): bash treats `<PORT>` as redirection
in docs; libp2p refuses self-dial (single-machine pull tests need a second
data dir + identity); dashboard JS must never poll proxied endpoints (it
once inflated counters by ~10k); admission compares RAM against the
absolute reserve, not the derived budget; cross-crate references need the
dependency declared in that crate's Cargo.toml (E0433); appending a YAML
section that already exists produces duplicate keys — inject UNDER the
existing section; pipeline tricks like `cargo check | tail` mask exit
codes — grep for errors explicitly.

Full historical roadmap detail (M9–M24, P0–P11, Q4): see
`docs/ROADMAP_HISTORY.md`.
