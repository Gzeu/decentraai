# DecentraAI — CURRENT STATUS

> Updated: 2026-08-24 post all-merges

## main @ 7f747ae+ — 1386 tests · clippy clean · CI 100% green

## Merged milestones (all on main)

| Milestone | PR | Tag |
|---|---|---|
| Fabric Intelligence | — | `milestone/fabric-intelligence` |
| Sharing is Caring (DFCP v1) | — | `milestone/sharing-is-caring` |
| Agent OS + Obsidian Memory | — | `milestone/agent-os` |
| M15 Autonomous Pressure | — | `milestone/autonomous-pressure` |
| Training Lab | — | merged |
| M16 Agent Gateway | #37 | `milestone/agent-gateway` |
| M17.1 Collective Orchestration | #38 | `milestone/collective-orchestration` |
| M18 Collective Memory | #39 | `milestone/collective-memory` |
| M19 Memory Fabric (semantic+sync+propagation) | #40 | `milestone/memory-fabric` |
| MI-ops (persistence, governance API) | #48 | `milestone/model-colony-ops` |
| Full-loop integration test | #50 | — |
| Intelligence Loop (active colony) | #55 | — |
| MVX Devnet Adapter + Identity | #45+#46 | — |
| MX-8004 Write Path research | #47 | — |

## MultiversX Testnet — LIVE

DecentraGovernor registered as soulbound NFT:
- Agent nonce: **7**
- txHash: `f25eed6bd9e5551289833f323b0de06a93ad96e43363d927bc410ca88806af33`
- Identity Registry: `erd1qqqqqqqqqqqqqpgq8qn7lr9287vzkjtr55lz3r3c56dgthyzr5es626nms`

All three registries VERIFIED (codeHash confirmed via gateway probe):
Identity · Validation · Reputation — see MULTIVERSX_DEVNET_ADDRESSES.md.

## Component index

Full component→purpose→location mapping: docs/INVENTORY.md
VPS operations: docs/VPS_OPERATIONS.md
Economic model: docs/ECONOMIC_MODEL.md
MX integration: docs/MULTIVERSX_AGENT_INTEGRATION.md
Model Intelligence: docs/MODEL_INTELLIGENCE.md

## Remaining work (all need external resources)

| Task | Blocker |
|---|---|
| GGUF models on worker | download Qwen/Gemma/Phi Q4 files |
| Embeddings backend on VPS | llama-server --embedding |
| LAN two-node validation | physical hardware access |
| Governor daemon → Router integration | build (medium) |
| Dashboard Colony actions UI | build (medium) |
| P2P announce-side lan_discovery gating | p2p behaviour rework |
