# DCAI — economic role, derived from the live economy (NOT issued)

> Status: ARCHITECTURE DECISION, no token exists. No supply, no decimals,
> no conversion rate, no distribution is defined here — inventing them
> before the economy justifies the mechanism would be fiction. This note
> records WHAT the token must do (derived from real flows), WHERE it
> attaches (exact integration points), and what stays OFF-chain forever.

## 1. Two monies, two jobs

| | Cr (World Currency) | DCAI (ecosystem asset) |
|---|---|---|
| Unit of | gameplay account | ecosystem commitment |
| Speed | tick-fast, abundant | slow, scarce by design |
| Created by | quest rewards only (`treasury_minted`) | NOTHING yet — unissued |
| Destroyed by | tithe 10%, listing fee, refine fee (`treasury_burned`) | n/a |
| Lives | `db/world.json` (off-chain) | MultiversX (when it exists) |
| Converts to the other | — | NO rate, NO bridge. Deliberately undecided. |

`1 Cr = 1 DCAI` is explicitly rejected, and so is any free conversion
until on-chain activity justifies a mechanism. Cr measures play; DCAI
will measure commitment.

## 2. Roles DCAI must fill (each traced to a live flow)

1. **Quest stakes** — today elite quests gate on off-chain reputation
   (`required_reputation`). The on-chain form is a stake lock: accept →
   lock DCAI → complete refunds + reward, abandon slashes to treasury.
   Attach point: `M18State` escrow lifecycle (`create → release →
   settle`), which already carries `evidence_hash` + `tx_hash`; a stake
   field belongs on `EscrowRecord` alongside them, NOT in a new ledger.
2. **Provider bonds** — today NPC/provider trust is a float fed by
   confirmed settlements. The on-chain form is a slashable bond posted
   before serving paid work. Attach point: `AgentContract.terms`
   (`escrow_required` already exists as the boolean; the amount comes later).
3. **Compute contribution rewards** — the fabric side (`QuotaLedger`,
   contribution credits) already tracks verified work off-chain. DCAI is
   the natural settlement asset there, NOT Cr: compute value leaves the
   game world, so it must not be payable in game money.
4. **Premium access / treasury / incentives** — reserved names for
   decisions the live economy has not earned yet. Listed so they are not
   silently stuffed into Cr flows later.

What DCAI will NEVER be: quest rewards, service prices, listing fees,
refining fees. Those are Cr by construction (fast, abundant, reversible).

## 3. On-chain vs off-chain split (enforced by construction today)

```text
OFF-CHAIN forever (World ticks, needs, prices, inventory, Cr balances,
  demand counters, reputation floats, quest state)
        │  only events worth anchoring cross the boundary
        ▼
EVIDENCE (BLAKE3 digests, deterministic, re-verifiable by anyone)
        ▼
M18 (contracts + escrow + trust — the trust layer for important txs)
        ▼
MultiversX (0-value self-transfer anchoring TODAY: proof bytes + sender
  signature + tx hash; contract calls + stakes WHEN registry addresses
  verify and DCAI exists)
```

The boundary rule: NOTHING crosses per tick. Only completed economic
facts with an earner, an amount ≥ `SETTLEMENT_MIN_CREDITS`, and an
evidence hash — via `submit_proof@jobHex@digestHex` payloads.

## 4. Preconditions before ANY issuance discussion

- [ ] Cr sinks ≈ sources over long runs (`treasury` counters observable
      per tick; persistent inflation or deflation understood, not guessed)
- [x] Stake/slash flows PROVEN with Cr-equivalent value first — LIVE since
      Economy v2 integration: `Quest.stake` locks at accept
      (`WorldState.quest_stakes`), refunds in full on `complete_quest`,
      burns on `enforce_deadlines` expiry. Elite quests (stake 10Cr) prove
      the loop end-to-end on every veteran cycle. The DCAI form is a pure
      denomination swap of this exact state machine.
- [ ] VERIFIED MX-8004 registry addresses (contract calls replace
      self-transfer anchoring; DCAI needs contracts to live in)
- [ ] Compute-side contribution settlement design (DCAI's first real
      demand: paying for verified work, not gameplay)

Field map for the future swap (no new ledger when it happens):

```text
Quest.stake (u64 Cr)              → quest stake in DCAI (same lock/refund/slash)
M18 EscrowRecord.{evidence,tx}    → + stake field beside them (lock at
                                    create, refund/slash at settle)
WorldState.quest_stakes           → reads as today (denomination-agnostic map)
EconomicEvidence / trust anchors  → unchanged (evidence carries no currency)
```

Until then: no supply, no decimals, no sale, no promises. The economy
below must stay fun and solvent with Cr alone — that is the test DCAI
has to pass before it deserves to exist.
