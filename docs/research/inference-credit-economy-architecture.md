# Inference Credit Economy — Architecture (research)

Status: experimental
Branch: `research/inference-credit-economy`
Mainline impact: none

## Vision

DecentraAI is not a token marketplace in this track. It is a **mutual-credit inference fabric**:

1. A node contributes a *temporary* external resource (API quota, GPU, CPU, storage, bandwidth).
2. Other nodes consume that resource through the fabric, never through leaked credentials.
3. Verified usage (existing P13 receipts / provider accounting) is the only minting event.
4. The contributor receives **durable CU**, detached from the original provider, model, and quota window.
5. Those CU are a budget to consume *any eligible* fabric resource later — including a different API, a remote GPU, or CPU.

```text
TEMPORARY EXTERNAL RESOURCE     DECENTRAAI CU
(API daily quota, GPU hours)    (append-only, reserved, spent)
        ≠
quota may reset to zero         settled CU survive forever (until spent)
```

This is closer to BOINC-style verified contribution plus an internal clearing house than to an L1 token.

## Control plane (landed, unwired)

```text
Client (OpenCode / OpenAI SDK)
        ↓  GatewayChatNeed          crates/credit-fabric
Credit check + estimate
        ↓  ResourcePlanner
Catalog of eligible ads (no secrets)
        ↓  ExecutionSession
Reserve consumer CU
        ↓  (execution happens in existing workers / local provider adapters)
VerifiedUsage  (projection of P13 receipt)
        ↓  two-sided settlement
Contributor earns CU | Consumer spends CU
        ↓  crates/credit-economy
Append-only ledger + provider quota ledger
        ↓  SettlementEngine
InternalSettlement today | CryptoSettlementAdapter later
```

Existing DecentraAI planes stay authoritative:

- discovery / workers / scheduler / VRAM reservations
- P13 signed receipts
- P14 `CreditLedger` (synthetic compute credits) — **not replaced**
- `QuotaLedger` (contribution→quota) — **not** provider API quota

The research crates sit beside those planes until an opt-in adapter is reviewed.

## Two-sided session

A session is the unit of economic truth, not a naked balance mutation.

```text
open_session     estimate + plan + reserve consumer CU
mark_executing   worker / provider call in progress (keys stay local)
complete_session verified receipt → contributor Earn + consumer Consume
fail_session     release CU, no earn
Held             actual CU > reserved → operator reconcile, no silent overspend
```

Self-dealing (consumer == contributor) is denied by default.

## Resource fungibility

CU carry *provenance* (origin resource/provider/model/receipt/policy) but are **not** spend-locked to origin. Eligibility is a planner/catalog property (`eligible_for_spend`), not a color of money.

Canonical hops:

- DeepSeek API contribution → CU → Qwen API consumption
- local GPU contribution → CU → remote GPU consumption
- GPU contribution → CU → API consumption

## Provider security

```text
Correct:  contributor node → authenticated HTTP → provider API
Incorrect: API key → P2P advertisement / receipt / catalog
```

Catalog stores `credential_ref` such as `env:DEEPSEEK_KEY`. Values resembling `sk-` or `api_key` are rejected.

## OpenCode

`GatewayChatNeed` / `GatewayPlan` are protocol envelopes, not an OpenCode plugin. Runtime already exposes `/v1/chat/completions`. Wiring must be opt-in middleware: credit check → planner → existing execution.

## Crypto future

`SettlementEngine` remains the only extension point. No wallet, chain, or token issuance in these crates. A future adapter may lock CU and emit an external settlement id. Inference must work when that adapter is `InternalSettlement`.

## What this is not

- Not a replacement for P14 credits or compute `QuotaLedger`
- Not a blockchain
- Not permission to violate provider ToS
- Not automatic sharing of third-party API keys
