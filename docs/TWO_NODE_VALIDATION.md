# Two-Node Fabric — Live Validation Report

Date: 2026-08-16. Scope: make the DecentraAI fabric rock-solid as a real
**Laptop i5 ↔ Desktop i7** two-node fabric (mobile/Android stays roadmap-only).

## Environment

- This node: Laptop i5, node id `dca-GriBWu`, peer `12D3KooWGriBWu…`,
  `allow_remote_inference: true`, port 8080, running the binary rebuilt from
  HEAD `71744a6`.
- Remote node: Desktop i7, node id `dca-NGE65Z`, peer `12D3KooWNGE65…`,
  `node_name: decentraai-node`.

## What was verified LIVE

1. **Both nodes discover each other.** After restarting this node with the
   current binary, `GET /v1/compute` shows both workers, and
   `GET /v1/network` `connected` lists the Desktop's peer id
   (`12D3KooWNGE65…`) — a real P2P connection over the LAN (192.168.1.132).
2. **Local inference works.** `POST /v1/chat/completions` on this node
   returns a real completion ("node-ok") served by the local model.
3. **This node advertises `accepts_remote_inference: true`** (after the
   restart), so a **current** Desktop would route remote work to it.
4. **Trust + honest rejection.** Trusting the Desktop via
   `POST /api/admin/worker/trust` succeeds, and routing a request to it via
   `worker_hint` is **honestly rejected** with "does not serve model (or is
   not trusted / does not accept remote inference)" because the Desktop
   advertises `accepts_remote_inference: false`.

## Finding — real version-mismatch failure scenario (proven live)

The Desktop node (`dca-NGE65Z`) advertises `accepts_remote_inference: false`.
This node's installed binary was built **2026-08-15 19:06**, and the
`accepts_remote_inference` advertisement field landed in later work; an older
binary omits the field, so it **deserializes to the conservative default
`false`** (per the advertisement's backward-compatibility contract). Result:
the Desktop is visible and P2P-connected but **refuses remote work** until it
is upgraded to a current binary that advertises `accepts_remote_inference: true`.

This is the exact "worker version mismatch" failure case from the hardening
priority list, demonstrated on real hardware. The system behaves **honestly**:
it does not route work to a worker that does not opt in, and it reports the
reason. The fix is operational (upgrade the Desktop binary), not a code bug.

## LOCAL-BLOCKED — live remote execution

Live **remote** execution (this node routes a request to the Desktop, or the
Desktop routes to this node) could not be fully verified end-to-end in this
session because:

- The Desktop runs an older binary and advertises `accepts_remote_inference:
  false`, so it rejects remote work (see finding above). Upgrading the Desktop
  is a manual/remote step outside this environment.
- This node's execution view (`/v1/execution`) is empty because live requests
  were served locally (local models win per `resolve_chat_route`), not through
  the distributed dispatch path.

Required to complete LIVE remote validation:
1. A current DecentraAI binary on the Desktop node (`dca-NGE65Z`) so it
   advertises `accepts_remote_inference: true`.
2. A model served by **only one** node (or an explicit `worker_hint`/forced
   remote route) so a request actually crosses the P2P link.

Until then, distributed execution is covered by the E2E two-node tests
(`crates/distributed/tests/compute_e2e.rs`), which spin up real libp2p nodes on
loopback and prove advertisement → trust → reservation → route → fallback.

## Exact upgrade procedure for the Desktop node

The Desktop (`dca-NGE65Z`, LAN 192.168.1.129) is **not reachable** from this
laptop for SSH or its control-plane API (only its P2P port is open), so the
upgrade must be run **on the Desktop machine by its operator**. The steps are:

### On the Desktop, from a terminal

```bash
# 1. Pull the current source.
cd ~/decentraai            # wherever the repo lives on the Desktop
git pull --rebase

# 2. Run the exact upgrade (builds current HEAD, swaps the binary, restarts
#    the systemd user service, verifies accepts_remote_inference).
bash scripts/upgrade-node.sh

# 3. Confirm the node now advertises remote inference:
systemctl --user status decentraai-node      # should be active
curl -s -H "Authorization: Bearer $(cat ~/.decentraai/runtime/api.token)" \
  http://127.0.0.1:8080/v1/compute | python3 -m json.tool
#   -> the Desktop's own row should show "accepts_remote_inference": true
```

The script (`scripts/upgrade-node.sh`) is idempotent and only swaps the binary
+ restarts; it never touches node data/config/identity.

### Prerequisites on the Desktop
- The decentraai repository checked out (so it can `git pull` + `cargo build`).
- Rust toolchain (`cargo`).
- The node already installed via `scripts/install-app.sh` (systemd user
  service). If not, run `bash scripts/install-app.sh --no-llama` first.

### After the Desktop upgrade
On the Laptop (`dca-GriBWu`), trust the Desktop and route a remote request:

```bash
# Trust the Desktop peer (already done in this session).
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl -s -X POST -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  http://127.0.0.1:8080/api/admin/worker/trust \
  -d '{"peer_id":"12D3KooWNGE65ZF4rCdLx7DVcna8zp4AcR3RiWgpdR49sixmvkRs"}'

# Route a request to a model the Desktop serves (tinyllama), forced remote.
curl -s -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  http://127.0.0.1:8080/v1/chat/completions \
  -d '{"model":"tinyllama.gguf","worker_hint":"dca-NGE65Z","messages":[{"role":"user","content":"hello from laptop"}],"max_tokens":16,"stream":false}'
```

A successful response tagged `X-Decentra-Origin: remote` (or a model path on
the Desktop) proves Laptop → Desktop remote execution.
