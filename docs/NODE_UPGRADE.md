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

## Current state (2026-08-18, verified live from the Laptop)

> Corrected by Pylon (source of truth), 2026-08-18. An earlier version of this
> doc claimed the Laptop binary was outdated; that was wrong — the Laptop is on
> a current binary and *processes* agent advertisements correctly.

- **Laptop i5: UP TO DATE (binary + repo).** The installed binary is current
  (built 2026-08-18), runs the orchestrator/AgentRuntime/workflow endpoints and
  the AGENTS dashboard view. `/v1/agents` on the Laptop shows the Desktop as a
  **remote agent** (`dca-NGE65Z:generalist`, `remote: true`), which proves the
  Laptop deserializes and verifies agent advertisements correctly.
- **Desktop i7: agent-visible, but its compute worker does not yet appear on
  the Laptop.** The Desktop is connected and advertises a `generalist` agent
  (build is new), yet `/v1/compute` on the Laptop lists only the local worker
  (`dca-GriBWu`). The Laptop's log shows recurring
  `WARN rejected signed agent advertisement error=missing field protocol_version`
  for a peer that sends a signed agent advertisement whose inner payload lacks
  `protocol_version` — to be traced to which peer/version (the local binary
  already carries `protocol_version`). The Desktop's **compute worker** must
  become visible before two-node remote inference can be validated.

## What to do on the Desktop (i7) so it shows up as a remote worker

> Hardware note (verified 2026-08-18): the **Laptop (i5) has 30 GiB RAM** and
> holds both the tiny model (Llama-3.2-1B) and Mistral-7B on disk; the
> **Desktop (i7) has 8 GiB RAM and only the tiny model** — do NOT point
> `node.model` at Mistral on the Desktop (it cannot host it comfortably).
> Both nodes serving the same tiny model is fine: remote routing is forced by
> trust + `route_request_on`/a remote-only placement, not by a distinct model.

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
