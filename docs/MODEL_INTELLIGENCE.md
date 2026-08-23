# MODEL INTELLIGENCE — Model Colony foundation

> Branch: `feat/model-intelligence` · Namespace: Agent-OS line (post-M19).
> This is NOT M20 (model parallelism) — parallelism stays parked.

## What this phase is

DecentraAI can now **evaluate and route between multiple local models by
capability and verified observed performance** — without pre-deciding a
winner. Three Q4 candidates form the first colony; evidence decides who
serves what.

```text
             GOVERNOR  (proposes the capability it needs)
                 │
        deterministic ROUTING  ◄── Collective Memory (verified observations)
        hard gates → score → fallbacks
                 │
     qwen3-1.7b · gemma-3-1b · phi-4-mini      ← registry facts only
                 │
              RESULT ──► graded verdict + evidence ref
                 │
   model_performance.record_observation()      (verified → memory)
                 │
        Training Lab benchmark corpus (24 tasks)
                 │
        compare_shadow_models()  ── recommendation ONLY
                 │
        OPERATOR applies governance transition
                 │
            approved production model
```

## The two axes (never conflated)

| Axis | Values | Meaning |
|---|---|---|
| `AvailabilityState` | available / degraded / unavailable | runtime fact on THIS node |
| `GovernanceStage` | experimental → shadow → candidate → approved; ↘ rejected (terminal) | what traffic the model MAY take |

Routing gates consume both. A governance conclusion can never be implied by
a runtime blip.

## Deterministic routing (`fabric::model_routing`)

Hard gates first — a gate failure is a REJECTION with reason (auditability),
never a silent penalty:

1. capability claimed (closed `CapabilityKind` taxonomy)
2. traffic class × governance stage (production=approved only;
   shadow=shadow/candidate; benchmark=anything not rejected)
3. availability ≠ unavailable
4. context_length ≥ required

Then the integer-only score:

```text
score = 2 × effective_capability_strength        # verified claims count double
      + quality points                           # verified success% × 3
      + latency points                           # 150 − ms/20, floor 0
      − degraded penalty                         # −80 when runtime degraded
      + context headroom                         # up to +50 for comfortable fit
      − footprint-proportional pressure penalty  # small models shine under pressure
```

Cold-start floors (quality 120 / latency 60): unobserved models compete,
but a single strong verified observation outweighs any inferred claim.
Ties break by `model_id` ascending. Same inputs → same selection.

## Performance observations (`distributed::model_performance`)

Every graded execution with an evidence reference becomes a VERIFIED
`model_evaluation` memory entry in scope `model.intel`:

- deterministic entry id `mi:<model>:<task>:<evidence>` → re-running the
  same benchmark batch is an exact-duplicate no-op (honest counting)
- subject key `model:<id>:<task>` groups per-task history
- aggregation is integer math over verified entries only; empty = honest
  zeros, never fabricated numbers
- **nothing here trains anything**

## Benchmark corpus (`benchmark_datasets::model_intelligence_tasks`)

24 self-contained tasks, 12 areas × 2, all with short gold answers gradable
by the existing `grade_answer`: governor role, core invariant, architecture
verification (BLAKE3/merkle), DFCP message order, delegation identity,
MCP/consumer keys, structured output discipline, security posture, failure
recovery/quarantine, collective memory lifecycle, hallucination resistance,
and **Romanian language** (graded against Romanian golds).

## Shadow mode

`compare_shadow_models(production, candidate)` — pure and deterministic:
- `<8 graded samples/side` → `InsufficientEvidence`
- candidate accuracy ≥ +10 % → `OperatorReviewRecommended`
- accuracy near-tie (≤1 %) → latency breaks the hair band only there
- otherwise `KeepProduction`

A recommendation NEVER promotes: promotion is the operator-applied
governance transition (`candidate → approved`) after reviewing evidence.
The invariant holds: AI proposes → deterministic policy decides → workers
execute.

## Colony candidates (seeds — all EXPERIMENTAL, all claims INFERRED)

| model_id | ctx | ram needed | strengths (inferred) | romanian |
|---|---|---|---|---|
| `qwen3-1.7b-q4` | 32k | ~3 GiB (+2 GiB floor) | tool calling 80, reasoning 75, structured 75 | 60 |
| `gemma-3-1b-q4` | 32k | ~2 GiB (+2 GiB floor) | summarization 75, classification 70, chat 70 | 70 |
| `phi-4-mini-q4` | 16k | ~3 GiB (+2 GiB floor) | structured 80, function calling 75, reasoning 70 | 50 |

Registry-first: these are manifest records, not downloads. Deployment onto
a worker happens later through the existing admission/planner path, using
the hardware fit check (`HardwareRequirements::fits`).

## API surface (operator+)

- `GET /v1/models/intel` — colony view: registry facts joined with live
  availability and verified observation aggregates. Read-only.
- `POST /v1/models/route` — DRY-RUN projection: `{capability,
  min_context_tokens?, traffic?}` → selected + fallbacks + every rejection
  reason. Advisory only; the planner still owns real placement.

## Remaining work for actual deployment

1. Pull GGUF artifacts onto a worker via the existing verified transfer path
   (manifest + chunk verification already exist).
2. Feed live RAM pressure from the system probe into routing at plan time
   (the dry-run endpoint passes 0 today).
3. Wire shadow task fan-out: run the same corpus against production +
   shadow models on a schedule and persist observations.
4. Operator UI for governance transitions (the state machine already gates
   them; a small dashboard panel closes the loop).
