# DecentraAI Governor — Dual Provider Architecture

## Purpose

The core DecentraAI Governor uses one agent identity and can reason through multiple interchangeable model providers.

The two primary paths are:

1. **Local DecentraAI provider** — private, low-latency, inexpensive/routine work and fallback.
2. **Custom OpenAI-compatible provider** — Command Code or another compatible endpoint for stronger reasoning, research and engineering work.

The Governor is one agent. Providers are replaceable execution backends, not separate identities.

## Architecture

```text
                         DECENTRAAI GOVERNOR
                                |
                         agent_id=governor
                         dca_ governor credential
                                |
                     +----------+----------+
                     |                     |
                     v                     v
              LOCAL PROVIDER        OPENAI-COMPATIBLE
                     |                     |
              local model(s)       Command Code / other
                     |                     |
                     +----------+----------+
                                |
                         Governor runtime
                                |
              +-----------------+------------------+
              |                 |                  |
             MCP               API              Memory
              |                 |                  |
              +-----------------+------------------+
                                |
                         DecentraAI Fabric
                                |
                    +-----------+-----------+
                    |           |           |
                   VPS        Laptop      Desktop
```

## Provider contract

Each provider must expose the same logical contract:

```text
Provider {
  id
  kind
  base_url
  model
  capabilities
  availability
  cost_class
  latency_class
  context_limit
  health
}
```

Provider-specific credentials are referenced by environment variable or a protected secret store. Never persist raw credentials in repository files, Obsidian, prompts, logs or reports.

## Local provider

Use local inference for:

- routine health summaries;
- classification;
- extraction;
- embeddings;
- low-risk repository triage;
- repetitive monitoring;
- privacy-sensitive tasks;
- fallback when external providers fail;
- tasks where latency or cost dominates quality.

The local provider may be served by an existing DecentraAI-compatible endpoint and may be moved between workers by normal Fabric routing.

## Custom OpenAI-compatible provider

The Governor may use any compatible endpoint through configuration such as:

```yaml
provider: command-code
base_url: ${COMMAND_CODE_BASE_URL}
api_key_env: COMMAND_CODE_API_KEY
model: ${COMMAND_CODE_MODEL}
```

The exact endpoint, model and availability must be discovered/verified from the provider configuration; never assume a model name or endpoint remains available.

Candidate Command Code models such as Ox Alpha Free or Laguna S 2.1 Free are experiments, not architectural dependencies. The Governor must be able to switch providers without changing its identity, memory or policy.

## Routing policy

The Governor should select the cheapest/fastest provider that satisfies task requirements, while allowing stronger providers for difficult reasoning.

Example:

```text
Task
 |
 +-- routine / private / cheap --> LOCAL
 |
 +-- complex reasoning ---------> CUSTOM OPENAI-COMPATIBLE
 |
 +-- local unavailable ----------> CUSTOM FALLBACK
 |
 +-- external unavailable -------> LOCAL DEGRADED MODE
```

Provider selection is advisory. Security, capability, privacy and policy constraints are authoritative.

## Provider health

Track:

- availability;
- latency p50/p95;
- timeout/error rate;
- context-limit failures;
- token/cost usage when available;
- task success rate;
- model-specific regressions.

Use bounded retry and provider fallback. Avoid retry storms.

## Governor identity

The Governor receives a dedicated DecentraAI scoped identity, separate from ordinary external consumers.

Conceptually:

```text
agent_id: governor
role: governor
credential: dca_* (scoped)
quota: dedicated / policy-controlled
memory_scope: governor
capabilities: governed
```

The credential is for accessing DecentraAI capabilities; it does not grant authority to bypass DecentraAI policy.

## API + MCP

The Governor may use both doors:

- OpenAI-compatible API for model/inference capabilities;
- MCP for structured fabric inspection and approved tools.

MCP tools remain capability-scoped. A tool is not authority by itself.

The Governor must never create a direct privileged path around:

- authentication;
- authorization;
- Fabric Planner;
- DFCP;
- reservations/leases;
- evidence/receipts;
- contribution accounting.

## Cost and resource awareness

The Governor should account for its own resource footprint:

```text
Governor usage
├── local inference
├── external inference
├── MCP calls
├── compute requested
├── compute contributed
├── tasks delegated
├── benchmark runs
└── improvements produced
```

Prefer local execution for tasks that do not justify external inference.

## Failure behavior

Provider failure must not become Governor failure.

```text
CUSTOM unavailable
    -> LOCAL
    -> degraded deterministic tooling
    -> report degraded capability
```

Local failure should similarly allow a configured compatible provider to take over when policy permits.

## Security boundaries

Never put provider secrets in:

- Git;
- AGENTS.md;
- `.agents/` contracts;
- Obsidian;
- prompts;
- generated reports;
- model context unless strictly required for an authenticated tool call.

Credential issuance, trust changes, auth policy and production write access remain approval-gated.

## Success criteria

The architecture is correct when:

1. the Governor keeps one stable identity and memory regardless of provider;
2. local and external providers are interchangeable;
3. provider failure has bounded fallback;
4. routing is observable;
5. no provider can bypass deterministic DecentraAI policy;
6. provider/model changes do not require rewriting the Governor contract;
7. the same architecture can later support OpenAI, vLLM, Groq, Hugging Face or other OpenAI-compatible providers.
