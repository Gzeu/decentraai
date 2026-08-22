# Skill: sharing — Compute Assist quick reference

## Trigger (M15 will automate this)

Today: `POST /v1/intel/assist` (operator+):

```json
{
  "capability": "chat",
  "cpu_cores": 2,
  "ram_mb": 512,
  "lease_seconds": 60,
  "payload": {"messages":[...], "max_tokens": 15}
}
```

## What happens

1. RESOURCE_REQUEST broadcast to connected trusted peers.
2. Each worker answers with an owner-limit-checked OFFER or silence.
3. Deterministic selection: hard gates → fairness-scored ranking.
4. RESERVE handshake against the winner's ledger (lease starts).
5. ASSIST_TASK_ASSIGN; the worker executes on ITS local engine.
6. RESULT returns inside the lease window (+5s transport slack).
7. RELEASE on success; TTL backstop otherwise.
8. Credit recorded for the worker through the existing ledger — success
   only.

## Verified live (2026-08-22, 3 nodes)

Laptop → Desktop chat assist: 845ms round-trip, Llama-1B real completion,
full cycle in journals on both nodes. Worker down → clean fail in 77ms,
no lease leak, zero false credit.

## Executors available worker-side

- `embeddings` → local `/v1/embeddings`
- `chat` / `text_generation` → local `/v1/chat/completions`

Adding an executor = one more branch in
`runtime::intel_assist::execute_capability` + tests.
