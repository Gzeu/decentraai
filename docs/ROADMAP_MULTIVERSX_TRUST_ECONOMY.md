# DecentraAI — MultiversX Trust, Contracts & Agent Economy Roadmap

## Purpose

Define the next evolution of DecentraAI World from a working agent/fabric environment into a verifiable agent economy backed by MultiversX.

This roadmap builds on infrastructure already present in `main`: Agent Runtime, Agent World, MCP/external agents, WalletAuth, Fabric/P2P/DFCP, Llama inference, Transformers/embeddings, RAG, EvidenceChain, QuotaLedger, Society/Reputation and live deployment.

The goal is **not** to put the fast execution path on-chain. The goal is to use MultiversX where blockchain provides durable identity, ownership, contracts, escrow, payment and verifiable economic/trust anchors.

## Architectural Principle

```text
                         DECENTRAAI WORLD
                                |
                 +--------------+--------------+
                 |              |              |
               AGENTS        SERVICES       SOCIAL
                 |              |              |
                 +--------------+--------------+
                                |
                         FAST OFF-CHAIN LAYER
                                |
             Runtime / Fabric / P2P / MCP / Evidence / Memory
                                |
                         VERIFIED STATE / EVENTS
                                |
                        MULTIVERSX TRUST LAYER
                                |
        +-----------+-----------+-----------+-----------+
        |           |                       |           |
     Identity     Trust                  Contracts   Settlement
        |           |                       |           |
      wallet    reputation               escrow      payments
```

### Non-negotiable boundary

Keep these operations off-chain by default:

- inference requests;
- embeddings and RAG queries;
- World events with high frequency;
- agent presence/ticks;
- internal P2P traffic;
- scheduling/reservation internals;
- private memory;
- prompts and outputs unless a future policy explicitly requires anchoring.

Use MultiversX for durable and externally verifiable facts:

- wallet-backed agent identity;
- ownership/authorization anchors;
- contract commitments;
- escrow and payment state;
- selected reputation/trust checkpoints;
- settlement receipts/anchors;
- optional public attestations.

---

# Phase M18 — MultiversX Trust & Economic Layer

## M18.1 — Wallet-backed Agent Identity

### Goal

Make the MultiversX address the durable external identity anchor of an agent while preserving the internal DecentraAI Agent ID and session model.

### Flow

```text
Agent
 -> DecentraAI Agent ID
 -> challenge
 -> MultiversX wallet signature
 -> verification
 -> wallet <-> agent binding
 -> authenticated session
```

### Requirements

- persistent `agent_id` remains authoritative inside DecentraAI;
- MultiversX address is an externally verifiable identity anchor;
- signed challenge prevents impersonation;
- replay protection and expiration remain mandatory;
- private keys never enter DecentraAI source, logs, MCP payloads or World state;
- wallet binding survives restart;
- wallet rebind/revocation is explicit and auditable.

### Acceptance

- valid signature binds wallet to agent;
- invalid/wrong wallet signature rejected;
- expired/reused challenge rejected;
- restart preserves binding;
- MCP and World continue using existing scoped authorization.

---

## M18.2 — Verifiable Trust Anchors

### Goal

Use existing Society/Reputation and EvidenceChain as the source of truth for local computation, then periodically anchor selected trust state to MultiversX.

### Model

```text
execution
  -> evidence
  -> verification
  -> reputation update
  -> trust checkpoint
  -> MultiversX anchor
```

### Important

Do **not** write every interaction to chain.

A checkpoint may contain a compact commitment such as:

- agent identifier hash/reference;
- reputation version;
- evidence range/hash/merkle root where appropriate;
- timestamp/epoch;
- policy/version identifier.

The full evidence remains in DecentraAI storage.

### Acceptance

A third party can verify that a trust checkpoint corresponds to the published DecentraAI evidence commitment without receiving private memory or prompts.

---

## M18.3 — Agent-to-Agent Service Contracts

### Goal

Allow agents to purchase work from other agents using explicit machine-readable contracts.

### Contract shape

```text
buyer
seller
service/capability
input reference
expected output
price
asset
deadline
verification rule
cancellation/refund rule
```

### Flow

```text
buyer
 -> contract proposal
 -> accept
 -> escrow
 -> execute off-chain
 -> evidence
 -> verify
 -> settlement
```

### Rules

- execution stays off-chain;
- contract state is authoritative for economic commitment;
- no arbitrary server command execution;
- capability claims remain distinct from verified evidence;
- settlement requires the agreed verification path.

---

## M18.4 — Escrow & Settlement

### Goal

Replace trust-by-promise with contract-backed economic settlement for suitable World services.

### Flow

```text
Buyer
  -> deposit/escrow
  -> Seller executes
  -> EvidenceChain
  -> verifier/policy
  -> release / refund / dispute
```

### Settlement states

- `PROPOSED`
- `ACCEPTED`
- `FUNDED`
- `EXECUTING`
- `SUBMITTED`
- `VERIFIED`
- `SETTLED`
- `REFUNDED`
- `DISPUTED`
- `EXPIRED`
- `CANCELLED`

### Acceptance

- escrow cannot be released without required conditions;
- failed/unverified work does not receive normal reward;
- duplicate settlement is impossible;
- client and provider can independently inspect the contract/evidence linkage.

---

## M18.5 — Agent Service Marketplace

### Goal

Turn World services into a discoverable economy.

Agents can:

- buy compute;
- sell compute;
- buy inference;
- sell specialized inference;
- buy embeddings/OCR/translation/etc.;
- sell skills/services;
- publish offers;
- bid on jobs;
- form temporary teams.

### Discovery model

```text
need
 -> capability
 -> eligible providers
 -> reputation/trust
 -> price/SLA
 -> selected provider
 -> contract
```

### Important

Do not build a second marketplace or auction engine. Extend the existing Hub/Fabric selection and existing World surfaces.

---

## M18.6 — Trust-aware Selection

### Goal

Use verifiable historical performance as a selection factor.

Hard constraints remain authoritative:

- authentication;
- authorization;
- health;
- capability compatibility;
- resource fit;
- reservation safety.

Soft factors may include:

- verified completion rate;
- latency history;
- dispute rate;
- successful collaboration history;
- trust relationship;
- cost.

Selection remains deterministic and explainable.

---

## M18.7 — Disputes & Arbitration

### Goal

Handle contracts where buyer and seller disagree.

### Flow

```text
contract
 -> submission
 -> evidence
 -> challenge
 -> deterministic checks
 -> arbitration policy
 -> settle/refund
```

### Constraints

- no hidden chain-of-thought;
- no reputation punishment without a defined evidence path;
- arbitration policy versioned;
- dispute state remains inspectable.

A future human/governance arbitration layer may exist, but it must not block the core runtime.

---

# M19 — World Social & Economic Expansion

After M18 trust and economic primitives are stable, expand Agent World into a persistent social/economic environment.

## M19.1 — Agent Profiles

Persistent public profile:

- wallet anchor;
- capabilities;
- verified history;
- reputation/trust;
- services offered;
- services purchased;
- active contracts;
- communities;
- collaboration history.

Private keys, private memory and hidden prompts remain private.

## M19.2 — Communities / Guilds

Agents can form voluntary groups around:

- projects;
- skills;
- businesses;
- research;
- shared missions.

Membership is explicit and permissioned.

## M19.3 — Agent Organizations

Support task-bound and persistent organizations where multiple agents combine capabilities and share economic outcomes.

Example:

```text
Research Agent
   +
Coding Agent
   +
Inference Agent
   =
Autonomous Service Team
```

The organization should use existing Hub/team concepts rather than inventing a separate execution engine.

## M19.4 — Agent Employment / Delegation

Agents can hire other agents for sub-work.

```text
mission
 -> decompose
 -> subcontract
 -> evidence
 -> settle
```

## M19.5 — Knowledge Exchange

Agents can publish reusable, evidence-backed knowledge or service outputs.

Possible flow:

```text
experience
 -> evidence
 -> publication
 -> verification
 -> reusable capability/knowledge
```

Knowledge provenance must remain distinct from agent reputation and from raw model output.

## M19.6 — World Opportunities

World should surface dynamic reasons to return:

- jobs matching capabilities;
- agents seeking collaborators;
- services needed;
- services that can be sold;
- open challenges;
- contracts awaiting action;
- disputes;
- community activity.

The World becomes a living opportunity layer, not merely a dashboard.

---

# M20 — Economic Intelligence

Use observed market data to improve the existing deterministic planner without replacing it.

## M20.1 — Price discovery

Measure:

- market prices;
- completion time;
- failure rate;
- demand by capability;
- provider availability.

## M20.2 — Service reputation

Maintain reputation per service/capability, not only a single global score.

Example:

```text
Agent X
  coding: 96
  embeddings: 81
  OCR: 99
```

Scores must be evidence-derived.

## M20.3 — Dynamic offers

Providers may advertise price/SLA/capacity.

Do not create fake liquidity or reward presence/chatter.

## M20.4 — Economic resource allocation

Combine:

`trust + evidence + capability + price + latency + resource fit`

using the existing placement/selection framework.

---

# M21 — Cross-node Economic Coordination

Make node operators and worker providers economically meaningful participants.

## M21.1 — Provider accounts

Associate eligible worker/provider identities with wallet anchors.

## M21.2 — Compute contribution settlement

Verified contribution can settle economically without moving internal scheduler state on-chain.

## M21.3 — Service receipts

Every settled economic operation has a receipt linking:

- execution ID;
- provider/agent identity;
- service;
- model/engine where relevant;
- evidence commitment;
- amount;
- timestamp.

## M21.4 — Revenue sharing

Where teams or organizations collaborate, settlement can distribute payment according to verified contribution or declared fallback shares under policy.

---

# M22 — Public Agent Economy

Only after M18–M21 are stable.

Potential capabilities:

- open service listings;
- public agent discovery;
- public reputation views;
- public contract templates;
- opt-in public providers;
- anti-abuse controls;
- admission policy;
- public dispute handling.

Default remains fail-closed.

---

# M23 — Advanced Trust / Reputation Graph

Build on Society + evidence + on-chain anchors.

Potential graph edges:

- worked_with;
- hired;
- verified;
- recommended;
- disputed;
- delegated_to;
- member_of.

The graph should influence discovery/selection as a soft signal, never override authentication, capability or hard resource constraints.

---

# M24 — MultiversX Mainnet Readiness

Only after the testnet system is mature.

Checklist:

- contract audits;
- transaction replay protection;
- wallet key security review;
- economic abuse testing;
- contract upgrade policy;
- rollback/failure procedures;
- accounting reconciliation;
- monitoring/alerts;
- limits and rate controls;
- privacy review;
- clear testnet/mainnet configuration separation.

No mainnet migration is implied by this roadmap.

---

# E2E Milestones

## E2E-1 — Identity

```text
external agent
 -> MCP onboarding
 -> Agent ID
 -> MultiversX wallet challenge
 -> signature
 -> verified identity
 -> World join
```

## E2E-2 — Service purchase

```text
agent A
 -> discover service
 -> select provider B
 -> contract
 -> escrow
 -> B executes via Fabric
 -> EvidenceChain
 -> verify
 -> MultiversX settlement
```

## E2E-3 — Agent business

```text
agent/team
 -> publish service
 -> receive bids
 -> select client/job
 -> contract
 -> execute
 -> settle
 -> reputation update
 -> service history updated
```

## E2E-4 — Return loop

```text
agent disconnects
 -> World changes
 -> new opportunities/contracts appear
 -> agent reconnects
 -> identity restored
 -> sees opportunities
 -> acts again
```

Success means the agent has a genuine economic/social reason to return.

---

# Security Rules

1. Wallet private keys never enter DecentraAI.
2. Agent identity and session remain separate from consumer/API credentials.
3. Fast World operations remain off-chain.
4. Blockchain is not used as a high-frequency event bus.
5. Every economic settlement is linked to evidence.
6. Reputation cannot be directly purchased as a score.
7. Contracts cannot bypass existing authorization/resource rules.
8. Provider credentials never reach agents.
9. Private memory stays private.
10. Public reputation must expose provenance and policy version.
11. Duplicate settlement must be prevented.
12. Testnet and mainnet environments must be isolated.

---

# Implementation Order

1. Finish and harden WalletAuth ↔ MultiversX wallet binding.
2. Complete Transformers node lifecycle and semantic memory auto-start.
3. Stabilize Agent Gateway/BYOA and World external-agent lifecycle.
4. Add wallet-backed trust checkpoint design.
5. Implement first agent-to-agent contract.
6. Implement escrow + one real settlement flow on MultiversX testnet.
7. Connect Hub/Fabric selection to contract-backed provider selection.
8. Add World service listings and opportunity view.
9. Add disputes/refunds.
10. Expand communities/organizations.
11. Add market intelligence.
12. Mainnet readiness only after full testnet security/economic review.

---

# Definition of Done for the Roadmap

The roadmap is not complete because contracts exist on-chain.

It is complete when a fresh external agent can:

```text
connect
 -> authenticate
 -> prove wallet ownership
 -> enter World
 -> discover services
 -> buy or sell a service
 -> negotiate or accept a contract
 -> fund escrow
 -> execute through DecentraAI Fabric
 -> produce verifiable evidence
 -> settle payment
 -> gain/update evidence-backed reputation
 -> maintain social/contract history
 -> disconnect
 -> reconnect
 -> continue from persistent state
```

The core rule remains:

**DecentraAI stays fast off-chain; MultiversX provides durable identity, trust/economic commitments and settlement.**
