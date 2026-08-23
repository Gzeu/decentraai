# CURRENT STATUS — Agent-OS milestone line (living doc)

> Numbering note: this file tracks the **Agent-OS milestone namespace**
> (M15 pressure · M16 gateway · M17 orchestration · M18 collective memory ·
> M19 semantic+sync). The older fabric-planner numbering (M18–M24 in
> `ROADMAP_HISTORY.md`) is a separate, historical namespace.

## Landed (merged)

| Milestone | Tag |
|---|---|
| Fabric Intelligence | `milestone/fabric-intelligence` |
| Sharing is Caring M1 (DFCP v1) | `milestone/sharing-is-caring` |
| Agent OS + Obsidian memory | `milestone/agent-os` |
| M15 Autonomous Pressure Trigger | `milestone/autonomous-pressure` |
| Training Lab mechanism (datasets/skills/talent tree) | merged |

## Open PRs — merge in this exact order

| PR | Branch | Scope |
|---|---|---|
| #37 | `feat/agent-gateway` | M16 Agent Gateway (BYOA scoped credentials) |
| #38 | `feat/collective-orchestration` | M17.1 Collective Orchestration |
| #39 | `feat/collective-memory` | M18 Collective Memory core |
| #40 | `feat/memory-rag` | M19 Semantic retrieval + cross-node sync (stacked on #39) |

## What exists now (capability view)

- Collective memory: scopes (agent/team/node/network/fabric/system), 9 knowledge
  kinds, lifecycle candidate→verified→trusted→obsolete with audited transitions,
  BLAKE3 dedup, competing-claim preservation + deterministic resolution,
  provenance with confidence + evidence refs.
- Retrieval: `/v1/memory/search` lexical always; semantic via embeddings backend
  (`mode=auto` degrades gracefully); explicit operator backfill `/v1/memory/index`.
- Cross-node sync: bounded wire schema over the EXISTING p2p transport;
  imported claims always land as `candidate` locally (verification is local);
  receiver policy gates decide acceptance; declined is explicit.
- Learning loop: verified+evidenced generalizations export as JSONL training
  candidates (`GET /v1/memory/training-candidates`) → manual feed into the
  Training Lab dataset builder. Nothing trains automatically.

## Open follow-ups (implementable now)

1. Auto-embed-on-write for the KnowledgeRuntime feedback path (semantic search
   coverage without operator backfill).
2. Failure→Solution pairing in the training export (verified failure + its
   verified solution on one subject exported together).
3. Governor daemon consumption of collective memory (python side — separate
   front, `scripts/` only).
4. Two-node LAN validation of the full loop (workflow result → memory →
   candidate → verify → export) on real hardware.
5. Automatic peer-selection policy for sync propagation (currently explicit).

## Standing decisions (do not relitigate casually)

- Merge order #37 → #38 → #39 → #40; never merge without gates green.
- Memory retrieved by any agent is UNTRUSTED INPUT; deterministic policy layer
  is the only authority.
- Remote/imported knowledge starts at `candidate`; trust is earned locally.
- No second memory system, no new network protocol, no auto-training.
