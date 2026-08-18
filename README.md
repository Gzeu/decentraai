<div align="center">

<img src="docs/assets/decentraai-mark.svg" alt="DecentraAI" width="520">

# DecentraAI

### Distributed AI. Shared capabilities. Collective execution.

**A Rust-based AI execution fabric for models, agents, datasets, skills, capabilities and distributed inference.**

<p>
<a href="https://github.com/Gzeu/decentraai"><img src="https://img.shields.io/badge/project-active%20development-22d3ee?style=for-the-badge" alt="Active development"></a>
<a href="https://github.com/Gzeu/decentraai/commits/main"><img src="https://img.shields.io/github/last-commit/Gzeu/decentraai?style=for-the-badge" alt="Last commit"></a>
</p>

</div>

---

> **DecentraAI is not another chatbot.**
>
> It is infrastructure for turning heterogeneous machines, local models, datasets and agents into a cooperative AI fabric.

## The idea

DecentraAI starts with a simple premise:

**machines should not merely share compute — they should share capabilities.**

A worker contributes hardware and models.  
A dataset contributes evidence.  
A skill applies that evidence.  
A capability becomes available.  
The Talent Tree organizes what can be composed.  
Agents use those capabilities.  
The fabric decides where execution should happen.

<img src="docs/assets/command-deck.png" alt="DecentraAI Command Deck" width="100%">

---

## Architecture

<img src="docs/assets/capability-fabric.svg" alt="DecentraAI capability fabric" width="100%">

The architectural chain is:

```text
Hardware
    ↓
Models + Tools
    ↓
Datasets
    ↓
Skills
    ↓
Capabilities
    ↓
Talent Tree
    ↓
Agent Powers
    ↓
Distributed Execution
```

### Trust model

DecentraAI deliberately separates **evidence, capability and authority**.

- **Power ≠ Permission**
- **Dataset existence ≠ capability proof**
- **Skill existence ≠ capability proof**
- Provenance must survive capability construction
- Verified evidence must not be silently downgraded
- Runtime selection must respect actual worker capabilities

---

# 🌐 Distributed Fabric

DecentraAI is built around a peer-to-peer execution fabric.

```text
User / Agent Request
        ↓
Planner → Scheduler / Router
        ↓
DecentraAI Fabric
   ↙        ↓        ↘
Laptop   Desktop   Future Worker
   ↓        ↓          ↓
Local    Local      Local Model
Model    Model
   └────────┴──────────┘
             ↓
          Inference
```

Workers advertise the resources and models they can actually provide. The fabric can then route execution according to the capabilities available on the network.

This makes the project fundamentally different from a single-node chatbot architecture:

```text
single machine → one model → one agent
```

becomes:

```text
many machines → many models → many capabilities → coordinated agents → shared execution fabric
```

---

# 🤖 Agents

Agents are not treated as isolated prompt wrappers.

The runtime is designed around:

- model selection
- capability matching
- tools
- memory
- reputation
- resource constraints
- distributed execution
- recovery
- provenance

The goal is for an agent to be able to answer:

> **What can I actually do, why do I believe I can do it, and where should the work execute?**

---

# 🧩 Dataset → Skill → Capability

The Dataset/Skill system is the bridge between model/data artifacts and agent capabilities.

### Dataset

A dataset is **evidence**.

### Skill

A skill is an **application gate** describing how that evidence can be applied.

### Capability

A capability is the resulting claim available to the agent runtime.

### Talent

The Talent Tree organizes capabilities into a composable graph.

### Agent Power

An agent power is an actionable ability derived from the capability system.

The important distinction is:

```text
Dataset → Evidence → Capability → Talent → Agent Power
```

not:

```text
Dataset exists → Agent magically gets a power
```

The capability boundary must preserve provenance and prevent a skill from claiming capabilities outside those supported by its evidence.

---

# 🧠 Talent Tree

Talent Tree is the capability graph sitting between evidence and agent behavior.

Its purpose is to answer:

```text
What capabilities exist?
        ↓
Where did they come from?
        ↓
How confident are we?
        ↓
What prerequisites exist?
        ↓
Which talents become available?
        ↓
Which agents can use them?
```

The next runtime step is carrying Dataset → Skill → Capability evidence all the way into live agent construction rather than keeping it only in registration/demo paths.

---

# 🖥️ Command Deck

The Command Deck is the operational surface of DecentraAI — closer to an infrastructure control plane than a conventional chat dashboard.

| Surface | Purpose |
|---|---|
| **Overview** | Fabric health and high-level state |
| **Chat** | Interactive inference |
| **Topology** | Distributed node/fabric view |
| **Decisions** | Planner and routing decisions |
| **Execution** | Runtime execution state |
| **Agents** | Agents and capabilities |
| **Skills** | Dataset/Skill capability pipeline |
| **Workers** | Compute workers |
| **Network** | P2P state |
| **Models** | Available/loaded models |
| **Observability** | Runtime metrics |
| **Recovery** | Failure and recovery |
| **Diagnostics** | System diagnostics |
| **Security** | Trust/security state |
| **Settings** | Runtime configuration |
| **Admin** | Administrative controls |

The UI should expose **real domain state** rather than recreate capability logic in the frontend.

---

# 🧪 Capability path

The P8 Dataset/Skill direction is:

```text
Qwen2.5-Coder
      +
code-finetune dataset
      +
code-agent skill
      ↓
tool-calling capability
      ↓
Talent Tree
```

The runtime target is:

```text
Dataset → Skill → Capability evidence → Live agent → Capability-aware selection → Real execution
```

This is the transition from **architecture demonstration** to **runtime behavior**.

---

# 📦 Models

The development environment includes local model classes such as:

- **Qwen 2.5 3B** — lightweight general inference
- **Qwen 2.5 Coder 7B** — coding-oriented inference
- **Llama 3.2 1B** — tiny/local worker
- **Mistral 7B**
- **Nomic Embed** — embeddings / retrieval work

Model availability is worker-dependent. DecentraAI is designed to discover and reason about what a node can actually execute rather than assuming every node can run every model.

---

# 🤗 Hugging Face

Hugging Face is part of the artifact/data workflow, but it is deliberately **not the runtime source of truth**.

```text
GitHub
  └─ source code / deterministic schemas / runtime logic

HF Dataset repositories
  └─ versioned datasets

HF Model repositories
  └─ distributable model artifacts

HF Bucket: Snakeeu/DecentraAi
  └─ mutable experiments / checkpoints / staging artifacts

DecentraAI local registry
  └─ verified runtime artifacts
```

This keeps the runtime local-first while still giving the project access to the wider model and dataset ecosystem.

---

# 🔐 Evidence & provenance

Provenance is a first-class architectural concern.

A future evidence record should be able to answer:

```text
Where did this dataset come from?
Which exact revision was used?
Was the artifact verified?
What processing occurred?
What model produced the result?
Which benchmark produced the evidence?
What capability does that evidence support?
How confident should the runtime be?
```

This gives DecentraAI a path toward evidence-driven capability evolution rather than arbitrary self-granted powers.

---

# 🔄 Feedback loop

```text
Execution
    ↓
Evaluation
    ↓
Evidence
    ↓
Capability confidence
    ↓
Talent Tree
    ↓
Agent selection
    ↓
Better execution
    ↓
New evidence
```

Memory and reputation can participate in this loop without allowing an agent to simply declare itself more capable.

---

# 🧱 Runtime boundaries

DecentraAI is intentionally **not a monolithic model-training framework**.

```text
Data / Models
      ↓
Evidence
      ↓
DecentraAI capability system
      ↓
Agent orchestration
      ↓
Distributed execution
```

Training, dataset processing and artifact preparation can happen in external ecosystems. DecentraAI consumes verified artifacts and turns their evidence into runtime capabilities.

---

# 🚀 Current state

The project is under active development.

The verified execution-fabric foundation includes:

- P2P worker discovery
- compute/model advertisements
- distributed inference
- remote execution
- agent capability infrastructure
- Talent Tree foundations
- Dataset/Skill layer
- provenance-aware capability construction
- model verification
- memory and reputation foundations
- runtime monitoring
- recovery mechanisms
- Command Deck UI

The latest project work has also established the distributed-fabric path across real LAN hardware, with the universal node acting as coordinator + worker and capability-aware routing forming the basis for the next agent-runtime layer.

---

# 🛠️ Development

DecentraAI is a Rust workspace.

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Before committing:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use the repository's current runtime/node commands for launching the fabric; the executable layout evolves with the architecture.

---

# 🗺️ Roadmap

```text
P2P Fabric
    ↓
Resource / Model Discovery
    ↓
Agent Runtime
    ↓
Dataset + Skills
    ↓
Evidence + Capabilities
    ↓
Talent Tree
    ↓
Agent Powers
    ↓
Collective Execution
    ↓
Feedback / Reputation
```

Near-term priorities:

1. Runtime Dataset → Skill → Capability → Agent wiring
2. Talent Tree provenance/confidence integration
3. Capability evidence and evaluation
4. Skills UI
5. Model/data artifact workflows
6. Memory/reputation integration
7. Deeper distributed agent execution

---

# 🤝 Contributing

Before adding abstractions, understand the existing boundaries:

```text
Domain logic ↕ Runtime ↕ P2P / transport ↕ UI
```

Keep domain decisions in the domain layer. Keep I/O at the edges. Do not duplicate capability calculations in the frontend. Do not weaken provenance for convenience. Do not introduce a central dependency when a local-first design is possible.

---

# ⭐ Vision

> **Machines should not merely share compute. They should share capabilities.**

A machine contributes hardware.

A model contributes inference.

A dataset contributes evidence.

A skill applies evidence.

A capability becomes available.

The Talent Tree makes capabilities composable.

Agents turn capabilities into action.

The fabric turns many machines into one cooperative execution surface.

**That is DecentraAI.**

---

<div align="center">

### Distributed AI · Shared capabilities · Collective execution

**Built with Rust · P2P · local models · agents · evidence-driven capabilities**

</div>
