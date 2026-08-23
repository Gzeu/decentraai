# DecentraAI Vision

DecentraAI is designed as a distributed execution fabric for AI models, compute, and logical agents, with a strong emphasis on verifiability, privacy, and LAN-first trust.

This document captures the long-term product vision, building on the existing architecture and roadmap.

## Personal AI Fabric OS

- Each DecentraAI node is a "personal AI OS" on top of your hardware: it owns identity, policies, scheduling, model registry, and telemetry.
- The universal node combines coordinator, worker, dashboard, and OpenAI-compatible API into a single process, with llama-server or equivalent engines always kept behind loopback.
- Users interact in terms of intents and capabilities ("do OCR", "code review", "RAG on my docs"), not specific model names; the fabric decides which models and workers to use based on trust, cost, latency, and policy.

## Capability Fabric

- Models, tools, and agents are first-class capabilities with provenance (VERIFIED vs INFERRED), not just files or endpoints.
- Capability matching is provenance-aware: a VERIFIED requirement is never satisfied by an INFERRED claim.
- "CAN I RUN THIS?" is a central question the fabric can answer for any (model, capability, evidence level) across the whole mesh.
- Intent resolution maps free-form user intents to capabilities and models; decisions are explainable and deterministic.

## Distributed Compute and Quota Economy

- The fabric shares compute/GPU capacity across trusted peers on the LAN, not just model files.
- Contribution is measured from real, verified work (tokens, processing time, reliability) and accumulated in a compute contribution ledger.
- Quota and consumer API keys (`dca_`) provide an account-based authorization layer: applications consume compute against quota, with deterministic, auditable accounting.
- Tiers and tokens reflect actual contribution and reputation, not arbitrary roles.

## Lightweight and Mobile Workers

- Standalone workers (`decentraai-worker`) can join an existing fabric, advertise real hardware and models, and serve remote inference without running the full control plane.
- Adaptive contribution factors adjust effective capacity based on real thermal, utilization, and battery signals; stressed workers are ranked lower but never fully excluded.
- Device class (server/desktop/laptop/mobile/edge) and capacity state (FULL/LIMITED/UNAVAILABLE) make it easy to build fabrics from heterogeneous devices.
- The long-term direction includes mobile/edge workers with strict sandboxing and battery-aware scheduling, without assuming any single platform or update mechanism.

## Collective Intelligence Layer

- Agents are logical execution contexts hosted on nodes, not new processes: identity + capabilities + policies + memory + reputation + relationships.
- Agent tasks generalize from pure inference to arbitrary structured inputs/outputs, with required capabilities, resources, budgets, and verification contracts.
- Delegation uses DAGs of tasks: discover → match → plan → reserve → execute(per-hop) → verify → learn → release.
- Verification and consensus are first-class: critics, consensus policies, and disagreement resolution operate on task results before they enter memory and reputation.
- Collective memory is scoped (agent, team, network, fabric) with explicit ownership, access, retention, privacy, and provenance policies.
- Agent reputation is per-(agent, capability) and decomposed (reliability, quality, latency, uptime, safety, provenance); only cryptographic/policy violations penalize safety.
- Policy engine separates power from permission: an agent with strong capabilities does not automatically gain strong rights.

## Digital Twin and Observability

- The fabric exposes a digital twin (`/v1/fabric`, `/v1/resources`, `/v1/execution`, `/v1/stats`) so operators and agents can see nodes, models, capabilities, executions, sessions, network and KV locality from real state.
- Recovery and self-healing are explicit: health monitoring, reapers, bounded retries, false-ready prevention, and recovery timelines are part of the execution story.
- Historical execution statistics and resource intelligence provide measured performance and capacity, never synthetic.
- OpenTelemetry GenAI metrics (`gen_ai.*`) align DecentraAI with emerging observability conventions.

## North Star and Invariants

North Star: DecentraAI is a LAN-first, identity-strong, capability-driven AI fabric OS, not a marketplace or token economy.

Non-negotiable invariants:

- Verify-before-use extends from artifacts (models) to task results.
- Secrets stay local; agents never gain access to private keys or raw tokens.
- Prompts and outputs are never logged in audit; only security events are.
- Engines are external processes (no unsafe FFI), driven through OpenAI-compatible HTTP APIs.
- Determinism in scoring, planning, and persistence (canonical serialization, tmp+sync+rename).
- Agent power != permission: policies and sandbox are enforced at every hop.
