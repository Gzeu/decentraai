# Tokenomics Simulator

Pure, config-driven, reproducible. **No final token parameters are chosen.**

## Usage

```bash
cargo run -p decentraai-economy --example simulate -- \
    configs/economy/example-params.json <nodes> <avg_award_micro_cu>
```

Same config → byte-identical report (tested). Parameters live in JSON
(closed schema — unknown fields are rejected).

## Parameters (all required, all explicit)

total_supply · epochs · schedule (fixed | halving{n} | linear_decay) ·
allocation split in bps (contributors/validators/development/treasury,
must sum to 100 %) · network fee bps · burn share of fees · vesting epochs ·
slashing bounds · min/max per-node reward.

## Sustainability — defined, not vibes

A scenario is sustainable iff, for EVERY epoch:
1. the reward pool never goes negative;
2. cumulative emissions never exceed total supply;
3. every node receives at least `min_reward_micro_cu`.

The first failing condition is reported with its epoch.

## Scenario ladder (see docs/economy/scenario-report.json)

10 / 100 / 1k / 10k / 100k nodes against `configs/economy/example-params.json`.
The example config is illustrative: an earlier draft WAS flagged unsustainable
by this simulator at epoch 4 — that is the tool working as intended.
