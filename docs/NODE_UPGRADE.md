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

### `scripts/upgrade-remote-node.sh` — REMOTE upgrade (run from the coordinating machine)

- SSHes into the target, runs `git fetch + checkout main + pull --rebase` +
  `bash scripts/upgrade-node.sh` there, then verifies from the local machine
  that the target appears as a remote worker in `/v1/compute`.
- **Bidirectional** — point it at either node:
  - from the Laptop → Desktop: `bash scripts/upgrade-remote-node.sh dca@192.168.1.138`
  - from the Desktop → Laptop: `bash scripts/upgrade-remote-node.sh i5@192.168.1.132`
  - `VALIDATE_LAN=1 bash scripts/upgrade-remote-node.sh …` also runs
    `validate-lan.sh` (end-to-end remote routing) after the upgrade.
- **This is operator ops over SSH between George's own machines — NOT remote
  shell from the application.** DecentraAI never runs remote shell or pushes
  binaries through the mesh.
- **Prerequisite on the target: sshd must be running**
  (`sudo systemctl enable --now ssh`) and the local machine's key in
  `~/.ssh/authorized_keys`. As of 2026-08-18 the laptop's SSH port is closed.
- Usage: `bash scripts/upgrade-remote-node.sh [user@host]` (`REMOTE_PORT=…` to
  override the port).

## Current state (2026-08-19, verified live)

> State as of the 2026-08-19 re-validation. Both nodes are on the same HEAD,
> both run the self-upgrade watcher, and two-node remote inference is verified
> end-to-end from the Laptop (`validate-lan.sh` → reply `REMOTE`).

- **Laptop i5: UP TO DATE at `979acbf` + `--auto-upgrade` active.** The unit
  file starts the node with `--auto-upgrade` (6h watcher). `/v1/compute`
  shows 2 workers (local + Desktop, both `remote_ok: true`); `/v1/network`
  measured `rtt_ms: 174`, `locality: Lan`, `bandwidth: 1000`.
- **Desktop i7: UP TO DATE at `979acbf` + `--auto-upgrade` active.** SSH port
  closed — the operator ran `git pull && bash scripts/upgrade-node.sh` on the
  machine, which patched its unit file idempotently (see below). Verified from
  the Laptop: trusted, `remote_ok: true`, serves
  `Llama-3.2-1B-Instruct-Q4_K_M.gguf`, `ONLINE` in `/v1/fabric`.
- **Auto-upgrade unit patch**: `scripts/upgrade-node.sh` now ensures the
  systemd unit runs the node with `--auto-upgrade` + pins `WorkingDirectory`
  to the repo checkout (idempotent; `ENABLE_AUTO_UPGRADE=0` to opt out). The
  deploy template `deploy/decentraai-node.service` has the same two lines, so
  fresh installs start self-upgrading out of the box.
- Open item (no longer blocking): SSH remains closed on the Desktop, so remote
  upgrades must go through the operator (`upgrade-node.sh` on the machine) —
  or wait for the self-upgrade watcher, which needs no SSH.

## Two-node remote inference — VERIFIED both directions (2026-08-18)

- **Laptop → Desktop** (verified by Devin on the Laptop): `curl
  /v1/chat/completions` with `model=tinyllama.gguf` (served only on the
  Desktop) returned a real remote reply; M19 RTT probe measured
  `links rtt_ms=180, locality=Lan`.
- **Desktop → Laptop** (verified by Pylon on the Desktop, HEAD `d42dfd9`):
  `bash scripts/validate-lan.sh` found the remote worker `dca-GriBWu`
  (trusted, remote_ok, model `qwen2.5-coder-7b-instruct-q4_k_m.gguf`), routed
  a real request to it; `/v1/execution` confirms the planner chose the Laptop
  peer (network_cost 849ms, score 0.496) and rejected the local candidate
  (breach `trusted`). The model replied `LOCAL` (it did not interpret the
  prompt), but the remote routing is proven by the execution view.

## What to do on the Desktop (i7) so it shows up as a remote worker

> Hardware note (verified 2026-08-18): the **Laptop (i5) has 30 GiB RAM** and
> holds both the tiny model (Llama-3.2-1B) and Mistral-7B on disk; the
> **Desktop (i7) has 8 GiB RAM and only the tiny model** — do NOT point
> `node.model` at Mistral on the Desktop (it cannot host it comfortably).
> Both nodes serving the same tiny model is fine: remote routing is forced by
> trust + `route_request_on`/a remote-only placement, not by a distinct model.

### Fully automatic (recommended) — one command from the Laptop

The Desktop must have sshd running once (`sudo systemctl enable --now ssh`)
and the Laptop's public key in `~/.ssh/authorized_keys`. After that, from the
Laptop:

```bash
cd ~/decentraai
git pull --rebase
bash scripts/upgrade-remote-node.sh dca@192.168.1.138   # builds + swaps + restarts the Desktop
VALIDATE_LAN=1 bash scripts/upgrade-remote-node.sh dca@192.168.1.138  # …and then proves remote routing
```

### Fully automatic — self-upgrade on schedule (no SSH needed)

The node can refresh itself from its own git remote (no operator login needed
on the machine, works on any node):

```bash
# one-shot check: is there a newer main?
decentraai upgrade check
# apply now (build + binary swap + service restart, rollback on failure)
decentraai upgrade apply
# keep checking every 6h and upgrade automatically when a new main exists
decentraai node --auto-upgrade --auto-upgrade-interval-secs 21600
# or the standalone watcher
decentraai upgrade auto --interval-secs 21600
```

Safety: never touches node data/config/identity, requires a clean working
tree, backs up the binary before the swap, and stops the service only for the
brief swap (not for the minutes-long build).

### Manual (fallback)

```bash
cd ~/decentraai
git pull --rebase
bash scripts/upgrade-node.sh     # rebuild, set allow_remote_inference, restart
# ensure ~/.decentraai/node.yaml has:
#   inference.allow_remote_inference: true
#   node.model: "Llama-3.2-1B-Instruct-Q4_K_M.gguf"  (the tiny model it can host)
systemctl --user restart decentraai-node
```

After the restart the Desktop should re-advertise as a **worker** and the
Laptop's `/v1/compute` should list it as a trusted, `remote_ok` remote worker.
Then verify two-node remote inference from the Laptop:
`bash scripts/validate-lan.sh` (forced-remote routing).

## Version-mismatch failure mode (do not re-diagnose)

An old binary omits newer advertisement fields; they deserialize to the
conservative default (`accepts_remote_inference: false`, agent ads rejected for
missing `protocol_version`). The system behaves honestly: it does not route work
to a node that does not opt in, and it logs the reason. **The fix is always
operational (upgrade the binary), never a code bug.** See
`docs/TWO_NODE_VALIDATION.md` for the original 2026-08-16 validation (roles were
reversed then: the Desktop was the outdated node).
