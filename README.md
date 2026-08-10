# DecentraAI

Decentralized P2P distribution of AI model artifacts and verifiable local or remote inference.
Nodes discover each other on the LAN, exchange cryptographically verified model chunks,
and serve inference through a local OpenAI-compatible endpoint.

## What exists today

| Milestone | Scope | Status |
|---|---|---|
| M0-M1 | Rust workspace, CI quality gates, system probe + admission checks, Ed25519 identity | Done |
| M2 | Local GGUF registry with path safety and deterministic persistence | Done |
| M3 | LAN swarm: signed protocol, Merkle manifests, libp2p transport, verified chunk transfer with resume, E2E tests | Done |
| M4 | Inference runtime: llama-server manager, admission gate, OpenAI-compatible API with auth and idle unload | Done |
| M5 | Swarm intelligence: peer reputation with bans, signed manifest announcements, deterministic multi-provider scheduler | Done |
| M6 | Hardening: quarantine workflow, audit logging, packaging | Planned |

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

On the machine that **wants** a model: peers discover each other via mDNS;
transfers are verified per chunk (BLAKE3) with a final Merkle-root check,
resumable after interruption, and recorded in the local reputation store
(`db/reputation.json`). Peers that serve corrupted chunks are temporarily
banned (`security.max_invalid_chunks_per_peer` / `ban_duration_minutes`).

### 3. Run inference with an OpenAI-compatible API

```bash
decentraai serve start --model tinyllama.gguf
# prints: API: http://127.0.0.1:8080/v1 (fixed port from inference.api_port)
#         Auth: Bearer token: ~/.decentraai/runtime/api.token
```

The gate before every load: config mode (`inference.enabled`), live RAM/GPU
budgets, and GPU temperature from the system probe. The model unloads
automatically after `idle_model_unload_minutes` without requests.

```bash
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl http://127.0.0.1:8080/v1/models -H "Authorization: Bearer $TOKEN"
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model":"tinyllama","messages":[{"role":"user","content":"Hello"}],"max_tokens":20}'
```

Browsers can open `http://127.0.0.1:8080/` for the info page.

### Current limitations

- GGUF models only (matches the verification capability)
- Transfers are peer-explicit; automatic peer catalogs land with the
  scheduler evolution
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
decentraai serve start --model <name>           # gated inference + OpenAI API :8080
```

## Architecture highlights

- **Identity**: Ed25519 keypair at `<data_dir>/identity/key.pem` (0600); the libp2p
  keypair is derived from it, binding transport PeerIds to node keys
- **Protocol** (`crates/protocol`): `deny_unknown_fields`, size caps, base64 binary
  fields, canonical signing (`sign_manifest` / `verify_manifest_signature`)
- **Manifests** (`crates/manifest`): GGUF magic validation, 4 MiB chunks, BLAKE3,
  deterministic Merkle root over raw digests, atomic JSON writes
- **Transfer** (`crates/p2p/transfer.rs`): per-chunk verification, `.part` staging +
  `.done` resume bitmap, full-file hash + Merkle gate, atomic rename; single-peer
  (`download`) or ranked multi-provider waves (`download_multi`)
- **Reputation** (`crates/p2p/reputation.rs`): only cryptographic failures count
  toward bans; scores persist atomically; deterministic ranking (score desc,
  PeerId asc) feeds the scheduler
- **Runtime** (`crates/runtime`): llama-server as a managed subprocess (never FFI),
  health-probed, killed on drop; thin axum proxy with Bearer auth; token at
  `runtime/api.token` (0600)

## Layout

- `crates/config` — typed YAML configuration with validation
- `crates/identity` — Ed25519 keypairs and PeerId derivation
- `crates/manifest` — GGUF manifests: chunk hashes, Merkle root, atomic writes
- `crates/protocol` — swarm message schemas and canonical signing
- `crates/p2p` — libp2p transport actor, verified transfer, reputation, scheduler
- `crates/registry` — local model registry with path safety
- `crates/runtime` — llama-server manager, admission gate, OpenAI API proxy
- `crates/system-probe` — hardware probing and admission decisions
- `crates/node-cli` — the `decentraai` binary
- `action-plan.md`, `ROADMAP.md`, `docs/` — design and handoff documents

## Security baseline

No artifact is usable before hash, manifest, and policy verification. Manifest and
chunk responses carry no signatures by design: integrity is anchored in the signed
manifest's `chunk_hashes` and Merkle root, enforced by per-chunk BLAKE3
verification at assembly. Prompts and outputs are never logged by default.
Private keys and API tokens never enter Git or telemetry.
