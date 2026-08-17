# DecentraAI

Decentralized P2P distribution of AI model artifacts and verifiable local or remote inference.
Nodes discover each other on the LAN, exchange cryptographically verified model chunks,
and serve inference through a local OpenAI-compatible endpoint with a live web dashboard.
Free, contribution-tiered subscriptions gate models and request rates per token.

> **v1.0.0** — initial production release. See [`CHANGELOG.md`](CHANGELOG.md) for
> the milestone/session history and [`ROADMAP.md`](ROADMAP.md) for the honest
> current-state checklist.

DecentraAI is a **distributed execution fabric**, not a marketplace, cloud
marketplace, token/economic product, or centralized server product.

## Verified today (current status)

The execution-fabric foundation (M18) has been verified on **real hardware /
LAN** — two physical Ubuntu machines (Desktop ↔ Laptop), each running the
single universal `decentraai node`:

- **Collective Intelligence (P0+P1)**: nodes host **logical agents** (execution
  contexts on the node — not extra processes) with signed capability claims,
  advertise them over the P2P channel, and answer unified semantic+physical
  capability questions with one compositional matcher verdict. The dashboard
  AGENTS view shows the collective agent layer (local + remote). See
  `docs/COLLECTIVE_INTELLIGENCE.md` for the architecture and the next
  milestones (P2 messaging → P3 delegation → P4 verification).

- **Universal node**: every installation is a symmetric coordinator + worker;
  one process does onboarding, P2P discovery, worker advertisement, model
  serving and distributed inference. No separate `decentraai distributed`
  runs in the product flow.
- **Discovery / transport**: automatic mDNS discovery + libp2p P2P transport
  on the LAN (no manual topology or addresses).
- **Trusted admission**: a remote coordinating node only schedules to workers the
  user has explicitly trusted (`decentraai trust add <peer>`).
- **Capability-aware fabric planner**: `plan_and_reserve` selects a trusted,
  eligible worker and holds a resource reservation for the request.
- **Real remote inference**: the coordinator sends a P2P `InferRequest`; the
  **remote worker calls its own local llama-server on `127.0.0.1`**; streamed
  `InferProgress` frames return to the coordinator, ending in a terminal
  `InferResponse`. The loopback backend URL is **never advertised** as a remote
  endpoint.
- **Streaming responses** and cooperative cancellation.
- **Worker reuse** and correct capacity tracking (concurrent requests use
  separate request IDs and non-colliding reservations).
- **Bidirectional execution**: Desktop → Laptop and Laptop → Desktop both work;
  every node can coordinate and contribute.
- **Persistent workers**: the worker's local llama-server stays bound to
  loopback and is **not idle-unloaded**, so a node never advertises ready while
  its engine is dead (`5758e05`).
- **Explicit remote-sharing opt-in**: `inference.allow_remote_inference` is
  now enforced end-to-end — a worker that does not opt in rejects remote
  `InferRequest`s at its own gate and is never selected by a coordinator
  (`NotAcceptingRemote`); the local peer is always eligible. Advertisements
  carry the flag (`accepts_remote_inference`, default `false` for old peers).
- **Per-node identity everywhere**: every node sees the fabric from its own
  perspective — real LAN addresses (new p2p `Peers` snapshot →
  `/v1/network.addresses` + `local_addresses`), per-worker CPU/RAM/GPU/
  engine/model resources, a live trust chain
  DISCOVERED → UNTRUSTED → APPROVED → CONNECTED → WORKER READY, and a real
  discovery event feed (discovered / offline / reconnected) in the
  dashboard.
- **Identity = compact ID**: every node's default name is its own
  `dca-xxxxxx` indicator derived from the identity at `setup` time — a fresh
  node is already distinct on the fabric, no manual naming needed. The ID is
  shown on the canvas, the Fabric nodes cards and the Workers view;
  `setup --name <nume>` is only an optional semantic label.

Verified: worker selection, reservation create/release, P2P execution,
streaming, worker health, and two-machine sequential + concurrent + reuse
requests — all on real hardware, no mocks.

## What exists today

| Milestone | Scope | Status |
|---|---|---|
| M0-M1 | Rust workspace, CI quality gates, system probe + admission checks, Ed25519 identity | Done |
| M2 | Local GGUF registry with path safety and deterministic persistence | Done |
| M3 | LAN swarm: signed protocol, Merkle manifests, libp2p transport, verified chunk transfer with resume, E2E tests | Done |
| M4 | Inference runtime: llama-server manager, admission gate, OpenAI-compatible API with auth and idle unload | Done |
| M5 | Swarm intelligence: peer reputation with bans, signed manifest announcements, deterministic multi-provider scheduler | Done |
| M6 | Hardening: quarantine workflow, audit logging | Done |
| M7 | Sharing + UX: peer catalog, `decentraai pull`, live web dashboard | Done |
| M8 | Packaging: `scripts/install.sh` + [deployment guide](docs/deployment.md) | Done |
| M9 | Distributed inference: route requests to peer GPUs, paid in reputation | Done |
| P1 | Subscriptions: hashed token registry, `decentraai token` CLI, per-tier model allowlists + rate limits | Done |
| P6 | Zero-touch sharing: `swarm start` auto-downloads announced models (mDNS + consent modes) | Done |
| P2 | Chat UI in the dashboard (single embedded dashboard, SSE-streamed chat + non-streaming fallback, normal-user view + opt-in advanced block) | Done |
| M18 | **Distributed execution fabric foundation** — universal node, mDNS/libp2p discovery, trusted admission, fabric planner, reservations, real remote inference, streaming, worker reuse, bidirectional Desktop ↔ Laptop (verified on real LAN) | **Done** |
| M19 | Network-aware scheduler (latency, bandwidth, topology, transfer cost) — real RTT via InferPing/InferPong, fold reach cost into planner scoring | Done |
| M20 | KV-aware inference fabric — coordinator-side KV/session accounting, continuation affinity, KV headroom/locality (llama-server live-KV occupancy not claimed) | Done |
| M21 | Distributed MoE / expert fabric | Next |
| M22 | Multi-engine runtime abstraction | Next |
| M23 | Autonomous execution planner | Next |
| M24 | Resilient distributed fabric (lifecycle, failure detection, recovery, bounded idempotency-safe request retry, explicit bounded P2P reconnect loop) | **Done** |

## Using DecentraAI today

### Install

```bash
git clone https://github.com/Gzeu/decentraai && cd decentraai
bash scripts/install.sh              # --no-llama to skip the llama.cpp build
export DECENTRAAI_LLAMA_SERVER=$HOME/llama.cpp/build/bin/llama-server
```

The installer checks the Rust toolchain (installs rustup when missing),
`cargo install`s the `decentraai` binary, builds llama.cpp's
`llama-server`, and runs `decentraai init`. For production setups
(systemd, firewall, security checklist) see
[docs/deployment.md](docs/deployment.md).

### 0.1. Chat with Open WebUI (primary user-facing Chat)

Open WebUI is the primary Chat; the DecentraAI dashboard stays the technical
control plane (it is not replaced). Connect Open WebUI to a running node as an
OpenAI-compatible backend — DecentraAI already serves the standard `/v1/models`
and `/v1/chat/completions` (streaming + non-streaming) that Open WebUI reads.
See [docs/openwebui.md](docs/openwebui.md) for the exact steps and security
notes.

### 0. Install as a desktop application (Ubuntu)

For a normal-user "Download → Install → Open → Ready" experience, use the
app installer instead: it auto-detects hardware, creates the node identity,
auto-selects a model, installs a systemd user service (auto-start on login +
reboot) and a desktop launcher — no manual config, ports or topology:

```bash
bash scripts/install-app.sh          # build + onboard + service + launcher
```

- Dashboard: `http://127.0.0.1:8080/` (or `decentraai open`)
- Node status / start / stop / restart / logs:
  `systemctl --user status|start|stop|restart decentraai-node`,
  `journalctl --user -u decentraai-node -f`
- Uninstall: `bash scripts/uninstall-app.sh` (`--purge` also removes data)

Upgrade: re-running `bash scripts/install-app.sh` reinstalls the latest binary
(`cargo install --force`) and restarts the service, **keeping your existing
config and identity** under `~/.decentraai`. Use it as the documented upgrade
path.

> Note: `scripts/install.sh` is the older **developer** bootstrap (`cargo
> install` + `init` only — no service, no launcher, no uninstall). Normal
> users should use `scripts/install-app.sh` (section 0) — the production
> install path.

Two machines with DecentraAI installed on the same LAN discover each other
automatically (mDNS + verified auto-share); no one configures topology.

### 0b. One universal node, bidirectional execution (verified)

Each machine runs ONE process — `decentraai node` — which is simultaneously a
coordinator and a worker. Install the same app on both machines, trust each
other's PeerId, and either side can route inference to the other:

```bash
# On both machines: install + run the universal node (systemd keeps it up)
#   DESKTOP   (PeerId 12D3KooW…A)  →  LAPTOP    (PeerId 12D3KooW…B)
decentraai trust add --peer 12D3KooW…B     # on the Desktop, trust the Laptop
decentraai trust add --peer 12D3KooW…A     # on the Laptop,  trust the Desktop

# Desktop → Laptop: a request routed by the fabric planner to the Laptop worker
decentraai node --prompt "Explain what DecentraAI is."
#   fabric planner selected worker (12D3KooW…B  reservation_id=…)
#   P2P InferRequest → Laptop → its local llama-server → streamed → done

# Laptop → Desktop: the symmetric direction (run on the Laptop)
decentraai node --prompt "Explain what DecentraAI is."
```

For every routed request the coordinator: selects a trusted worker via the
fabric planner → holds a reservation → sends a P2P `InferRequest` → the remote
worker executes on its **own local llama-server (loopback)** → streams chunks
back → releases the reservation. The worker's engine is kept alive (never
idle-unloaded) so it stays selectable. This has been verified on real
hardware (two Ubuntu machines on one LAN), including sequential, concurrent,
worker-reuse and both directions — not mocks.

### 0c. Cross-subnet and internet connectivity (relay / DHT / DCUtR)

Since the transport gained NAT traversal, nodes can also reach each other
outside a single LAN subnet:

- **`bootstrap_peers`** — each node can dial a fixed list of peers (LAN or
  public), so the fabric is robust to DHCP IP churn (mDNS alone can miss a
  peer whose address changed). Configure under `network.bootstrap_peers` in
  `node.yaml`.
- **DHT (`dht_enabled: true`)** — Kademlia discovery for cross-subnet peers.
- **Relay + DCUtR (`relay_enabled: true`)** — a node behind NAT can connect
  through a relay server (`/p2p-circuit`) and hole-punch to a direct
  connection where possible.
- **Identify** — the node learns and registers its observed external address.

To join nodes on different subnets / the internet, run one small **public
relay + bootstrap node** (see `docs/PUBLIC_RELAY_NODE.md` — a VPS with port
4001 open, `dht_enabled` + `relay_enabled`), then add its multiaddr to every
member's `bootstrap_peers`. A VPN (Tailscale/WireGuard) is the simpler private
option for cross-subnet-only setups.

> Public IPFS/libp2p bootstrap nodes are **not** drop-in: their transport
> (QUIC/WebTransport + IPFS-specific Noise) does not complete the DecentraAI
> TCP+Noise+Yamux handshake. A self-hosted public node is the reliable path.

### 1. Index your local models

```bash
decentraai init                              # creates ~/.decentraai + Ed25519 identity
decentraai doctor                            # hardware budgets, GPU, PeerId, admission verdict
decentraai registry scan --directory ~/models
decentraai registry list
```

### 2. Share models between two machines on the LAN

On the machine that **has** the models:
```bash
decentraai registry scan --directory ~/models
decentraai swarm start
# prints: Listening: /ip4/0.0.0.0/tcp/<PORT>/p2p/<PEER_ID>
#         Serving: N model(s) announced (signed with the node identity)
#         Sharing: auto — downloading announced models as they appear
```

On the machine that **wants** the models, just run the same:
```bash
decentraai swarm start
# mDNS discovers the peer, and `sharing.mode: auto` downloads and verifies
# every announced model into ~/.decentraai/models automatically.
```

Zero manual steps: mDNS discovery auto-dials LAN peers, and `swarm start`
reacts to model announcements. Each downloaded artifact is verified
(per-chunk BLAKE3 + Merkle root) before it is indexed and re-announced to
the rest of the swarm. Set `sharing.mode: ask` in the config to be
prompted per model, or `off` to ignore announcements.

The manual `pull` path still works (e.g. to pick one model or one peer):
```bash
# browse what the peer serves:
decentraai pull --from /ip4/192.168.0.113/tcp/37079/p2p/12D3KooW... --list
#   Peer 12D3KooW... serves 1 model(s):
#     tinyllama.gguf (0.62 GiB, id: a1b2c3d4...)

# download it (per-chunk BLAKE3 + Merkle verification, resumable):
decentraai pull --from /ip4/192.168.0.113/tcp/37079/p2p/12D3KooW... --model tinyllama.gguf
#   Downloaded and verified: ~/.decentraai/models/tinyllama.gguf
decentraai registry scan --directory ~/.decentraai/models
```

New models can also come straight from the HuggingFace Hub, verified against
the Hub's SHA-256 before they land in the registry:
```bash
# search GGUF models and see what tool categories exist:
decentraai model search qwen --limit 10
decentraai model search llama --categories          # text-generation, question-answering, ...
decentraai model search llama --category text-generation

# download (auto-picks the largest GGUF, or pin a specific file):
decentraai model pull hf:Qwen/Qwen2.5-1.5B-Instruct-GGUF
decentraai model pull hf:Qwen/Qwen2.5-1.5B-Instruct-GGUF:qwen2.5-1.5b-instruct-q4_k_m.gguf
#   Downloading hf:... (Qwen/... / auto (largest GGUF)) ...
#   Downloaded ~/.decentraai/models/qwen2.5-...gguf (N bytes, sha256 ...)
#   Registry updated: N models at ~/.decentraai/db/registry.json
```

Note: the address is a real multiaddr — do not keep the literal `<PORT>`/
`<PEER_ID>` placeholders; quote the whole address if your shell complains.
Testing pull on a single machine requires a second data dir with a second
identity (libp2p refuses self-dial):

```bash
decentraai init --data-dir /tmp/decentraai-b
cp configs/node.example.yaml /tmp/node-b.yaml   # set node.data_dir: /tmp/decentraai-b
decentraai pull --config /tmp/node-b.yaml --from /ip4/127.0.0.1/tcp/<PORT>/p2p/<PEER_ID> --list
```

Transfers are verified per chunk (BLAKE3) with a final Merkle-root check,
resumable after interruption, and recorded in the local reputation store
(`db/reputation.json`). Peers that serve corrupted chunks are temporarily
banned and corrupted staging artifacts land in `quarantine/` with metadata.
Security events (bans, verification failures, admission rejections) are
appended to `logs/audit.jsonl`.

### 3. Run inference with an OpenAI-compatible API

```bash
decentraai serve start --model tinyllama.gguf
# prints: Dashboard: http://127.0.0.1:8080/ (status, peers, share guide)
#         API: http://127.0.0.1:8080/v1 (OpenAI-compatible)
#         Auth: master token: ~/.decentraai/runtime/api.token
#         Subscriptions: tiers: on
#         Threads: N (logical CPUs minus reserve)
```

The gate before every load: config mode (`inference.enabled`), live RAM/GPU
budgets, and GPU temperature from the system probe. llama-server starts
with `--threads` (physical-core budget), `--flash-attn on`, and `--jinja`
(the model's own chat template). The model unloads automatically after
`idle_model_unload_minutes` without requests.

An OpenAPI 3.0 document describing the `/v1` surface (models, chat and text
completions, status, peers, and the operator/admin compute views) is served at
[`/openapi.json`](http://127.0.0.1:8080/openapi.json) so tooling can
introspect the contract (H6).

### 4. Subscriptions: issue tokens with tiers (P1)

Everything is free; the tier reflects contribution. The master token in
`runtime/api.token` is unlimited admin; issued tokens get per-tier model
allowlists and rate limits, applied at the next request (no restart):

```bash
decentraai token create --name alice --tier 1        # guest: allowlisted models only, 10 req/min
decentraai token create --name bob --tier 2           # contributor: all models, 60 req/min
decentraai token create --name ops --tier 2 --role operator  # operator: read-only operational views (H4)
decentraai token list
decentraai token revoke --name alice
```

Tokens are `dsk_<64 hex>`, shown once at creation and stored only as
BLAKE3 hashes in `db/tokens.json`. The `tiers:` section of the config
defines each tier's `models` allowlist (empty = all models) and
`rate_limit_per_minute`. Subscribers get 403 for out-of-tier models and
429 past the rate limit; both are audited and visible on the dashboard.

### 4b. Invites & join (P5)

A private swarm adds a seat without manual token transfer. The operator
issues an invite that bundles a reachable address with a **Tier-1 Guest
token** (least privilege — a leaked invite is never more than a guest):

```bash
# On the coordinator:
decentraai invite --addr /ip4/192.168.1.5/tcp/4001 --ttl 1440   # optional TTL in minutes
# prints:  /ip4/192.168.1.5/tcp/4001/p2p/<peer-id> dsk_<64hex>

# On the fresh node (quote the whole invite):
decentraai join "/ip4/192.168.1.5/tcp/4001/p2p/<peer-id> dsk_<64hex>"
```

`--ttl` makes the guest seat expire after the given minutes (default 0 = no
expiry); an expired token is inactive at the next request (H3).

`join` auto-provisions identity + config, stores the guest token as the
node's credential (`runtime/invite.token`, 0600 — shown nowhere else) and
verifies the coordinating peer is reachable over the verified P2P path.
Revoke a seat at any time with `decentraai token revoke --name invite-<n>`.

### 5. Watch the node in your browser (dashboard)

Open `http://127.0.0.1:8080/` while `serve` runs. The dashboard is the
node's **Command Deck** — a living control plane that answers visually in
seconds: *"what is DecentraAI doing right now?"*

**The Overview is the fabric itself.** A live canvas stage renders the
local node at the center and every advertised worker (Laptop/Desktop/GPU
nodes) as living entities — status color, load ring, trust badge, real
LAN address, connection state and `REMOTE-OK` / `local-only` label —
connected by measured P2P links (M19 RTT). A pipeline strip
(USER → REQUEST → PLANNER → RESERVATION → FABRIC → WORKER → ENGINE →
STREAM → RESULT) lights up from real queue, recent-request and decision
data: when a request is served the planner activates, the selected worker
lights up (named `local` / `remote` in the WORKER stage), the reservation
appears and tokens visibly stream; when idle the stage is calm and
atmospheric. The M23 planner has a visible identity and an
autonomous-decision strip shows safe operational facts only (CLASSIFYING →
N CANDIDATES → NETWORK COST → KV AFFINITY → ENGINE → SELECTED WORKER →
EXECUTING, no chain-of-thought). M24 recovery is part of the story: on a
real failure the affected worker changes state and the replan becomes
visible. A **Fabric nodes** strip below renders every node (local +
discovered workers) as an identity card — CPU/RAM/VRAM, engine, served
models, LAN address, live trust chain
DISCOVERED → UNTRUSTED → APPROVED → CONNECTED → WORKER READY — and a
discovery feed surfaces real discovered / offline / reconnected events.
**Nothing is faked** — every light, particle and state comes from
`/status`, `/v1/peers`, `/v1/compute`, `/v1/network` and `/v1/execution`.

Metrics, tables and the other 12 views (Chat, Topology, Decisions,
Execution, Workers, Network, Models, Observability, Recovery, Diag,
Security, Settings) remain available below the stage and via the sidebar
rail with a command palette (Ctrl+K):

- **Overview** — living fabric stage (primary) + decision strip +
  secondary Model/Inference/Queue cards, recent inference calls and the
  share guide
- **Chat** — streams model responses (SSE) token-by-token by default, with
  a non-streaming toggle, abort and retry; keeps the conversation across
  page reloads (client-side `localStorage`)
- **Topology** — the same fabric engine on a larger stage
- **Autonomous decisions** — the M23 decision ring: workload class,
  candidate score breakdowns (tps/latency/load/queue/headroom/net/kv),
  constraint breaches, KV affinity, expected mode, reasoning and the
  safe-reasons trace
- **Execution** — recent planner decisions: selected worker, score, stages,
  continuation, network RTT, KV/session headroom, outcome and reasoning
- **Workers** — each worker's status, load, queue, tok/s, latency, free RAM,
  in-flight requests; **Network** — measured per-peer links (RTT, bandwidth,
  locality) + connected peers; **Models** — served models with engine,
  context, RAM/VRAM footprint, active/loaded
- **Observability** — latency/tok-per-second sparklines and gauges;
  **Recovery** — engine auto-restart (respawn) count, active KV sessions and
  resilience events; **Diag** — node health, P2P/network, workers, engine
  endpoint, audit events
- **Security** — token create/list/revoke plus the audit event stream
  (incl. token and rate-limit events); **Settings** — node name, discovery,
  tracked/trusted peers, model + engine, the real resource limits/guards
  from config (CPU/RAM reserve, GPU policy), the generation defaults
  (sampling + system prompt) and the subscription tier policies

**Settings is live-editable (master-gated).** From the dashboard you can
change the **generation defaults** (temperature, top_p, top_k,
repeat_penalty, system prompt) and the **resource admission limits** (CPU/RAM
%, reserve cores/RAM/VRAM, GPU VRAM cap, temperature stop). Generation changes
apply immediately to the next inference request; resource changes persist to
`node.yaml` and apply on the next start (they gate startup admission). Both
are audited and survive a restart.

The page polls only the read-only status/control endpoints (`/status`,
`/v1/peers`, `/v1/compute`, `/v1/network`, `/v1/execution`) — it never
touches a proxied inference endpoint on its own, so watching the page
neither inflates the request counter nor resets the idle-unload clock.
The Chat POST happens only when you actually send a message.

```bash
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model":"tinyllama","messages":[{"role":"user","content":"Hello"}],"max_tokens":20}'
```

### 0.2. MCP for external AI agents

A running node exposes its fabric to external AI agents over the [Model Context
Protocol](https://modelcontextprotocol.io) as a read-only JSON-RPC endpoint:
`POST /mcp` on the API port, authenticated with the same `dsk_` Bearer token.
Negotiate with `initialize`, then `tools/list` / `tools/call`. Read-only tools:
`get_status`, `list_workers`, `list_models`, `list_executions`, `list_peers`.
No new token/identity system — MCP is a thin translation over the existing API.

```bash
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl -s http://127.0.0.1:8080/mcp -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
```

When the node runs as a fabric node (`decentraai node` with a compute
manager), the chat proxy can also serve models advertised by *trusted remote
workers*. The chat model picker offers **Auto (best available)** (default:
the largest model actually served anywhere in the fabric, local wins ties),
**Local models**, and **Remote workers** (every advertised model labelled
with its node, even when a local copy exists). A manual remote choice sends
`worker_hint`; responses are tagged `X-Decentra-Origin: remote` (+
`X-Decentra-Worker`/`X-Decentra-Node`) with a "served by `dca-xxxx` ·
remote" badge. Remote routing never occupies the local inference queue.

### 6. Better answers: model quality and speed

TinyLlama 1.1B / TinyLlama-class models are smoke tests — they prove the
pipeline but answer weakly and hallucinate languages. For real conversations
on a 16 GB / 4-thread CPU node, drop a stronger GGUF into the node's `models/`
dir and it will auto-detect it:

| Model | Size (Q4_K_M) | Why |
|---|---|---|
| **Qwen3-8B** (recommended default) | ~5.0 GB | 8B dense, multilingual (incl. Romanian), much stronger reasoning/code; fits 16–32 GB RAM |
| **Qwen3-4B-Instruct** (2507) | ~2.5 GB | Best 3–4B tier on weaker/4-thread CPUs; recommended minimum |
| **Phi-4-mini** (3.8B) | ~2.5 GB | Best reasoning/math at this size (English-first) |

```bash
# Recommended default — download into the node's models dir, then re-select:
# Qwen3-8B-Q4_K_M.gguf from https://huggingface.co/Qwen/Qwen3-8B-GGUF
mkdir -p ~/.decentraai/models
# ...move/copy Qwen3-8B-Q4_K_M.gguf into ~/.decentraai/models/ ...
#   (auto-detection picks alphabetically-first, so remove/rename any older
#    tinyllama.gguf that would sort before it)
decentraai registry scan --directory ~/.decentraai/models
decentraai serve start --model Qwen3-8B-Q4_K_M.gguf
# or let the universal node auto-select it:  decentraai node
# and add the file name to the tiers allowlists you want to grant
```

Context: `decentraai setup` keeps `max_context_tokens` at 4096 on a 16 GB
node and 8192 on 32 GB — both suit Qwen3-8B. The default generation settings
(temperature 0.7, top_p 0.9) suit Qwen3 non-thinking mode.

Built-in speed tuning (automatic since this release): `--threads` set
to logical CPUs minus the configured reserve, `--flash-attn on`,
`--jinja` (proper chat templates), and a 4096-token default context.

### Current limitations

- GGUF models only (matches the verification capability)
- Dashboard binds to loopback only (no LAN exposure yet)
- Auto-share downloads are serialized on one worker (reputation writes are
  not concurrent); large bursts of announcements queue up
- Remote inference between nodes runs through the universal node's fabric
  planner (P2P `InferRequest` → remote worker → its own local llama-server);
  the lower-level `decentraai distributed` mode is not required for the
  product flow
- After idle unload, restart `serve` to reload the model
- Token usage counters are in-memory (persistence lands with P3)

### M9: Distributed Inference (low-level P2P routing mode)

Distributed inference across the P2P network is built into the **universal
`decentraai node`** (every node is a coordinator and a worker; see "0b. One
universal node, bidirectional execution"). Below is the lower-level
`decentraai distributed` command that underlies worker registration, request
routing, queueing and streaming. The product flow does **not** require running
`decentraai distributed` separately — it is provided for low-level use and
validation. Reputation-based compensation for workers (M9-9) is implemented
as a synthetic contribution-credits ledger; see `decentraai tier suggest`.

**Start a low-level distributed node** (acts as both worker and client):

```bash
decentraai distributed start --model tinyllama.gguf
# prints: DecentraAI distributed node running
#         PeerId: 12D3KooW...
#         Listening: /ip4/0.0.0.0/tcp/<PORT>/p2p/12D3KooW...
#         Mode: worker
```

**Distributed node modes:**
- **worker**: Serves models for inference (requires `--model`); spawns a real
  `llama-server` subprocess, streams tokens back to the requester, and aborts
  generation on `InferCancel`
- **client**: Routes requests to other workers (no `--model`); pass `--prompt`
  to run one request against the best discovered worker and stream the answer

**One-shot prompt from the coordinator:**

```bash
decentraai distributed --config configs/node.coord.yaml --prompt "Tell me a haiku"
```

**Standalone client (`decentraai-p2p-invoke`)**: exercises the real path
(dial worker -> InferRequest -> queue -> llama-server -> streamed progress) for
validation and scripting:

```bash
decentraai-p2p-invoke \
  --peer /ip4/127.0.0.1/tcp/<PORT>/p2p/12D3KooW... \
  --model /path/to/tinyllama.gguf \
  --prompt "Hello"
# or pass --model-hash <blake3> instead of --model; Ctrl-C cancels generation
```

**Key features:**
- Worker discovery and real-time capacity reporting (periodic announcement broadcasts)
- Intelligent request routing based on worker capacity, latency, and throughput
- Automatic fallback to alternative workers when primary workers fail
- Request queue management with FIFO processing, cancellation and timeout handling
- Streaming `InferProgress` frames and cooperative cancellation (`InferCancel`)
- Reputation-based compensation for worker contributions

**Configuration options:**
```bash
# Custom configuration file
decentraai distributed start --config custom.yaml --model my-model.gguf

# Multiple models
decentraai distributed start --model model1.gguf --model model2.gguf
```

**Monitoring:**
The distributed node exposes the same dashboard at `http://127.0.0.1:8080/` with
additional distributed inference metrics including worker count, queued requests,
and routing statistics.

### 7. Compute sharing: capability-aware routing (M11–M13)

DecentraAI's core product is sharing **compute/GPU capacity**, not only model
files. Since M11, distributed nodes advertise real hardware
(`decentraai-compute`): GPU model/VRAM, RAM, CPU cores, load, health, and the
models each node serves. The coordinator picks a worker only when it serves
the requested model **and** has RAM/VRAM headroom, and books a resource
reservation that is released when the request finishes — two workloads can
never double-book the same VRAM.

```bash
# A node offering its GPU as a compute worker:
decentraai distributed start --name gpu-rig --model tinyllama.gguf

# A coordinator routing by capability (fallback to legacy capacity routing):
decentraai distributed start --name coordinator --prompt "Tell me a haiku"
```

**How it works:**
- `ComputeAdvertisement` frames are broadcast on the heartbeat interval and
  processed by every node's P2P handler into a local compute registry
- The `ComputeScheduler` ranks eligible workers deterministically (score
  desc, PeerId asc) and returns a `Placement` with a held reservation
- `route_request` selects via the compute path first; the legacy
  announcement-based router remains the fallback
- Compute workers are only trusted after pairing (`trust.db`), mirroring the
  existing trust model; an empty trust set disables compute selection

## CLI quick reference

```bash
decentraai --log-format json node --config <path>  # structured JSON logs (H8); the worker's
                                                   # logs tag each request with request_id/trace_id
decentraai setup --data-dir ~/.decentraai        # one-command onboarding: detect HW → identity → model → validated config → READY
decentraai init --data-dir ~/.decentraai        # bootstrap dirs + Ed25519 identity
decentraai doctor --config <path>               # budgets, GPU, PeerId, admission verdict
decentraai config validate --file <path>        # strict config check
decentraai registry scan --directory <path>     # index local GGUF models
decentraai registry list                        # show registered models
decentraai swarm start --config <path>          # serve + announce models; auto-share (sharing.mode)
decentraai pull --from <multiaddr> --list       # browse a peer's catalog
decentraai pull --from <multiaddr> --model <f>  # verified download from a peer
decentraai serve start --model <name>           # gated inference + dashboard :8080
decentraai node --config <path>                 # universal node (coordinator + worker); primary product process
decentraai node --config <path> --prompt "..."  # universal node: run one routed inference, then exit
decentraai open                                 # open the running node's dashboard
decentraai trust add --peer <PeerId> --name <n> # trust a peer (enables scheduler to select it)
decentraai distributed --model <name>           # low-level distributed node (worker mode)
decentraai distributed                          # low-level distributed node (client mode)
decentraai token create --name <n> --tier 1..3  # issue a subscription token
 decentraai token list                           # show issued tokens
 decentraai token revoke --name <n>              # revoke (effective next request)
 decentraai invite --addr <host:...>             # new-seat invite + guest token (P5)
 decentraai join "<addr /p2p/<peer-id> dsk_...>" # join from an invite (P5)
 decentraai serve start --backend http://H:P    # Q3: local auth/tiers/queue + remote model
 decentraai agent list --config <path>          # show this node's logical agents (P1)
```

## Architecture highlights

- **Identity**: Ed25519 keypair at `<data_dir>/identity/key.pem` (0600); the libp2p
  keypair is derived from it, binding transport PeerIds to node keys
- **Protocol** (`crates/protocol`): `deny_unknown_fields`, size caps, base64 binary
  fields, canonical signing (`sign_manifest` / `verify_manifest_signature`),
  catalog messages for peer browsing
- **Manifests** (`crates/manifest`): GGUF magic validation, 4 MiB chunks, BLAKE3,
  deterministic Merkle root over raw digests, atomic JSON writes
- **Distributed inference** (`crates/distributed`, `crates/p2p-invoke`): worker
  registration + periodic announcement broadcasts, capacity-aware router, FIFO
  queue with cancellation, `register_worker_backend` streaming (queue ->
  OpenAiCompatibleBackend -> streamed `InferProgress` -> terminal `InferResponse`),
  and a standalone `decentraai-p2p-invoke` client for the real end-to-end path
- **Execution fabric** (`crates/fabric`): engine-neutral execution planning — an
  `ExecutionPlan` (single / sequential / fan-out) with fallback, built by an
  `ExecutionPlanner` that weighs engine capability (M22), network reach/RTT
  (M19), KV-cache state (M20) and expert-routing capability (M21). Integrated
  into `route_request` via `plan_and_reserve`; `reserve_worker` keeps capacity
  authority in the scheduler. A coordinator reaper evicts dead workers with
  audit (M24).
- **Collective intelligence** (`crates/agents`, P0+P1): logical agents as
  *execution contexts on nodes* (not extra processes) — `AgentRecord`
  (identity + semantic capability claims + models + tools + policies),
  `AgentRegistry`, a **unified capability matcher** (one compositional verdict:
  hub provenance-aware semantic gate + agent model allowlist + compute
  physical gate), and `SignedAgentAdvertisement` over the P2P channel
  (anti-spoof signature verification). Nodes advertise their agents on the
  heartbeat; the dashboard AGENTS view shows local + remote agents; see
  `docs/COLLECTIVE_INTELLIGENCE.md`.
- **Transfer** (`crates/p2p/transfer.rs`): per-chunk verification, `.part` staging +
  `.done` resume bitmap, full-file hash + Merkle gate, atomic rename; single-peer
  (`download`) or ranked multi-provider waves (`download_multi`); corrupted
  artifacts are quarantined with metadata
- **Reputation** (`crates/p2p/reputation.rs`): only cryptographic failures count
  toward bans; scores persist atomically; deterministic ranking (score desc,
  PeerId asc) feeds the scheduler; serializable summaries feed the dashboard
- **Runtime** (`crates/runtime`): llama-server as a managed subprocess (never FFI)
  with tuned flags (`--threads`, `--flash-attn on`, `--jinja`); thin axum proxy
  with tiered Bearer auth (master + subscription tokens), rate limits, inference
  metrics, and the live dashboard; token at `runtime/api.token` (0600)
- **Tokens** (`crates/tokens`): subscription registry; plaintext shown once,
  BLAKE3 hashes only on disk
- **Audit** (`crates/audit`): append-only `logs/audit.jsonl` for security events

## Layout

- `crates/audit` — append-only security audit log
- `crates/agents` — collective-intelligence agent model: logical agents, unified semantic+execution capability matcher, agent tasks, registry, advertisements
- `crates/config` — typed YAML configuration with validation (incl. tiers)
- `crates/distributed` — P2P distributed inference: worker discovery, request routing, queue management, fallback handling, agent manager
- `crates/fabric` — execution planner: single / sequential / fan-out plans, network/KV/expert-aware scoring
- `crates/hub` — HuggingFace catalog + verified download + semantic capability taxonomy/intent
- `crates/identity` — Ed25519 keypairs and PeerId derivation
- `crates/manifest` — GGUF manifests: chunk hashes, Merkle root, atomic writes
- `crates/protocol` — swarm message schemas (incl. catalog) and canonical signing
- `crates/p2p` — libp2p transport actor, verified transfer, reputation, scheduler
- `crates/registry` — local model registry with path safety
- `crates/runtime` — llama-server manager, admission gate, OpenAI API + dashboard
- `crates/system-probe` — hardware probing and admission decisions
- `crates/tokens` — subscription token registry (hashed, tiered)
- `crates/node-cli` — the `decentraai` binary
- `scripts/install.sh`, `docs/deployment.md` — installer and production guide
- `AGENTS.md`, `action-plan.md`, `ROADMAP.md`, `docs/` — design and handoff docs

## Security baseline

No artifact is usable before hash, manifest, and policy verification. Manifest and
chunk responses carry no signatures by design: integrity is anchored in the signed
manifest's `chunk_hashes` and Merkle root, enforced by per-chunk BLAKE3
verification at assembly. Prompts and outputs are never logged by default.
Private keys and API tokens never enter Git or telemetry; subscription tokens
are stored only as hashes. The dashboard never exposes secrets; the API token
guards every `/v1/*` endpoint.
