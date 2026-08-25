# DecentraAI — API Reference

> Live endpoints exposed by a running node (`http://<node>:<api_port>`).
> Auth: `Authorization: Bearer <token>`. Master token is admin; `dca_…`
> consumer keys are scoped (quota + rate limit) and can drive `/v1/governor/execute`.

## Public / UI
| Method | Path | Notes |
|--------|------|-------|
| GET | `/` | Node dashboard (operator) |
| GET | `/fabric` | Live fabric dashboard (agent-first landing) |
| GET | `/flow` | Animated live fabric pipeline |
| GET | `/ui2` | Dashboard v2 |
| GET | `/status` | Node status (model loaded, engine) |
| GET | `/v1/token` | Loopback master token (for the dashboard JS) |

## Fabric intelligence / Governor (operator+)
| Method | Path | Notes |
|--------|------|-------|
| POST | `/v1/governor/execute` | Resource-aware decision + execution. **Accepts `dca_` keys.** Body: `{task_id, task_kind, instruction, content}`. Returns `execution_id`, verdict (LOCAL/DISTRIBUTED/QUEUE/REJECT), model_selected, per_worker, credited_workers, reduce_status. |
| POST | `/v1/model-parallel` | Explicit map-reduce (serial baseline + speedup). |
| POST | `/v1/intel/plan` | Intelligence layer: proposal only, never a command. |
| POST | `/v1/intel/assist` | Offload one capability task via DFCP (Sharing is Caring). |
| GET | `/v1/intel/status` | Intelligence layer status. |

## Compute / Pool
| Method | Path | Notes |
|--------|------|------|
| POST | `/v1/pool/bench` | Partition a workload across nodes, batched DFCP. `capability: "embeddings"` or `"chat"`. |
| GET | `/v1/compute` | Compute advertisements / workers. |
| GET | `/v1/peers` | Tracked peers (verified/failed chunks, score, banned). |
| GET | `/v1/fabric` | Fabric graph / digital twin (operator+). |
| GET | `/v1/network` | Network links. |
| GET | `/v1/execution` | Recent executions. |

## Models (Model Colony)
| Method | Path | Notes |
|--------|------|------|
| GET | `/v1/models` | Served models. |
| GET | `/v1/models/{id}` | Model detail. |
| GET | `/v1/models/intel` | Model intelligence / capabilities. |
| POST | `/v1/models/route` | Route a request to a model (deterministic). |
| POST | `/v1/models/governance` | Governance transition (operator+). |

## Agents / Collective
| Method | Path | Notes |
|--------|------|------|
| GET | `/v1/agents` | Logical + remote agents. |
| POST | `/v1/agents/onboard` | Onboard an agent. |
| GET | `/v1/agents/capabilities` | Agent capability search. |
| POST | `/v1/agents/orchestrate` | Orchestrate agents. |
| POST | `/v1/agents/workflow` | Multi-stage workflow; each stage drives the Governor. |

## Evidence & Economy
| Method | Path | Notes |
|--------|------|------|
| GET | `/v1/evidence` | Evidence index totals + recent entries. |
| POST | `/v1/evidence/query` | Structural evidence query. |
| GET | `/v1/evidence-chain` | Evidence chain for an execution_id. |
| GET | `/v1/credits/balance` | Credit balances per worker. |
| GET | `/v1/credits/events` | Credit ledger events. |
| POST | `/api/admin/quota/grant` | Fund a consumer account (master). |
| GET | `/api/admin/consumer-key/list` | List consumer keys (master). |

## Inference (OpenAI-compatible)
| Method | Path | Notes |
|--------|------|------|
| POST | `/v1/chat/completions` | Chat. |
| POST | `/v1/completions` | Completions. |
| POST | `/v1/embeddings` | Embeddings (dedicated backend). |

## MCP
| Method | Path | Notes |
|--------|------|------|
| POST | `/mcp` | JSON-RPC 2.0 MCP server. Tools: decide, execute_decision, list_workers, list_peers, fabric graph, evidence, compute request, embeddings. Consumer keys get decide+execute. |

## Consumer (BYOA) flow
1. `decentraai consumer-key create --account my-agent --quota-ceiling 5000 --scopes inference` → `dca_…`
2. `POST /api/admin/quota/grant {account, amount}` (master) → fund quota
3. Agent drives fabric: `POST /v1/governor/execute` with `Authorization: Bearer dca_…`