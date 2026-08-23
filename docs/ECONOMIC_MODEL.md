# DecentraAI Economic Model

Status: SIMULATION ONLY. No token launch, no mainnet, no financial promises.
Crate: `decentraai-economy` (pure, integer-only, no network).

## Contribution Units (CU v2)

`ECONOMICS_VERSION = 2`. Value of one verified contribution, in micro-CU:

```text
µCU = BASE(1_000_000) × verified_units
    × quality      [30–130 %]     graded correctness
    × reliability  [50–120 %]     clean ratio history
    × latency      [60–110 %]     vs task-class baseline
    × efficiency   [70–110 %]     work per resource byte
    × scarcity     [100–300 %]    capability scarcity index
    × difficulty   [100–500 %]    declared task class weight
```

**Verification is a gate, not a factor:** pending/invalid pay exactly 0.
Every factor is recorded in `ContributionFacts`; same facts + same version
→ the same value, bit-exact (`contribution::tests::reproducibility`).

## Reward engine invariants

| Invariant | Enforcement |
|---|---|
| No mint path | value enters only via `award()` of verified, evidenced facts |
| No self-reward | `verifier_id != worker_id` gate |
| No verification bypass | non-verified pays zero and creates no account |
| Append-only history | `gross_earned` monotonic; reversals/penalties touch balance only |
| Bounded punishment | penalty ≤ 25 % of current balance per event |
| Reversal bounded | invalidated results return at most what was paid; never negative |

## Anti-gaming model

| Attack | Counter | Test |
|---|---|---|
| Self-verification | verifier ≠ worker gate | `self_verification_is_rejected_at_the_door` |
| Result/evidence replay | per-worker evidence dedup | `evidence_replay_pays_once` |
| Missing evidence | award rejects without evidence_ref | `unverified_work_creates_no_account` |
| Low-quality spam | quality floor ≈ 3 % of base + reputation erosion elsewhere | `factors_move_value…` |
| Sybil workers | no cross-account amplification; pool-level emission caps | `sybil_workers_cannot_amplify_one_account` |
| Collusion | deterministic rules here; detection heuristics = open decision | see scenario-report assumptions |

## Model Intelligence integration

Verified model observations convert to facts via
`model_performance::observation_to_economic_facts` — success + evidence →
payable; failures pay zero. Claims are NEVER auto-rewarded: an operator or
policy layer calls the engine explicitly.

## Governance invariant

AI proposes → deterministic economics validates → cryptographic evidence
proves → settlement executes. The Governor cannot mint, self-reward, bypass
verification, alter supply/emission, or rewrite history — each rule is a
named test in `governance_invariants.rs`.
