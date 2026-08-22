# DecentraAI Agent Operating System

> **Worker = compute identity. Agent = cognitive identity.**
> One worker can serve many agents; one agent can use many workers.

An organization of specialized agents — each with a role, scopes,
forbidden actions, approval requirements and its own memory scope —
coordinated by a Governor that has zero authority over the deterministic
policy layer.

## The registry (`.agents/registry/`)

Every agent is DEFINED as a contract file with YAML frontmatter:

```yaml
agent:
  id: rust-engineer
  role: developer
  scopes:        [repo.read, repo.write, tests.run]
  forbidden:     [secrets.read, credentials.issue, trust.modify]
  approval_required: [auth changes, protocol breaking changes]
  memory_scope:  agents/rust-engineer
  model_hint:    qwen2.5-coder-7b   # advisory; agent ≠ model
```

| id | role | may | never |
|---|---|---|---|
| governor | coordinator | decompose, delegate, review, synthesize | write code, mutate policy |
| architect | design | ADRs, invariant review | modify code |
| rust-engineer | developer | Rust workspace + tests | auth/trust changes w/o approval |
| api-engineer | developer | REST/MCP/OpenAI-compat surface | credential flows w/o approval |
| fabric-engineer | developer | intel pipeline, providers, routing | policy/limit changes w/o approval |
| qa | quality | tests, live verify, **reject work** | feature code |
| security | audit | propose security fixes | apply critical fixes itself |
| vps-operator | operations | services/models/logs/health/deploy | application code |
| memory-keeper | knowledge | consolidate/link/mark obsolete | permanent deletion |
| researcher | knowledge | search web/HF/docs → INBOX only | direct permanent writes |
| concierge | gateway | onboard external agents, explain, request scoped creds | self-serve credentials, admin tools |

## Memory separation (Obsidian vault)

```text
05_AGENTS/agents/<id>/   ← private per-agent memory (lessons, sessions)
08_SHARED/               ← architecture, decisions, fabric facts, knowledge
00_INBOX/                ← raw session output awaiting consolidation
```

Rules:
- Agent-private notes carry `agent: <id>` in frontmatter (`store --agent <id>`).
- `08_SHARED/*` is readable by all agents; WRITES go through the
  Memory Keeper consolidation path.
- Nothing is deleted: forgetting = `status=obsolete` + reason, history in git.

## Governance invariants

1. **AI proposes → deterministic Rust decides.** The Governor and every
   specialist propose; policy engines, planners and ledgers decide.
2. **RBAC is enforced at dispatch**: the Governor cannot route a task to an
   agent whose `scopes` exclude it.
3. **QA verdicts are binding** ("feature incomplete" blocks until resolved).
4. **Security proposes, never self-applies** critical changes.
5. **Approval-required lists** gate auth/trust/credential/breaking changes
   behind human (+Architect) sign-off.

## Application teams

The same pattern composes per product: an APP Governor with Frontend/
Backend/AI agents + QA/Security/Ops — independent from the Fabric team,
sharing only the registry format and the vault layout.

## Roadmap position

Landed before M15 so that Autonomous Pressure (M15), Agent Gateway (M16)
and Collective Memory (V2) already have an organization to attach to:

```text
M14.x Agent Operating System (this) → M15 Pressure+Fairness → M16 Gateway
→ … → AI Colony
```
