# M10 Agent Execution Checklist

## Operating mode

Work only on `feature/m10-inference-vertical-slice`. Read `AGENTS.md`, `docs/M10_PRODUCTION_VISION.md`, `docs/M10_IMPLEMENTATION_BLUEPRINT.md`, `docs/api/m10-openapi.yaml` and this file before editing.

Use small commits. After each task run the narrowest relevant tests. Do not mark a task complete from compilation alone.

## Phase 0: repository truth

- [ ] Inspect current workspace members, package names and binary names.
- [ ] Inspect the actual `RequestHandler` trait and `ChainedHandler` behavior.
- [ ] Inspect current protocol serialization and identity/signature APIs.
- [ ] Inspect existing HTTP/frontend architecture before adding a new server.
- [ ] Inspect actual config schema before creating YAML examples.
- [ ] Record mismatches between the M10 documents and current code.

Stop condition: no implementation starts until every proposed new module has an owner and existing extension point.

## Phase 1: backend adapter

- [ ] Add a backend-neutral adapter trait.
- [ ] Implement OpenAI-compatible non-streaming completion.
- [ ] Implement streaming chunk conversion.
- [ ] Implement cancellation using request context/deadline.
- [ ] Add connect/request timeout and server-side limits.
- [ ] Add deterministic mock HTTP server tests only for the adapter; never use the mock as production inference.
- [ ] Redact prompts, tokens and authorization headers from errors/logs.

Validation:

```bash
cargo test -p <adapter-package>
cargo clippy -p <adapter-package> --all-targets -- -D warnings
```

## Phase 2: real worker wiring

- [ ] Construct the inference callback before `P2PNode::new()`.
- [ ] Use `DistributedP2PHandler::with_both` or an equivalent composed handler.
- [ ] Verify registry/model readiness before worker registration.
- [ ] Return typed errors for unavailable model/backend.
- [ ] Add an integration test proving an `InferRequest` reaches the adapter.

Stop condition: no mock response exists on the worker production path.

## Phase 3: trust boundary

- [ ] Bind announcement peer ID to transport peer ID.
- [ ] Verify announcement signatures.
- [ ] Reject expired, replayed, malformed or revoked workers.
- [ ] Enforce model/capacity/token/context capabilities.
- [ ] Add tests for forged, replayed and mismatched announcements.

Stop condition: an unauthenticated worker cannot influence routing.

## Phase 4: lifecycle and reliability

- [ ] Define one authoritative request lifecycle state machine.
- [ ] Emit one audit/metric event per valid state transition.
- [ ] Enforce bounded shared queues.
- [ ] Implement cancellation and timeout cleanup.
- [ ] Retry only transient failures and preserve idempotency.
- [ ] Add circuit breaker and safe fallback behavior.
- [ ] Define restart behavior for queued/running requests.

Validation:

```bash
cargo test --workspace
cargo test --workspace -- --test-threads=1
```

## Phase 5: control plane and frontend

- [ ] Implement or extend the existing API boundary; do not create a parallel server without justification.
- [ ] Implement `/health`, `/ready`, workers, models, inference, cancel, metrics and audit endpoints.
- [ ] Enforce bearer token scopes and rate limits.
- [ ] Implement SSE/WebSocket terminal events and cancellation.
- [ ] Connect frontend chat to the live endpoint.
- [ ] Display request ID, worker, model, status, latency and errors.
- [ ] Add worker approval/revoke and readiness views.

## Phase 6: E2E and deployment

- [ ] Add a deterministic backend fixture for CI.
- [ ] Add two-node startup and pairing test.
- [ ] Add happy-path streamed inference test.
- [ ] Add timeout, cancellation, forged worker and worker shutdown tests.
- [ ] Verify Docker files against actual binary/config names.
- [ ] Do not require a real model download in default CI.
- [ ] Provide a documented opt-in real llama-server test.

## Required final report

Before PR, report:

- exact files changed;
- exact commands and results;
- tests skipped and why;
- security decisions;
- config/migration changes;
- known limitations;
- manual reproduction steps;
- whether the result is `scaffold`, `integration-ready`, or `production-validated`.

## Never claim complete when

- inference still returns a mock response;
- the handler callback is constructed but not attached;
- queue state is copied instead of shared;
- worker identity is not verified;
- Docker references files that do not exist;
- CI runs commands inconsistent with the repository lockfiles;
- only `cargo check` passed without integration tests.
