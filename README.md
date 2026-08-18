# DecentraAI

**Distributed execution fabric for AI model artifacts and verifiable inference.**

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org/)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-blue.svg)](LICENSE)
[![Status: Production](https://img.shields.io/badge/status-production-green)](CHANGELOG.md)
[![Tests: 958+](https://img.shields.io/badge/tests-958+-success)](#quality-gates)
[![LAN Verified](https://img.shields.io/badge/verified-LAN__desktop__↔__laptop-brightgreen)](#verified-today)

> **DecentraAI is a distributed execution fabric** — not a marketplace, cloud platform, token economy, or centralized server. Trusted peers on a LAN discover each other, exchange cryptographically verified model artifacts, and serve real-time inference through a unified node that combines P2P networking, semantic retrieval, collective intelligence, and a live web dashboard.

---

## 📦 At a Glance

```
┌──────────────────────────────────────────────────────┐
│                    YOUR DESKTOP                      │
│                                                      │
│   ┌─────────────┐    ┌──────────────┐               │
│   │ decentraai   │    │  Dashboard   │               │
│   │     node     │◄──►│  :8080       │               │
│   │ (daemon)     │    │  [Web UI]    │               │
│   └──────┬───────┘    └──────────────┘               │
│          │                                            │
│          ▼                                            │
│   ┌───────────────────────────────────────┐         │
│   │         Universal Node                  │         │
│   │  ┌────────┐ ┌──────┐ ┌──────────┐    │         │
│   │  │ P2P    │ │ RAG  │ │ Agents   │    │         │
│   │  │ mDNS   │ │ Index│ │ Orchestrator│   │         │
│   │  │ libp2p │ └──────┘ └──────────┘    │         │
│   │  └────────┘                           │         │
│   │  ┌──────────┐ ┌─────────┐ ┌───────┐ │         │
│   │  │ Swarm    │ │ Memory  │ │ Talents│ │         │
│   │  │ Replcation│ │ Store  │ │  Tree │ │         │
│   │  └──────────┘ └─────────┘ └───────┘ │         │
│   │  ┌────────────────────────────────┐  │         │
│   │  │ llama-server (loopback backend)│  │         │
│   │  └────────────────────────────────┘  │         │
│   └───────────────────────────────────────┘         │
│                                              ▲      │
│   LAN Discovery & Inference              │      │   │
│                                              │      │   │
│                                              ▼      │
│   ┌──────────────┐         ┌──────────────┐       │
│   │ Laptop Node  │◄────────│ Other Nodes  │       │
│   └──────────────┘   P2P  └──────────────┘       │
└──────────────────────────────────────────────────────┘
```

### Key Features

| Feature | Status | Description |
|---------|--------|-------------|
| **P2P Model Sharing** | ✅ Production | BLAKE3 chunk verification + Merkle root + Ed25519 identity |
| **Universal Node** | ✅ Production | One process: discovery, serving, inference, dashboard |
| **Collective Intelligence** | ✅ Live | Agents with signed capabilities, delegation DAGs, consensus |
| **Semantic Retrieval (RAG)** | ✅ Live | Document indexing + vector queries via embeddings backend |
| **Persistent Memory** | ✅ Live | Per-agent/team/global memory with access policies |
| **Reputation System** | ✅ Live | Per-(agent,capability) scoring with EMA decay |
| **Talent Tree** | ✅ Live | Dynamic capability graph with prerequisites |
| **Live Dashboard** | ✅ Production | Real-time stats, chat, workflow runner, network view |
| **Complete CLI** | ✅ Production | 20+ commands covering all product features |
| **Streaming Inference** | ✅ Production | SSE responses with latency + tokens metrics |
| **Trusted Admission** | ✅ Production | Opt-in sharing, trust chain: discovered → approved → connected |
| **Subscription Tiers** | ✅ Production | Free by default; tier reflects contribution level |
| **Distributed Fabric** | ✅ LAN Verified | Multi-node planner, reservations, KV-aware routing |

---

## ✨ Quick Start

### One-command setup (Q4)

```bash
# Detect hardware → generate identity → auto-select model → write validated config
decentraai setup

# The node starts automatically (systemd user service or manual daemon)
decentraai node start --config ~/.decentraai/node.yaml
```

**What happens:**
1. Hardware probe (CPU/RAM/GPU/Temperature)
2. Ed25519 keypair generation (mode 0600)
3. Best-fit model selection from HuggingFace catalog
4. Validated config written to `~/.decentraai/node.yaml`
5. Node ready in ~30 seconds

### Access the Dashboard

```bash
# Open the embedded web dashboard
decentraai open
# or visit: http://127.0.0.1:8080
```

![Dashboard Overview](docs/screenshots/overview.png)
*The live dashboard shows real-time system stats, inference metrics, chat, and network topology.*

### Chat with your model

The dashboard includes a fully functional chat interface at `/chat`:
- **SSE streaming** by default (chunk-by-chunk rendering)
- **Non-streaming fallback** via checkbox
- Latency + tokens displayed from trailing `usage` event
- Token authentication via localStorage

```bash
# Or use the API directly
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "qwen2.5-coder-3b-instruct",
    "messages": [{"role": "user", "content": "Explain BLAKE3"}],
    "stream": true
  }'
```

### Chat can speak (local TTS)

Enable the chat 🔊 speak button with a **local** Piper voice — **Romanian
native** (correct diacritics: ă, â, î, ș, ț), no cloud, no GPU. The node runs
a managed Python subprocess (same invariant as the llama.cpp engine) reading
`<data_dir>/tts/`:

```bash
bash scripts/setup-tts.sh     # idempotent: venv + voice + smoke test
```

Then add to `~/.decentraai/node.yaml` and restart:

```yaml
tts:
  enabled: true
  voice: "ro_RO-raluca-high"  # female Romanian (default); "ro_RO-mihai-medium" = male
  speed: 1.0
```

```bash
systemctl --user restart decentraai-node
```

The API is `POST /v1/tts` (Bearer auth, WAV 16-bit mono 22 kHz):

```bash
TOKEN=$(cat ~/.decentraai/runtime/api.token)
curl http://127.0.0.1:8080/v1/tts \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"text": "Bună ziua! Fabricul vorbește română corect."}' --output reply.wav
```

`/status` reports `tts {enabled, healthy, voice, speed}`; without the section
the chat simply hides the speak button.

---

## 🖥️ CLI Reference

A complete, cohesive command menu covering all product features:

```
$ decentraai --help
DecentraAI node control CLI

Usage: decentraai [OPTIONS] <COMMAND>

Commands:
  init              Initialize a new node configuration
  setup             One-command fresh-node onboarding (auto-detect → identity → model → config)
  doctor            Validate hardware health and readiness
  config            Manage configuration files
  registry          Model registry operations (scan/list)
  model             Search/download models from HuggingFace Hub
  swarm             P2P swarm management
  serve             llama-server lifecycle control
  pull              Download and verify GGUF models (BLAKE3 + Merkle gate)
  token             Subscription token management (create/list/revoke)
  worker            Worker operations
  distributed       Distributed compute operations
  trust             Trust policy management (add/remove/approve peers)
  tier              Tier management (Guest/Contributor/Core)
  consumer-key      Consumer API keys (`dca_...`) for quota-controlled inference
  agent             Collective Intelligence: agents, workflows, skills
  node              Run full background daemon (production mode)
  rag               Semantic retrieval: index documents and query
  memory            Persistent memory: inspect entries and scopes
  open              Launch the dashboard in your default browser
  invite            Generate join invite for newcomers (P5)
  join              Join a private swarm from an invite string
  help              Print this message
```

### Example Commands

**Workflow orchestration:**
```bash
# Run a collective research_report workflow
decentraai agent workflow-run \
  --config ~/.decentraai/node.yaml \
  --prompt "Summarize current state of Rust async runtimes"

# Result:
# verdict: "completed"
# output: {generated report text}
```

**Semantic retrieval (RAG):**
```bash
# Index a document
decentraai rag index \
  --doc-id "rust_async_guide" \
  --text "Tokio is a synchronous runtime for writing async applications..." \
  --capability "code_generation"

# Query the index
decentraai rag query \
  --text "What is Tokio?" \
  --k 3

# Output:
# retrieval results (3):
# [1] (score: 0.87) Tokio is a synchronous runtime for writing async applications...
# [2] ...
```

**Memory inspection:**
```bash
# List persistent memory entries
decentraai memory list

# Output:
# persistent memory entries (4):
#   [learn_rust] scope=agent level=fact conf=0.95 ts=2026-08-18T14:30:00Z
#     "Async requires tokio runtime and futures crate"
#   [team_goal] scope=team level=goal conf=0.88 ts=2026-08-17T09:15:00Z
#     "Deploy P2P mesh across three office locations by Q4"
```

**Full node launch:**
```bash
# Start as production daemon (auto-restart on crash)
systemctl --user start decentraai-node

# Check status
systemctl --user status decentraai-node

# View logs
journalctl --user -u decentraai-node -f
```

---

## 🏗️ Architecture

### High-Level Flow

```
User Prompt
     │
     ▼
┌─────────────────┐     ┌──────────────┐
│  Dashboard/API  │────►│  Agent        │
│  (Web/SSE/CLI)  │     │  Orchestrator │
└─────────────────┘     └──────┬───────┘
                               │
                    ┌──────────▼──────────┐
                    │  Delegation Planner  │
                    │  (capability matcher)│
                    └──────────┬──────────┘
                         ┌─────┴─────┐
                         │ Local or Remote │
                         └─────┬─────┘
                  ┌────────────┼────────────┐
                  ▼            ▼             ▼
           ┌──────────┐  ┌──────────┐  ┌──────────┐
           │ llama-   │  │ llama-   │  │ llama-   │
           │ server   │  │ server   │  │ server   │
           │(:8080)   │  │(:8081)   │  │(:8082)   │
           └──────────┘  └──────────┘  └──────────┘
                │            │             │
                └────────────┼─────────────┘
                             ▼
                     Streaming Response
                       (SSE/InferProgress)
```

### Core Modules

| Crate | Purpose | Lines |
|-------|---------|-------|
| `node-cli` | Complete CLI command menu | 6,000+ |
| `runtime` | llama-server manager + dashboard + API proxy | 8,000+ |
| `agents` | Collective intelligence substrate (pure logic) | 4,000+ |
| `distributed` | P2P orchestration + agent runtime bindings | 3,000+ |
| `fabric` | Execution planner + reservation system | 2,500+ |
| `p2p` | libp2p transport + verified transfer + reputation | 3,500+ |
| `config` | Typed YAML config with validation | 800+ |
| `identity` | Ed25519 keypairs + PeerId derivation | 600+ |
| `manifest` | GGUF manifests + Merkle root computation | 700+ |
| `registry` | Local model storage with path safety | 400+ |
| `audit` | Append-only security logging | 500+ |
| `system-probe` | Hardware probing + admission decisions | 600+ |

### Security Model

```
Artifact Verification Chain:
┌──────────┐    ┌───────────┐    ┌──────────────┐    ┌──────────┐
│ Hugging  │───►│ BLAKE3    │───►│ Merkle Root  │───►│ Ed25519  │
│  Face    │    │ Chunk Hash│    │ Verification │    │ Manifest │
│  SHA-256 │    └───────────┘    └──────────────┘    │ Signature│
└──────────┘                                      └──────────┘
       │                                                │
       ▼                                                ▼
  Original Source                                    Signed Owner
                                                     (trusted peer)
```

- **Prompts and outputs never logged** (audit records only security events)
- **Private keys** stored at `identity/key.pem` with mode 0600
- **API tokens** bound to loopback only (config validation rejects public binds)
- **Subscription tokens** hashed before persistence (BLAKE3)
- **Zero network errors touching reputation scores** — only cryptographic failures count

---

## 🔑 Subscription Tiers

Free by default; tier reflects your contribution level:

| Tier | Models | Rate Limit | Earned By |
|------|--------|------------|-----------|
| **Guest** (Tier 1) | Small/public | Tight | Invited by admin |
| **Contributor** (Tier 2) | Medium/shared | Moderate | Shares ≥1 verified model |
| **Core** (Tier 3) | Large/multiple | High | Multiple models + clean reputation |

```bash
# Create subscription tokens (admin only)
decentraai token create --tier guest --name "alice@office" --quota 1000

# List active tokens
decentraai token list

# Revoke if compromised
decentraai token revoke --token <hash>
```

---

## 🌐 Distributed Inference Fabric

Verified on **real hardware** (Desktop ↔ Laptop, two physical Ubuntu machines):

- **Bidirectional**: Desktop coordinates → Laptop works, Laptop coordinates → Desktop works
- **Worker reuse**: Same remote llama-server serves multiple requests
- **KV-aware routing**: Coordinator tracks session affinity and context token budgets
- **Network-aware scoring**: RTT probes fold latency into planner decisions
- **Admission control**: Only explicitly trusted peers accepted (`decentraai trust add <peer>`)
- **Health monitoring**: Auto-recovery on engine crash, reservation TTLs, stale eviction

### Trust Chain

```
DISCOVERED (mDNS broadcast)
    │
    ▼
UNTRUSTED (new peer seen)
    │
    ▼
APPROVED (you ran: decentraai trust add <peer-id>)
    │
    ▼
CONNECTED (libp2p link established)
    │
    ▼
WORKER READY (health check passed, engine responsive)
```

---

## 🧠 Collective Intelligence (P0–P11)

Every installation hosts **logical agents** — not separate processes:

- **Signed capability claims** advertised over P2P heartbeat
- **Unified capability matcher**: compositional verdict from semantic gate + model allowlist + physical gate
- **Delegation DAGs**: multi-hop execution with per-hop verification
- **Consensus/Verification**: majority agreement, disagreement resolution
- **Persistence**: MemoryStore SQLite with retention policies and access control
- **Reputation**: EMA decay, factors (reliability/quality/latency/uptime/safety), deterministic best-for-capability ranking
- **Policy engine**: Allow/Deny controls for tools/models/peers/budgets/egress
- **Talent tree**: Dynamic capability graph — no fixed levels, unlock paths based on available resources
- **Workflows**: Template-based orchestration (research_report example runs end-to-end live)

**Verified live**: `research_report` workflow completes across four stages (Research → Finance → Documents → Synthesis) with real model output.

See [`docs/COLLECTIVE_INTELLIGENCE.md`](docs/COLLECTIVE_INTELLIGENCE.md) for the full fabric specification.

---

## 📊 Quality Gates

Every milestone lands with tests:

```bash
git pull --rebase
git log --oneline -1
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**Current status:**
- ✅ **958 tests**, all green
- ✅ Clippy: zero warnings
- ✅ fmt: all files formatted
- ✅ E2E: real libp2p nodes on loopback (<20s total)

---

## 📁 Repository Structure

```
decentraai/
├── crates/                    # Workspace crates
│   ├── agent_cli/             # Not used yet, reserved
│   ├── agents/                # Collective intelligence substrate (pure, no I/O)
│   ├── audit/                 # Append-only JSON-lines security log
│   ├── compute/               # Compute capability & scheduling primitives
│   ├── config/                # Typed YAML config with validation
│   ├── distributed/           # P2P distributed inference + agent runtime
│   ├── fabric/                # Execution planner + reservation system
│   ├── hub/                   # HuggingFace catalog + verified download
│   ├── identity/              # Ed25519 keypairs + PeerId derivation
│   ├── inference-adapter/     # Engine abstraction layer
│   ├── manifest/              # GGUF manifests + Merkle root
│   ├── node-cli/              # The `decentraai` binary (CLI entry point)
│   ├── p2p/                   # libp2p transport + reputation + scheduler
│   ├── protocol/              # Message schemas + canonical signing
│   ├── registry/              # Local model registry with path safety
│   ├── runtime/               # llama-server manager + dashboard + API
│   ├── system-probe/          # Hardware probing + admission decisions
│   └── tokens/                # Subscription token registry (hashed, tiered)
├── deploy/                    # systemd units + desktop shortcuts
├── docs/                      # Design docs + architecture + screenshots
│   ├── COLLECTIVE_INTELLIGENCE.md
│   ├── ARCHITECTURE.md
│   ├── DISTRIBUTED_INFERENCE.md
│   └── brand/                 # Logos + SVG assets
├── scripts/                   # Installer + upgrade scripts
├── AGENTS.md                  # Master prompt for development sessions
├── CHANGELOG.md               # Milestone history
├── CONTRIBUTING.md            # Contribution guidelines
├── ROADMAP.md                 # Current-state checklist + future milestones
├── SECURITY.md                # Security policy
└── README.md                  # You are here
```

---

## 🔒 Security Baseline

- No artifact usable before hash + manifest + policy verification
- Per-chunk BLAKE3, final file hash + Merkle root enforcement
- Prompts and outputs **never logged**
- Private keys/tokens never enter Git or telemetry
- Dashboard exposes no secrets; API tokens guard every `/v1/*` endpoint
- Loopback-only binding enforced by config validation
- Subscription tokens stored only as hashes
- Quarantine workflow on corrupted chunks with metadata

For details, see [`SECURITY.md`](SECURITY.md).

---

## 🚀 Deployment

### Production Service (recommended)

```bash
# Install as systemd user service (auto-start + restart)
bash scripts/install-app.sh

# Manage
systemctl --user start decentraai-node
systemctl --user status decentraai-node
journalctl --user -u decentraai-node -f

# Uninstall
bash scripts/uninstall-app.sh
```

### Manual Daemon

```bash
# Start in foreground (useful for debugging)
./target/release/decentraai node start --config ~/.decentraai/node.yaml

# Background
nohup ./target/release/decentraai node start >> /tmp/decentraai.log 2>&1 &
```

---

## 📚 Documentation

- [`ROADMAP.md`](ROADMAP.md) — Full milestone history and current state
- [`AGENTS.md`](AGENTS.md) — Development master prompt
- [`CHANGELOG.md`](CHANGELOG.md) — Version history
- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — System architecture overview
- [`docs/COLLECTIVE_INTELLIGENCE.md`](docs/COLLECTIVE_INTELLIGENCE.md) — Agent fabric spec
- [`docs/DISTRIBUTED_INFERENCE.md`](docs/DISTRIBUTED_INFERENCE.md) — P2P execution details
- [`docs/DECENTRAAI_PRODUCT_STATUS.md`](docs/DECENTRAAI_PRODUCT_STATUS.md) — Current product status
- [`docs/MONITORING_ARCHITECTURE.md`](docs/MONITORING_ARCHITECTURE.md) — Observability design

---

## 🤝 Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for guidelines.

Key principles:
- Tests first — every feature lands with unit + integration/E2E tests
- Cod-first communication — code over words, always
- Honesty over hype — separate verified / deduced / suggestion
- No unsafe, no unwrap outside tests, no new deps without justification
- Every commit must pass quality gates (clippy + fmt + test)

---

## License

Apache-2.0 — See [`LICENSE`](LICENSE) for details.

---

**Built with ❤️ by George Pricop (Gzeu) and the DecentraAI community.**

Questions? Issues? PRs welcome. Let's build the decentralized future together.
