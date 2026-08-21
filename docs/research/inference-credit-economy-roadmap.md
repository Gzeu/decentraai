# Inference Credit Economy — Roadmap

## Purpose

Implement the contribution-credit system incrementally on `research/inference-credit-economy` while preserving `main`.

## Phase 0 — Research / contract freeze — DONE

Inspected live primitives (P13 receipts, P14 `CreditLedger`, `QuotaLedger` reserve/settle, `ReservationLedger`, compensation, capability ads, runtime `/v1/chat/completions`).

Frozen terms: `Contribution`, `Usage`, `Receipt`, `CreditEvent`, `Reservation`, `Settlement`, `CU`.

Source of truth: usage = existing signed receipt / provider accounting; balances = experimental `InferenceCreditEconomy` (not P14 ledger).

## Phase 1 — Ledger core — DONE (experimental crate)

Landed in `crates/credit-economy`:

- integer CU + `CreditBalance`
- append-only `CreditEvent`
- `CreditReservation`
- idempotency (receipt / contribution / reservation ids)
- mutex-protected concurrent reserve

Tests in `crates/credit-economy/src/lib.rs`:

- earn / spend / reserve / release / settle
- duplicate event / receipt
- concurrent reservations
- insufficient balance
- failed execution → no CU

```text
cargo test --manifest-path crates/credit-economy/Cargo.toml
```

## Phase 2 — Contribution verification — DONE (crate-local)

- `ResourceAdvertisement` without secrets
- `ProviderQuota` (temporary, expirable)
- PENDING → VERIFIED → SETTLED / REJECTED
- quota exhaustion refuses settlement
- expired quota leaves settled CU spendable
- policy version pinned on events

Not yet: live P13 verifier call inside `ComputeManager`.

## Phase 3 — Policy engine — DONE (v1)

`CreditPolicy` (`ice-v1`): integer weights for input/output tokens, GPU ms, CPU ms, storage, bandwidth. Replaceable via `set_policy`. Not 1 token = 1 CU.

Later: reliability, quality, scarcity modifiers.

## Phase 4 — Scheduler integration — NOT STARTED

Opt-in only. Do not change default `ComputeScheduler` / `route_request` until an explicit flag exists.

## Phase 5 — API resource providers — NOT STARTED

Local credential store + provider adapters. Keys stay on the contributor node.

## Phase 6 — OpenCode integration — NOT STARTED

Runtime already has `/v1/chat/completions`. Credit check middleware must be opt-in and not coupled to OpenCode.

## Phase 7 — Cross-resource economy — DONE (unit)

Crate tests prove:

- DeepSeek API earn → Qwen API consume
- local GPU earn → remote GPU consume

## Phase 8 — Reputation / anti-abuse — PARTIAL

Landed: forged receipt reject, duplicate receipt, overspend, secret-in-ad reject, mutex races.

Not landed: Sybil cost, anti-self-dealing, redundant verification.

## Phase 9 — Crypto adapter — INTERFACE ONLY

`SettlementEngine` + `InternalSettlement`. No wallet, no chain SDK.

## Phase 10 — Production hardening — NOT STARTED

Workspace membership, persistence, metrics, property tests, security review.

## Success scenario (crate-level)

Covered by `expired_provider_quota_settled_cu_remain_valid`, `reservation_release_and_insufficient_balance`, `concurrent_reservation_cannot_overspend`, `consumption_on_different_resource`.

End-to-end on a live two-node fabric is **not** claimed.
