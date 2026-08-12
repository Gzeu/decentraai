# M10 Production Vision

## Mission

Turn DecentraAI from a modular distributed-inference prototype into a complete, production-oriented application: a user can open the frontend, authenticate, select a model, submit a prompt, receive streamed output from a trusted remote worker, and inspect the complete request lifecycle in the dashboard.

## Product flow

User -> Frontend Chat -> Node API -> Auth/Policy -> Request Router -> Trusted Worker -> llama-server -> Streaming response -> Metrics/Audit -> Dashboard

## Non-negotiable principles

1. Real inference only: no mock response on the production path.
2. Secure by default: every worker announcement and inference request is bound to an authenticated peer identity.
3. Observable by default: every request has request_id, trace_id, lifecycle state, latency and final outcome.
4. Bounded resources: queue depth, prompt size, output tokens, concurrency and deadlines are enforced server-side.
5. Recoverable operation: node restart, worker failure and network retry must not corrupt request state.
6. Contract-first integration: Rust protocol types, API types and frontend types must remain aligned.
7. One vertical slice before feature expansion: prove two-node inference end-to-end before adding more tokenomics or marketplace features.

## End-to-end acceptance criteria

- `decentraai init` creates a usable node identity and validated configuration.
- A worker can be paired, approved and revoked from the control plane.
- A worker publishes model hash, capacity, health and readiness.
- The frontend can create a chat and submit an inference request.
- The node authenticates and validates the request before routing it.
- The router selects only an eligible trusted worker.
- The worker calls a real llama-server/OpenAI-compatible backend.
- Tokens stream back to the client through SSE or WebSocket.
- Cancellation propagates from client to worker/backend.
- Queue overflow, deadline expiry, worker failure and retry are deterministic.
- A failed worker is removed from eligibility and fallback is attempted when safe.
- Dashboard shows workers, models, queue depth, throughput, errors and P50/P95/P99 latency.
- Audit records include request_id, trace_id, client identity, worker identity, model hash, timestamps and outcome.
- A two-node automated E2E test runs without manual intervention.
- Docker Compose starts a reproducible local network with one coordinator, one worker and a mock/real inference backend.

## Target architecture

### Data plane

- `crates/distributed`: P2P messages, worker coordination and request transport.
- `crates/inference-adapter`: llama-server/OpenAI-compatible adapter, streaming and cancellation.
- `crates/scheduler`: eligibility, scoring, retries, fallback and circuit breakers.
- `crates/protocol`: versioned wire contracts and lifecycle states.

### Control plane

- `crates/api`: health, node status, workers, models, inference, SSE and admin endpoints.
- `crates/security`: token validation, capabilities, signatures, replay protection and rate limits.
- `crates/audit`: immutable request and administrative audit events.
- `crates/monitoring`: structured logs, metrics, health and alert data.

### Frontend

- Chat and streamed responses.
- Worker pairing, approval, revoke and readiness.
- Model registry and availability.
- Network, logs, metrics and request trace view.
- Admin tokens, roles, quotas and audit events.

## Required request lifecycle

`received -> authenticated -> validated -> queued -> assigned -> running -> streaming -> completed`

Terminal alternatives are `failed`, `cancelled` and `timeout`. Every transition must be observable and tested.

## Security gates

- Verify announcement signature and bind announced peer_id to transport peer_id.
- Reject expired, replayed or malformed announcements.
- Enforce capability: model, max tokens, context and concurrency.
- Apply per-token, per-peer and global rate limits.
- Never log prompts, secrets or bearer tokens by default.
- Use server-side limits even when clients provide their own values.
- Fail closed when worker trust or readiness is unknown.

## Reliability gates

- Shared queue state must not be cloned.
- Per-worker queues are bounded and cancellation-safe.
- Request deadlines use a server-side maximum.
- Retries require idempotency and only happen for transient failures.
- Circuit breakers prevent repeatedly routing to unhealthy workers.
- Restart recovery explicitly handles queued, running and unknown requests.
- Health checks distinguish process health, backend health and model readiness.

## Delivery phases

### Phase 1: real data plane

Fix shared queue semantics, add the real inference adapter, wire `DistributedP2PHandler::with_both`, implement streaming, cancellation and lifecycle events.

### Phase 2: secure control plane

Add authenticated API endpoints, worker identity binding, capabilities, rate limits, request correlation and audit events.

### Phase 3: integrated product

Connect the SvelteKit chat and dashboards to live APIs; add worker/model/admin screens and an operator-friendly error model.

### Phase 4: production proof

Add two-node E2E tests, failure injection, Docker Compose, CI gates, security checks, runbook and release checklist.

## Definition of done

M10 is done only when the same documented command sequence starts the stack, pairs a worker, submits a prompt from the frontend, streams a real response, displays metrics, records an audit event, survives a worker failure and passes automated CI.

## Explicit non-goals for the first vertical slice

- On-chain settlement in the critical inference path.
- Tokenomics-driven scheduling before reliability is proven.
- Unbounded public worker admission.
- Mock inference presented as production readiness.
