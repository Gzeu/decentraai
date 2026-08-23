# M18 — Collective Memory

## Objective

Cross-agent, cross-node knowledge layer where verified execution results,
decisions, lessons and facts are stored with provenance, searchable via
fabric RAG, and scoped per agent/shared/fabric.

## Architecture

```text
Agent A (Governor)          Agent B (QA)           Agent C (Researcher)
  │                           │                        │
  ▼                           ▼                        ▼
private scope              private scope            private scope
  │                           │                        │
  └───────────────┬───────────┘                        │
                  ▼                                    ▼
          Memory Keeper consolidation                INBOX
                  │
                  ▼
          08_SHARED/ knowledge
                  │
                  ▼
          Fabric RAG (embeddings via DFCP)
                  │
                  ▼
          Agent queries shared knowledge
```

## What exists already (REUSE, don't duplicate)

| Component | Location | Status |
|---|---|---|
| MemoryStore (SQLite) | `crates/distributed/src/agent_memory.rs` | ✅ working |
| Memory scopes + entries | same file, `MemoryScope`/`MemoryEntry` | ✅ |
| Obsidian vault CLI | `scripts/agent-memory.py` | ✅ working |
| Embedding client | `crates/distributed/src/embedding.rs` | ✅ |
| RAG retrieval manager | `crates/distributed/src/retrieval_manager.rs` | ✅ |
| Fabric Intelligence plan | `crates/fabric-intelligence` | ✅ |

## What to BUILD (new code only)

### 1. Provenance metadata on every memory entry

Extend `MemoryEntry` with:

```rust
pub provenance: MemoryProvenance,
```

```rust
pub struct MemoryProvenance {
    pub source_agent: String,      // e.g. "governor", "qa"
    pub source_type: String,       // "execution", "observation", "decision"
    pub evidence_id: Option<String>, // link to receipt/evidence when available
    pub confidence: f32,           // 0..1
    pub created_at_ms: u64,
}
```

### 2. Scope hierarchy

```
agent_private/<agent_id>     ← read/write by that agent ONLY
team/<team_name>             ← read/write by team members
fabric_public                ← readable by all, writable by Memory Keeper
```

Enforced at query time: agent A cannot read agent B's private scope.

### 3. Deduplication + conflict resolution

Before storing a new entry:
- Hash the content; if an identical hash exists → skip
- If similar (same capability domain, overlapping text) → link as related, don't duplicate
- Conflicting facts → keep both with `status: contested`, flag for review

### 4. Semantic search via fabric embeddings

Wire the existing `EmbeddingClient` into the retrieval path so agents can
search shared knowledge semantically (not just keyword match).

### 5. Governor retrieval API

`GET /v1/memory/search?query=...&scope=fabric_public&limit=10`
Returns matching entries with full provenance.

### 6. Execution feedback loop

After each workflow stage completes:
- Store the stage result in the agent's private scope
- On verified success → propose promotion to shared scope
- Memory Keeper consolidates periodically

## Implementation plan (ordered)

1. Extend `MemoryEntry` with `MemoryProvenance`
2. Add scope enforcement to `MemoryStore::read/write`
3. Add dedup check before insert
4. Wire semantic search through existing `EmbeddingClient`
5. Add `GET /v1/memory/search` endpoint
6. Hook workflow completion → memory write (feedback loop)
7. Tests: provenance propagation, scope isolation, dedup, semantic search

## NOT in M18

- Full-text search engine (use simple grep + embedding search)
- Vector database (embeddings stored inline in SQLite BLOB)
- Cross-node memory sync protocol (M19+)
- Automatic model fine-tuning from memory (future)

## Success criteria

- Agent stores a lesson in its private scope ✓
- Memory Keeper consolidates it into shared ✓  
- Another agent searches and finds it with provenance ✓
- Agent A cannot read Agent B's private notes ✓
- Duplicate detection prevents double-store ✓
- Semantic search finds relevant knowledge without exact keywords ✓
