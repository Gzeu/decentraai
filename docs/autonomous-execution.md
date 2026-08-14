# Autonomous execution (M23 Full Autonomy) — honest status

This document states, plainly, what is **operational today** versus what is
**foundation only**. It follows the DecentraAI rule: never mark a capability
done unless the running runtime can execute it.

## What is genuinely OPERATIONAL (production-verified on main)

The autonomous **decision** and **adaptation** model is live and observable:

- **Explainable execution decision per request.** Every routed compute-path
  request produces an `ExecutionDecision`: workload class (completion /
  streaming / continuation / batch), request priority, every candidate worker
  with its hard-constraint result (trusted, healthy, serves-model, context /
  RAM / VRAM fit, engine compatibility) and its score breakdown, the selected
  worker, the plan + fallback orders, expected execution mode, network reach
  cost from the measured peer graph, KV/session affinity, engine capability,
  and a lifecycle **trace** (discovered → classified → planned → reserved →
  executing → adapting → replanned → completed/released/failed).

- **Event-driven adaptation.** On a retryable transport failure the
  coordinator applies `decentraai_fabric::adapt()` — the OBSERVE → ADAPT /
  RECOVER / REPLAN step — which uses real state (retryable, whether the session
  is a continuation, remaining re-plan budget, remaining eligible workers) to
  decide **Retry / Replan / Abort**. It is idempotency- and safety-bound: a
  request that already produced output is never retried (no duplicated
  partial output), and a definitive worker rejection or cancellation is never
  re-sent.

- **Priority-aware planning.** Higher-priority requests are steered toward the
  fastest, least-queued available worker (latency/queue boost proportional to
  priority).

- **Control-plane visibility.** `/v1/execution` returns the decisions alongside
  executed plans, and the dashboard shows an "Autonomous decisions (M23
  lifecycle)" card with the trace and per-candidate reasons — safe operational
  facts only, **never chain-of-thought or request content**.

These reason from real runtime state: worker health, CPU/RAM, GPU/VRAM, model
compatibility, engine capability, context/KV/session locality, network
latency/bandwidth/topology, throughput, queue depth, reservations, request
priority/type/streaming, failures/retryability, availability, provisioning and
current execution state.

## What remains foundation (NOT claimed)

DecentraAI does **not** yet execute **multi-worker** inference across the
fabric, and does **not** fake it:

- **No tensor / pipeline / expert-MoE / layer splitting, no KV migration.**
  `PlanKind` stays `Single` unless a real engine advertises the supporting
  capability, which no engine DecentraAI runs advertises today. The fan-out /
  sequential plan forms and the `fan_out_candidacy` advisory exist and are
  honest, but they are gated off and never produce a split the runtime cannot
  execute.
- **No autonomous re-selection mid-stream.** A running streaming generation is
  single-attempt by design (M24); adapting a request after output exists is not
  done, because it would duplicate tokens to the client.

## Lifecycle this phase implements

```text
REQUEST → DISCOVER fabric → CLASSIFY workload → CANDIDATES → HARD CONSTRAINTS
        → SCORE/OPTIMIZE → SELECT plan → RESERVE → EXECUTE → OBSERVE
        → ADAPT / RECOVER / REPLAN → COMPLETE → RELEASE
```

DISCOVER → SELECT, OBSERVE → ADAPT, and the explainable decision + trace are
implemented. RESERVE/EXECUTE/RELEASE run through the existing reservation,
P2P fabric and streaming executor (M18/M20/M24) unchanged. Real multi-worker
*execution strategies* slot in here as engine capabilities mature — the
architecture is extensible so they can be added without rewriting the fabric.