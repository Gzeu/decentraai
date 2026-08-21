# Inference Credit Economy — Architecture (research)

Status: experimental (crypto-ready settlement interfaces landed)
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
(API daily quota, GPU hours)    (append-only, reserved, spent, locked)
        ≠
quota may reset to zero         settled CU survive forever (until spent or bridged)
```

## Modular Settlement & Crypto Readiness

DecentraAI decouples internal compute execution from external financial/token rails:

```text
       DecentraAI Core
    [InferenceCreditEconomy]
              │
              ├── [CreditBalance: earned = available + reserved + consumed + locked_for_settlement]
              │
              └── Escrow State Machine:
                  AVAILABLE ──(create_settlement_intent)──> LOCKED_FOR_SETTLEMENT
                                                                │
                                                ┌───────────────┴───────────────┐
                                                │ (external tx confirmed)       │ (timeout / error)
                                                ▼                               ▼
                                            FINALIZED                       REFUNDED
                                       (consumed permanently)           (returns to available)
                                                │
                                                ▼
                                    [SettlementEngine trait]
                                                │
                                 ┌──────────────┴──────────────┐
                                 │                             │
                     InternalSettlement (default)   ExternalAssetAdapter (Web3/Crypto bridge)
                     (clearinghouse in DecentraAI)  (MultiversX, EVM, Solana, Lightning, etc.)
```

### Safety & Independence Rules:
- **Zero Blockchain Dependencies in Core**: No EVM/WASM runtime, no chain SDKs, no RPC clients inside `decentraai_compute` or `credit-economy`.
- **No Private Keys in Core**: Signatures and external token claims are handled outside the node via `ExternalAssetAdapter` hooks.
- **Strict Invariant**: `earned == available + reserved + consumed + locked_for_settlement`.
- **Two-Phase Commit**: CU are escrowed (`LOCKED_FOR_SETTLEMENT`) while awaiting external transaction finality; only upon cryptographic or oracle confirmation are they burned (`FINALIZED`). Any transport failure safely triggers `REFUNDED` back to `available`.
