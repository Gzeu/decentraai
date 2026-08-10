# DecentraAI

Decentralized P2P distribution of AI model artifacts and verifiable local or remote inference.
Nodes discover each other on the LAN, exchange cryptographically verified model chunks,
and serve inference through a local OpenAI-compatible endpoint with a live web dashboard.

## What exists today

| Milestone | Scope | Status |
|---|---|---|
| M0-M1 | Rust workspace, CI quality gates, system probe + admission checks, Ed25519 identity | Done |
| M2 | Local GGUF registry with path safety and deterministic persistence | Done |
| M3 | LAN swarm: signed protocol, Merkle manifests, libp2p transport, verified chunk transfer with resume, E2E tests | Done |
| M4 | Inference runtime: llama-server manager, admission gate, OpenAI-compatible API with auth and idle unload | Done |
| M5 | Swarm intelligence: peer reputation with bans, signed manifest announcements, deterministic multi-provider scheduler | Done |
| M6 | Hardening: quarantine workflow, audit logging | Done |
| M7 | Sharing + UX: peer catalog, `decentraai pull`, web dashboard | Done |
| M8 | Packaging and deployment guide | Planned |

## Using DecentraAI today

### Install

```bash
git clone https://github.com/Gzeu/decentraai && cd decentraai
cargo install --path crates/node-cli
# llama.cpp for inference (only needed for `serve`):
cmake -S /path/to/llama.cpp -B /path/to/llama.cpp/build
cmake --build /path/to/llama.cpp/build --config Release --target llama-server
export DECENTRAAI_LLAMA_SERVER=/path/to/llama.cpp/build/bin/llama-server
```

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
#         Auth: Bearer token: ~/.decentraai/runtime/api.token
```

The gate before every load: config mode (`inference.enabled`), live RAM/GPU
budgets, and GPU temperature from the system probe. The model unloads
automatically after `idle_model_unload_minutes` without requests.

### 4. Watch the node in your browser (dashboard)

Open `http://127.0.0.1:8080/` while `serve` runs. The dashboard refreshes
every 3 seconds and shows:

- the loaded model (read from the backend's `/v1/models`), requests served,
  idle timer, backend and API addresses
- tracked peers with verified/failed chunks, score, and ban status
  (`GET /v1/peers`, token-guarded)
- the latest security events from the audit log
- a share guide with the exact `swarm start` + `pull` commands for this node

The dashboard and `GET /status` are public on loopback (no secrets);
the API endpoints require the Bearer token.

```bash
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl http://127.0.0.1:8080/v1/models -H "Authorization: Bearer $TOKEN"
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model":"tinyllama","messages":[{"role":"user","content":"Hello"}],"max_tokens":20}'
```

### Current limitations

- GGUF models only (matches the verification capability)
- Dashboard binds to loopback only (no LAN exposure yet)
- Remote inference between nodes is not enabled yet (private-swarm first)
- After idle unload, restart `serve` to reload the model

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
```

## Architecture highlights

- **Identity**: Ed25519 keypair at `<data_dir>/identity/key.pem` (0600); the libp2p
  keypair is derived from it, binding transport PeerIds to node keys
- **Protocol** (`crates/protocol`): `deny_unknown_fields`, size caps, base64 binary
  fields, canonical signing (`sign_manifest` / `verify_manifest_signature`),
  catalog messages for peer browsing
- **Manifests** (`crates/manifest`): GGUF magic validation, 4 MiB chunks, BLAKE3,
  deterministic Merkle root over raw digests, atomic JSON writes
- **Transfer** (`crates/p2p/transfer.rs`): per-chunk verification, `.part` staging +
  `.done` resume bitmap, full-file hash + Merkle gate, atomic rename; single-peer
  (`download`) or ranked multi-provider waves (`download_multi`); corrupted
  artifacts are quarantined with metadata
- **Reputation** (`crates/p2p/reputation.rs`): only cryptographic failures count
  toward bans; scores persist atomically; deterministic ranking (score desc,
  PeerId asc) feeds the scheduler; serializable summaries feed the dashboard
- **Runtime** (`crates/runtime`): llama-server as a managed subprocess (never FFI),
  health-probed, killed on drop; thin axum proxy with Bearer auth and the
  live dashboard; token at `runtime/api.token` (0600)
- **Audit** (`crates/audit`): append-only `logs/audit.jsonl` for security events

## Layout

- `crates/audit` — append-only security audit log
- `crates/config` — typed YAML configuration with validation
- `crates/identity` — Ed25519 keypairs and PeerId derivation
- `crates/manifest` — GGUF manifests: chunk hashes, Merkle root, atomic writes
- `crates/protocol` — swarm message schemas (incl. catalog) and canonical signing
- `crates/p2p` — libp2p transport actor, verified transfer, reputation, scheduler
- `crates/registry` — local model registry with path safety
- `crates/runtime` — llama-server manager, admission gate, OpenAI API + dashboard
- `crates/system-probe` — hardware probing and admission decisions
- `crates/node-cli` — the `decentraai` binary
- `action-plan.md`, `ROADMAP.md`, `docs/` — design and handoff documents

## Security baseline

No artifact is usable before hash, manifest, and policy verification. Manifest and
chunk responses carry no signatures by design: integrity is anchored in the signed
manifest's `chunk_hashes` and Merkle root, enforced by per-chunk BLAKE3
verification at assembly. Prompts and outputs are never logged by default.
Private keys and API tokens never enter Git or telemetry. The dashboard never
exposes secrets; the API token guards every `/v1/*` endpoint.
