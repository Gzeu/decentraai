# Two-Node Fabric — Live Validation Report

Date: 2026-08-16. Scope: make the DecentraAI fabric rock-solid as a real
**Laptop i5 ↔ Desktop i7** two-node fabric (mobile/Android stays roadmap-only).

> UPDATE (same date): the Desktop was upgraded to current HEAD and configured
> with `allow_remote_inference: true`. Live validation results are below.

> UPDATE (2026-08-19): full two-node validation re-run on the current HEAD —
> both nodes upgraded to `979acbf`, remote inference verified end-to-end both
> ways, dashboard/chat-streaming/execution views validated, collective memory
> bug fixed and verified live. Details in the **2026-08-19 re-validation**
> section below.

## 2026-08-19 re-validation (current HEAD, both nodes)

Hardware (unchanged): Laptop i5 `dca-GriBWu` (192.168.1.132, 30 GiB RAM,
serves `qwen2.5-coder-7b-instruct-q4_k_m.gguf`) ↔ Desktop i7 `dca-NGE65Z`
(192.168.1.138, 8 GiB RAM, serves `Llama-3.2-1B-Instruct-Q4_K_M.gguf`).

### Verified live on the LAN (2026-08-19)

1. **Both nodes on current HEAD `979acbf`** (binary + repo). The Desktop's SSH
   port is closed (operator ran `upgrade-node.sh` locally); remote inference is
   forced by trust + remote-only model placement, exactly as designed.
2. **`scripts/validate-lan.sh` from the Laptop → reply `REMOTE`.** The script
   picked a model served ONLY by the Desktop (`Llama-3.2-1B-Instruct-Q4_K_M.gguf`),
   so routing was forced remote; the Desktop's real reply came back over P2P.
3. **Chat streaming remote** — `POST /v1/chat/completions` with
   `stream:true` to the Desktop's model returned token-by-token SSE deltas.
4. **Execution view shows the P1 strategy live** — the planner records
   `single_worker` decisions with rationale (`"single worker … serves the
   model; multi-worker strategies rejected without batch context"`),
   network_cost ~100_000 µs.
5. **Network view (M19)** — live RTT probe: `rtt_ms: 174`, `locality: Lan`,
   `bandwidth: 1000`.
6. **Fabric view** — Desktop `ONLINE`, `trusted: true`.
7. **Collective memory + reputation + workflows live** — see the notes below;
   the memory-entry-id bug found and fixed this session is documented in the
   ROADMAP §118.

### Self-upgrade now active on both nodes

`--auto-upgrade` (6h watcher) is enabled in the systemd unit on both nodes;
`upgrade-node.sh` patches the unit file idempotently (`ENABLE_AUTO_UPGRADE=0`
to opt out) — the Desktop got it on its next `git pull && upgrade-node.sh`.
See `docs/NODE_UPGRADE.md`.

## Environment

- This node: Laptop i5, node id `dca-GriBWu`, peer `12D3KooWGriBWu…`,
  `allow_remote_inference: true`, port 8080, running the binary rebuilt from
  HEAD (includes `/v1/batch` + streamed batch routing).
- Remote node: Desktop i7, node id `dca-NGE65Z`, peer `12D3KooWNGE65…`,
  `node_name: decentraai-node`, upgraded to current HEAD with
  `allow_remote_inference: true`.

## LIVE VERIFIED (real Laptop → Desktop remote execution, before the Desktop
## went offline)

The Desktop **did** serve remote inference over the real LAN link:

1. **Discovery / P2P / trust / capability**: both nodes discovered each other;
   the Desktop peer appeared in `/v1/network` `connected`; the Desktop was
   trusted via `POST /api/admin/worker/trust`; it advertised
   `accepts_remote_inference: true` and `served_models: [tinyllama.gguf]`.
2. **Laptop → Desktop remote inference**: a `worker_hint: dca-NGE65Z` chat
   request to `tinyllama.gguf` returned a real completion, with response headers
   `X-Decentra-Origin: remote`, `X-Decentra-Worker: 12D3KooWNGE65…`,
   `X-Decentra-Node: dca-NGE65Z` — proving the request crossed the P2P link and
   the Desktop served it.
3. **Provenance**: audit recorded `inference_completed` with the Desktop worker
   id, `status: completed`, `tokens_used`, and `processing_time_ms`; execution
   view recorded request_id + model_hash + tokens + latency.
4. **Quota**: the Desktop's worker identity (`12D3KooWNGE65…`) earned quota
   (`total_earned: 6276`) from the real measured remote executions
   (credit events per request_id, policy v1). Quota reserve → execute → settle
   works across the fabric.
5. **Failure / recovery**: when a request to the Desktop timed out, the system
   released the reservation (in_flight back to 0) and recorded `inference_failed`
   — correct recovery behavior.

## Batch / adaptive fan-out — partially blocked (Desktop went offline)

`/v1/batch` was added (operator/admin-gated) to dispatch independent requests
via `route_batch`. The batch endpoint routes each request through the **streamed**
send path (which is reliable over the high-latency LAN), pinning each to its
allocated worker via `route_request_streamed_on`. Local-model requests completed
(7 tokens each); requests routed to the Desktop timed out because the Desktop
went offline mid-validation.

**Root-cause fix landed**: `route_batch` now uses `route_request_streamed_on`
(streamed + pinned) instead of the non-streamed `send_request`, which timed out
waiting for a buffered final response over the 10-18 s RTT LAN.

## CURRENT STATE / LOCAL-BLOCKED

At report time the Desktop node's P2P port (192.168.1.129:38231) is **closed /
connection refused** — the Desktop has gone offline (or changed listeners). This
node still serves locally. Full cross-node re-validation of remote execution +
batch requires the Desktop to be back online.

LIVE remote execution was proven before the Desktop went offline (see above).
The remaining item is to re-run the batch validation once the Desktop is back.

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
#    IMPORTANT: this also enables remote inference in the node config
#    (inference.allow_remote_inference + network.private_swarm), because the
#    binary alone does NOT advertise accepts_remote_inference — the config
#    flag does. Set ENABLE_REMOTE=0 to leave the config untouched.
bash scripts/upgrade-node.sh

# 3. Confirm the node now advertises remote inference:
systemctl --user status decentraai-node      # should be active
curl -s -H "Authorization: Bearer $(cat ~/.decentraai/runtime/api.token)" \
  http://127.0.0.1:8080/v1/compute | python3 -m json.tool
#   -> the Desktop's own row should show "accepts_remote_inference": true
```

The script (`scripts/upgrade-node.sh`) is idempotent: it swaps the binary +
restarts the service, and (unless `ENABLE_REMOTE=0`) flips
`inference.allow_remote_inference` and `network.private_swarm` to `true` in the
config (with a timestamped backup) so the node advertises remote inference. It
never touches node data/identity.

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
