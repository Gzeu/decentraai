# DecentraAI Architecture and Ownership

## Four planes

### Identity plane

Owns node keys, PeerId, pairing, signatures, TrustGrant, scopes, revocation and protocol authentication.

Primary areas: `crates/identity`, `crates/discovery`, `crates/security` when introduced.

### Control plane

Owns discovery, model registry, worker registry, policy, scheduler, execution plans, reservations, reputation and administrative APIs.

Primary areas: `crates/discovery`, `crates/registry`, `crates/distributed`, `crates/node-cli`, future `crates/api` and `crates/scheduler`.

### Data plane

Owns authenticated P2P streams, dispatch, worker queues, inference adapters, model execution, cancellation and token streaming.

Primary areas: `crates/p2p`, `crates/protocol`, `crates/distributed`, future `crates/inference-adapter`.

### Evidence plane

Owns audit, metrics, traces, benchmarks, provenance, incidents and optional settlement records.

Primary areas: `crates/audit`, `crates/monitoring`, `docs/M11_BENCHMARK_MATRIX.md`, future benchmark and settlement modules.

## Service ownership rules

- Identity owns cryptographic identity; no other crate creates a second key format.
- Protocol owns wire schemas; frontend and backend adapters do not define P2P messages.
- Discovery owns peer finding; inference does not implement its own discovery mechanism.
- Registry owns model metadata; workers report availability but do not redefine model identity.
- Scheduler owns plan selection; workers may accept or reject but cannot silently alter a plan.
- Reservation coordinator owns capacity leases; queue code cannot create untracked capacity.
- Adapter owns provider-specific JSON; protocol never exposes llama.cpp/vLLM/SGLang payloads.
- Audit owns immutable lifecycle events; metrics own measurements.
- Frontend owns presentation state, never trust or routing policy.

## Critical state machines

```text
Node: CREATED → IDENTITY_READY → DISCOVERING → VERIFIED → AVAILABLE
      → BUSY → DEGRADED → QUARANTINED → REVOKED/OFFLINE

Peer: UNTRUSTED → DISCOVERED → PAIRING_PENDING → VERIFIED
      → USER_APPROVED → TRUSTED → DEGRADED/REVOKED

Model: DISCOVERED → DOWNLOADING → VERIFIED → SANDBOX_TESTED
       → APPROVED → AVAILABLE → DEPRECATED/REVOKED

Reservation: NONE → RESERVED → COMMITTED → ACTIVE → RELEASED/ABORTED

Request: RECEIVED → AUTHENTICATED → VALIDATED → PLANNED
         → RESERVED → DISPATCHING → ACCEPTED → QUEUED
         → PREFILLING → DECODING → STREAMING
         → COMPLETED/CANCELLED/TIMEOUT/FAILED/FALLBACK_FAILED
```

## Deployment profiles

### Local desktop + laptop LAN

- desktop acts as API/coordinator;
- laptop acts as trusted worker;
- mDNS plus direct libp2p stream;
- manual QR approval;
- local-only or LAN-only policy;
- deterministic backend test available.

### Trusted GPU cluster

- explicit cluster membership;
- verified low-latency interconnect;
- tensor/pipeline plans enabled;
- cluster traffic isolated from public mesh;
- multi-worker prepare/commit/abort.

### Public mesh

- no arbitrary tensor synchronization;
- complete model replicas preferred;
- signed manifests and reputation required;
- strict privacy policy;
- fallback and quarantine enabled.

### Offline/edge mode

- local model execution;
- queued requests while disconnected;
- registry reconciliation after reconnect;
- no assumption of permanent coordinator availability.

## Agent operating rules

1. Read `AGENTS.md` and the roadmap before modifying code.
2. Inspect existing extension points before adding crates.
3. Implement one release gate at a time.
4. Keep M10 production path free of mock inference.
5. Add tests for every state transition.
6. Add negative tests for forged, stale, revoked and malformed peers.
7. Do not claim a deployment works until Compose/config/binary names are verified.
8. Do not enable M11 parallelism before M10 E2E passes.
9. Keep high-risk automation reversible.
10. Report exact commands, failures, limitations and manual test steps in every PR.

## Required evidence per release

- source diff;
- unit tests;
- integration tests;
- E2E output;
- security test output;
- benchmark artifact;
- deployment command;
- failure/recovery procedure;
- known limitations;
- rollback procedure.

## Non-goals for the inference hot path

- blockchain consensus for each token;
- arbitrary remote code execution;
- unverified public worker admission;
- raw prompt logging;
- irreversible automatic upgrades;
- model sharding across unknown internet peers.
