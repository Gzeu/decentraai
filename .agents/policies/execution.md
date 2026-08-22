# Policy: execution

## Who decides what

```text
AI proposes → deterministic Rust decides → workers execute
```

The planner owns WHO; the scheduler enforces capacity; the worker executes;
evidence proves; the ledger credits. Never collapse these roles.

## Execution rules

1. **Local-first when locality wins**: do not distribute work when local
   execution is clearly better.
2. **Reservations before work**: no task starts against an unreserved
   worker (planner reserves, scheduler enforces).
3. **Retry policy**: transport-level failures retry on a fresh planner-
   chosen worker up to config limits, releasing each reservation and
   re-planning per attempt. Definitive rejections and cancellations are
   NEVER retried (non-idempotent work must not duplicate). Streaming stays
   single-attempt + legacy fallback.
4. **Timeouts derive from ONE shared budget** (`decentraai_config::
   backend_request_timeout`): backend idle-read, P2P request/response,
   remote route — never shorter per-layer caps that cut healthy slow work.
5. **Streaming**: idle-read budget only, SSE keepalives while upstream is
   silent, no cumulative wall clock on streamed bodies.

## Failure handling

- Lease expires/releases on any failure path; resources are never leaked.
- No contribution credit for failed, timed-out or unverified work.
- Duplicate results are rejected safely (idempotency keys / dedup sets).

## Explainability

Selection decisions carry their factors (capability fit, resource fit,
latency/network cost, queue, trust, locality). A decision that cannot be
explained from recorded facts is treated as a bug.
