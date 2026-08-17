# Node Upgrade — operator reference (read this after every `git pull`)

> Living doc for the two-node LAN fabric (Laptop i5 ↔ Desktop i7). If you are an
> agent that just pulled this repo, read this file to know what the upgrade
> scripts are, which machine is which, and what state each node is in.

## The two machines

| Machine | Node id | Peer id | LAN IP | Role |
|---|---|---|---|---|
| **Desktop i7** | `dca-NGE65Z` | `12D3KooWNGE65…` | 192.168.1.138 / .130 | coordinator, serves models |
| **Laptop i5** | `dca-GriBWu` | `12D3KooWGriBWu…` | 192.168.1.132 | worker, runs `validate-lan.sh` |

Both run `decentraai-node` as a systemd **user** service. The API binds to
loopback only (config validation rejects public binds) — you cannot reach one
node's API from the other; only the P2P port is open between them.

## The upgrade scripts (what they are)

### `scripts/upgrade-node.sh` — LOCAL upgrade (run on the machine itself)

- Builds the current checkout (`cargo build --release --bin decentraai`),
  backs up `~/.cargo/bin/decentraai`, stops the service (ETXTBSY guard),
  swaps the binary, restarts the service.
- Unless `ENABLE_REMOTE=0`: flips `inference.allow_remote_inference` +
  `network.private_swarm` to `true` in `~/.decentraai/node.yaml` (with a
  timestamped backup) — the binary alone does NOT advertise
  `accepts_remote_inference`; the config flag does.
- Never touches node data/identity. Idempotent.
- Usage: `bash scripts/upgrade-node.sh [commit]`

### `scripts/upgrade-remote-node.sh` — REMOTE upgrade (run from the Desktop)

- SSHes into the target (default `i5@192.168.1.132:22`), runs
  `git pull --rebase` + `bash scripts/upgrade-node.sh` there, then verifies
  from the Desktop that the target appears in `/v1/fabric`.
- **This is operator ops over SSH between George's own machines — NOT remote
  shell from the application.** DecentraAI never runs remote shell or pushes
  binaries through the mesh.
- **Prerequisite on the target: sshd must be running**
  (`sudo systemctl enable --now ssh`) and the Desktop's key in
  `~/.ssh/authorized_keys`. As of 2026-08-18 the laptop's SSH port is closed.
- Usage: `bash scripts/upgrade-remote-node.sh [user@host]` (`REMOTE_PORT=…` to
  override the port).

## Current state (2026-08-18)

- **Desktop i7: UP TO DATE.** Repo at `f7dbe10` (= `origin/main`), binary
  rebuilt from HEAD via `upgrade-node.sh`. Advertises
  `accepts_remote_inference: true`. `/v1/agents` → 1 agent (`generalis`),
  `/v1/compute` → 1 worker (itself).
- **Laptop i5: BINARY OUTDATED.** P2P-connected (peer visible in
  `/v1/network` `connected`, score 46) but its installed binary predates
  `protocol_version` on agent advertisements — the Desktop logs show:
  `WARN rejected signed agent advertisement error=missing field protocol_version`
  and `WARN request ignored: no handler configured`. The laptop therefore does
  NOT appear as a worker/agent/node in the fabric even though it is connected.
  The repo on the laptop may already be at HEAD; the **binary** is what is old.

## What to do after a pull on the laptop (i5)

```bash
cd ~/decentraai
git pull --rebase
bash scripts/upgrade-node.sh     # rebuilds the binary, restarts the service
```

After the restart the laptop re-advertises with the current protocol and is
re-classified CURRENT in the fabric. Verify from the Desktop:

```bash
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:8080/v1/fabric
# expect 2 nodes; the laptop row should show version 1.0.0, lifecycle ONLINE
```

## Version-mismatch failure mode (do not re-diagnose)

An old binary omits newer advertisement fields; they deserialize to the
conservative default (`accepts_remote_inference: false`, agent ads rejected for
missing `protocol_version`). The system behaves honestly: it does not route work
to a node that does not opt in, and it logs the reason. **The fix is always
operational (upgrade the binary), never a code bug.** See
`docs/TWO_NODE_VALIDATION.md` for the original 2026-08-16 validation (roles were
reversed then: the Desktop was the outdated node).