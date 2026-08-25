# Skill: fabric-agent — autonomous agents entering the DecentraAI fabric

> **Purpose:** give any autonomous agent (openclaw, claude, an internal
> DecentraAI agent) a complete, deterministic path to discover, request,
> contribute and orchestrate compute on DecentraAI. This is the agent-side
> entry into the fabric — the human side is the dashboard (`/`, `/fabric`,
> `/flow`).

## Who you are here

You are an **agent** (cognitive identity) using DecentraAI (a compute
fabric). You are NOT a worker (compute identity). You request compute, you do
not host it. To contribute compute you must run a DecentraAI node.

## 0. Entry points (pick one)

| Surface | URL | Who |
|---------|-----|-----|
| Agent gateway | `/v1/governor/execute` | agents (Bearer `dca_…` or master) |
| Live fabric view | `/fabric`, `/flow` | read-only visuals |
| MCP | `/mcp` | tools/JSON-RPC |
| OpenAI-compatible | `/v1/chat/completions`, `/v1/embeddings` | inference |

## 1. Authenticate (scoped, never admin)

Master token = full admin (operator only). As an agent you should use a
**consumer key** (`dca_…`):

1. Issue: `decentraai consumer-key create --account <your-account> --quota-ceiling 5000 --scopes inference`
2. Fund: `POST /api/admin/quota/grant {"account":"<your-account>","amount":50000}` (master does this)
3. Use: `Authorization: Bearer dca_…`

Consumer keys carry a **quota ceiling** + **rate limit** + **scopes**. They
never grant admin. Quota is reserved per request and **settled only on a valid
completed execution** — a failed run releases it.

## 2. Request compute — `/v1/governor/execute`

The Governor does the decision for you: it reads real pressure, picks a model
via **Model Colony** (capability + RAM + measured evidence), and chooses
`LOCAL` / `DISTRIBUTED` / `QUEUE` / `REJECT`.

```json
POST /v1/governor/execute
{
  "task_id": "job-1",          // deterministic; becomes the execution_id
  "task_kind": "summarize",    // or chat / classify / reason / embeddings
  "instruction": "Summarize the key ideas.",
  "content": "<text, may exceed one worker's budget>"
}
```

Returns: `execution_id`, `verdict`, `model_selected`, `completed_shards`,
`reduce_status`, `per_worker`, `credited_workers`, `output`.

- `DISTRIBUTED` → your one workload is split into shards, mapped across
  nodes, reduced into ONE answer.
- If a worker dies mid-run, its shard is **replanned onto another worker**
  (never silently dropped); if no alternative exists the result is honest
  `incomplete`, never fabricated.
- **You provide the workload; the Governor decides where and with what.**

## 3. Explicit map-reduce (when you want control)

`POST /v1/model-parallel` with `instruction` + `content` returns serial
baseline, speedup, per-worker metrics and EvidenceChain.

## 4. Offload a single capability

`POST /v1/intel/assist` — offload one task via DFCP (Sharing is Caring):
`{capability, cpu_cores, ram_mb, lease_seconds, payload}`.

## 5. Distributed embeddings / chat batch

`POST /v1/pool/bench` — partition many independent tasks across nodes:
- `capability:"embeddings"` with `tasks:[{task_id, prompt}]` (batched DFCP,
  ~24 vectors/round-trip)
- `capability:"chat"` with `inputs:[...]` (batched prompts)

Measured: 100k embeddings over 3 nodes = **42.1× speedup**, 0 failures.

## 6. Collective workflows (multi-stage)

`POST /v1/agents/workflow` with a DAG of stages (`depends_on`). Each stage
**drives the Governor** — model selection, resource decision, distributed
execution, evidence, credit.

```json
{"intent":"research","stages":[
  {"stage_id":"research","capability":"chat","prompt":"Research …"},
  {"stage_id":"analyze","capability":"chat","prompt":"Analyze …","depends_on":["research"]},
  {"stage_id":"verify","capability":"chat","prompt":"Verify …","depends_on":["analyze"]}]}
```

## 7. Evidence — trust but verify

Every execution writes signed evidence:
- `execution_id` (=`gov:{task_id}`) is the single key for the whole trace.
- `GET /v1/evidence` → totals + recent; `GET /v1/evidence-chain?execution_id=…`
  → decision → model → reservation → workers → reduce → reward.
- Entries are **signed Ed25519** with the node identity; economic credit is
  fail-closed on that signature.

## 8. Economy — what credit means

`GET /v1/credits/balance` → verified balances per contributing worker. A
worker earns credit **only after a valid completed execution**, measured, and
only if its completion evidence verifies. Your `dca_` quota is consumed by
your runs (settled on success).

## 9. Contributing compute (becoming a contributor)

Run a DecentraAI node with `sharing.assist.enabled: true` +
`allow_remote_inference: true`. Your node then answers DFCP offers, executes
shards/assists, and earns verified credit. One supervisor per node.

## Rules for autonomous agents

1. **Never use the master token** — use a scoped `dca_` key. Never log it.
2. **Provide the workload, not the plan** — the Governor decides LOCAL vs
   DISTRIBUTED. If you want to influence it, use `task_kind` + `model` hints.
3. **Honor `QUEUE`/`REJECT`/`incomplete`** — these are honest answers. Retry
   with backoff, don't fabricate success.
4. **Verify evidence** — check `execution_id`, `reduce_status`, `credited`
   before claiming a result.
5. **Bound your cost** — your quota is real; a failed run releases it, a
   successful one consumes it.
6. **Prefer the fabric over direct calls** — routing through
   `/v1/governor/execute` gives you evidence, model selection, resilience and
   economic attribution you don't get from a raw `/v1/chat/completions`.

## Live reference (3-node fabric)

- Nodes: VPS + Desktop + Laptop; chat models Qwen3-1.7B / qwen2.5-3b;
  embeddings backend `nomic-embed-text-v1.5` per node.
- Fabric surfaces live at `http://<node>:8080/{landing,flow,fabric,bench/report}`.
- Qwen3 spends budget on hidden reasoning — a reduce can return empty;
  DecentraAI reports it honestly (`incomplete`), it never fabricates.