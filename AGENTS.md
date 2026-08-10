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
- `crates/audit` — append-only JSON-lines security log
  (`logs/audit.jsonl`): peer bans, chunk verification failures,
  admission rejections, inference starts. Prompts and outputs are never
  audit material.
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
  non-streaming v1, SSE later.
- **P3 — Admin dashboard**: `/admin` behind the master token; create /
  revoke tokens, set tiers, usage per token, peer catalogs; everything
  audited.
- **P4 — Contribution-based tiers**: periodic job computes suggested
  tier from models shared × verified chunks − failures; admin confirms
  or rules auto-promote; `tier_changed` audit event.
- **P5 — Invites & join**: admin generates an invite (bootstrap
  multiaddr + Tier-1 token); `decentraai join <invite>` bootstraps a
  fresh node.
- **M9 (later) — Distributed inference**: route requests to peer GPUs,
  paid in reputation. v1 stays hub-and-spoke on the admin's machine.

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
