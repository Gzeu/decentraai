# Agent Memory — Obsidian Knowledge Layer (M14.x)

The node agent's second brain: an Obsidian-compatible vault of typed,
linked, versioned notes — searchable, consolidatable, and forgettable in a
controlled way.

## Principles

1. **Typed memory**: every note carries YAML frontmatter
   (`type` / `confidence` / `status` / `created` / `source`).
   Types: `fact`, `decision`, `experiment`, `lesson`, `hypothesis`,
   `session`, `failure`, `evidence`, `inbox`.
2. **Graph, not folders**: `[[Wiki Links]]` connect knowledge bidirectionally
   (`related` walks both directions).
3. **Consolidation**: raw session output lands in `00_INBOX`; only reviewed
   notes are promoted (`consolidate`) into permanent homes.
4. **Forgetting is marking, not deletion**: `forget` sets
   `status=obsolete` with a reason and timestamp; history stays in git.
5. **Secrets NEVER enter the vault**: `store` redacts key-shaped strings
   automatically; reference credentials by env-var NAME instead.

## Install

```bash
# on the node (VPS/Desktop/Laptop):
python3 scripts/agent-memory.py --vault ~/decentraai-agent init
```

Creates the standard tree (`00_INBOX … 07_EVIDENCE`), a README contract,
git history (agent-local identity; never touches global git config), and a
`.obsidian/`-friendly layout for the Obsidian desktop app over SSHFS/sync.

## Commands

```bash
agent-memory.py store --type decision --title "T" --body "…" \
    --tags m14,sharing --links "DFCP,Credit Ledger" --confidence verified
agent-memory.py search "sharing"            # ranked lexical search
agent-memory.py related sharing-is-caring-m1 # graph walk (→ out, ← in)
agent-memory.py list --status active        # inventory
agent-memory.py consolidate <id> --type experiment  # INBOX → permanent
agent-memory.py forget <id> --reason "superseded by X"
```

Every mutating command auto-commits with the agent-local identity, so the
vault is fully versioned without touching global git config.

## V2 roadmap

- Semantic search via fabric embeddings (`nomic-embed` on Desktop through
  DFCP assist — the agent's memory indexed BY the fabric it lives on).
- Cross-node collective memory: per-agent vaults + authorized shared
  knowledge over the fabric RAG path.
- Evidence-linked knowledge: benchmark notes carrying signed receipt ids.
