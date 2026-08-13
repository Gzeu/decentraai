# DecentraAI

Decentralized P2P distribution of AI model artifacts and verifiable local or remote inference.
Nodes discover each other on the LAN, exchange cryptographically verified model chunks,
and serve inference through a local OpenAI-compatible endpoint with a live web dashboard.
Free, contribution-tiered subscriptions gate models and request rates per token.

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
| P2 | Chat UI in the dashboard | Next |

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
```

On the machine that **wants** a model — copy the address the server printed
(replace the interface IP with the server's LAN IP if needed):

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
- Remote inference between nodes is partially enabled via distributed mode (see below)
- After idle unload, restart `serve` to reload the model
- Token usage counters are in-memory (persistence lands with P3)

### M9: Distributed Inference (P2P request routing)

DecentraAI now supports distributed inference across the P2P network. Nodes can
register as workers to serve models and route inference requests to peer GPUs.
Work is compensated in reputation.

**Start a distributed node** (acts as both worker and client):

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

## CLI quick reference

```bash
decentraai init --data-dir ~/.decentraai        # bootstrap dirs + Ed25519 identity
decentraai doctor --config <path>               # budgets, GPU, PeerId, admission verdict
decentraai config validate --file <path>        # strict config check
decentraai registry scan --directory <path>     # index local GGUF models
decentraai registry list                        # show registered models
decentraai swarm start --config <path>          # serve + announce models on the LAN
decentraai pull --from <multiaddr> --list       # browse a peer's catalog
decentraai pull --from <multiaddr> --model <f>  # verified download from a peer
decentraai serve start --model <name>           # gated inference + dashboard :8080
decentraai distributed start --model <name>     # distributed inference node (worker mode)
decentraai distributed start                    # distributed inference node (client mode)
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
