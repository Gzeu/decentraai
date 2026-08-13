# DecentraAI

Decentralized P2P distribution of AI model artifacts and verifiable local or remote inference.
Nodes discover each other on the LAN, exchange cryptographically verified model chunks,
and serve inference through a local OpenAI-compatible endpoint with a live web dashboard.
Free, contribution-tiered subscriptions gate models and request rates per token.

DecentraAI is a **distributed execution fabric**, not a marketplace, cloud
marketplace, token/economic product, or centralized server product.

## Verified today (current status)

The execution-fabric foundation (M18) has been verified on **real hardware /
LAN** — two physical Ubuntu machines (Desktop ↔ Laptop), each running the
single universal `decentraai node`:

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
| P2 | Chat UI in the dashboard (single embedded dashboard: normal-user view + opt-in advanced block) | Done |
| M18 | **Distributed execution fabric foundation** — universal node, mDNS/libp2p discovery, trusted admission, fabric planner, reservations, real remote inference, streaming, worker reuse, bidirectional Desktop ↔ Laptop (verified on real LAN) | **Done** |
| M19 | Network-aware scheduler (latency, bandwidth, topology, transfer cost) — real RTT via InferPing/InferPong, fold reach cost into planner scoring | Done |
| M20 | KV-aware inference fabric — coordinator-side KV/session accounting, continuation affinity, KV headroom/locality (llama-server live-KV occupancy not claimed) | Done |
| M21 | Distributed MoE / expert fabric | Next |
| M22 | Multi-engine runtime abstraction | Next |
| M23 | Autonomous execution planner | Next |
| M24 | Resilient distributed fabric (lifecycle, failure detection, recovery) | Next |

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

### 4. Subscriptions: issue tokens with tiers (P1)

Everything is free; the tier reflects contribution. The master token in
`runtime/api.token` is unlimited admin; issued tokens get per-tier model
allowlists and rate limits, applied at the next request (no restart):

```bash
decentraai token create --name alice --tier 1   # guest: allowlisted models only, 10 req/min
decentraai token create --name bob --tier 2     # contributor: all models, 60 req/min
decentraai token list
decentraai token revoke --name alice
```

Tokens are `dsk_<64 hex>`, shown once at creation and stored only as
BLAKE3 hashes in `db/tokens.json`. The `tiers:` section of the config
defines each tier's `models` allowlist (empty = all models) and
`rate_limit_per_minute`. Subscribers get 403 for out-of-tier models and
429 past the rate limit; both are audited and visible on the dashboard.

### 5. Watch the node in your browser (dashboard)

Open `http://127.0.0.1:8080/` while `serve` runs. The dashboard refreshes
every 3 seconds and shows:

- the loaded model with file size, plus uptime and idle timer
- inference metrics: completed requests, total tokens generated, last
  request speed (tok/s), and the last 12 inference calls with prompt /
  completion tokens and duration
- live system pressure: free/total RAM, CPU threads, GPU name,
  temperature, free VRAM, utilization
- tracked peers with verified/failed chunks, score, and ban status
  (`GET /v1/peers`, token-guarded)
- the latest security events from the audit log (incl. token and
  rate-limit events)
- a share guide with the exact `swarm start` + `pull` commands for this node

The dashboard reads only `GET /status` and `GET /v1/peers` — watching the
page never touches the inference backend, so it neither inflates the
request counter nor resets the idle-unload clock.

```bash
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model":"tinyllama","messages":[{"role":"user","content":"Hello"}],"max_tokens":20}'
```

### 6. Better answers: model quality and speed

TinyLlama 1.1B is a smoke-test model — it proves the pipeline, but its
answers are weak and it hallucinates languages. For real conversations
on a 16 GB / 4-thread CPU node:

| Model | Size (Q4_K_M) | Why |
|---|---|---|
| **Qwen3-4B-Instruct** (2507) | ~2.5 GB | Best multilingual 3–4B tier, includes Romanian; recommended default |
| **Phi-4-mini** (3.8B) | ~2.5 GB | Best reasoning/math at this size (English-first) |
| Qwen3-8B (stretch) | ~5 GB | Noticeably smarter, ~half the speed; fits in 16 GB RAM |

```bash
# download the GGUF (Q4_K_M variant) from Hugging Face into your models dir,
# e.g. https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507-GGUF
decentraai registry scan --directory ~/models
decentraai serve start --model qwen3-4b-instruct-2507-q4_k_m.gguf
# and add the new file name to the tiers allowlists you want to grant
```

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
validation. Reputation-based compensation for workers (M9-9) is not yet
implemented.

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
- `crates/config` — typed YAML configuration with validation (incl. tiers)
- `crates/distributed` — P2P distributed inference: worker discovery, request routing, queue management, f
## Layout

- `crates/audit` — append-only security audit log
- `crates/config` — typed YAML configuration with validation (incl. tiers)
- `crates/distributed` — P2P distributed inference: worker discovery, request routing, queue management, fallback handling
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
