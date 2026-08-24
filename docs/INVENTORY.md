# DecentraAI — COMPONENT INVENTORY (what we have & what it's for)

> Living index. One row per component: WHAT it is, WHY it exists, WHERE it
> lives. Deep docs linked. Updated through the Model Intelligence +
> Economic/MVX phases (2026-08-23).

## 1. Fabric core (execution authority)

| Component | What | For | Where |
|---|---|---|---|
| Deterministic planner | ExecutionPlan, plan_and_reserve, reservations | AI proposes → policy decides → workers execute | `crates/fabric` |
| Fabric Intelligence | reasoning layer; validated structured plans from untrusted model output | intel PROPOSES only | `crates/fabric-intelligence` |
| P2P transport | libp2p actor, request/response codec, verified transfer, DFCP dispatch cascade, memory-sync decode branch | all mesh traffic incl. memory sync | `crates/p2p` |
| MX-8004 protocol | infer messages + DFCP v1 + MemorySyncRequest/Response (bounded, deny_unknown_fields) | wire truth | `crates/protocol` |

## 2. Agent OS & intelligence

| Component | What | For | Where |
|---|---|---|---|
| Agent records/orchestration | delegation DAG, unified matcher, workflows | collective execution | `crates/agents` |
| Collective memory | scopes (agent→system), lifecycle candidate→verified→trusted→obsolete, BLAKE3 dedup, conflict links, deterministic resolution | shared verified knowledge; imported = candidate | `crates/agents/src/memory.rs`, `crates/distributed/src/agent_memory.rs` |
| Semantic retrieval | embeddings BLOB + cosine search + backfill endpoint | find knowledge by meaning | same + `/v1/memory/search?mode=` |
| Cross-node sync | additive merge over existing p2p transport | propagation without a second protocol | `memory_sync.rs` + p2p branch |
| Auto-propagation | opt-in (`DECENTRAAI_MEMORY_PROPAGATE=1`) verified-knowledge push to peers | colony learning spreads | `distributed::memory_propagator` |
| Training export | verified+evidenced generalizations → JSONL candidates; failure→solution pairing | feeds Training Lab BY HAND only | `agents::training_export`, `/v1/memory/training-candidates` |
| Model Intelligence | registry (availability × governance axes), capability claims, deterministic routing, shadow comparison | evaluate & route multiple local models without pre-picking winners | `hub::model_intel`, `fabric::model_routing`, `/v1/models/*` |
| Performance observations | verified executions → `model.intel` scope → router input | models earn trust through evidence | `distributed::model_performance` |

## 3. Runtime & ops

| Component | What | For | Where |
|---|---|---|---|
| Node daemon | one-process background node (identity/config/discovery/serve/dashboard) | always-on public node | `decentraai node`, `deploy/decentraai-node.service` |
| Dashboard | embedded control plane (Overview/Chat/Workers/…/**Model Colony**) | one UI, real state only | `crates/runtime/src/dashboard*.rs` |
| Governor daemon | polls node state, pressure hysteresis, UNTRUSTED memory context, operator actions | autonomous observation + manual verification/export | `scripts/governor-daemon.py` (+ tests) |
| Audit | append-only event log (`logs/`) | traceability of every important mutation | `crates/audit` |
| CI | fmt/clippy/test/audit/gitleaks/roadmap gates | green baseline discipline | `.github/workflows` |

## 4. Economy + MultiversX (foundation — simulation/interfaces ONLY)

| Component | What | For | Where |
|---|---|---|---|
| CU v2 formula | integer µCU/bps award math, versioned (`ECONOMICS_VERSION=2`); verification is a GATE | deterministic economic value | `economy::contribution` |
| RewardEngine | monotonic accounts, bounded reversals (25 % penalty cap), audited mutations | bookkeeping that can't go negative or rewrite history | `economy::engine` |
| Anti-gaming | self-verify gate, evidence replay dedup per worker, missing-evidence rejection, sybil non-amplification | rewards depend on VERIFIED contribution | engine gates + tests |
| Crypto evidence | EconomicEvidence → canonical bytes → BLAKE3 → Ed25519 (audited primitives) → 5-step verify incl. economic recheck | independently provable claims | `economy::evidence` |
| Settlement seam | BlockchainAdapter trait + LocalTestAdapter; future WalletIdentity/TransactionSigner/BalanceQuery/NetworkFeeQuote traits | chain-agnostic, replaceable; fabric runs chainless | `economy::settlement` |
| Tokenomics simulator | config-driven params, schedules/allocations/fees/burn/vesting/slashing; DEFINED sustainability verdict; scenario ladder 10→100k nodes | test economics without choosing parameters | `economy::tokenomics`, `examples/simulate.rs` |
| MVX devnet client | read-only agents/reputation queries over injectable transport; live base = taskclaw host | external identity/reputation reference | `economy::multiversx_devnet` |
| Registry addresses | Identity/Validation VERIFIED via indexer tx inspection; Reputation PARTIAL | anchors for future writes | `registry_addresses` + `docs/MULTIVERSX_DEVNET_ADDRESSES.md` |
| Identity link | Ed25519 pubkey byte-equality proof local ↔ MX record; manifest/body builders (validated offline) | first on-chain agent identity, operator-executed | `economy::multiversx_identity`, runbook in `docs/MULTIVERSX_AGENT_REGISTRATION_PROCEDURE.md` |
| Tx preparation | unsigned intents for every source-verified v2.1 call (register/proof/validation/job/feedback) | operator signs with own wallet tooling; code never signs | `economy::multiversx_tx` |

## 5. Hard rules (unchanged)

No token launch · no mainnet · no auto-signing · no keys in repo ·
external scores are untrusted · internal ledger authoritative ·
AI proposes → deterministic policy decides → workers execute.

## 6. Models deployed on VPS (testnet)

| Model | Size | Governance | Status |
|---|---|---|---|
| Qwen3-1.7B Q4_K_M | 1.2 GB | approved | ✅ LOADED + benchmarked (16% MI corpus) |
| Gemma-3-1B-it Q4_K_M | 769 MB | experimental | downloaded, awaiting deployment |
| Phi-4-mini-instruct Q4_K_M | 2.4 GB | experimental | downloaded, awaiting deployment |
