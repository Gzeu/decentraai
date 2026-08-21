# DecentraAI — Inference Credit Economy Research

Status: research / experimental (ledger crate landed)
Branch: `research/inference-credit-economy`
Mainline impact: none

## Executive summary

DecentraAI can support a crypto-agnostic internal economy in which participants contribute verifiable AI resources—API quota, GPU/CPU inference, storage, or bandwidth—and receive reusable **Contribution Credits (CU)** after the contribution is measured and verified.

The key semantic rule is:

> A provider's temporary API quota is a resource. CU are durable accounting units earned from verified contribution. Expiration of the original provider quota does not invalidate already-settled CU.

The system remains **off-chain**. A future `SettlementEngine` adapter may convert settled CU without changing scheduling, discovery, execution, or receipt verification.

## Existing primitives (inspected, not replaced)

| Concern | Existing type | Location | Role vs this experiment |
|---|---|---|---|
| Resource ads / capability | `ComputeCapability`, `GpuSpec`, `ServedModel` | `crates/compute/src/capability.rs` | Keep; ads stay compute-plane |
| Discovery / workers | `ComputeRegistry`, distributed `ComputeManager` | `crates/compute`, `crates/distributed` | Keep |
| Compute admission | `ReservationLedger`, `ResourceReservation` | `crates/compute/src/reservation.rs` | Keep (VRAM/slots, not CU) |
| Scheduler | `ComputeScheduler` | `crates/compute/src/scheduler.rs` | Keep; no credit gate yet |
| Signed receipts | P13 `VerifiedComputeReceipt` + CLI sign/verify | compute / node-cli | **Source of truth** for usage; not reimplemented |
| Contribution→quota | `QuotaLedger` (EARNED→AVAILABLE→RESERVED→CONSUMED) | `crates/compute/src/quota.rs` | Keep; **not** provider API quota |
| P14 synthetic credits | `CreditLedger`, `CreditPolicy`, `CreditEvent` | `crates/compute/src/credits.rs` | Keep; earn/consume, **no CU reservation**, f64 policy |
| Compensation | `CompensationLedger` | `crates/compute/src/compensation.rs` | Keep |
| Resource contribution | `ResourceContribution`, `Provenance` | `crates/compute/src/resource_contribution.rs` | Keep; feed into P14 |
| Identity | libp2p peer id / existing account strings | `crates/identity`, p2p | Reuse as `AccountId` |
| OpenAI-compatible HTTP | `/v1/chat/completions` already on runtime | `crates/runtime` | Gateway credit middleware **not** wired |

Rejected alternative: mutating P14 `CreditLedger` or `QuotaLedger` in place. Those are live production accounting paths. The research layer is a **new crate**.

## Experimental crate

`crates/credit-economy` (`decentraai-credit-economy`) is optional, not a workspace member (so `cargo test --workspace` is unchanged), and not wired into scheduler/runtime/P2P.

Run isolated tests:

```text
cargo test --manifest-path crates/credit-economy/Cargo.toml
```

### Accounting split

```text
ProviderQuota          CreditBalance (CU)
available/reserved     earned / available / reserved / consumed
consumed/expired       invariant: earned = available + reserved / consumed
reset_at               durable after quota expiry
```

### Lifecycle

```text
PENDING → VERIFIED → SETTLED → AVAILABLE → RESERVED → CONSUMED
               ↘ REJECTED  (no spendable CU)
```

### Policy (`ice-v1`)

Integer weights, versioned. Default is **not** 1 token = 1 CU:

- input token → 1 CU
- output token → 2 CU
- GPU ms → 1 CU
- CPU ms → 1 CU

### Security assumptions

- P13 signature verification happens **before** `VerifiedUsage.signature_valid = true`.
- Advertisements may only carry `credential_ref` (e.g. `env:DEEPSEEK_KEY`). Values looking like `sk-` / `api_key` are rejected.
- Contributor node holds the real API key and calls the provider locally.
- Idempotency: receipt_id (verify), contribution_id (settle), reservation_id (reserve/consume).
- Concurrent reserve is mutex-serialized; overspend is refused.

### Future crypto

```text
CreditLedger (this crate)
    ↓
SettlementEngine
    ├─ InternalSettlement   (landed, no-op)
    └─ CryptoSettlementAdapter (not implemented; no chain/wallet in core)
```

### Known limitations

- Not attached to `ComputeManager` or `/v1/*`.
- Not a workspace member yet (avoids touching root `Cargo.toml` / `main` behavior).
- No persistence (P14 credits.json is a separate store).
- No OpenCode/gateway middleware.
- No Sybil/reputation economics (use existing identity/trust gates).
- `WorkerTelemetry` / `Unknown` measurements cannot settle.
- Reliability/quality/scarcity weights are reserved for later policy versions.

## Canonical example

1. Node A advertises 100,000 API tokens (no key on the wire).
2. Others consume 60,000; usage verified via receipt / provider accounting.
3. Node A is settled 60,000 CU under `ice-v1` input weight (or more if output tokens exist).
4. Provider daily quota expires; CU remain.
5. Node A reserves/spends CU on Qwen API or remote GPU.
6. Replay of the same receipt does not change balances.

## Decisions and rejected alternatives

- **New crate, not a P14 patch** — preserves backward compatibility.
- **Integer CU** — P14 uses `f64` rates; research accounting is integer-only.
- **CU not bound to origin provider/model** — provenance is recorded, spend is not restricted by origin.
- **No blockchain** — `InternalSettlement` only.
- **Do not duplicate receipts** — `VerifiedUsage` is a projection of existing signed receipts.
