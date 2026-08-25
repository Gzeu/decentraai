# DecentraAI

<p align="center">
  <strong>Autonomous Distributed AI Compute Fabric</strong><br>
  <sub>AI models, agents, memory, shared compute, cryptographic evidence and verifiable economics — working as one fabric.</sub>
</p>

<p align="center">
  <a href="https://github.com/Gzeu/decentraai/actions"><img src="https://img.shields.io/badge/CI-green-brightgreen" alt="CI"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.85%2B-orange" alt="Rust"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="License"></a>
  <img src="https://img.shields.io/badge/3--node%20fabric-live-00c853" alt="3 node fabric live">
  <img src="https://img.shields.io/badge/100k%20embeddings-42.1x%20speedup-7c4dff" alt="100k embeddings speedup">
  <img src="https://img.shields.io/badge/1386%20tests-green" alt="1386 tests">
</p>

> **DecentraAI is not just an AI endpoint. It is a cooperative compute fabric.** A Governor observes real resource pressure, selects the right model, borrows verified capacity from trusted peers, executes work across the fabric, records evidence, and credits the contributors.

> 📘 **Full product documentation: [`docs/PRODUCT.md`](docs/PRODUCT.md)** — what DecentraAI is, how the Compute Fabric / Model Colony / Governor / EvidenceChain / Economy work together, validated live results (100k embeddings @ 42.1×, autonomous pressure trigger, BYOA gateway, shard-failure recovery), security boundaries and operations guide.

---

## ⚡ What DecentraAI is

DecentraAI turns independent machines into a **coordinated AI compute fabric**.

```text
                        ┌──────────────────────┐
                        │      GOVERNOR        │
                        │  observe → decide    │
                        └──────────┬───────────┘
                                   │
                          Model Intelligence
                                   │
                         ┌─────────▼─────────┐
                         │    MODEL COLONY    │
                         │ capability + RAM  │
                         │ evidence + latency │
                         └─────────┬─────────┘
                                   │
                    LOCAL / DISTRIBUTED / QUEUE / REJECT
                                   │
                   ┌───────────────▼───────────────┐
                   │       SHARED CPU POOL         │
                   │     Sharing is Caring + DFCP  │
                   └───────┬────────┬────────┬─────┘
                           │        │        │
                        VPS      Desktop    Laptop
                           │        │        │
                           └────────┼────────┘
                                    ▼
                              map → reduce
                                    │
                                    ▼
                              EvidenceChain
                                    │
                         ┌──────────┴──────────┐
                         ▼                     ▼
                   Contribution           Memory
                         │                     │
                         ▼                     ▼
                   RewardEngine          Training Lab
                         │
                         ▼
                    MultiversX
```

### The governing invariant

**AI proposes → deterministic Rust decides → workers execute.**

Models are advisory. Policy, reservations, trust, evidence and economic accounting remain deterministic.

---

## 🔥 Proven on real hardware

The fabric has moved beyond architecture diagrams. The following capabilities have been demonstrated on the live 3-node setup:

| Capability | Verified result |
|---|---|
| **Sharing is Caring / CPU Pool** | Real remote CPU execution across VPS, Desktop and Laptop |
| **Benchmark distribution** | **1.81× speedup** |
| **Embeddings pool** | **100,000 embeddings · 0 failures · 42.1× speedup** |
| **Chat batch** | **4.3× speedup**, 3-node execution |
| **Distributed map/reduce** | **2.26× speedup**, one logical workload, 5 shards |
| **Autonomous Governor** | Real LOCAL / DISTRIBUTED / QUEUE / REJECT decision path |
| **Model Colony** | Capability + RAM + measured evidence based selection |
| **Evidence / economy** | Remote verified execution → contribution credit |
| **MultiversX MX-8004** | DecentraGovernor registered on Testnet as a soulbound agent identity |
| **Token/economic foundation** | Integer-only Contribution Units, RewardEngine, anti-gaming and cryptographic evidence |

> These figures are **observed results from the current test environment**, not universal performance guarantees. Network conditions, model choice, hardware and workload shape the result.

---

## 🧠 How the fabric works

### 1. Governor

The Governor is the operational brain. It does not get direct authority to mutate system state.

It can:

- observe real CPU/RAM/queue/latency pressure;
- ask Model Intelligence which model is appropriate;
- decide whether execution fits locally;
- request shared compute when local capacity is insufficient;
- trigger distributed map/reduce execution;
- record why the decision was made.

### 2. Model Colony

Multiple models can coexist and compete on evidence instead of claims.

Current candidates include:

- Qwen3 1.7B Q4
- Gemma 3 1B Q4
- Phi-4-mini Q4

Selection considers capability, RAM fit and measured performance evidence. A model is not promoted merely because it is preferred by an LLM.

### 3. Sharing is Caring + DFCP

Trusted workers advertise capacity, negotiate reservations and execute through the existing DFCP path.

```text
RESOURCE_REQUEST
      ↓
RESOURCE_OFFER
      ↓
RESOURCE_RESERVE
      ↓
ASSIGN
      ↓
RESULT
      ↓
RELEASE
```

This is not a second scheduler. The CPU Pool builds on the same deterministic fabric primitives.

### 4. Distributed execution

The current backend does **not** provide tensor/pipeline parallelism for a single llama.cpp forward pass across separate machines. DecentraAI therefore uses a real, measurable map/reduce strategy for context-split workloads and batch parallelism.

```text
logical workload
      ↓
 deterministic shards
      ↓
  parallel workers
      ↓
 partial results
      ↓
 validated reduce
      ↓
 one final result
```

### 5. Evidence and economics

Every verified remote contribution can flow through:

```text
execution
  ↓
EvidenceChain
  ↓
SignedComputeReceipt / EconomicEvidence
  ↓
ContributionUnit V2
  ↓
RewardEngine
  ↓
CompensationLedger
```

The economic layer does not let the Governor mint rewards arbitrarily. Unverified, replayed, self-verified or otherwise invalid work does not receive normal credit.

---

## 🌐 MultiversX integration

DecentraAI has a real MultiversX integration path rather than a blockchain placeholder.

The current stack includes:

- MX-8004 agent identity preparation and registration;
- Ed25519 identity binding;
- verified Devnet/Testnet registry discovery;
- deterministic transaction-data builders;
- `BlockchainAdapter` abstraction;
- proof/validation/reputation integration groundwork;
- an economic model designed for future settlement.

**Current scope is controlled integration and testnet/devnet work. Mainnet issuance and public-token launch are not presented as completed functionality.**

---

## 🧩 Agent OS, MCP and collective intelligence

DecentraAI includes an Agent Operating System for specialized roles and external agents.

Core concepts:

- Governor
- QA / Security
- Concierge
- Memory Keeper
- Fabric / Rust / API engineers
- VPS operator
- Researcher
- skills + policies + RBAC
- MCP and OpenAI-compatible gateway surfaces
- private agent memory + shared collective memory

The system is designed so that **tools are capabilities, not authority**.

---

## 🧠 Collective Memory & learning loop

The fabric maintains agent-scoped and collective memory with provenance and evidence.

A verified execution can become:

```text
verified execution
      ↓
collective memory
      ↓
learning candidate
      ↓
Training Lab
      ↓
model experiment
      ↓
benchmark
      ↓
shadow candidate
```

A model is never allowed to silently retrain or promote itself in production.

---

## 🧪 Training Lab

The repository contains an end-to-end training pipeline for adapting a base model with LoRA/QLoRA-style workflows.

The first smoke test proved the complete path:

```text
corpus → LoRA training → adapter → evaluation
```

The long-term target is a DecentraAI-specific model trained from **verified experiences**, not raw uncontrolled logs.

---

## 🛡️ Security model

Security is part of the architecture, not an afterthought.

- Ed25519 node/agent identity
- BLAKE3 / Merkle verification for model artifacts and evidence
- scoped `dca_` consumer credentials
- RBAC for agents
- explicit trust/admission flow
- append-only audit paths
- anti-replay / anti-Sybil / self-verification defenses
- secrets kept outside repository and agent memory
- deterministic policy gates around execution and economics

### Hard invariant

```text
AI output = untrusted input

AI proposes
    ↓
Policy validates
    ↓
Rust decides
    ↓
Worker executes
    ↓
Evidence verifies
    ↓
Economy credits
```

---

## 🚀 Quick start

### One-command setup

```bash
decentraai setup
decentraai node start --config ~/.decentraai/node.yaml
```

The setup flow probes hardware, creates the node identity, selects a suitable local model and writes validated configuration.

### Dashboard

```bash
decentraai open
```

Default local dashboard:

```text
http://127.0.0.1:8080
```

### Direct chat API

```bash
TOKEN=$(cat ~/.decentraai/runtime/api.token)

curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "model":"qwen2.5-coder-3b-instruct",
    "messages":[{"role":"user","content":"Explain BLAKE3"}],
    "stream":true
  }'
```

---

## 🖥️ CLI

The CLI covers node lifecycle, models, P2P, workers, distributed execution, trust, agents, RAG, memory and gateway credentials.

```text
init
setup
doctor
config
registry
model
swarm
serve
pull
worker
distributed
trust
tier
consumer-key
agent
node
rag
memory
open
invite
join
```

Run:

```bash
decentraai --help
```

---

## 🏗️ Repository map

| Area | Purpose |
|---|---|
| `crates/agents` | Agent OS, Governor logic, workflows and orchestration |
| `crates/distributed` | P2P bindings, CPU Pool, model-parallel primitives |
| `crates/fabric` | deterministic planning, reservations and placement |
| `crates/p2p` | libp2p transport and peer connectivity |
| `crates/compute` | compute/resource abstractions |
| `crates/runtime` | node daemon, API and model runtime integration |
| `crates/decentraai-economy` | Contribution Units, rewards, economic evidence and settlement abstraction |
| `.agents/` | skills, policies and agent contracts |
| `docs/` | architecture, security, memory, training and MultiversX research |

---

## ✅ Quality gates

The project currently maintains a green Rust workspace gate with:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Current baseline: **1386 tests passing** in the reported live state.

---

## 🗺️ What comes next

The next major frontier is **true model parallelism across machines**: a backend capable of splitting the actual model execution itself across nodes, rather than the current context-split map/reduce strategy.

Other active engineering work includes:

- finishing operational edge cases around cross-network P2P;
- increasing real model deployment coverage on workers;
- deeper resource/pressure feedback into routing;
- extending the Model Colony from profiles to continuously refreshed measured evidence;
- progressing the MultiversX settlement path in a controlled testnet-first manner.

---

## 📚 Documentation

Start here:

- [`AGENTS.md`](AGENTS.md) — Agent Operating Contract
- [`docs/AGENT_ORGANIZATION.md`](docs/AGENT_ORGANIZATION.md) — Agent OS roles and RBAC
- [`docs/AGENT_MEMORY.md`](docs/AGENT_MEMORY.md) — Obsidian / collective memory model
- [`docs/MULTIVERSX_MX8004_WRITE_PATH.md`](docs/MULTIVERSX_MX8004_WRITE_PATH.md) — MX-8004 protocol research
- [`docs/MULTIVERSX_DEVNET_ADDRESSES.md`](docs/MULTIVERSX_DEVNET_ADDRESSES.md) — verified network addresses and discovery notes
- [`docs/MODEL_TRAINING.md`](docs/MODEL_TRAINING.md) — training pipeline

---

## ⚖️ License

Apache-2.0. See [`LICENSE`](LICENSE).

---

<p align="center">
  <strong>DecentraAI</strong><br>
  <sub>Observe. Decide. Borrow compute. Execute. Verify. Learn.</sub>
</p>
