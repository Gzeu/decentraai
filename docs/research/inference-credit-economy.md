# DecentraAI — Inference Credit Economy Research

Status: research / experimental
Branch: `research/inference-credit-economy`
Mainline impact: none

## Executive summary

DecentraAI can support a crypto-agnostic internal economy in which participants contribute verifiable AI resources—API quota, GPU/CPU inference, storage, or bandwidth—and receive reusable **Contribution Credits (CU)** after the contribution is measured and verified.

The key semantic rule is:

> A provider's temporary API quota is a resource. CU are durable accounting units earned from verified contribution. Expiration of the original provider quota does not invalidate already-settled CU.

Example: a node contributes 100,000 free-tier API tokens today. Other participants consume 60,000. The contributor receives the policy-defined CU for the verified 60,000 of actual usage. Those CU remain available for later consumption even after the provider's daily quota resets or expires.

The system should initially remain **off-chain and crypto-agnostic**. A future crypto settlement adapter may convert settled CU to an external token or other settlement asset without changing DecentraAI core scheduling, discovery, execution, or receipt verification.

## Design goals

1. Reuse existing DecentraAI primitives rather than creating a parallel execution stack.
2. Reward only measured and verifiable contribution.
3. Separate temporary provider quota from durable CU accounting.
4. Make CU reusable across eligible network resources, not tied to the original provider/model.
5. Prevent double-credit, replay, over-quota use, forged usage, and credit overspend.
6. Keep API keys local to contributor nodes; never advertise secrets.
7. Keep the core independent of blockchain technology.
8. Make policy and exchange rules operator-settable and versioned.
9. Preserve compatibility with OpenAI-compatible clients such as OpenCode through a gateway layer.

## Proposed accounting layers

### 1. QuotaLedger

Tracks temporary external capacity offered by a provider:

- provider
- model
- quota type
- available units
- consumed units
- reset time
- concurrency/rate limits
- contributor identity

Quota is not itself a currency and may expire.

### 2. Contribution / Compensation Ledger

Tracks verified resource contribution and its valuation under a versioned policy.

### 3. CreditLedger

Authoritative append-only CU accounting:

- earn
- spend
- reserve
- release/refund
- adjustment
- settlement-lock
- settlement-release

Every credit event must reference a unique source event and be idempotent.

## Core lifecycle

```text
RESOURCE ADVERTISEMENT
        ↓
DISCOVERY
        ↓
RESERVATION
        ↓
EXECUTION
        ↓
MEASUREMENT
        ↓
SIGNED RECEIPT
        ↓
VERIFICATION
        ↓
CREDIT POLICY
        ↓
SETTLEMENT
        ↓
CREDIT BALANCE
        ↓
FUTURE CONSUMPTION
```

Contribution state:

```text
PENDING → VERIFIED → SETTLED
                    ↘ REJECTED / DISPUTED
```

No spendable CU should be created from an unverified contribution.

## Contribution types

### API quota

Examples:

- free-tier provider quota available today
- prepaid provider tokens
- provider-specific token plans

Required metadata should include provider/model, quota scope, reset time, rate limits, and an opaque credential reference. API secrets remain local to the contributor.

### GPU / CPU inference

Possible metering units:

- GPU-seconds
- GPU memory-seconds
- CPU-seconds
- tokens generated
- successful task completion

### Storage / bandwidth

Possible metering units:

- byte-hours / GiB-hours
- ingress/egress bytes

These non-token resources should be mapped into CU through the same policy abstraction.

## Credit policy

Do not hard-code `1 token = 1 CU` as the permanent economic rule.

Use a versioned `CreditPolicy` that can consider:

- resource type
- provider
- model
- input tokens
- output tokens
- cached tokens when available
- GPU/CPU time
- service quality / reliability modifiers
- externally published pricing references
- scarcity or network policy

The policy must be deterministic for a given version and receipt, so the same receipt cannot yield different credit results unless an explicit adjustment is made.

A conceptual model is:

```text
CU = f(measured_usage, resource_type, model, provider, policy_version)
```

Avoid using only raw token volume as the sole valuation signal when resources have materially different costs or utility.

## Wallet and reservation semantics

Each participant has a wallet/account with at least:

- settled balance
- reserved balance
- pending balance
- earned total
- spent total
- history / event references

For a task consuming estimated `X` CU:

1. atomically reserve `X`
2. execute
3. settle actual usage
4. deduct final spend
5. release any unused reservation

Invariant:

```text
available_balance + reserved_balance = spendable_balance
```

A failed or expired reservation must be safely refundable exactly once.

## Signed receipts and provenance

Existing receipt infrastructure should be extended, not replaced.

A contribution receipt should bind at least:

- receipt ID
- request/task ID
- provider node identity
- consumer identity
- model/provider identifier
- metered usage
- reservation ID
- timestamps / sequence or nonce
- relevant content hashes where appropriate
- policy version
- signature

Receipt processing must be idempotent by receipt ID and protected against replay.

For API-backed resources, actual provider usage data should be preferred where the provider exposes authoritative accounting data. A node claim alone should not be sufficient for high-trust settlement.

## Security / anti-abuse

Threats and baseline mitigations:

| Threat | Mitigation |
|---|---|
| Fake usage | Signed receipts + provider/account evidence where available |
| Receipt replay | Unique receipt IDs, nonces, idempotent ledger writes |
| Double-credit | Unique source-event constraints |
| Over-quota | Quota reservation and reconciliation |
| Overspend | Atomic credit reservation |
| Fake model claim | Bind actual model/provider to receipt |
| Collusion | Rate limits, reputation, anti-self-dealing policy, audits |
| Sybil nodes | Identity cost / admission policy / reputation; do not rely on credit alone |
| Stale advertisements | TTLs, heartbeats, quota reconciliation |
| Secret leakage | Credentials remain local; ads/receipts contain references, never API keys |

## OpenCode / API integration

DecentraAI should expose an OpenAI-compatible gateway independently from the economic implementation.

Conceptually:

```text
OpenCode
   ↓
DecentraAI /v1/*
   ↓
Credit check + reservation
   ↓
Resource scheduler
   ↓
API provider / GPU worker / CPU worker
   ↓
Signed receipt
   ↓
Verify + settle
   ↓
CU debit / contributor CU credit
```

The client should not need to know which provider ultimately executed the task.

## Future crypto migration

Crypto must remain an optional settlement layer.

Recommended abstraction:

```text
CreditLedger
    ↓
SettlementEngine interface
    ├── InternalSettlement (current)
    └── CryptoSettlementAdapter (future)
```

A future adapter may:

- lock settled CU
- compute a conversion according to a versioned settlement policy
- initiate an external transaction
- reconcile transaction state
- release or burn the corresponding locked CU

Important: blockchain availability must never be a prerequisite for normal DecentraAI inference in the initial design.

Possible future flow:

```text
settled CU
   ↓
lock CU
   ↓
settlement policy
   ↓
external token / asset
   ↓
transaction confirmation
   ↓
mark settlement complete
```

The exact token, chain, issuance model, conversion rate, and legal structure are intentionally deferred.

## Recommended repository shape for this branch

```text
research/inference-credit-economy
├── docs/research/inference-credit-economy.md
├── docs/research/inference-credit-economy-architecture.md
├── docs/research/inference-credit-economy-threat-model.md
├── docs/research/inference-credit-economy-roadmap.md
└── (future implementation modules only after interfaces are reviewed)
```

No changes to `main` are required for this research track.

## Implementation milestones

### M1 — accounting skeleton

- CU types / fixed precision arithmetic
- wallet/account model
- append-only credit events
- idempotency constraints
- reservation model
- reconciliation invariants

Acceptance: deterministic unit tests prove correct earn/spend/reserve/release behavior under retries.

### M2 — verified contribution loop

- resource contribution record
- receipt integration
- verification state machine
- provider quota reconciliation
- policy versioning

Acceptance: contributor A offers 100k API units, consumer B uses a verified subset, and only the verified amount generates CU.

### M3 — cross-resource consumption

- scheduler credit checks
- consume CU on any eligible resource
- price/weight policy
- provenance query

Acceptance: CU earned from provider X can be spent on provider Y or local compute when policy permits.

### M4 — OpenCode gateway

- `/v1/models`
- `/v1/chat/completions`
- streaming compatibility
- credit reservation middleware
- usage reporting

Acceptance: OpenCode can use DecentraAI without knowing the underlying provider.

### M5 — crypto readiness

- `SettlementEngine` interface
- internal no-op/current implementation
- locked-CU lifecycle
- external settlement state machine
- no blockchain dependency in core

Acceptance: a mock crypto adapter can settle CU without changing scheduler/worker APIs.

## Research conclusions

The strongest design is a **mutual-credit inference economy** rather than a token marketplace in the first release.

The economic unit should represent verified contribution, not raw provider quota. Temporary quota can be shared today, converted into settled CU through verified usage, and the CU can be consumed later from another resource in the network. This creates the behavior desired by the project without prematurely introducing blockchain complexity.

The branch should therefore prioritize correctness of measurement, receipt verification, accounting, reservation, reconciliation, and interoperability. Crypto should remain a replaceable settlement adapter layered above a stable internal ledger.

## Reference principles

- BOINC-style verified contribution accounting demonstrates the usefulness of internal computational credits before monetary settlement.
- Signed inference receipts provide a useful pattern for binding model identity, measured token usage, cost metadata, and content hashes to a verifiable execution record.
- OpenAI-compatible APIs provide the appropriate interoperability boundary for clients such as OpenCode.
- Provider pricing and quota mechanics should inform—but not hard-code—the CU policy.

This document is a research/design artifact. It is not a claim that any external provider's free quota is transferable or shareable under that provider's terms. Each provider integration must separately verify applicable API terms, quota restrictions, credential-sharing rules, and acceptable-use requirements before enabling shared consumption.