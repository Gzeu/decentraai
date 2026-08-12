# M10 Implementation Blueprint

## Goal

Deliver one production-oriented vertical slice: a client submits an authenticated request, the node validates and routes it to a trusted worker, the worker calls a real llama-server-compatible backend, tokens stream back, and metrics/audit expose the complete lifecycle.

## Boundaries

### Control plane

Owns HTTP/SSE, authentication, authorization, pairing, worker approval, model visibility, admin operations and observability queries.

### Data plane

Owns P2P transport, worker announcements, routing, queueing, inference execution, streaming chunks, cancellation, retry and fallback.

Never let frontend code call a worker directly. The node API is the policy boundary.

## Required modules

Prefer existing crates when their responsibilities match. Add a crate only when a boundary cannot be kept clear.

- `protocol`: versioned request, response, chunk, lifecycle and error contracts.
- `distributed`: P2P transport and worker coordination.
- `inference-adapter`: backend trait plus OpenAI-compatible HTTP implementation.
- `api`: HTTP endpoints, SSE, auth middleware and DTO conversion.
- `security`: token scopes, signatures, peer binding, replay protection and limits.
- `audit`: append-only request/admin events.
- `monitoring`: metrics, structured logs and health/readiness.

## Inference adapter

Define a backend-neutral trait with at least:

```rust
#[async_trait]
pub trait InferenceBackend: Send + Sync {
    async fn health(&self) -> Result<BackendHealth, BackendError>;
    async fn complete(&self, request: BackendRequest) -> Result<BackendResponse, BackendError>;
    async fn stream(&self, request: BackendRequest) -> Result<BackendStream, BackendError>;
    async fn cancel(&self, request_id: Uuid) -> Result<(), BackendError>;
}
```

The first implementation targets llama-server's OpenAI-compatible `/v1/chat/completions` endpoint. Configuration must support base URL, model, connect timeout, request timeout, max prompt bytes, max output tokens and TLS verification.

Do not expose backend-specific JSON types through the P2P protocol or frontend.

## Request lifecycle

Every request carries `request_id`, `trace_id`, client identity, model hash, creation time and deadline.

```text
received
  -> authenticated
  -> validated
  -> queued
  -> assigned
  -> running
  -> streaming
  -> completed
```

Terminal states: `failed`, `cancelled`, `timeout`, `rejected`.

Each transition emits one structured event. Repeated transitions must be rejected or made idempotent.

## Routing policy

A worker is eligible only when all conditions hold:

- trusted and not revoked;
- peer identity matches the transport identity;
- announcement is fresh and not replayed;
- model hash is available;
- backend readiness is healthy;
- queue is below its configured limit;
- worker capability allows requested tokens/context;
- circuit breaker is closed.

Routing score may consider latency, queue depth, capacity and success rate, but eligibility is always evaluated first.

## Reliability rules

- Queue state must be shared, bounded and cancellation-safe.
- Client timeout cannot exceed the node's server-side maximum.
- Retry only transient transport/backend errors.
- Never retry a request after an externally visible completion unless idempotency is proven.
- A worker failure opens its circuit breaker and triggers one policy-controlled fallback.
- On restart, queued requests are either recovered from durable state or explicitly marked unknown; never silently dropped.
- Health, readiness and liveness are separate signals.

## Security rules

- Verify announcement signature and bind announced `peer_id` to transport `peer_id`.
- Reject expired announcements, duplicate nonces and malformed capabilities.
- Require an authenticated token for control-plane inference requests.
- Enforce scopes such as `inference:submit`, `workers:read`, `workers:manage` and `audit:read`.
- Apply per-token, per-peer and global rate limits.
- Do not log prompts, bearer tokens, private keys or full authorization headers.
- Return stable public error codes without leaking internal topology.

## Delivery order

1. Protocol lifecycle/error types and adapter trait.
2. OpenAI-compatible backend adapter with deterministic tests.
3. Real handler wiring before P2P node construction.
4. Non-streaming request path.
5. Streaming and cancellation.
6. Identity binding and admission checks.
7. API/SSE control plane.
8. Frontend integration.
9. Two-node E2E and failure injection.
10. Docker, CI and operations documentation.

## Definition of done

The implementation is accepted only when a clean local environment can start the documented stack, pair a worker, submit a request through the frontend/API, receive a streamed response from the configured backend, observe metrics/audit data, stop the worker, observe safe failure/fallback, and pass the validation workflow.
