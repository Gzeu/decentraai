# Deployment & Onboarding (placeholder)

## Onboarding (decentraai setup)

The `decentraai setup` command will guide a new node through:
- creating the data directory (default: ~/.decentraai)
- generating and storing an identity (private key)
- validating and writing a configuration (configs/node.example.yaml)
- optional remote backend onboarding

This file is a placeholder; the interactive wizard will be implemented in a follow-up change.

---

# Node Lifecycle & Upgrade

## Node Lifecycle

Each node advertises its DecentraAI build as `node_version` (empty = UNKNOWN
for older peers that do not advertise it). The coordinator projects this into a
lifecycle in `/v1/fabric`, which also carries a `coordinator.version` block and
per-node `node_version`, `version_status`, `outdated`, and `lifecycle` fields.

```
DISCOVERED → TRUSTED → ONLINE → OUTDATED
```

- `DISCOVERED` — seen on the network, not yet trusted.
- `TRUSTED` — identity accepted by the trust model.
- `ONLINE` — trusted and serving.
- `OUTDATED` — trusted/online but advertising a version that differs from the
  coordinator's.

`UPDATING` and `VERIFIED` are **future** states and are **not** currently
produced: there is no real remote-update mechanism yet, so the fabric cannot
represent an in-progress update or a verified post-update build. The dashboard
renders only the states above, with badges per node and an honest
"N node(s) need update" count.

## Honest version semantics

`version_status(coordinator, remote)` classifies a node:

- `CURRENT` — remote version equals the coordinator's version.
- `OUTDATED` — remote version is a different *known* version. The coordinator
  cannot prove which is newer, only that they differ.
- `UNKNOWN` — the peer did not advertise a version (empty `node_version`).

**DecentraAI never claims an update is available for a node whose version is
UNKNOWN.** The `outdated` flag follows only the OUTDATED classification, so an
UNKNOWN node is never counted as needing an update.

## Upgrade workflow (investigation — safe, no remote execution)

DecentraAI does **not** push binaries or run arbitrary commands on remote
nodes. Updating a remote node is an operator action:

1. The coordinator detects OUTDATED nodes via `/v1/fabric`.
2. The operator updates each OUTDATED node out-of-band — rebuild/redeploy the
   new binary on that node. No remote shell or arbitrary command execution is
   performed by DecentraAI.
3. After the node restarts on the new build, its advertisement carries the new
   `node_version`, and the coordinator re-classifies it as `CURRENT` (verified
   via `/v1/fabric`).

Trust is unchanged through this flow. The update is verified purely by the
advertised version matching the coordinator's, which the existing trust model
already gates — a peer is only scheduled if it is trusted.

**Future direction (not implemented):** a platform-specific, opt-in updater
(systemd/Linux, Windows, ARM) that fetches and verifies a signed release
artifact locally. This remains an update applied on the node itself — never
arbitrary remote shell.

## Platform-agnostic architecture note

No single update mechanism is assumed. The update path must be platform-aware
(Linux systemd user unit, Windows, ARM, and future mobile/lightweight workers),
each with its own packaging. DecentraAI's fabric stays platform-agnostic
because it only observes the advertised `node_version` and never assumes how a
node is updated.
