# DecentraAI Deep Technical Overview

This document is a high-level technical overview of the DecentraAI fabric as implemented in the current repository. It complements `README.md`, `ARCHITECTURE.md`, `DECENTRAAI_PRODUCT_STATUS.md`, and `VISION.md` by connecting architecture, product status, roadmap, and long-term intent in one place.

## 1. Product Positioning

DecentraAI is a decentralized AI compute fabric: people connect computers/GPUs, and DecentraAI turns that capacity into a verified, automatically orchestrated distributed AI infrastructure.[cite:30]

The project is explicitly **not** a GPU marketplace, cloud hosting product, or simple llama-server wrapper. Engines such as llama.cpp/llama-server, vLLM, or SGLang are treated as execution backends; DecentraAI is the orchestration layer that:
- discovers heterogeneous compute;
- verifies and provisions models;
- plans execution with a fabric-wide view;
- schedules workloads;
- adapts to network and runtime conditions.[cite:27][cite:30]

The North Star, as captured in `docs/VISION.md`, is a LAN-first, identity-strong, capability-driven AI fabric OS.[cite:29]

## 2. Architecture Planes and Node Lifecycle

The architecture is divided into three main planes:[cite:24]

- **Control plane** — configuration, Ed25519 node identity, capabilities, policies, scheduling, model registry, local reputation, telemetry.
- **Data plane** — libp2p transport, peer discovery, manifest exchange, chunk transfer, request authentication, rate limiting, retries, peer scoring.
- **Inference plane** — runtime supervisor that manages engines (llama-server, etc.), enforces resource limits, streams tokens, supports cancellation, and restarts crashed workers.

A node’s lifecycle (from `ARCHITECTURE.md`) is:

1. BOOT.
2. Validate configuration.
3. Load or generate node identity.
4. Probe CPU/RAM/GPU/VRAM/disk/network.
5. Derive resource profile and policy limits.
6. Reconcile model cache, partial downloads, quarantine.
7. Start local API and inference supervisor.
8. Start LAN/private P2P transport.
9. Publish signed capability announcement.
10. Reach READY state.[cite:24]

The universal node (`decentraai node`) combines coordinator, worker, dashboard, API proxy, agents orchestrator, RAG, memory store, and talent tree in a single daemon, with engines bound only on loopback.[cite:27][cite:29]

## 3. Model Storage and Verification

Model artifacts (typically GGUF) are handled through a content-addressed pipeline:[cite:24]

1. Split the model into fixed-size chunks (4 MiB by default).
2. Compute BLAKE3 for each chunk while streaming from disk.
3. Build a Merkle root over ordered chunk hashes.
4. Canonicalize and sign a manifest with an approved publisher key.
5. Store chunks only after individual hash verification.
6. Assemble the final artifact atomically only after full verification.
7. Keep incomplete or invalid artifacts in staging/quarantine, never in the verified cache.

`README.md` reinforces this via an "Artifact Verification Chain" diagram (Hugging Face SHA‑256 → BLAKE3 chunk hash → Merkle root → Ed25519 manifest signature).[cite:27] This chain ensures that local and remote models are cryptographically verified before they can be used or shared.

## 4. Distributed Inference Fabric

The distributed inference fabric ties together:

- `compute` — pure domain for capability, availability, advertisements, requirements, reservations, schedulers.
- `fabric` — execution planner, network graph, KV planner, expert router.
- `distributed` — compute manager, router, worker registration, agents bindings.
- `runtime` — engine lifecycle, API proxy, dashboard.[cite:24][cite:27]

Product status (`DECENTRAAI_PRODUCT_STATUS.md`) marks M10–M16 as DONE:[cite:30]

- M10 — real distributed inference path: coordinator → P2P `InferRequest` → worker queue → OpenAI-compatible backend → real llama-server → real GGUF model → streamed `InferProgress` → terminal `InferResponse`.[cite:30]
- M11 — capability-aware compute sharing.
- M12 — real hardware advertisement.
- M13 — capability-aware routing and reservations, with worker-side enforcement.
- M14 — on-demand model provisioning.
- M15 — worker-side reservation enforcement.
- M16 — live compute metrics.

These milestones together provide verified remote execution across LAN peers, with reservations, queueing, health monitoring, and failure-aware fallback.

## 5. Capability and Model Fabric

The model fabric (P1–P10) introduces a provider plane (OpenRouter, OpenAI, Groq, Together, Fireworks) that behaves like a first-class model source alongside local registry and P2P fabric:[cite:27]

- Providers are configured via CRUD + credential store where API keys live only in memory; persisted records carry references, never secrets.
- Health and circuit breakers gate reachability; only cryptographic failures punish peers.
- Sharing policy defaults to OFF; models are private until explicitly shared.
- Cost-aware `auto` routing chooses an enabled provider model when local/fabric models cannot run, keeping local/fabric as first tier.[cite:27]

Next‑Gen roadmap extends this into a capability fabric:

- Capabilities (e.g., OCR, coding, vision) are classified from Hub metadata and persisted as claims on model records.[cite:23]
- Capability requirements (`CapabilityRequirement`) are resolved against claims to produce VERIFIED/INFERRED/MISSING verdicts.
- "Capability fit" is surfaced for single models, comparisons, planner decisions, and digital twin views.
- MCP tools and `/v1` endpoints answer "which models can do X?" and "which workers can run this capability on this model?" with explainable verdicts.[cite:23]

This fabric is what the VISION doc refers to as "models, tools, and agents as first-class capabilities with provenance" and "CAN I RUN THIS?" as a central question.[cite:29]

## 6. Compute Sharing, Contribution, and Quota

Compute sharing is the core product rather than an add‑on:[cite:30]

- Workers advertise real compute capabilities (GPU/VRAM/RAM/CPU, engine, served models), health and load.
- The scheduler selects workers via capability matching and resource reservations, with worker-side enforcement preventing overcommit.
- Model provisioning allows workers to auto‑download and verify required models when policy permits.

The Compute Contribution & Quota roadmap introduces:[cite:23]

- **Measured contribution** — real verified work (tokens, processing time, completions) recorded without double counting.
- **Contribution policy** — versioned mapping from measured work to quota units (synthetic, non‑monetary).
- **Quota ledger** — per-account EARNED/AVAILABLE/RESERVED/CONSUMED balances with idempotent credit/reserve/settle/release semantics.
- **Consumer API keys (`dca_…`)** — account-scoped keys with quota ceilings and rate limits that authorize inference consumption against the ledger.

Combined, these layers support synthetic "economics" based on real compute served, aligning with the goal that tiers and tokens reflect contribution and reputation, not arbitrary roles.[cite:30][cite:29]

## 7. Lightweight Workers and Device Classes

To separate worker and control planes, the repository introduces a standalone `decentraai-worker` binary that:

- loads identity and config;
- spawns a local engine (llama-server);
- advertises compute capabilities over P2P;
- serves remote inference requests from coordinators.[cite:23]

Workers are annotated with `device_class` (server/desktop/laptop/mobile/edge) based on real hardware, and `capacity_state` (FULL/LIMITED/UNAVAILABLE) based on health, load, queue.[cite:23] Adaptive contribution factors combine GPU thermal, GPU utilization, CPU load, and battery percentage to scale effective capacity in scheduling.[cite:23]

The VISION doc’s "Lightweight and Mobile Workers" section aligns with this, describing standalone workers that join existing fabrics, contribute compute according to stress and battery, and form heterogeneous meshes without assuming any single platform.[cite:29]

## 8. Collective Intelligence Layer

The collective intelligence layer builds on existing primitives (identity, capabilities, fabric planner, reservations, registry, trust, contribution, quota, recovery, observability) to define agents as logical execution contexts hosted on nodes.[cite:25]

Key concepts proposed in `COLLECTIVE_INTELLIGENCE.md` include:[cite:25]

- **AgentRecord** — identity, node host, capabilities (semantic and execution), role, policies, memory scopes, reputation, relationships, lifecycle.
- **AgentTask** — generic task with input/output schema, required capabilities and resources, budgets, verification mode, priority, owner.
- **AgentMessage** — typed messages (ask/delegate/reply/verify/ping) over the existing libp2p channel, with canonical signing and nonce.
- **CapabilityAdvertisement** — extension of `ComputeAdvertisement` with semantic claims and tool descriptors.

The delegation path generalizes the current `route_request` loop to a multi-stage DAG:

> discover → match → plan(DAG) → reserve → execute(per-hop) → verify → learn → release.[cite:25]

Verification and consensus (critics, confidence floors, disagreement resolution) feed memory and reputation, ensuring that shared knowledge is backed by checked results.[cite:25] Policy engine and sandbox modes enforce "agent power ≠ permission", controlling tool, model, peer, budget, and egress access.

## 9. Digital Twin and Observability

Observability is a first-class concern:

- Dashboard views (Models, Workers, Network, Execution, Diagnostics, Security) render real data from `/v1/compute`, `/v1/network`, `/v1/resources`, `/v1/execution`, `/v1/stats`, `/v1/capabilities`, `/v1/fabric`, `/v1/sessions`.[cite:23][cite:27]
- Fabric graph endpoints (`/v1/fabric`, MCP `get_fabric_graph`) expose nodes, models, capabilities, executions with recovery timelines, network links, KV sessions as a "digital twin" of the mesh.[cite:23]
- Recovery timeline projections summarize outcomes, phase transitions, recoveries, and orchestration actions per execution decision.[cite:23]
- OpenTelemetry GenAI metrics (`gen_ai.*`) align runtime measurements with industry observability standards.[cite:23]

This observability supports operators and agents in understanding fabric health, capacity, and behavior, and underpins future self‑optimization loops.

## 10. Security Baseline and Invariants

The security baseline, as documented in `README.md` and `SECURITY.md`, includes:[cite:27]

- No artifact usable before hash + manifest + policy verification.
- Per-chunk BLAKE3, final hash and Merkle root enforcement.
- Prompts and outputs never logged; audit logs record only security events.
- Private keys and tokens never enter Git or telemetry; private keys stored with mode 0600.
- Dashboard exposes no secrets; API tokens guard every `/v1/*` endpoint.
- Loopback-only binding enforced by config validation.
- Subscription tokens stored only as hashes; quarantine workflow on corrupted chunks.

`VISION.md` extends these into invariants for the fabric OS:

- Verify-before-use applies to both artifacts and task results.
- Secrets stay local; agents never gain access to private keys or raw tokens.
- Engines are external processes driven over HTTP (no unsafe FFI).
- Determinism in scoring, planning, and persistence.
- Agent power ≠ permission; policies and sandbox enforced at every hop.[cite:29]

## 11. Roadmap and Long-Term Outcome

`DECENTRAAI_PRODUCT_STATUS.md` and the roadmap sections describe forward milestones M18–M24 and beyond:[cite:30]

- M18 — distributed execution engine (multi-worker plans, pipeline/tensor parallel primitives).
- M19 — network-aware scheduler (latency/bandwidth/topology-aware placement).
- M20 — KV-aware inference fabric (prefill/decode separation, KV locality and reuse).
- M21 — distributed MoE/expert fabric (sparse experts across nodes).
- M22 — multi-engine runtime (llama.cpp/vLLM/SGLang backends).
- M23 — autonomous execution planner (unified signals → adaptive plans).
- M24 — resilient decentralized AI fabric (failure detection, recovery, reputation, privacy/tenancy).

The long-term user experience is envisioned as:

- **INSTALL / CLICK** → detect hardware → detect/obtain compatible model → verify model → start local runtime → create node identity → establish trust → discover network → advertise compute → READY.
- **PROMPT** → understand workload → select execution plan → reserve compute → provision model if required → run local or distributed inference → stream result → observe performance → recover/replan on failure → release resources.[cite:30]

The goal is that users do not need to understand workers, VRAM, model placement, inference engines, P2P routing, KV cache, or execution topology; DecentraAI makes heterogeneous computers behave like one intelligent, verified AI compute fabric.[cite:30]

---

This document is intentionally descriptive rather than normative. For implementation details and up-to-date status, always consult `ROADMAP.md`, `DECENTRAAI_PRODUCT_STATUS.md`, `ARCHITECTURE.md`, `COLLECTIVE_INTELLIGENCE.md`, and the code under `crates/`.
