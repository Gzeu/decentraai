# Agent Task: Implement M10 Production Vertical Slice

## Context

Repository: `Gzeu/decentraai`
Branch: `feature/m10-inference-vertical-slice`
Read first: `AGENTS.md`, `README.md`, `ROADMAP.md`, `docs/DISTRIBUTED_INFERENCE.md`, `docs/M10_PRODUCTION_VISION.md`.

## Objective

Implement the smallest complete vertical slice that proves two-node distributed inference with a real llama-server/OpenAI-compatible backend, authenticated worker routing, streamed output, observability and automated tests.

## Rules

- Do not use mock inference on the production path.
- Do not bypass identity, trust or authorization checks.
- Do not weaken existing security checks to make tests pass.
- Preserve backward compatibility where practical; document intentional API changes.
- Prefer small commits and keep each commit compiling.
- Never place credentials, bearer tokens or real secrets in source, fixtures or logs.
- Add tests with every behavior change.
- Update documentation and configuration examples with implementation changes.

## Required implementation order

1. Inspect current P2P `RequestHandler`, protocol serialization and node construction.
2. Verify the queue fix with concurrent enqueue/dequeue/cancel/timeout tests.
3. Add an inference adapter trait with a llama-server/OpenAI-compatible implementation.
4. Support non-streaming first, then streaming and cancellation.
5. Construct the distributed handler with both worker manager and inference callback before `P2PNode::new()`.
6. Add request lifecycle state and correlation IDs.
7. Add peer identity binding and announcement verification.
8. Add API endpoints for health, workers, models and inference streaming.
9. Connect the frontend chat to the live API.
10. Add a two-node E2E harness and failure tests.

## Expected deliverables

- Production-safe inference adapter.
- P2P handler wired to the real backend.
- Server-side limits and deadline enforcement.
- Authenticated worker admission.
- SSE or WebSocket streaming with cancellation.
- Metrics and audit events for every request.
- API and frontend integration.
- Unit, integration, security and two-node E2E tests.
- Docker Compose local deployment.
- Updated README, configuration examples and operations runbook.

## Validation commands

Run the applicable commands and report exact output:

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cd frontend && npm ci && npm run check && npm run build
```

If a command cannot run because an external service is unavailable, add a deterministic test double and document the limitation; do not silently skip the test.

## Acceptance tests

- A trusted worker with a healthy backend receives an inference request.
- An untrusted or mismatched peer is rejected.
- An unavailable model is rejected without routing.
- Streaming emits ordered chunks and a terminal event.
- Cancellation stops backend work and closes the stream.
- Queue limits reject excess work deterministically.
- Deadline expiry produces a typed timeout result.
- Worker failure triggers safe fallback at most once per policy.
- Metrics and audit contain the same request and trace IDs.
- Two-node E2E passes from startup to streamed response.

## Completion report

Before opening a PR, report:

- Files changed and architectural reason.
- Commands executed and pass/fail status.
- Known limitations.
- Security implications.
- Migration or configuration changes.
- Exact manual test steps for the operator.
