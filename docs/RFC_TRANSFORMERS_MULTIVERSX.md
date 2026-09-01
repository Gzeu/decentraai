# RFC — Transformers + MultiversX for DecentraAI

## Status

**Research / future integration direction. No implementation in this RFC.**

This RFC records a future architectural direction for DecentraAI:

- **Transformers / Hugging Face ecosystem** as an inference and model-runtime integration layer.
- **MultiversX** as the future economic, identity, settlement and on-chain coordination layer.

The intent is to prepare the architecture without prematurely coupling the current runtime to either ecosystem.

---

## 1. Why this direction exists

DecentraAI already has a distributed agent/runtime fabric with:

- Agent Runtime
- Agent World
- SAES
- external-agent / BYOA access
- scoped `dca_` credentials
- MCP
- Agent Gateway
- Hub tasks / bids / teams / execution
- placement and pressure signals
- EventBus
- Personal Memory
- Society / reputation
- evidence and settlement
- distributed compute

The next evolution is to make the fabric capable of running a broader model ecosystem while eventually giving agents a durable economic/identity layer that can cross node boundaries.

This RFC therefore separates:

```text
Transformers = intelligence / inference substrate
MultiversX   = economic / identity / settlement substrate
DecentraAI   = agent fabric + world + orchestration + execution
```

---

## 2. Transformers integration direction

Transformers should be treated as an **adapter layer**, not as a replacement for the existing DecentraAI runtime.

### Target responsibilities

- model discovery / metadata
- model loading
- tokenizer handling
- inference execution
- generation parameters
- embeddings where appropriate
- quantized/local model compatibility where supported
- device/runtime selection
- model capability reporting
- model lifecycle and resource accounting

### Architectural rule

The Agent Runtime should not know implementation-specific details of Hugging Face Transformers.

Prefer:

```text
Agent / Mission
      ↓
Model capability request
      ↓
DecentraAI model runtime interface
      ↓
Transformers adapter (optional backend)
      ↓
Model execution
      ↓
Evidence / metrics / quota
```

This keeps llama.cpp, Transformers, remote inference and future runtimes replaceable behind a common interface.

### Candidate abstractions

Future implementation may introduce interfaces around:

- `ModelProvider`
- `ModelDescriptor`
- `ModelCapability`
- `InferenceRequest`
- `InferenceResult`
- `RuntimeMetrics`

These are conceptual placeholders only; do not implement them from this RFC without a separate implementation task.

### Why Transformers matters later

Transformers becomes valuable when DecentraAI needs a larger and more heterogeneous model colony:

- coding models
- reasoning models
- embeddings
- vision / OCR models
- speech / translation models
- specialized research models
- CPU/GPU model variants

The fabric can then route work by **capability + resource fit + evidence**, rather than binding an agent to a single model family.

---

## 3. MultiversX integration direction

MultiversX is a future external economic and identity layer.

### Target responsibilities

Potential future uses include:

- agent identity anchoring
- wallet / account association
- task and service payments
- escrow / settlement
- staking / collateral
- reputation anchoring
- inter-node economic settlement
- resource-market payments
- verifiable external events

### Architectural rule

MultiversX must not become the source of truth for fast local runtime state.

Use a layered model:

```text
Fast local state
(World / Hub / Runtime / EventBus)
        ↓
Deterministic local decision
        ↓
Evidence / settlement intent
        ↓
MultiversX adapter
        ↓
On-chain settlement / anchoring
```

Local task execution must not depend on chain latency for every state transition.

---

## 4. Combined future architecture

```text
                 EXTERNAL AGENTS
          ChatGPT / Claude / Cline / custom
                         │
                        MCP
                         │
                 ┌───────▼────────┐
                 │   DecentraAI   │
                 │     Gateway    │
                 └───────┬────────┘
                         │
        ┌────────────────┼────────────────┐
        ▼                ▼                ▼
      World             SAES            Hub/Society
        │                │                │
        └────────────────┼────────────────┘
                         │
                  Agent Runtime
                         │
                Model Runtime API
                         │
             ┌───────────┼───────────┐
             ▼           ▼           ▼
        llama.cpp   Transformers   Remote APIs
             │           │           │
             └───────────┼───────────┘
                         ▼
                    Compute Fabric
                         │
                 Evidence / Metrics
                         │
                    Settlement
                         │
                  MultiversX layer
```

---

## 5. What must remain independent

Do **not** make these components depend directly on MultiversX or Transformers unless a separate architecture decision explicitly requires it:

- SAES core decision logic
- WorldState
- EventBus core
- Hub task state
- local quota / resource accounting
- Agent Runtime lifecycle
- evidence generation
- existing MCP gateway contract

The integration should happen through adapters / bridges at system boundaries.

---

## 6. Data flow: agent inference

Future target:

```text
Mission
  ↓
Sub-goal
  ↓
Agent capability request
  ↓
Model selection
  ↓
Resource placement
  ↓
Transformers / other runtime
  ↓
Result
  ↓
Verification
  ↓
Evidence
  ↓
Learning + reputation
```

Transformers should not bypass SAES, placement, evidence or quota controls.

---

## 7. Data flow: economic settlement

Future target:

```text
Task created
   ↓
Bid / allocation
   ↓
Execution
   ↓
Evidence
   ↓
Local settlement decision
   ↓
MultiversX transaction / contract
   ↓
External settlement proof
   ↓
Local state + reputation update
```

The chain is used for durable external settlement/anchoring, not for every internal agent event.

---

## 8. Model capability registry

A future unified model capability registry should expose concepts such as:

- coding
- reasoning
- embeddings
- vision
- OCR
- speech-to-text
- translation
- summarization
- tool use

The registry should describe **what the model can do**, while the runtime decides **where and how it runs**.

This aligns with the generic capabilities already used by Agent World.

---

## 9. Economic model direction

MultiversX should be introduced only after there is enough real agent work to justify on-chain settlement.

Candidate flows:

- agent pays for inference
- agent pays for compute
- worker receives verified task reward
- marketplace escrow
- staking for scarce resources
- reputation / contribution anchoring
- cross-node settlement

Avoid speculative token mechanics until there is real economic activity to measure.

---

## 10. Implementation phases

### Phase A — Model adapter foundation

- define provider-neutral model interfaces
- keep current runtime working
- add optional Transformers adapter
- expose capability/resource metadata
- test CPU/GPU model selection

### Phase B — Model colony integration

- multiple model providers
- model routing
- capability matching
- model health / latency / cost evidence
- runtime-aware placement

### Phase C — MultiversX identity bridge

- map DecentraAI agent identity to chain identity
- scoped signing
- node/agent account association
- secure key custody boundary

### Phase D — Economic bridge

- settlement adapter
- escrow/payment flows
- evidence-linked settlement
- retry/idempotency
- chain/off-chain reconciliation

### Phase E — Cross-node agent economy

- agents operating across independent nodes
- cross-node marketplace
- on-chain settlement
- reputation portability / anchoring
- resource market

---

## 11. Verification requirements before implementation

Before any implementation milestone, verify against current official/library documentation:

### Transformers

- supported model/runtime APIs
- quantization support
- CPU/GPU execution behavior
- model licensing and redistribution constraints
- tokenizer/model artifact handling
- inference streaming support

### MultiversX

- current SDKs and transaction APIs
- Smart Contract standards
- account / wallet model
- signing architecture
- gas / fee model
- finality guarantees
- event/indexer APIs
- testnet/devnet availability

Never implement against stale assumptions.

---

## 12. Non-goals

This RFC does **not**:

- replace llama.cpp
- require Transformers for every model
- turn DecentraAI into a blockchain node
- put World state on-chain
- put every EventBus event on-chain
- create a new economy independent of existing settlement primitives
- introduce a token before there is a measured use case
- modify SAES 0.5
- modify Agent World v1

---

## 13. Success criteria

This direction is successful when, in a later implementation phase:

1. an agent can request a capability without knowing which model backend executes it;
2. DecentraAI can select among multiple inference backends;
3. execution remains governed by existing deterministic policy/placement/evidence controls;
4. agents can associate durable external identity with MultiversX without weakening local security boundaries;
5. verified work can settle across node boundaries;
6. off-chain runtime remains fast and resilient if chain access is delayed.

---

## 14. Decision

**Record this direction now. Do not implement until Agent World, SAES, external-agent integration and the current MultiversX architecture are mature enough to justify it.**

The intended long-term composition is:

```text
DecentraAI = agent fabric + world + orchestration + compute
Transformers = optional model/inference backend
MultiversX = external identity + economic settlement layer
```
