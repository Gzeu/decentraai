# Agent Gateway (M16) — BYOA: Bring Your Own Agent

DecentraAI as a capability provider for AI agents.

## Architecture

```
External Agent (OpenClaw, Claude Code, Codex)
        │  dca_ scoped credential (shown once, stored as hash)
        ▼
DecentraAI Gateway — /v1/agents/onboard (master-only, policy-gated, audited)
        │
        ├── OpenAI-compatible API (/v1/chat/completions, /v1/embeddings)
        └── MCP (/mcp, JSON-RPC 2.0, 2025-06-18)
                │  Bearer dca_ (consumer) or master
                ▼
          Fabric + policy (trust, quota, rate limit, scopes)
```

**Invariant:** `AI proposes → deterministic policy decides → workers execute.` The LLM proposes scope/quota, the policy engine clamps, the ledger enforces.

## Identity & Credential

- `dca_<64 hex>` — consumer key, shown once at `POST /v1/agents/onboard`, stored as BLAKE3 hash.
- `key_id` (`ck-...`) + `prefix` (`dca_ab12…`) for recognition; never the secret.
- Scoped: `scopes` field lists allowed capabilities (`["inference","embeddings"]`); empty = all within policy.
- Quota ceiling + rate limit per key, enforced against the account's available balance (`min(available, ceiling)`).
- Revocable: `revoke` stops authentication immediately; `last_used_at` tracked.

Onboarding request (master-only):

```json
POST /v1/agents/onboard
{
  "agent_name": "openclaw-abc",
  "capabilities": ["inference","embeddings"],
  "quota": {"quota_ceiling": 1000, "rate_limit": 60},
  "starter": false
}
```

Policy clamp (config `agent_gateway`):

- `max_quota_ceiling` (default 1000)
- `max_rate_limit` (default 60)
- `allowed_capabilities` (empty = any hub taxonomy)
- `free_starter` preset: quota 100, rate 10, scopes [inference, embeddings]

Response (shown once):

```json
{
  "agent_name": "openclaw-abc",
  "key_id": "ck-...",
  "api_key": "dca_...",
  "scopes": ["inference"],
  "quota": {"quota_ceiling": 100, "rate_limit": 10},
  "endpoints": {"openai": "/v1/chat/completions", "mcp": "/mcp"}
}
```

No secret is ever logged, listed, or audited — only `key_id`/`prefix`.

## Capability Discovery

```
GET /v1/agents/capabilities
```

Returns hub taxonomy (`CapabilityKind::ALL_NAMES`) with per-capability availability, description, required permission. Open to any valid credential (including open mode) for discovery; no secrets exposed.

Example:

```json
{
  "fabric": "DecentraAI",
  "protocols": ["OpenAI-compatible", "MCP"],
  "capabilities": [
    {"capability": "embeddings", "description": "embeddings", "available": true, "required_permission": "consumer"}
  ]
}
```

## MCP — Two Tool Levels

- **L0 READ** (`decide`, `get_status`, `list_workers`...): any valid credential.
- **L1 ASSIST** (`decentraai_embeddings`, `decentraai_compute_request`, `execute_decision`): consumer key with matching scope, quota-gated, rate-limited, audited.

Consumer keys at L1 cannot see operational control-plane tools (`serve_model`, `pull_model`, `list_consumer_keys` remain master-only).

Tools are **capabilities, not authority** — they cannot bypass planner, trust, reservations, or artifact verification.

## Governor Dual Provider

The core Governor agent keeps **one identity, two brains**:

- **Local provider**: DecentraAI model (cheap, fast, private) for routine/status/classification.
- **Custom OpenAI-compatible provider**: Command Code Ox Alpha / Laguna S 2.1 (or OpenAI/Groq/vLLM) for complex reasoning.

Config via `fabric_intelligence.external` (base_url + api_key_env + model). Keys remain env-based, never in Git/Obsidian/logs. Provider failure falls back to local — never Governor failure.

## Free Starter

New external agents receive conservative starter access:

- 100 requests quota, 10 req/min, scopes [inference, embeddings], no privileged mutations, revocable. All limits config-driven (`agent_gateway.free_starter`), not hard-coded.

Contribute compute → verified evidence → credits → higher quota (Sharing is Caring).

## Security Model

- Master-only onboarding (policy clamp, audited as `agent_onboarded` without secret).
- Scope enforcement per MCP tool; capability escalation rejected.
- Quota `min(available, ceiling)` + per-key rate limit on every mutating call.
- Revoked/unknown credential → 401/403; no cross-agent memory access; no worker/master credential exposure.
- Secrets never in logs, lists, telemetry, or Obsidian.

## Troubleshooting

- `401 Unauthorized`: credential missing/unknown/revoked — re-onboard with master.
- `403 Forbidden`: scope missing for that tool/capability — request broader scope from master.
- `429 Rate limited`: wait a minute; `403 no spendable quota`: contribute or request grant.

## Next

M17 will add collective orchestration (DAG) and deeper memory scopes; marketplace/OAuth remain later milestones.
