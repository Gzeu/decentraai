# Compute Assist — "Sharing is Caring" (M14/M15 milestone 1)

> A busy node can autonomously receive compute help from a trusted mesh
> worker; the contribution is evidence-backed and credited to the helper.

## The loop

```text
NODE A (busy)                NODE B (trusted worker)
     │  RESOURCE_REQUEST        │
     │ ────────────────────────▶│  owner limits checked by B
     │       RESOURCE_OFFER     │
     │ ◀────────────────────────│
     │  RESOURCE_RESERVE        │
     │ ────────────────────────▶│
     │      RESOURCE_RESERVED   │  lease starts (TTL backstop)
     │ ◀────────────────────────│
     │  ASSIST_TASK_ASSIGN      │
     │ ────────────────────────▶│  executes on LOCAL engine
     │      ASSIST_TASK_RESULT  │
     │ ◀────────────────────────│  → verified success → CONTRIBUTION CREDIT
     │  RESOURCE_RELEASE        │
```

## Owner authority

A worker answers capacity polls ONLY within its configured limits:

```yaml
sharing:
  assist:
    enabled: true            # opt-in; absent = shares nothing
    cpu_max_percent: 50      # of total cores
    ram_max_mb: 2048
    max_lease_seconds: 120   # every lease expires regardless of peers
    allowed_capabilities: ["chat", "embeddings"]
    allowed_peers: []        # empty = any TRUSTED peer
```

## Deterministic selection

Offers pass hard gates first (capability match, resource fit, freshness
≤30s, queue depth, recent failure), then a deterministic score where the
worker's `contribution_balance` biases ties by at most ±0.15 — enough to
reward givers, never enough to beat a hard gate or a better-fitting offer.

## Credit rule

Credit is recorded through the EXISTING `record_credited_contribution`
ledger path and only for a successful, transport-verified result. Failed,
timed-out or crashed assists earn NOTHING and always release the lease.

## Not this

Not cryptocurrency · not a centralized scheduler · not shared memory · not
model sharding. Resources stay physically owned; sharing stays revocable;
every hop is deterministic Rust.
