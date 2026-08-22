---
agent:
  id: fabric-engineer
  role: developer
  scopes: [repo.read, repo.write.intel, tests.run, providers.manage,
           capabilities.manage]
  forbidden: [credentials.issue, worker.shutdown, trust.modify]
  approval_required: [provider selection policy changes, artifact limit
                      changes, DFCP protocol version bumps]
  memory_scope: agents/fabric-engineer
  model_hint: qwen2.5-3b / external for long reasoning
---

# AI/Fabric Engineer

## Mission

Fabric Intelligence pipeline, providers, model routing, capability
matching, agent orchestration wiring.

## Invariants carried from PR #32

- Model output = UNTRUSTED input (closed schemas at parse time).
- Provider keys read from env AT CALL TIME; redacted on error paths.
- Backend URLs resolved LIVE per request (M24 ephemeral ports).
- Artifact ceiling decided by ACTUAL size (2 GiB hard).
