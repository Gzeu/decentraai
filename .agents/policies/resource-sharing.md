# Policy: resource-sharing (Sharing is Caring)

```text
If the local node has safe surplus      → it MAY contribute (opt-in limits).
If the local workload is under pressure → it MAY request assistance.
Never consume remote resources without: identity + authorization +
    reservation + lease + execution evidence.
Never claim contribution without verified evidence.
Owner limits are absolute; sharing is revocable at any moment.
```

## Owner authority (config `sharing.assist`)

- `enabled` — opt-in. Absent/false = the node shares NOTHING and behaves
  exactly as before the feature existed.
- `cpu_max_percent`, `ram_max_mb` — hard ceilings on offered capacity,
  clamped deterministically against every incoming request.
- `max_lease_seconds` — every lease is clamped to this and expires
  regardless of peer behaviour.
- `allowed_capabilities`, `allowed_peers` — positive allowlists; empty
  capabilities = all within other limits; empty peers = any trusted peer.

## Credit rule

Contribution credit is recorded ONLY for successful, transport-verified
results through the existing ledger path. Advertisements, reservations
alone, failed/cancelled tasks, spoofed metrics earn NOTHING.

## Failure rules

Worker disconnects / crashes / times out / returns malformed or invalid
evidence → lease expires or is released, no credit, task retries on another
valid worker, duplicate results are rejected safely, reservations are never
leaked.

## Fairness

`contribution_balance` biases scheduling ties by at most ±0.15 (saturated).
It can NEVER override security, owner limits, capability compatibility,
resource availability, health, or correctness. One node must not become
permanently preferred.
