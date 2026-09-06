# M17 Collective Orchestration Scope v0.1 — DECIDED 2026-09-06 (autonomous)

M17 connects what already exists live: external agents via gateway (M16) +
`AgentOrchestrator` plan/assign/delegate (operator HTTP, P3.5/P9) + M18
escrow/contracts/trust (live MCP). The missing middle — and ALL this scope
adds — is: **external agents as providers/requesters + settlement on
verified completion + MCP surface through the M16 pipeline.** No parallel
systems; agents REQUEST, the fabric DECIDES.

## 1. Goal (v0.1)

A requester (local, `dca_`, or `dga_` agent with quota) proposes multi-stage
work → fabric plans/assigns across LOCAL + P2P-remote + EXTERNAL providers →
stages execute through existing paths → each VERIFIED stage settles value
(requester → executor, quota credits, internal ledger only) → trust updates.
One live loop: propose → assign → execute → verify → settle → trust.

## 2. Invariants (inherit M15/M16 +)

1. **Agents request, fabric decides.** Orchestration tools never self-execute
   privileged/economic actions; per-stage authz = the same pipeline as M16.
2. **No chain from orchestration.** Settlement moves QUOTA CREDITS on the
   internal ledger only. TestnetEconomic/Privileged stay structurally
   unreachable from gateway (M16 §2 upheld).
3. **No autonomous spending.** Every paid stage needs requester quota reserved
   BEFORE assignment; bounded price per stage; requester ceiling absolute.
4. **Deny by default + no oracle.** Unknown plan/stage/provider → deny, no
   distinguishable reason to outsiders.
5. **Idempotent settlement.** Key `(plan_id, stage_id)` — retry/settle twice
   moves value once. Every lease expires; every failure releases.
6. **Evidence before credit.** No settlement without verified stage output;
   verification = existing `verify_value`/schema path, never re-scraped.
7. **No secrets/prompts in audit.** Counters + ids + verdicts only.

## 3. Deliverables

### A. External provider registry (pure + store)
- `orchestration::provider`: unified view over LOCAL workers + P2P-remote
  agents + EXTERNAL `dga_` agents (capabilities from credential grants,
  liveness timestamped, stale > 60s excluded).
- Deterministic selection: hard gates (capability fit, freshness, price ≤
  requester bound) → score → tie-break agent_id asc. Same fairness discipline
  as DFCP (contribution bias ≤ ±0.15, never dictator).
- Registration of external providers is LOCAL-operator or self-advertise
  with valid `dga_` (caps bound to credential grants — cannot claim more).

### B. Settlement hook (pure core + ledger calls)
- On VERIFIED stage completion: `settle_stage(plan_id, stage_id, requester,
  executor, price)` — idempotent; moves quota credits requester → executor
  through the M18 escrow path where accounts exist, else contribution-credit
  award; records trust signal on success/failure.
- Pure decision (`SettleDecision`) separated from ledger mutation; mutation
  only via existing ledger/escrow APIs.

### C. MCP surface (through M16 pipeline, explicit-grant gated)
- `orchestrate_propose` — build + validate DAG (≤8 stages, closed schema) +
  dry-run assignment. Read-only cost: requester quota untouched.
- `orchestrate_status` — plan state (stages, verdicts, settlement state).
- Both require explicit `orchestrate` capability grant (like `execute`).
  Actual execution dispatch stays fabric-side (existing orchestrator loop).

### D. Audit/evidence
- `orchestrate_propose / orchestrate_assign / orchestrate_settle` events:
  ids + verdicts + amounts, no prompts/outputs/secrets. Pre-settle record
  fail-closed; post records best-effort.

### E. CLI (operator): `orchestrate propose/status` + tests minimum
- Pure matrix (gates/scoring/tie-break/idempotency/deny).
- Integration: propose→assign→verified-settle→trust + double-settle-once +
  no-provider-deny + secret/prompt byte scans + full gates green.

## 4. Explicit NON-goals (STOP conditions)
- No chain TX, no mainnet, no tokenomics changes, no new economic paths.
- No TestnetEconomic/Privileged execution via gateway or orchestration.
- No M15/M16 behavior changes; no #94; no M18-contract rewrites (reuse only).
- No autonomous background spending loops (requester-initiated only).
