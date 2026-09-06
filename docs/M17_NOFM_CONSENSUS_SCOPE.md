# N-of-M Consensus Scope v0.1 — DECIDED 2026-09-06 (autonomous)

Next item on the agreed queue (after M17 complete). M17 stages are executed
by ONE provider and verified by ONE path (`verify_value`). This scope adds
**replicated execution with consensus**: the SAME stage runs on N providers,
outputs are compared deterministically, and a verdict comes from the
EXISTING `evaluate_consensus` (P4/P12) — no second consensus language.

## 1. Goal

A requester may request `replicas: N` (default 1, max 3) for a stage. The
fabric offers the stage to N distinct providers, each output is verified
independently, outputs are grouped by CANONICAL content, and the largest
group reaching the agreement threshold decides the stage verdict.
Settlement remains per-verified-replica through the existing idempotent
keys — replicated execution never overspends the requester bound
(`replicas × max_price ≤ budget`).

## 2. Invariants (inherit M16/M17 +)

1. **ONE consensus language**: verdicts use
   `decentraai_agents::verification::evaluate_consensus` only.
2. **Deterministic grouping**: outputs cluster by canonical JSON bytes
   (never by fuzzy similarity in v0.1 — "same answer" means "same bytes after
   canonicalization"). Distant-but-equivalent prose is honestly reported as
   disagreement.
3. **Per-replica idempotency**: credit keys extend to
   `m17cr:{plan}:{stage}:{replica}`; reservation per replica
   `m17res:{plan}:{stage}:{replica}`. Retry/restart cannot double-pay.
4. **Bounded fan-out**: `1 ≤ replicas ≤ 3`; each replica is a distinct
   provider (no provider claims two replicas of the same stage).
5. **Price bound**: `replicas × stage max_price ≤ requester budget_cap`,
   checked BEFORE assignment; existing per-stage `MAX_STAGE_PRICE` holds
   per replica.
6. **No oracle for providers**: each replica sees only the stage input, never
   sibling outputs.
7. **Honest Partial**: if fewer providers than `required` deliver verified
   output, verdict is `Uncertain` (existing enum), never fabricated Verified.

## 3. Deliverables

### A. Pure core (agents::orchestration)
- `ReplicaAssignment`: per-replica state machine (multi-claim support).
- `ConsensusInput { outputs: Vec<(agent_id, canonical_bytes)>, confidence }`.
- `consensus_verdict(inputs, replicas_required, threshold) -> VerificationVerdict`
  built on `evaluate_consensus` with `ConsensusPolicy`.
- `settle_for_replica(plan, stage, replica) -> SettleDecision` (keys above).

### B. Store + MCP
- `AssignmentStore`: `assign_many`, `claim_replica`, `submit_replica`;
  per-replica fields; stage verdict derived from replica verdicts.
- `orchestrate_propose`: optional `replicas` per stage (≤3); dry-run shows
  the N selected providers (distinct) or the honest unassigned reason.
- `orchestrate_status`: per-replica ledger lines (assigned/claimed/verified),
  final `stage_verdict`; never raw outputs.

### C. Tests
- Pure: grouping determinism, threshold edges (2/3, 1/3), per-replica
  idempotency keys, replica budget bound, distinct-provider guarantee,
  Uncertain on insufficient replicas.
- Integration: propose with replicas=2 → two distinct providers; status
  shows replicas; no outputs leaked; deny cases unchanged.

### D. Explicit NON-goals
- No fuzzy/LLM semantic similarity (v0.1 = canonical bytes only).
- No change to single-replica behavior (default `replicas:1` = M17 exactly).
- No N-of-M for `AgentOrchestrator` push path (operator HTTP) — pull model
  only, where value concentrates first.
- No chain/tokenomics/P2P-durability work (queue items stay queued).
