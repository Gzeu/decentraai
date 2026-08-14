# M10 E2E Test Plan

## Topology

- Coordinator node: API on `localhost:8080`, P2P on `localhost:4001`.
- Worker node: P2P on `localhost:4002`, paired and approved by coordinator.
- Inference backend: llama-server-compatible endpoint on the worker network.

## Happy path

1. Start the two-node Docker Compose stack.
2. Initialize identities and verify `/health` and `/ready`.
3. Pair the worker and approve it.
4. Confirm the worker model appears in `GET /v1/models`.
5. Submit `POST /v1/inference` with `Accept: text/event-stream`.
6. Verify ordered chunks and a terminal `completed` event.
7. Verify the same request ID in response, metrics and audit.

## Negative paths

- Missing bearer token returns 401.
- Unknown model returns a typed rejection.
- Untrusted worker is never selected.
- Announcement with mismatched peer ID is rejected.
- Expired/replayed announcement is rejected.
- Full queue returns deterministic overload error.
- Backend timeout produces `timeout` and releases queue capacity.
- Client cancellation propagates to backend and emits `cancelled`.
- Worker shutdown removes it from eligibility and invokes one safe fallback.

## Required assertions

- No prompt or bearer token appears in logs.
- Request lifecycle has no illegal transition.
- Queue depth returns to zero after completion/cancellation.
- Metrics contain request count, success/failure count and latency.
- Audit contains client, worker, model hash, timestamps and final state.
- Restart behavior is explicit and documented for in-flight requests.

## Agent output

The implementation agent must report exact commands, test output, changed files, known limitations and manual reproduction steps before opening a PR.
