# Skill: mcp — Model Context Protocol surface

## Two doors, one fabric

```text
External agent ──┬── OpenAI-compatible API (/v1/chat/completions,
                 │   /v1/models, /v1/embeddings)
                 └── MCP (/mcp, JSON-RPC 2.0)
                            │ Bearer credential (dca_ consumer key /
                              master token)
                            ▼
                      Fabric + policy
```

## Protocol

MCP `2025-06-18`, JSON-RPC 2.0 over HTTP POST at `/mcp`. Implementation:
`crates/runtime/src/mcp.rs` (I/O-free translation layer; the HTTP handler
builds an `McpContext` snapshot from live state).

## Auth split

- Consumer keys (`dca_…`): `decide` (read-only planning projection) and
  `execute_decision` (quota-gated mutation with per-key rate limiting).
- Operator/master: control-plane tools (`list_workers`, `list_models`,
  `list_executions`, capability search, fabric graph, serve/pull model).

## Rules

1. **MCP tools are capabilities, not authority.** A tool may request an
   action; it cannot skip policy/trust/reservation/artifact verification.
2. Tool exposure is filtered by credential scope — an agent scoped to
   embeddings never sees admin or worker-management tools.
3. Telemetry stays counters-and-latencies; never prompts or outputs.
