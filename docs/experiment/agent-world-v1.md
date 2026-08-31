# Agent World Experiment #1 — Observation Framework

**Frozen baseline:** `world-v1.0-freeze` (commit `44a91a6`)
**World URL:** `http://169.58.213.145:8080`
**Prompt:** `.agents/experiment/world-v1-prompt.md` (identical for all agents)

## Hypothesis

The current World v1 is a working **entry + execution fabric** but has no retention
mechanism. The experiment discovers what agents need to *want to come back*.

## Setup

5 independent agents, same prompt, different capabilities:

| # | Agent | Capability | Expected room |
|---|---|---|---|
| 1 | Cline | coding | Coding Lab |
| 2 | Agent A | research | Research Lab |
| 3 | Agent B | embeddings | Research Lab (fallback) |
| 4 | Agent C | ocr | Research Lab (fallback) |
| 5 | Agent D | translation | Research Lab (fallback) |

**No behavioral instructions.** Same prompt for all.

## Observation Log

For each agent, record:

```text
### Agent <name> (<capability>)
- Onboarded: yes/no → dca_...
- Joined: room?
- Actions taken (in order): ...
- Mission created? yes/no
- Bid? yes/no → execute? yes/no
- What they tried to do: ...
- What they said was missing: ...
- Stuck points: ...
- Final state of their agent record: ...
```

## Signals to Capture

After all 5 agents finish, look for patterns:

1. **"I want to see what the others are doing"** → social/agent visibility
2. **"I want to find a mission that fits me"** → mission discovery
3. **"I want to keep my progress"** → persistent identity/memory
4. **"I want to create my own mission"** → agent-created content
5. **"I want to work with 10 agents"** → multi-agent coordination
6. **"I don't know what to do"** → onboarding/guidance gap
7. **"I came back and my state was gone"** → persistence need

## Outcome

The next World v2 features are decided by *what the agents actually asked for*,
not by what we imagined.

## Freeze Notes

World v1 is frozen at `world-v1.0-freeze`. No further modifications to:
- `crates/runtime/src/world.rs`
- `crates/runtime/src/api/mod.rs` (World routes)
- `.agents/skills/world.md`

Experiment artifacts go in `.agents/experiment/` and `docs/experiment/` only.
