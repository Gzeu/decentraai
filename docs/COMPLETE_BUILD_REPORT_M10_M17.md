# DecentraAI Complete Build Report: M10-M17

## Executive decision

Build DecentraAI as a staged decentralized intelligence fabric, not as one large rewrite. M10 proves real two-node inference. M11 adds measured adaptive compute. M12 hardens the mesh. M13 verifies resources and models. M14 adds privacy and tenancy. M15 adds autonomous operations. M16 adds optional accounting. M17 adds governance and federated control.

## Target product

A desktop, laptop or server node creates an identity, discovers peers, pairs trusted workers, profiles hardware, verifies models, admits requests, plans execution, reserves capacity, dispatches over authenticated libp2p, runs a real backend, streams ordered output, records evidence, recovers from failures and updates reputation.

## Four planes

- Identity plane: keys, PeerId, pairing, TrustGrant, scopes, signatures and revocation.
- Control plane: discovery, registry, policy, scheduler, plans, reservations, reputation and admin API.
- Data plane: P2P streams, dispatch, queues, adapters, execution, cancellation and streaming.
- Evidence plane: audit, metrics, traces, benchmarks, provenance, incidents and rollback.

## Repository reality

| Responsibility | Existing foundation | Required completion |
|---|---|---|
| Identity | `crates/identity` | encrypted persistence, rotation and restart proof |
| P2P | `crates/p2p` | authenticated streaming and sender binding |
| Protocol | `crates/protocol` | versioned envelopes, lifecycle, chunks and errors |
| Discovery | `crates/discovery` | LAN/WAN expiry, trust and reconciliation |
| Distributed | `crates/distributed` | real adapter wiring, reservation and recovery |
| Registry | `crates/registry`, `crates/manifest` | signed manifests and readiness |
| Audit/monitoring | `crates/audit`, `crates/monitoring` | correlated lifecycle evidence |
| CLI/frontend | `crates/node-cli`, `frontend` | live API, worker UI and operator flows |

## Dependency graph

```text
Identity → Discovery → Trust → Capabilities → Registry
→ Admission → Planning → Reservation → Dispatch
→ Real Adapter → Streaming/Cancellation → API/Frontend E2E
→ Adaptive Compute → Public Mesh/Governance
```

## Release gates

### M10: production vertical slice

Desktop coordinator + laptop worker, real llama-compatible backend, pairing, readiness, request envelope, bounded queue, authenticated dispatch, ordered streaming, cancellation, metrics, audit and failure recovery.

### M11: adaptive compute

Verified capabilities, replicas, batching, token budgets, prefix/KV cache routing, quantization-aware placement, benchmark gates, trusted-cluster TP/PP and opt-in speculative execution.

### M12: secure mesh

Signed presence, DHT discovery, GossipSub, registry deltas, anti-entropy, stale-state handling, partitions, replay protection, revocation and quarantine.

### M13: verifiable resources/models

Signed manifests, artifact hashes, license/compatibility policy, sandbox tests, hardware benchmarks, verifier nodes, output verification and reputation.

### M14: privacy/tenancy

Locality policies, namespaces, scoped tokens, queue/cache/metric isolation, retention/deletion, redacted audit, quotas and fair scheduling.

### M15: autonomous operations

Adaptive estimates, circuit breakers, explainable plans, staged upgrades, rollback, incidents, agent proposals and human approval gates.

### M16: optional economics

Local free mode first, usage metering, budgets, cost/latency preferences, optional credits and settlement outside the inference hot path.

### M17: governance

Policy bundles, delegated administrators, approval tiers, emergency revocation/shutdown, signed decisions, agent scopes, tamper-evident evidence and multi-operator recovery.

## PR sequence

1. Backend adapter and deterministic tests.
2. Shared queue and cancellation cleanup.
3. Versioned protocol envelope and lifecycle.
4. Real handler wiring before node startup.
5. Authenticated streaming libp2p dispatch.
6. Backend/model readiness and worker admission.
7. API, SSE and cancellation endpoints.
8. Frontend live chat, worker approval and trace view.
9. Two-node LAN E2E and failure matrix.
10. Docker/native packaging and runbook.
11. Capabilities profiler and verified benchmark.
12. Replica routing, batching and cache affinity.
13. Signed registry deltas and reconciliation.
14. Security hardening and adversarial tests.
15. Reputation, verifier and quarantine workflow.
16. Privacy, tenants, locality and retention.
17. Scoped agent operations and approvals.
18. Governance and emergency procedures.

## Protocol contract

Each request includes `protocol_version`, `request_id`, `trace_id`, `idempotency_key`, `model_hash`, `model_version`, `deadline_at`, `privacy_policy`, `tenant_id`, `execution_plan_id` and `reservation_id`.

Each stream message includes `message_type`, `request_id`, `sequence`, `sent_at`, `payload` and a terminal state when final. Terminal states are exactly `COMPLETED`, `CANCELLED`, `TIMEOUT`, `FAILED`, `REJECTED` or `FALLBACK_FAILED`.

## Test matrix

LAN: pairing, reconnect, real inference, ordered streaming, cancellation, worker shutdown, fallback and leakage prevention.

WAN/public mesh: stale announcements, forged PeerId, replay, revoked worker, protocol mismatch, latency budget, sensitive-data locality and partition reconciliation.

Model supply chain: corrupt artifact, wrong hash, unsupported quantization, incompatible tokenizer, revocation and rollback.

Performance: concurrency 1/4/16/64, short/medium/long contexts, outputs 32/128/512/2048, cold/warm start, cache hit/miss, p50/p95/p99, TTFT and resource usage.

## Production definition

A release is production-validated only when documented commands start the deployment, a real request is executed by a trusted worker, output streams correctly, cancellation/failure behave correctly, metrics/audit reconstruct the run, security negative tests pass and the operator can recover or roll back.
