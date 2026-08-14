# Implementation Matrix

## Status meanings

- `DONE`: code and evidence exist.
- `PARTIAL`: foundation exists but integration or proof is missing.
- `DESIGN`: documented only.
- `BLOCKED`: safe implementation requires resolving a repository/dependency mismatch.

## Priority matrix

| Priority | Work item | Dependencies | Evidence required | Initial status |
|---|---|---|---|---|
| P0 | Real inference adapter | protocol | adapter unit/integration tests | DESIGN |
| P0 | Handler wiring | adapter/P2P | InferRequest reaches backend | PARTIAL |
| P0 | Request lifecycle | protocol/audit | legal transition tests | DESIGN |
| P0 | Worker identity binding | identity/P2P | forged/mismatch tests | PARTIAL |
| P0 | API/SSE control plane | lifecycle | API/stream tests | DESIGN |
| P0 | Two-node LAN E2E | all P0 | repeatable real inference | DESIGN |
| P0 | Deployment quickstart | CLI/config | startup/health test | PARTIAL |
| P1 | Capabilities profiler | system probe | signed benchmark profile | DESIGN |
| P1 | Replica routing | profiler/scheduler | B0 vs B1 benchmark | DESIGN |
| P1 | Batching/cache | adapter/scheduler | throughput/cache benchmark | DESIGN |
| P1 | Mesh reconciliation | discovery/registry | partition/reconnect test | DESIGN |
| P1 | Security hardening | transports | adversarial suite | DESIGN |
| P2 | Model supply chain | registry | artifact verification/rollback | DESIGN |
| P2 | Privacy/tenancy | API/policy | isolation/retention tests | DESIGN |
| P2 | Reputation/verifier | metrics/audit | bad-output/quarantine test | DESIGN |
| P3 | Self-optimization | benchmarks | explainable replayable decisions | DESIGN |
| P3 | Agent operations | policy/audit | scoped tools/approvals | DESIGN |
| P4 | Economics | usage/audit | hot-path-independent accounting | DESIGN |
| P4 | Federated governance | identity/policy | signed decisions/recovery | DESIGN |

## First implementation work packets

### WP-001: backend adapter

Add a backend-neutral trait and OpenAI-compatible adapter. Support health, completion, streaming, cancellation, deadlines, prompt/output limits and deterministic HTTP-server tests.

### WP-002: real worker wiring

Construct the real inference callback before `P2PNode::new()`. Remove mock production behavior. Prove that a dispatched `InferRequest` reaches the adapter and returns a typed response.

### WP-003: lifecycle

Add request/trace/idempotency IDs, legal transitions, exactly-one terminal event and audit events.

### WP-004: secure dispatch

Bind announced PeerId to transport PeerId. Verify plan, reservation, nonce, expiry, scopes, tenant and model capability before queue admission.

### WP-005: two-node proof

Start coordinator and worker, pair them, publish a model, reserve capacity, dispatch real inference, stream ordered tokens, cancel one request, stop the worker and verify fallback or typed failure.

## PR acceptance checklist

- [ ] Scope limited to declared roadmap steps.
- [ ] Existing extension points inspected.
- [ ] No mock inference on production path.
- [ ] Unit tests added.
- [ ] Boundary tests added.
- [ ] Negative/security tests added.
- [ ] Metrics and audit updated.
- [ ] Config/docs updated.
- [ ] Exact validation commands recorded.
- [ ] Tracker percentages updated with evidence.
- [ ] Rollback procedure documented.
