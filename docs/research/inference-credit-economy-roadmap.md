# Inference Credit Economy — Roadmap

## Purpose

Implement the contribution-credit system incrementally on `research/inference-credit-economy` while preserving `main`.

## Phase 0 — Research / contract freeze

- Review `inference-credit-economy.md` with maintainers.
- Inventory exact existing DecentraAI types for discovery, resource advertisements, reservations, execution, usage, and signed receipts.
- Identify the minimum extension points; avoid duplicated resource/scheduler abstractions.
- Freeze canonical terminology: `Contribution`, `Usage`, `Receipt`, `CreditEvent`, `Reservation`, `Settlement`, `CU`.

Exit criteria:

- no unresolved ownership ambiguity for each ledger event
- explicit source-of-truth identified for usage and balances
- main branch remains untouched

## Phase 1 — Ledger core

Implement:

- `CreditAmount` fixed-precision type
- `CreditAccount`
- `CreditEvent`
- `CreditReservation`
- append-only ledger interface
- idempotency / unique source event rules
- balance reconciliation

Tests:

- earn
- spend
- reserve
- release
- settle
- duplicate event
- concurrent reservations
- insufficient balance
- crash/retry recovery

## Phase 2 — Contribution verification

Implement:

- `ResourceContribution`
- contribution lifecycle: pending → verified → settled/rejected
- receipt-to-contribution mapping
- quota reconciliation
- policy version pinning
- contributor provenance

Tests:

- valid signed receipt
- invalid signature
- replay
- mismatched reservation
- usage > advertised quota
- missing provider confirmation

## Phase 3 — Policy engine

Implement a deterministic, operator-settable `CreditPolicy`.

Inputs:

- resource type
- provider/model
- measured usage
- reliability/quality policy where applicable
- policy version

Outputs:

- contribution CU
- consumer cost CU

Never use floating-point balances for authoritative accounting.

## Phase 4 — Scheduler integration

Add credit-aware scheduling as an opt-in layer:

```text
request
  ↓
estimate
  ↓
credit reservation
  ↓
resource reservation
  ↓
execution
  ↓
receipt
  ↓
settlement
```

Failure behavior:

- task rejected before execution → release reservation
- provider failure → release/refund according to policy
- partial usage → settle actual usage and release remainder
- ambiguous outcome → hold reservation until reconciled

## Phase 5 — API resource providers

Add a generic provider adapter for API quota.

Requirements:

- encrypted/local credential storage
- no secrets in advertisements
- provider-specific rate-limit handling
- quota reset time
- usage capture
- optional provider accounting verification
- automatic stop when quota is exhausted

Initial adapters should be proof-of-concept only and must pass provider ToS/acceptable-use review before production sharing.

## Phase 6 — OpenCode integration

Expose an OpenAI-compatible gateway:

- `/v1/models`
- `/v1/chat/completions`
- streaming
- usage reporting

The gateway should be provider-neutral and route according to resource availability and CU budget.

## Phase 7 — Cross-resource economy

Demonstrate:

```text
API contribution → CU
GPU contribution → CU
CU → API consumption
CU → GPU consumption
```

Add resource-policy compatibility checks so a client cannot spend CU on disallowed resource classes.

## Phase 8 — Reputation / anti-abuse

Add:

- node reliability score
- receipt rejection rate
- timeout rate
- dispute history
- contribution caps
- per-identity rate limits
- anti-self-dealing rules
- optional redundant verification for high-value work

Reputation must influence scheduling/rewards only through explicit policy; it must not mutate the authoritative ledger directly.

## Phase 9 — Crypto adapter

Only after internal accounting is stable:

- `SettlementEngine` trait/interface
- `InternalSettlement`
- `CryptoSettlementAdapter` mock
- CU lock/unlock state machine
- transaction correlation IDs
- external confirmation / failure / retry semantics

No wallet implementation or chain SDK belongs in DecentraAI core until a specific chain and settlement model are selected.

## Phase 10 — Production hardening

- property-based tests
- load tests
- concurrency tests
- database/replay recovery tests
- ledger audit tooling
- metrics and tracing
- documentation
- migration tooling
- security review

## Success scenario

A single end-to-end acceptance test should eventually prove the original use case:

1. Node A has 100,000 temporary API quota.
2. Node A advertises it without exposing its API key.
3. Nodes B/C consume 60,000 of the quota.
4. Provider/resource usage is verified.
5. Node A receives the configured amount of settled CU.
6. The provider's original daily quota later expires/resets.
7. Node A's CU remains valid.
8. Node A spends some CU days later on a different eligible provider/resource.
9. Replaying the receipts does not change balances.
10. Concurrent spending cannot overspend the account.

This scenario is the canonical functional proof of the economic layer.