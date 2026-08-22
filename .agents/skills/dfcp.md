# Skill: dfcp — DecentraAI Fabric Communication Protocol v1

## Message flow

```text
RESOURCE_REQUEST → RESOURCE_OFFER → RESOURCE_RESERVE → RESOURCE_RESERVED
→ ASSIST_TASK_ASSIGN → ASSIST_TASK_RESULT → RESOURCE_RELEASE
```

## Wire rules

- Every message: `protocol_version` (reject unknown versions), random ids
  for correlation, `deny_unknown_fields`, bounded payloads (16 KiB),
  base64 binary fields.
- Identity comes from the libp2p secure channel — NEVER from payload
  sender fields.
- An OFFER is a claim until RESERVE succeeds against the worker's ledger.
- Leases expire by TTL even if RELEASE never arrives (crash backstop).

## Where the code lives

| Piece | File |
|---|---|
| Messages | `crates/protocol/src/dfcp.rs` |
| Offer gates + scoring | `crates/compute/src/assist.rs` |
| Owner limits | `sharing.assist` in config (`crates/config`) |
| Worker/requester runtime | `crates/runtime/src/intel_assist.rs` |
| Transport cascade | `crates/p2p/src/lib.rs` (`DfcInbound`) |

## Deterministic selection

Hard gates FIRST (capability match, resource fit, freshness ≤30s, queue
depth, recent failure), then a score where `contribution_balance` biases
ties by at most ±0.15. Ties break by peer id ascending. A recent failure is
a hard gate, not a soft penalty.
