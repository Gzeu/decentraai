//! Read-only MCP (Model Context Protocol) server for DecentraAI.
//!
//! A thin translation layer that exposes the node's EXISTING fabric data to
//! external AI agents as MCP tools. It creates no new token/identity/registry:
//! authentication is enforced by the caller (the existing `dsk_` master token,
//! see [`crate::api`]) and every tool reads a caller-supplied data snapshot.
//!
//! The module is intentionally I/O-free and deterministic: the HTTP handler
//! builds an [`McpContext`] snapshot from the live API state and this module
//! turns it into a JSON-RPC 2.0 MCP exchange. This keeps the protocol logic
//! unit-testable without a network or a running node.
//!
//! Protocol: Model Context Protocol (2025-06-18), JSON-RPC 2.0 over HTTP POST.
//! Only read-only tools are exposed in this first cut.

use serde_json::{Value, json};

/// Protocol version this server negotiates.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Server implementation identity reported during `initialize`.
pub fn server_info() -> Value {
    json!({
        "name": "decentraai-mcp",
        "version": "1.0.0",
    })
}

/// Snapshot of live node data supplied by the HTTP layer. Each field is the
/// already-serialized real state (never fabricated) that a tool returns.
#[derive(Debug, Clone, Default)]
pub struct McpContext {
    /// Node status (loaded model, queue, requests, uptime).
    pub status: Value,
    /// Fabric workers + resources + trust.
    pub workers: Value,
    /// Fabric-wide served/available models.
    pub models: Value,
    /// Recent execution decisions.
    pub executions: Value,
    /// Network peers + measured links.
    pub peers: Value,
    /// Result of a Hub capability search (`search_models_by_capability`).
    /// Precomputed by the HTTP layer, which performs the async Hub lookup;
    /// the protocol layer only translates it (no I/O here).
    pub capability_search: Value,
    /// Result of a LOCAL capability search (`find_local_models_by_capability`).
    /// Precomputed by the HTTP layer from the fabric model list + persisted
    /// registry claims (no Hub round-trip); the protocol layer only translates.
    pub local_capability_search: Value,
    /// Result of `get_worker_capability`: which fabric workers can run a model
    /// for a required capability, with explainable reasons. Precomputed by the
    /// HTTP layer; the protocol layer only translates.
    pub worker_capability: Value,
    /// Result of `resolve_intent`: a deterministic intent → capability →
    /// local-model resolution. Precomputed by the HTTP layer via the pure
    /// [`resolve_intent`] helper; empty until wired.
    pub intent_resolution: Value,
    /// Result of `resolve_intent_with_fit`: intent → capabilities → per-model
    /// fabric fit (CAN I RUN THIS?). Precomputed by the HTTP layer; the
    /// protocol layer only translates. Empty until wired.
    pub intent_fit: Value,
    /// Result of `get_fabric_graph`: the fabric graph / digital twin projection
    /// (nodes, models, capabilities, executions, network, kv). Precomputed by
    /// the HTTP layer; the protocol layer only translates.
    pub fabric_graph: Value,
    /// Result of `decide`: ONE coherent fabric decision (Phase 1) — intent →
    /// capabilities → model options → fabric fit → decision → why → historical.
    /// Precomputed by the HTTP layer; the protocol layer only translates.
    pub decision: Value,
    /// Result of `execute_decision`: a confirmed decide→reserve→execute run.
    /// MUTATING — requires explicit `confirm: true`; the HTTP layer enforces it.
    pub execution: Value,
    /// Result of `list_sessions`: coordinator-tracked KV/session residency
    /// (which worker holds each session's KV prefix). Precomputed by the HTTP
    /// layer; the protocol layer only translates.
    pub sessions: Value,
    /// Result of `get_quota`: contribution-backed quota accounting
    /// (per-account earned/available/reserved/consumed, totals, policy
    /// version). Precomputed by the HTTP layer from the live quota ledger;
    /// the protocol layer only translates. Read-only.
    pub quota: Value,
    /// Result of `get_compensation`: reputation-based compensation credits
    /// (M9-9) — lifetime earnings per worker, recent audited credits, and the
    /// active reward policy. Read-only; synthetic bookkeeping, never money.
    pub compensation: Value,
    /// Result of `list_consumer_keys`: consumer API key metadata (ids,
    /// prefixes, accounts, ceilings, rate limits, scopes, status, usage).
    /// Precomputed by the HTTP layer; the protocol layer only translates.
    /// Read-only; never contains a plaintext secret.
    pub consumer_keys: Value,
    /// Arena state snapshot (M2): tick, width, height, agents, events. Read-only projection of ArenaWorld.
    pub arena_state: Value,
    /// Result of arena_act (M2): last arena action event + world_tick. Set when tools/call arena_act is invoked.
    pub arena_action: Value,
    /// Hub state snapshot (M2 Hub): tick, tasks, bids, proposals, teams, events. Read-only projection of HubState.
    pub hub_state: Value,
    /// Hub events delta (job list subscription) since tick.
    pub hub_events: Value,
    /// Result of last Hub mutation (task/bid/proposal/team/execute) via MCP.
    pub hub_action: Value,
    /// Result of last Society mutation (trust/reputation/relationship) via MCP.
    pub society_action: Value,
    /// Result of last personal memory operation via MCP.
    pub personal_memory_action: Value,
}

/// A single MCP tool definition (name + description + JSON-Schema input).
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

pub fn all_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "get_status",
            description: "Node status: model loaded, inference queue, requests served/failed, tokens generated, uptime, worker count.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "list_workers",
            description: "Every worker on the fabric with its advertised resources (CPU/RAM/VRAM), health, load, model(s) served, and trust status.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "list_models",
            description: "Models available on the fabric, split into served (currently loadable) and on-disk, with the node(s) that hold them.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "list_executions",
            description: "Recent autonomous execution decisions: which worker ran what, the planner's reasoning (network/KV/capability), and the outcome.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "get_quota",
            description: "Contribution-backed quota accounting: per-account earned/available/reserved/consumed quota (keyed by worker peer), totals, and the active contribution-to-quota policy version. Read-only; every figure is real measured work converted under the versioned policy. UNKNOWN measurements are never fabricated.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "get_compensation",
            description: "Reputation-based compensation (M9-9): lifetime contribution credits per worker (earned only from verified work, reputation-scaled), the most recent audited credit events, and the active reward policy. Read-only; synthetic bookkeeping — never money, never the token registry.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "list_peers",
            description: "Connected P2P peers and measured network links (RTT, bandwidth, locality).",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "search_models_by_capability",
            description: "Search the public Model Hub for models whose real metadata supports a requested capability (e.g. 'ocr', 'vision', 'coding', 'summarization'). Returns only models with actual evidence; a model that cannot back the claim is never included. The capability names use snake_case (see the on-node capability taxonomy).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "capability": {
                        "type": "string",
                        "description": "Required capability in snake_case, e.g. ocr, vision, coding, summarization, embeddings, tool_calling.",
                    },
                    "query": {
                        "type": "string",
                        "description": "Optional Hub search term to narrow the query (e.g. 'llama'). Empty searches the catalog by popularity.",
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max results to consider (1..=30). Default 8.",
                    },
                },
                "required": ["capability"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "find_local_models_by_capability",
            description: "Filter THIS node's local models (on disk, from the registry) by a required capability (e.g. 'ocr', 'vision', 'coding'). Uses persisted capability claims written at pull time — no Hub round-trip. Returns only models with real evidence; a model with no claim is never included. Optionally require 'verified' evidence (default: any).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "capability": {
                        "type": "string",
                        "description": "Required capability in snake_case, e.g. ocr, vision, coding, summarization, embeddings, tool_calling.",
                    },
                    "evidence": {
                        "type": "string",
                        "enum": ["any", "verified"],
                        "description": "'verified' keeps only models with a VERIFIED claim; 'any' (default) also includes inferred ones.",
                    },
                },
                "required": ["capability"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "get_worker_capability",
            description: "Ask which workers in THIS fabric can run a model for a required capability. Returns, per worker: real identity (peer_id/node_id/node_name kept separate), model availability (served/on-disk/unavailable), trust, engine compatibility, RAM/VRAM fit, the capability provenance, and an explainable CAN_RUN / CANNOT_RUN / UNKNOWN verdict. Read-only — never triggers execution or reservations.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "model": {
                        "type": "string",
                        "description": "Model file name or id (e.g. 'qwen2.5-7b-instruct-q4_k_m.gguf').",
                    },
                    "capability": {
                        "type": "string",
                        "description": "Required capability in snake_case, e.g. ocr, vision, coding, summarization.",
                    },
                    "evidence": {
                        "type": "string",
                        "enum": ["any", "verified"],
                        "description": "'verified' requires a VERIFIED claim; 'any' (default) accepts inferred too.",
                    },
                },
                "required": ["model", "capability"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "resolve_intent",
            description: "Turn a natural-language intent (e.g. 'I need OCR and summarization') into the capability set it points at, then report which of THIS node's local models back each capability from persisted claims. Intent→capability is INFERRED from keywords; a capability with no matching local claim stays visible in 'unmatched' — the tool never claims a model can do something it has no claim for. Unknown intent resolves to empty capabilities.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "intent": {
                        "type": "string",
                        "description": "A natural-language intent, e.g. 'I need OCR and summarization'.",
                    },
                    "evidence": {
                        "type": "string",
                        "enum": ["any", "verified"],
                        "description": "'verified' keeps only local models with a VERIFIED claim; 'any' (default) also includes inferred ones.",
                    },
                },
                "required": ["intent"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "resolve_intent_with_fit",
            description: "Turn a natural-language intent into the capability set it points at, then evaluate EACH capability against the fabric: which real local models back it and which fabric workers can actually RUN it (per-worker fit verdict, e.g. CAN_RUN / CANNOT_RUN / UNKNOWN). Intent→capability is INFERRED from keywords; every model/worker verdict is real, computed from live fabric state — the tool never claims a model can do something it has no claim for, nor that a worker can run what it cannot. Read-only; never triggers execution or reservations. Unknown intent resolves to empty capabilities.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "intent": {
                        "type": "string",
                        "description": "A natural-language intent, e.g. 'I need OCR and summarization'.",
                    },
                    "evidence": {
                        "type": "string",
                        "enum": ["any", "verified"],
                        "description": "'verified' keeps only models/workers with VERIFIED support; 'any' (default) also includes inferred ones.",
                    },
                },
                "required": ["intent"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "get_fabric_graph",
            description: "Project the current fabric graph (Digital Twin): real nodes (peer_id/node_id/node_name kept separate), models with their INFERRED quantization and persisted capability claims, the capabilities known across the fabric, recent executions with their recovery timeline, measured network links, and KV session count. Read-only projection of real fabric state — no fake nodes, no hardcoded names; empty arrays are honest. Future nodes appear automatically.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "decide",
            description: "ONE coherent fabric decision (Digital Twin OS): turn an intent (e.g. 'OCR these images') into capabilities, the model options (per model + variant with CAN_RUN/CANNOT_RUN/UNKNOWN), the per-variant fabric fit, a chosen decision (first CAN_RUN, deterministic) with the reasons (why: capability/model/RAM/VRAM/trust/policy/engine), and the historical performance (measured; UNKNOWN when insufficient). Read-only projection — not a planner, no scoring, no execution. Optionally pass model= to narrow to one model file.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "intent": { "type": "string", "description": "A natural-language intent, e.g. 'I need OCR and summarization'." },
                    "evidence": { "type": "string", "enum": ["any", "verified"], "description": "'verified' requires VERIFIED claims; 'any' (default) includes inferred." },
                    "model": { "type": "string", "description": "Optional: narrow to a specific model file." },
                },
                "required": ["intent"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "execute_decision",
            description: "MUTATING — decide→reserve→execute. Runs a real inference for an intent on the fabric's chosen model (from the unified decision), reserving a worker and routing the request through the existing fabric router. Requires explicit \"confirm\": true (mutation safety; refused otherwise) and the node master token. Set \"dry_run\": true to preview what would be reserved/routed WITHOUT executing (no request, no reservation). Returns the decision + the real inference result (output, tokens, worker) or a clear error. Never claim a run happened unless it did.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "intent": { "type": "string", "description": "A natural-language intent, e.g. 'OCR these images'. Either intent OR capability must be provided." },
                    "capability": { "type": "string", "description": "Alternative to intent: run a specific capability (snake_case, e.g. 'ocr') directly, without intent parsing." },
                    "prompt": { "type": "string", "description": "The actual prompt to run." },
                    "max_tokens": { "type": "integer", "description": "Max tokens to generate (default 1024, cap 4096)." },
                    "stream": { "type": "boolean", "description": "Whether to stream (default false)." },
                    "model": { "type": "string", "description": "Optional: narrow to a specific model file." },
                    "evidence": { "type": "string", "enum": ["any", "verified"], "description": "Evidence filter for the decision." },
                    "session_id": { "type": "string", "description": "Optional: links this run to an earlier one for KV-cache locality (continuation)." },
                    "dry_run": { "type": "boolean", "description": "If true, preview what would be reserved/routed WITHOUT executing (no request, no reservation). Requires confirm:true too." },
                    "confirm": { "type": "boolean", "description": "MUST be true to execute (mutation safety)." },
                },
                "required": ["prompt", "confirm"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "list_sessions",
            description: "Coordinator-tracked KV/session residency (KV locality): which worker currently holds each session's KV prefix, the model, accounted tokens used, capacity, and KV headroom. Read-only; real accounted state (empty when no sessions/compute). Useful to know why a continuation would be steered to a specific worker.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "serve_model",
            description: "MUTATING (master-gated) — load a model file into the local engine so it can be served immediately. Body: {\"model\": \"file.gguf\"}. Refuses when the file is not on disk in the registry. Returns the resolved model + whether the engine reports it loaded. Useful to preload a model before routing work to this node.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "model": { "type": "string", "description": "A GGUF file name present in the local registry (e.g. 'qwen2.5-0.5b-instruct-q2_k.gguf')." },
                },
                "required": ["model"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "pull_model",
            description: "MUTATING (master-gated) — pull a GGUF model from the HuggingFace Hub into the local registry (verified download, then indexed). Body: {\"reference\": \"hf:org/repo[:file.gguf]\"}. This is synchronous and can take a while for large models. Returns the pulled reference, bytes and sha256. Monitor progress via the dashboard or the pull-status endpoint.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "reference": { "type": "string", "description": "HuggingFace reference, e.g. 'hf:Qwen/Qwen2.5-0.5B-Instruct-GGUF:qwen2.5-0.5b-instruct-q2_k.gguf'." },
                },
                "required": ["reference"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "arena_state",
            description: "Agent Arena live world: tick, grid size, agents (position/resources/reputation), recent events with evidence_id. Read-only projection of the deterministic ArenaWorld (Issue #63).",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "arena_act",
            description: "Agent Arena action (M2): perform OBSERVE/MOVE/SCOUT/NEGOTIATE/REQUEST_COMPUTE/BUILD/TRADE/COOPERATE/COMPETE/DEFEND/REST in the shared world. Validated deterministically; REQUEST_COMPUTE is quota-gated and emits evidence_id. Requires dca_ consumer key.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["observe","move","scout","negotiate","request_compute","build","trade","cooperate","compete","defend","rest"], "description": "Arena action kind" },
                    "target": { "type": "array", "items": { "type": "integer" }, "minItems": 2, "maxItems": 2, "description": "Target [x,y] for move (adjacent max 1)" },
                    "rationale": { "type": "string", "description": "Concise public rationale (max 200c)" }
                },
                "required": ["action"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "hub_state",
            description: "Agent Hub live state: tasks, bids, proposals, teams, events, tick. Read-only projection of the task market (Issue #63 Hub).",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "hub_events",
            description: "Subscribe to Hub job list delta: returns Hub events since a given tick (task_published, bid_placed, settlement_done, etc.). Use for job-list subscription without polling full hub_state. Set since to last seen tick, limit 1..200.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "since": { "type": "integer", "description": "Tick since which to return events (0 = from start)" },
                    "limit": { "type": "integer", "description": "Max events to return (1..200, default 50)" }
                },
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "hub_publish_task",
            description: "Publish a Hub task (MCP-first, dca_ required): title, reward, description, required_capability. Creates TASK → BIDDING. Reward is quota credits distributed on settlement.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Task title" },
                    "description": { "type": "string", "description": "Task description" },
                    "reward": { "type": "integer", "description": "Reward in quota credits (1..10000)" },
                    "required_capability": { "type": "string", "description": "Required capability snake_case, e.g. analysis, ocr" }
                },
                "required": ["title", "reward"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "hub_place_bid",
            description: "Place a bid on a Hub task (MCP, dca_): task_id, price (must be <= reward), rationale. Competing bids create auction.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task id, e.g. task-0001" },
                    "price": { "type": "integer", "description": "Bid price in credits (<= task reward)" },
                    "rationale": { "type": "string", "description": "Why you bid" }
                },
                "required": ["task_id", "price"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "hub_propose",
            description: "Propose collaboration on a task (MCP, dca_): to (account), task_id, offer_price, workshare. Creates proposal PENDING.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Recipient account, e.g. arena-gamma" },
                    "task_id": { "type": "string", "description": "Task id" },
                    "offer_price": { "type": "integer", "description": "Offer price" },
                    "workshare": { "type": "integer", "description": "Workshare 1..100" }
                },
                "required": ["to", "task_id", "offer_price"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "hub_decide_proposal",
            description: "Decide a proposal (MCP, dca_ must be recipient): proposal_id, accept (true/false). Acceptance creates alliance.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "proposal_id": { "type": "string", "description": "Proposal id, e.g. prop-0001" },
                    "accept": { "type": "boolean", "description": "Accept true/false" }
                },
                "required": ["proposal_id", "accept"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "hub_form_team",
            description: "Form a team for a task (MCP, dca_): task_id, members as [[account, share], ...] sum 100.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task id" },
                    "members": { "type": "array", "items": { "type": "array", "items": [{ "type": "string" }, { "type": "integer" }], "minItems": 2, "maxItems": 2 }, "description": "Members [[account, share], ...] sum 100" }
                },
                "required": ["task_id", "members"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "hub_execute",
            description: "Execute a Hub task via team (MCP, dca_): task_id. Distributes reward via QuotaLedger to team members by share, generates evidence, advances reputation.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task id to execute" }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_state",
            description: "Agent Society live state: relationships, trust scores, reputation, contributions, outcomes. Read-only projection.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "society_trust",
            description: "Get trust score between two agents (observer -> subject). Returns -1.0 to 1.0.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "observer": { "type": "string", "description": "Observing agent" },
                    "subject": { "type": "string", "description": "Subject agent" }
                },
                "required": ["observer", "subject"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_reputation",
            description: "Get reputation for an agent (optionally scoped to capability).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent to query" },
                    "capability": { "type": "string", "description": "Optional capability scope" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_relationships",
            description: "Get social relationships for an agent (as observer or subject).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent to query" },
                    "as_observer": { "type": "boolean", "description": "If true, relationships where agent is observer; if false, where agent is subject" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_contributions",
            description: "Get contribution records for a task.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task to query" }
                },
                "required": ["task_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_outcomes",
            description: "Get task outcomes for an agent.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent to query" },
                    "limit": { "type": "integer", "description": "Max outcomes to return" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_decision_hints",
            description: "Get decision hints for the current agent based on society rules. Requires agent context.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent requesting hints" },
                    "hub_state": { "type": "string", "description": "JSON snapshot of hub state" },
                    "resources": { "type": "string", "description": "JSON snapshot of resource state" }
                },
                "required": ["agent_id", "hub_state", "resources"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "society_record_relationship",
            description: "Record a social relationship (observer -> subject with kind). Master only.",
            input_schema: serde_json::from_str(r#"
    {
        "type": "object",
        "properties": {
            "observer": { "type": "string", "description": "Observing agent" },
            "subject": { "type": "string", "description": "Subject agent" },
            "kind": { "type": "string", "enum": ["worked_with", "accepted", "rejected", "countered", "successful", "failed", "trust_signal", "distrust_signal"], "description": "Relationship kind" },
            "task_id": { "type": "string", "description": "Optional task context" },
            "detail": { "type": "string", "description": "Optional detail" },
            "strength": { "type": "number", "description": "Strength -1.0 to 1.0" }
        },
        "required": ["observer", "subject", "kind"],
        "additionalProperties": false
    }
    "#).unwrap(),
        },
        ToolDef {
            name: "society_record_contribution",
            description: "Record a contribution for a task. Master only.",
            input_schema: serde_json::from_str(r#"
    {
        "type": "object",
        "properties": {
            "task_id": { "type": "string", "description": "Task ID" },
            "agent_id": { "type": "string", "description": "Contributing agent" },
            "planned_share": { "type": "integer", "description": "Planned share 1-100" },
            "verified_contribution": { "type": "number", "description": "Verified contribution 0.0-1.0" },
            "evidence_id": { "type": "string", "description": "Evidence ID" },
            "quality": { "type": "number", "description": "Quality 0.0-1.0" },
            "met_sla": { "type": "boolean", "description": "Met SLA" }
        },
        "required": ["task_id", "agent_id", "planned_share"],
        "additionalProperties": false
    }
    "#).unwrap(),
        },
        ToolDef {
            name: "society_record_outcome",
            description: "Record a task outcome with distributions. Master only.",
            input_schema: serde_json::from_str(r#"
    {
        "type": "object",
        "properties": {
            "task_id": { "type": "string", "description": "Task ID" },
            "issuer": { "type": "string", "description": "Task issuer" },
            "team_members": { "type": "array", "items": { "type": "string" } },
            "status": { "type": "string", "enum": ["completed", "settled", "failed", "disputed"] },
            "evidence_id": { "type": "string" },
            "total_reward": { "type": "integer" },
            "distributions": { "type": "array", "items": { "type": "object", "properties": { "agent_id": { "type": "string" }, "amount": { "type": "integer" }, "share_basis": { "type": "string", "enum": ["planned", "verified", "hybrid"] } }, "required": ["agent_id", "amount", "share_basis"] } },
            "contributor_records": { "type": "array", "items": { "type": "object", "properties": { "task_id": { "type": "string" }, "agent_id": { "type": "string" }, "planned_share": { "type": "integer" }, "verified_contribution": { "type": "number" }, "evidence_id": { "type": "string" }, "quality": { "type": "number", "description": "Quality 0.0-1.0" }, "met_sla": { "type": "boolean" } } } }
        },
        "required": ["task_id", "issuer", "team_members", "status", "total_reward", "distributions"],
        "additionalProperties": false
    }
    "#).unwrap(),
        },
        ToolDef {
            name: "society_record_reputation_event",
            description: "Record a reputation event. Master only.",
            input_schema: serde_json::from_str(r#"
    {
        "type": "object",
        "properties": {
            "agent_id": { "type": "string" },
            "event_type": { "type": "string", "enum": ["task_completed", "task_failed", "quality_high", "quality_low", "sla_met", "sla_missed", "contribution_verified", "contribution_missing", "proposal_accepted", "proposal_rejected", "bid_accepted", "bid_rejected"] },
            "task_id": { "type": "string" },
            "delta": { "type": "number" },
            "evidence_id": { "type": "string" },
            "detail": { "type": "string" }
        },
        "required": ["agent_id", "event_type"],
        "additionalProperties": false
    }
    "#).unwrap(),
        },
        ToolDef {
            name: "list_consumer_keys",
            description: "Consumer API key metadata (Compute Contribution & Quota): per-key id, display prefix, owner account, quota ceiling, rate limit, scopes, status, and live usage + the owner account's quota balance. Read-only; NEVER exposes the plaintext secret.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
        ToolDef {
            name: "decentraai_embeddings",
            description: "Generate embeddings for a text input (L1 ASSIST). Requires a consumer key with 'embeddings' scope. Rate-limited and quota-gated. Returns the embedding vector or an error if the capability is not available.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string", "description": "Text to embed." },
                    "model": { "type": "string", "description": "Optional model id; defaults to the node's available embedding model." }
                },
                "required": ["input"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "decentraai_compute_request",
            description: "Request remote compute assistance (L1 ASSIST, Sharing is Caring DFCP). Requires a consumer key with 'compute' or matching capability scope. The fabric planner decides the worker; the caller never selects a peer. Rate-limited, quota-gated, audited.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "capability": { "type": "string", "description": "Capability to offload, e.g. 'embeddings', 'ocr', 'chat'." },
                    "payload": { "type": "object", "description": "Task payload as JSON (e.g. {\"input\":\"text\"} or {\"messages\":[...]})" },
                    "lease_seconds": { "type": "integer", "description": "Max lease in seconds (1..120, default 60)." }
                },
                "required": ["capability", "payload"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_read",
            description: "Read own personal memory (Identity, Goals, Capabilities, People, Tasks, Relationships, Experiences, Decisions, Lessons). Requires memory scope.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID (must equal your account)" },
                    "categories": { "type": "array", "items": { "type": "string" }, "description": "Categories to read (default: all)" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_write",
            description: "Write/update own personal memory entry (experiences, lessons, people, etc.). Requires memory scope. Agent_id must equal your account.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID (must equal your account)" },
                    "category": { "type": "string", "enum": ["identity", "goals", "capabilities", "people", "tasks", "relationships", "experiences", "decisions", "lessons"], "description": "Memory category" },
                    "entry": { "type": "object", "description": "Entry data (schema depends on category)" }
                },
                "required": ["agent_id", "category", "entry"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_search",
            description: "Search own personal memory by text query. Requires memory scope.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID" },
                    "query": { "type": "string", "description": "Search query" },
                    "categories": { "type": "array", "items": { "type": "string" }, "description": "Categories to search (default: all)" },
                    "limit": { "type": "integer", "description": "Max results (default: 10)" }
                },
                "required": ["agent_id", "query"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_snapshot",
            description: "Get decision-ready snapshot of own personal memory. Requires memory scope.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "agent_memory_export",
            description: "Export full personal memory as Obsidian-compatible Markdown. Requires memory scope.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Your agent ID" }
                },
                "required": ["agent_id"],
                "additionalProperties": false
            }),
        },
        ToolDef {
            name: "discover_capabilities",
            description: "Discover all available capabilities/tools on this node and their required scopes. Essential for external agent onboarding — call this first to learn what you can do and what scopes to request.",
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        },
    ]
}

/// Extract the parameters of a `search_models_by_capability` call, if the
/// incoming message is one. Pure — lets the HTTP layer decide whether it needs
/// to precompute a Hub search into [`McpContext::capability_search`] before
/// dispatching. Returns `(capability, query, limit)`.
pub fn capability_search_request(raw: &str) -> Option<(String, Option<String>, usize)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "search_models_by_capability" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let capability = args.get("capability").and_then(|c| c.as_str())?.to_string();
    if capability.is_empty() {
        return None;
    }
    let query = args
        .get("query")
        .and_then(|q| q.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let limit = args
        .get("limit")
        .and_then(|l| l.as_u64())
        .map(|n| n as usize)
        .unwrap_or(8)
        .clamp(1, 30);
    Some((capability, query, limit))
}

/// Extract the parameters of a `find_local_models_by_capability` call, if the
/// incoming message is one. Pure — lets the HTTP layer precompute the local
/// filter into [`McpContext::local_capability_search`]. Returns
/// `(capability, evidence)` where evidence is "any" or "verified".
pub fn local_capability_search_request(raw: &str) -> Option<(String, String)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "find_local_models_by_capability" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let capability = args.get("capability").and_then(|c| c.as_str())?.to_string();
    if capability.is_empty() {
        return None;
    }
    let evidence = args
        .get("evidence")
        .and_then(|e| e.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "any".to_string());
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    }
    .to_string();
    Some((capability, evidence))
}

/// Extract the parameters of a `get_worker_capability` call, if the incoming
/// message is one. Pure — lets the HTTP layer precompute the per-worker
/// verdict into [`McpContext::worker_capability`]. Returns
/// `(model, capability, evidence)`.
pub fn worker_capability_request(raw: &str) -> Option<(String, String, String)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "get_worker_capability" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let model = args.get("model").and_then(|m| m.as_str())?.to_string();
    let capability = args.get("capability").and_then(|c| c.as_str())?.to_string();
    if model.is_empty() || capability.is_empty() {
        return None;
    }
    let evidence = args
        .get("evidence")
        .and_then(|e| e.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "any".to_string());
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    }
    .to_string();
    Some((model, capability, evidence))
}

/// Extract the parameters of a `resolve_intent` call, if the incoming message
/// is one. Pure — lets the HTTP layer precompute the resolution into
/// [`McpContext::intent_resolution`]. Returns `(intent, evidence)` where
/// evidence defaults to "any".
pub fn intent_request(raw: &str) -> Option<(String, String)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "resolve_intent" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let intent = args.get("intent").and_then(|c| c.as_str())?.to_string();
    if intent.is_empty() {
        return None;
    }
    let evidence = args
        .get("evidence")
        .and_then(|e| e.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "any".to_string());
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    }
    .to_string();
    Some((intent, evidence))
}

/// Extract the parameters of a `resolve_intent_with_fit` call, if the incoming
/// message is one. Pure — lets the HTTP layer precompute the per-model fabric
/// fit into [`McpContext::intent_fit`]. Returns `(intent, evidence)` where
/// evidence defaults to "any". Identical to [`intent_request`] but matches the
/// `resolve_intent_with_fit` tool name.
pub fn intent_fit_request(raw: &str) -> Option<(String, String)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "resolve_intent_with_fit" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let intent = args.get("intent").and_then(|c| c.as_str())?.to_string();
    if intent.is_empty() {
        return None;
    }
    let evidence = args
        .get("evidence")
        .and_then(|e| e.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "any".to_string());
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    }
    .to_string();
    Some((intent, evidence))
}

/// Whether the incoming message is a `get_fabric_graph` tool call. Pure — lets
/// the HTTP layer precompute the fabric-graph projection into
/// [`McpContext::fabric_graph`]. Returns `Some(())` when it is, else `None`.
pub fn fabric_graph_request(raw: &str) -> Option<()> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "get_fabric_graph" {
        return None;
    }
    Some(())
}

/// Extract the parameters of a `decide` call, if the incoming message is one.
/// Pure — lets the HTTP layer precompute the unified decision into
/// [`McpContext::decision`]. Returns `(intent, evidence, Option<model>)`.
pub fn decision_request(raw: &str) -> Option<(String, String, Option<String>)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "decide" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let intent = args.get("intent").and_then(|c| c.as_str())?.to_string();
    if intent.is_empty() {
        return None;
    }
    let evidence = args
        .get("evidence")
        .and_then(|e| e.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "any".to_string());
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    }
    .to_string();
    let model = args
        .get("model")
        .and_then(|m| m.as_str())
        .map(str::to_string);
    Some((intent, evidence, model))
}

/// Extract the parameters of an `execute_decision` call, if the incoming
/// message is one. Pure — lets the HTTP layer precompute the confirmed
/// execution into [`McpContext::execution`]. Returns the args Value (the HTTP
/// layer enforces `confirm: true` and master auth).
pub fn execution_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "execute_decision" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?.clone();
    let has_intent = args
        .get("intent")
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .trim()
        != "";
    let has_cap = args
        .get("capability")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim()
        != "";
    if !(has_intent || has_cap)
        || args
            .get("prompt")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .is_empty()
    {
        return None;
    }
    Some(args)
}

/// Whether the incoming message is a `serve_model` tool call. Pure — lets the
/// HTTP layer gate it (master-only mutation) and run it.
pub fn serve_model_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "serve_model" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?.clone();
    let model = args.get("model").and_then(|m| m.as_str()).unwrap_or("");
    if model.trim().is_empty() {
        return None;
    }
    Some(args)
}

/// Whether the incoming message is a `pull_model` tool call. Pure — lets the
/// HTTP layer gate it (master-only mutation) and run it.
pub fn pull_model_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "pull_model" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?.clone();
    let reference = args.get("reference").and_then(|r| r.as_str()).unwrap_or("");
    if reference.trim().is_empty() {
        return None;
    }
    Some(args)
}

/// Whether the incoming message is a `list_sessions` tool call. Pure — lets the
/// HTTP layer precompute the session snapshot into [`McpContext::sessions`].
pub fn sessions_request(raw: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return false;
    }
    msg.get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        == Some("list_sessions")
}

/// Whether the incoming message is a `get_quota` tool call. Pure — lets the
/// HTTP layer precompute the quota ledger snapshot into [`McpContext::quota`].
pub fn quota_request(raw: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return false;
    }
    msg.get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        == Some("get_quota")
}

/// Whether the incoming message is a `list_consumer_keys` tool call. Pure —
/// lets the HTTP layer precompute the consumer-key metadata snapshot into
/// [`McpContext::consumer_keys`].
pub fn consumer_keys_request(raw: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return false;
    }
    msg.get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        == Some("list_consumer_keys")
}

/// Whether the incoming message is a `get_compensation` tool call (M9-9).
/// Pure — lets the HTTP layer precompute the compensation ledger snapshot into
/// [`McpContext::compensation`].
pub fn compensation_request(raw: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return false;
    }
    msg.get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        == Some("get_compensation")
}

pub fn arena_state_request(raw: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(raw) else { return false; };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return false; }
    msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str()) == Some("arena_state")
}

pub fn arena_act_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    let name = msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())?;
    if name != "arena_act" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned();
    Some(args.unwrap_or(json!({})))
}

pub fn hub_state_request(raw: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(raw) else { return false; };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return false; }
    msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str()) == Some("hub_state")
}

pub fn hub_events_request(raw: &str) -> Option<(u64, usize)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "hub_events" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let since = args.get("since").and_then(|v| v.as_u64()).unwrap_or(0);
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
    Some((since, limit.min(200)))
}
pub fn hub_publish_task_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "hub_publish_task" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    // Add account from request if present, for multi-agent support
    if let Some(acc) = msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("account")).and_then(|v| v.as_str()) {
        let mut a = args.as_object().cloned().unwrap_or_default();
        a.insert("account".to_string(), json!(acc));
        Some(json!(a))
    } else {
        Some(args)
    }
}
pub fn hub_place_bid_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "hub_place_bid" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    // Add account from request if present, for multi-agent support
    if let Some(acc) = msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("account")).and_then(|v| v.as_str()) {
        let mut a = args.as_object().cloned().unwrap_or_default();
        a.insert("account".to_string(), json!(acc));
        Some(json!(a))
    } else {
        Some(args)
    }
}
pub fn hub_propose_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "hub_propose" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    // Add account from request if present, for multi-agent support
    if let Some(acc) = msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("account")).and_then(|v| v.as_str()) {
        let mut a = args.as_object().cloned().unwrap_or_default();
        a.insert("account".to_string(), json!(acc));
        Some(json!(a))
    } else {
        Some(args)
    }
}
pub fn hub_decide_proposal_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "hub_decide_proposal" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    // Add account from request if present, for multi-agent support
    if let Some(acc) = msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("account")).and_then(|v| v.as_str()) {
        let mut a = args.as_object().cloned().unwrap_or_default();
        a.insert("account".to_string(), json!(acc));
        Some(json!(a))
    } else {
        Some(args)
    }
}
pub fn hub_form_team_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "hub_form_team" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    // Add account from request if present, for multi-agent support
    if let Some(acc) = msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("account")).and_then(|v| v.as_str()) {
        let mut a = args.as_object().cloned().unwrap_or_default();
        a.insert("account".to_string(), json!(acc));
        Some(json!(a))
    } else {
        Some(args)
    }
}
pub fn hub_execute_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "hub_execute" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    // Add account from request if present, for multi-agent support
    if let Some(acc) = msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("account")).and_then(|v| v.as_str()) {
        let mut a = args.as_object().cloned().unwrap_or_default();
        a.insert("account".to_string(), json!(acc));
        Some(json!(a))
    } else {
        Some(args)
    }
}

pub fn society_state_request(raw: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(raw) else { return false; };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return false; }
    msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str()) == Some("society_state")
}
pub fn society_trust_request(raw: &str) -> Option<(String, String)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_trust" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let observer = args.get("observer").and_then(|v| v.as_str())?.to_string();
    let subject = args.get("subject").and_then(|v| v.as_str())?.to_string();
    Some((observer, subject))
}
pub fn society_reputation_request(raw: &str) -> Option<(String, Option<String>)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_reputation" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let capability = args.get("capability").and_then(|v| v.as_str()).map(|s| s.to_string());
    Some((agent_id, capability))
}
pub fn society_relationships_request(raw: &str) -> Option<(String, bool)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_relationships" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let as_observer = args.get("as_observer").and_then(|v| v.as_bool()).unwrap_or(true);
    Some((agent_id, as_observer))
}
pub fn society_contributions_request(raw: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_contributions" { return None; }
    msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("task_id")).and_then(|v| v.as_str()).map(|s| s.to_string())
}
pub fn society_outcomes_request(raw: &str) -> Option<(String, usize)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_outcomes" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    Some((agent_id, limit))
}
pub fn society_decision_hints_request(raw: &str) -> Option<(String, Value, Value)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_decision_hints" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let hub_state = args.get("hub_state").cloned().unwrap_or(json!({}));
    let resources = args.get("resources").cloned().unwrap_or(json!({}));
    Some((agent_id, hub_state, resources))
}

pub fn society_record_relationship_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_record_relationship" { return None; }
    msg.get("params").and_then(|p| p.get("arguments")).cloned()
}
pub fn society_record_contribution_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_record_contribution" { return None; }
    msg.get("params").and_then(|p| p.get("arguments")).cloned()
}
pub fn society_record_outcome_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_record_outcome" { return None; }
    msg.get("params").and_then(|p| p.get("arguments")).cloned()
}
pub fn society_record_reputation_event_request(raw: &str) -> Option<serde_json::Value> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "society_record_reputation_event" { return None; }
    msg.get("params").and_then(|p| p.get("arguments")).cloned()
}

pub fn agent_memory_write_request(raw: &str) -> Option<(String, String, Value)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "agent_memory_write" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let category = args.get("category").and_then(|v| v.as_str())?.to_string();
    let entry = args.get("entry").cloned().unwrap_or(json!({}));
    Some((agent_id, category, entry))
}


/// Extract request parameters for agent_memory_read
pub fn agent_memory_read_request(raw: &str) -> Option<(String, Option<Vec<String>>)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "agent_memory_read" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let categories = args.get("categories").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(|s| s.to_string())).collect());
    Some((agent_id, categories))
}

/// Extract request parameters for agent_memory_search
pub fn agent_memory_search_request(raw: &str) -> Option<(String, String, Option<Vec<String>>, usize)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "agent_memory_search" { return None; }
    let args = msg.get("params").and_then(|p| p.get("arguments")).cloned().unwrap_or(json!({}));
    let agent_id = args.get("agent_id").and_then(|v| v.as_str())?.to_string();
    let query = args.get("query").and_then(|v| v.as_str())?.to_string();
    let categories = args.get("categories").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|e| e.as_str().map(|s| s.to_string())).collect());
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
    Some((agent_id, query, categories, limit))
}

/// Extract request parameters for agent_memory_snapshot
pub fn agent_memory_snapshot_request(raw: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "agent_memory_snapshot" { return None; }
    msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("agent_id")).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Extract request parameters for agent_memory_export
pub fn agent_memory_export_request(raw: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return None; }
    if msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str())? != "agent_memory_export" { return None; }
    msg.get("params").and_then(|p| p.get("arguments")).and_then(|a| a.get("agent_id")).and_then(|v| v.as_str()).map(|s| s.to_string())
}

pub fn embeddings_request(raw: &str) -> Option<(String, Option<String>)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "decentraai_embeddings" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let input = args.get("input").and_then(|v| v.as_str())?.to_string();
    if input.is_empty() || input.len() > 8000 {
        return None;
    }
    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some((input, model))
}


/// Extract request parameters for discover_capabilities
pub fn discover_capabilities_request(raw: &str) -> bool {
    let msg: Value = match serde_json::from_str(raw) { Ok(v) => v, Err(_) => return false };
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") { return false; }
    msg.get("params").and_then(|p| p.get("name")).and_then(|n| n.as_str()) == Some("discover_capabilities")
}

/// Extract `decentraai_compute_request` parameters (L1 ASSIST, DFCP).
pub fn compute_request(raw: &str) -> Option<(String, Value, u64)> {
    let msg: Value = serde_json::from_str(raw).ok()?;
    if msg.get("method").and_then(|m| m.as_str()) != Some("tools/call") {
        return None;
    }
    let name = msg
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())?;
    if name != "decentraai_compute_request" {
        return None;
    }
    let args = msg.get("params").and_then(|p| p.get("arguments"))?;
    let capability = args.get("capability").and_then(|v| v.as_str())?.to_string();
    if capability.is_empty() {
        return None;
    }
    let payload = args.get("payload").cloned().unwrap_or(json!({}));
    let lease = args
        .get("lease_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(60)
        .clamp(1, 120);
    Some((capability, payload, lease))
}

/// Pure, deterministic intent → capability → local-model resolution.
///
/// (1) Maps the intent to capabilities via
/// `decentraai_hub::intent::capabilities_for_intent` (INFERRED keyword
/// heuristic — never a model claim).
/// (2) For each capability, scans `ctx.models.data[*].capability_claims` for a
/// matching claim (`{capability, provenance}`). `evidence` "verified" keeps
/// only VERIFIED claims; "any" also accepts INFERRED. A capability with no
/// matching local claim goes into `unmatched`.
///
/// The HTTP layer precomputes this into [`McpContext::intent_resolution`]; the
/// pure helper exists so the logic is testable without a running node. The
/// `note` field keeps the result honest.
pub fn resolve_intent(ctx: &McpContext, intent: &str, evidence: &str) -> Value {
    let require_verified = evidence == "verified";

    let capabilities = decentraai_hub::intent::capabilities_for_intent(intent);

    let mut capabilities_json = Vec::new();
    let mut matching_local_models = serde_json::Map::new();
    let mut unmatched = Vec::new();

    for capability in capabilities {
        let cap_str = serde_json::to_string(&capability)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_default();
        capabilities_json.push(json!({
            "capability": cap_str,
            "label": capability.label(),
            "evidence_required": if require_verified { "verified" } else { "any" },
        }));

        // Scan ctx.models.data[*].capability_claims for this capability.
        let mut matched_models: Vec<Value> = Vec::new();
        let mut has_match = false;
        for m in ctx.models["data"].as_array().cloned().unwrap_or_default() {
            let Some(claims) = m["capability_claims"].as_array() else {
                continue; // no persisted claims -> UNKNOWN, not a match
            };
            if let Some(hit) = claims.iter().find(|c| {
                let cap = c["capability"].as_str().unwrap_or("");
                let prov = c["provenance"].as_str().unwrap_or("");
                cap.eq_ignore_ascii_case(&cap_str)
                    && (!require_verified || prov.eq_ignore_ascii_case("verified"))
            }) {
                has_match = true;
                matched_models.push(json!({
                    "id": m["id"],
                    "evidence": hit["provenance"],
                }));
            }
        }
        if has_match {
            matching_local_models.insert(cap_str.clone(), Value::Array(matched_models));
        } else {
            unmatched.push(cap_str);
        }
    }

    json!({
        "intent": intent,
        "capabilities": capabilities_json,
        "matching_local_models": matching_local_models,
        "unmatched": unmatched,
        "note": "intent-to-capability is INFERRED from keywords; a model's actual support is verified against persisted claims.",
    })
}

/// Handles one JSON-RPC 2.0 MCP message and returns an optional response.
/// Returns `None` for notifications (no `id`), which MCP clients do not await.
pub fn handle_message(ctx: &McpContext, raw: &str) -> Option<Value> {
    let msg: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => {
            return Some(error_response(Value::Null, -32700, "Parse error"));
        }
    };
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    let is_notification = msg.get("id").is_none();
    let method = match msg.get("method").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => {
            return Some(error_response(
                id,
                -32600,
                "Invalid Request: missing method",
            ));
        }
    };

    let outcome = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": server_info(),
            "instructions": "DecentraAI exposes its local model fabric to AI agents. All tools are read-only. Authentication: the same dsk_ Bearer token as the rest of the API."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": all_tools()
                .iter()
                .map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                }))
                .collect::<Vec<_>>()
        })),
        "tools/call" => match msg
            .get("params")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            Some(name) => match call_tool(
                ctx,
                name,
                msg.get("params").and_then(|p| p.get("arguments")).cloned(),
            ) {
                Some(result) => Ok(result),
                None => Err((-32602, format!("unknown tool: {name}"))),
            },
            None => Err((-32602, "tools/call requires a tool name".to_string())),
        },
        "notifications/initialized" => {
            // Notification: no response.
            return None;
        }
        _ => {
            return Some(error_response(
                id,
                -32601,
                format!("Method not found: {method}"),
            ));
        }
    };

    if is_notification {
        // A notification with an unrecognized method yields no response.
        return None;
    }
    Some(match outcome {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => error_response(id, code, message),
    })
}

/// Dispatches a read-only tool call. Returns `None` for an unknown tool name
/// so the caller can emit a clean `-32602` error.
fn call_tool(ctx: &McpContext, name: &str, _args: Option<Value>) -> Option<Value> {
    let data = match name {
        "get_status" => &ctx.status,
        "list_workers" => &ctx.workers,
        "list_models" => &ctx.models,
        "list_executions" => &ctx.executions,
        "list_peers" => &ctx.peers,
        "search_models_by_capability" => &ctx.capability_search,
        "find_local_models_by_capability" => &ctx.local_capability_search,
        "get_worker_capability" => &ctx.worker_capability,
        "resolve_intent" => &ctx.intent_resolution,
        "resolve_intent_with_fit" => &ctx.intent_fit,
        "get_fabric_graph" => &ctx.fabric_graph,
        "decide" => &ctx.decision,
        "execute_decision" => &ctx.execution,
        "serve_model" => &ctx.execution,
        "pull_model" => &ctx.execution,
        "list_sessions" => &ctx.sessions,
        "get_quota" => &ctx.quota,
        "get_compensation" => &ctx.compensation,
        "list_consumer_keys" => &ctx.consumer_keys,
        "arena_state" => &ctx.arena_state,
        "arena_act" => &ctx.arena_action,
        "hub_state" => &ctx.hub_state,
        "hub_events" => &ctx.hub_events,
        "hub_publish_task" => &ctx.hub_action,
        "hub_place_bid" => &ctx.hub_action,
        "hub_propose" => &ctx.hub_action,
        "hub_decide_proposal" => &ctx.hub_action,
        "hub_form_team" => &ctx.hub_action,
        "hub_execute" => &ctx.hub_action,
        "society_state" => &ctx.society_action,
        "society_trust" => &ctx.society_action,
        "society_reputation" => &ctx.society_action,
        "society_relationships" => &ctx.society_action,
        "society_contributions" => &ctx.society_action,
        "society_outcomes" => &ctx.society_action,
        "society_decision_hints" => &ctx.society_action,
        "agent_memory_read" => &ctx.personal_memory_action,
        "agent_memory_write" => &ctx.personal_memory_action,
        "agent_memory_search" => &ctx.personal_memory_action,
        "agent_memory_snapshot" => &ctx.personal_memory_action,
        "agent_memory_export" => &ctx.personal_memory_action,
        "discover_capabilities" => &ctx.personal_memory_action,
        _ => return None,
    };
    Some(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string()),
        }],
    }))
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> McpContext {
        McpContext {
            status: json!({ "model_loaded": true }),
            workers: json!([{ "node_id": "w1", "trusted": true }]),
            models: json!([{ "file_name": "model.gguf" }]),
            executions: json!([{ "chosen_worker": "w1" }]),
            peers: json!([{ "peer_id": "p1" }]),
            capability_search: json!({ "query": "", "matched": 1, "models": [{ "id": "org/vision" }] }),
            local_capability_search: json!({ "matched": 1, "models": [{ "id": "local.gguf", "evidence": "verified" }] }),
            worker_capability: json!({ "model": "m", "capability": "ocr", "fit": { "verdict": "CAN_RUN", "counts": { "can_run": 1 } }, "workers": [{ "worker": { "node_id": "w1" }, "verdict": "CAN_RUN" }] }),
            intent_resolution: json!({}),
            intent_fit: json!({ "intent": "i", "capabilities": [] }),
            fabric_graph: json!({ "nodes": [], "models": [], "capabilities": [], "executions": [], "network": [], "kv": { "sessions_active": 0 } }),
            decision: json!({ "request": "ocr", "capabilities": [], "decision": null, "why": [], "historical": { "records": 0 } }),
            execution: json!({}),
            sessions: json!({ "sessions_active": 0, "sessions": [] }),
            quota: json!({ "accounts": [], "total_earned": 0, "total_consumed": 0, "policy_version": 1 }),
            compensation: json!({ "accounts": [], "total_earned": 0, "recent_events": [], "policy": null }),
            consumer_keys: json!({ "keys": [] }),
            arena_state: json!({ "tick": 0, "width": 20, "height": 20, "agents": [], "events": [] }),
            arena_action: json!({}),
            hub_state: json!({ "tick": 0, "tasks": [], "bids": [], "proposals": [], "teams": [], "events": [] }),
            hub_events: json!({ "tick": 0, "events": [] }),
            hub_action: json!({}),
            society_action: json!({}),
            personal_memory_action: json!({}),
        }
    }

    fn call(msg: &str) -> Value {
        handle_message(&ctx(), msg).unwrap()
    }

    #[test]
    fn initialize_negotiates_protocol() {
        let r = call(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
        );
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(r["result"]["capabilities"]["tools"].is_object());
        assert_eq!(r["result"]["serverInfo"]["name"], "decentraai-mcp");
    }

    #[test]
    fn ping_returns_empty_result() {
        let r = call(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#);
        assert_eq!(r["result"], json!({}));
    }

    #[test]
    fn tools_list_exposes_read_only_tools() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"list_workers"));
        assert!(names.contains(&"get_status"));
        assert!(names.contains(&"list_executions"));
        assert!(names.contains(&"search_models_by_capability"));
        assert!(names.contains(&"find_local_models_by_capability"));
        assert!(names.contains(&"get_worker_capability"));
        // Each tool declares a JSON schema.
        assert!(tools.iter().all(|t| t["inputSchema"].is_object()));
    }

    #[test]
    fn tools_call_returns_real_snapshot_data() {
        let r = call(
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_workers","arguments":{}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("w1"), "must return the supplied snapshot");
    }

    #[test]
    fn capability_search_returns_the_precomputed_hub_result() {
        // The HTTP layer precomputes the Hub search into `capability_search`;
        // the protocol layer returns it unchanged (no I/O here).
        let r = call(
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"search_models_by_capability","arguments":{"capability":"vision"}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            content.contains("org/vision"),
            "must return the supplied snapshot"
        );
        assert!(content.contains("\"matched\":1"));
    }

    #[test]
    fn local_capability_search_returns_precomputed_local_filter() {
        // Same pattern: HTTP layer precomputes the local-claims filter; the
        // protocol layer returns it unchanged.
        let r = call(
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"find_local_models_by_capability","arguments":{"capability":"ocr","evidence":"verified"}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            content.contains("local.gguf"),
            "must return the supplied snapshot"
        );
        assert!(content.contains("verified"));
    }

    #[test]
    fn local_capability_search_request_parses_args_and_defaults_evidence() {
        // Default evidence is "any" when omitted.
        let (cap, ev) = local_capability_search_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_local_models_by_capability","arguments":{"capability":"ocr"}}}"#,
        )
        .unwrap();
        assert_eq!(cap, "ocr");
        assert_eq!(ev, "any");
        // Explicit verified is honored.
        let (_, ev) = local_capability_search_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"find_local_models_by_capability","arguments":{"capability":"vision","evidence":"verified"}}}"#,
        )
        .unwrap();
        assert_eq!(ev, "verified");
        // Non-matching methods yield None.
        assert!(
            local_capability_search_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#).is_none()
        );
    }

    #[test]
    fn get_worker_capability_returns_precomputed_verdict() {
        // HTTP layer precomputes the per-worker verdict; protocol returns it.
        let r = call(
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"get_worker_capability","arguments":{"model":"qwen.gguf","capability":"ocr","evidence":"verified"}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            content.contains("CAN_RUN"),
            "must return the supplied snapshot"
        );
        assert!(content.contains("\"node_id\":\"w1\""));
    }

    #[test]
    fn worker_capability_request_parses_args_and_defaults_evidence() {
        let (m, c, ev) = worker_capability_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_worker_capability","arguments":{"model":"qwen.gguf","capability":"ocr"}}}"#,
        )
        .unwrap();
        assert_eq!(m, "qwen.gguf");
        assert_eq!(c, "ocr");
        assert_eq!(ev, "any");
        // Explicit verified honored; non-matching method -> None.
        let (_, _, ev) = worker_capability_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_worker_capability","arguments":{"model":"qwen.gguf","capability":"ocr","evidence":"verified"}}}"#,
        )
        .unwrap();
        assert_eq!(ev, "verified");
        assert!(worker_capability_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#).is_none());
    }

    #[test]
    fn unknown_tool_is_invalid_params() {
        let r = call(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#,
        );
        assert_eq!(r["error"]["code"], -32602);
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let r = call(r#"{"jsonrpc":"2.0","id":6,"method":"bogus"}"#);
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn initialized_notification_yields_no_response() {
        assert!(
            handle_message(
                &ctx(),
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
            )
            .is_none()
        );
    }

    #[test]
    fn parse_error_is_reported() {
        let r = handle_message(&ctx(), "not json").unwrap();
        assert_eq!(r["error"]["code"], -32700);
    }

    #[test]
    fn intent_request_parses_args_and_defaults_evidence() {
        let (intent, ev) = intent_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"resolve_intent","arguments":{"intent":"I need OCR and summarization"}}}"#,
        )
        .unwrap();
        assert_eq!(intent, "I need OCR and summarization");
        assert_eq!(ev, "any");
        // Explicit verified honored; non-matching method -> None.
        let (_, ev) = intent_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"resolve_intent","arguments":{"intent":"chat","evidence":"verified"}}}"#,
        )
        .unwrap();
        assert_eq!(ev, "verified");
        assert!(intent_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#).is_none());
    }

    #[test]
    fn tools_list_exposes_resolve_intent() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let resolve = tools
            .iter()
            .find(|t| t["name"] == "resolve_intent")
            .unwrap();
        assert!(
            resolve["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r == "intent")
        );
    }

    #[test]
    fn resolve_intent_returns_matched_and_unmatched() {
        let c = McpContext {
            models: json!({
                "object": "list",
                "data": [
                    {
                        "id": "local-vision.gguf",
                        "capability_claims": [
                            { "capability": "vision", "provenance": "verified" },
                            { "capability": "ocr", "provenance": "verified" },
                        ],
                    },
                    {
                        "id": "plain-chat.gguf",
                        "capability_claims": [
                            { "capability": "chat", "provenance": "inferred" },
                        ],
                    },
                ],
            }),
            ..ctx()
        };
        let out = resolve_intent(&c, "I need OCR and summarization", "any");
        assert_eq!(out["intent"], "I need OCR and summarization");
        let caps = out["capabilities"].as_array().unwrap();
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0]["capability"], "ocr");
        assert_eq!(caps[0]["label"], "OCR");
        assert_eq!(caps[0]["evidence_required"], "any");
        assert_eq!(caps[1]["capability"], "summarization");
        // OCR matched to local-vision.gguf (verified).
        let matched = out["matching_local_models"]["ocr"].as_array().unwrap();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0]["id"], "local-vision.gguf");
        assert_eq!(matched[0]["evidence"], "verified");
        // Summarization has no local claim -> unmatched.
        assert_eq!(out["unmatched"], json!(["summarization"]));
        assert!(out["note"].as_str().unwrap().contains("INFERRED"));
    }

    #[test]
    fn resolve_intent_matched_model_returned_under_its_capability() {
        let c = McpContext {
            models: json!({
                "object": "list",
                "data": [
                    {
                        "id": "local-ocr.gguf",
                        "capability_claims": [
                            { "capability": "ocr", "provenance": "inferred" },
                        ],
                    },
                ],
            }),
            ..ctx()
        };
        // evidence "any" includes inferred claims.
        let out = resolve_intent(&c, "ocr", "any");
        assert_eq!(
            out["matching_local_models"]["ocr"][0]["id"],
            "local-ocr.gguf"
        );
        assert_eq!(
            out["matching_local_models"]["ocr"][0]["evidence"],
            "inferred"
        );
        assert!(out["unmatched"].as_array().unwrap().is_empty());
        // evidence "verified" excludes the inferred claim -> unmatched.
        let out = resolve_intent(&c, "ocr", "verified");
        assert!(out["matching_local_models"]["ocr"].is_null());
        assert_eq!(out["unmatched"], json!(["ocr"]));
    }

    #[test]
    fn resolve_intent_empty_intent_yields_empty_capabilities() {
        let out = resolve_intent(&ctx(), "", "any");
        assert!(out["capabilities"].as_array().unwrap().is_empty());
        assert!(out["matching_local_models"].as_object().unwrap().is_empty());
        assert!(out["unmatched"].as_array().unwrap().is_empty());
    }

    #[test]
    fn resolve_intent_capability_with_no_local_claim_is_unmatched() {
        let c = McpContext {
            models: json!({
                "object": "list",
                "data": [
                    { "id": "local-ocr.gguf", "capability_claims": [{ "capability": "ocr", "provenance": "verified" }] },
                ],
            }),
            ..ctx()
        };
        // "chat" maps to a capability but no local model claims it -> unmatched,
        // while ocr stays matched. This is the honest UNKNOWN path.
        let out = resolve_intent(&c, "chat and ocr", "any");
        let unmatched: Vec<&str> = out["unmatched"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(unmatched, vec!["chat"]);
        assert_eq!(
            out["matching_local_models"]["ocr"][0]["id"],
            "local-ocr.gguf"
        );
    }

    #[test]
    fn resolve_intent_unknown_intent_is_empty_not_guessed() {
        // An intent with no recognized keyword resolves to empty capabilities
        // (honest UNKNOWN) — nothing is fabricated or marked unmatched.
        let out = resolve_intent(&ctx(), "falafel please", "any");
        assert!(out["capabilities"].as_array().unwrap().is_empty());
        assert!(out["unmatched"].as_array().unwrap().is_empty());
        assert!(out["matching_local_models"].as_object().unwrap().is_empty());
    }

    #[test]
    fn resolve_intent_call_returns_precomputed_snapshot() {
        // The protocol layer returns the HTTP-precomputed snapshot unchanged.
        let mut c = ctx();
        c.intent_resolution = json!({ "intent": "ocr", "capabilities": [{ "capability": "ocr" }] });
        let r = handle_message(&c, r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"resolve_intent","arguments":{"intent":"ocr"}}}"#)
            .unwrap();
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("ocr"));
    }

    #[test]
    fn tools_list_exposes_resolve_intent_with_fit() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let fit = tools
            .iter()
            .find(|t| t["name"] == "resolve_intent_with_fit")
            .unwrap();
        assert!(fit["inputSchema"].is_object());
        assert!(
            fit["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|req| req == "intent")
        );
        assert!(fit["inputSchema"]["additionalProperties"] == json!(false));
        assert!(
            fit["description"]
                .as_str()
                .unwrap()
                .contains("capabilities")
        );
    }

    #[test]
    fn intent_fit_request_parses_args_and_defaults_evidence() {
        let (intent, ev) = intent_fit_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"resolve_intent_with_fit","arguments":{"intent":"I need OCR and summarization"}}}"#,
        )
        .unwrap();
        assert_eq!(intent, "I need OCR and summarization");
        assert_eq!(ev, "any");
        // Explicit verified honored; non-matching method -> None.
        let (_, ev) = intent_fit_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"resolve_intent_with_fit","arguments":{"intent":"chat","evidence":"verified"}}}"#,
        )
        .unwrap();
        assert_eq!(ev, "verified");
        assert!(intent_fit_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#).is_none());
        // A resolve_intent (no _with_fit) call is not matched by this extractor.
        assert!(intent_fit_request(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"resolve_intent","arguments":{"intent":"chat"}}}"#).is_none());
    }

    #[test]
    fn resolve_intent_with_fit_returns_precomputed_snapshot() {
        // The protocol layer returns the HTTP-precomputed fabric-fit snapshot
        // unchanged; it does not resolve intent itself.
        let mut c = ctx();
        c.intent_fit = json!({
            "intent": "ocr and summarization",
            "capabilities": [
                { "capability": "ocr", "fit": { "verdict": "CAN_RUN" } },
            ],
        });
        let r = handle_message(&c, r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"resolve_intent_with_fit","arguments":{"intent":"ocr and summarization"}}}"#)
            .unwrap();
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("ocr and summarization"));
        assert!(content.contains("\"verdict\":\"CAN_RUN\""));
    }

    #[test]
    fn tools_list_exposes_get_fabric_graph() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let g = tools
            .iter()
            .find(|t| t["name"] == "get_fabric_graph")
            .unwrap();
        assert!(g["inputSchema"].is_object());
        assert!(g["description"].as_str().unwrap().contains("fabric graph"));
    }

    #[test]
    fn fabric_graph_request_matches_only_the_tool() {
        assert!(fabric_graph_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_fabric_graph","arguments":{}}}"#
        )
        .is_some());
        assert!(fabric_graph_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#).is_none());
        assert!(fabric_graph_request(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_status","arguments":{}}}"#).is_none());
    }

    #[test]
    fn get_fabric_graph_returns_precomputed_projection() {
        // The protocol layer returns the HTTP-precomputed fabric graph unchanged.
        let r = call(
            r#"{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"get_fabric_graph","arguments":{}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("\"nodes\":[]"));
        assert!(content.contains("\"capabilities\":[]"));
        assert!(content.contains("\"sessions_active\":0"));
    }

    #[test]
    fn tools_list_exposes_decide() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let d = tools.iter().find(|t| t["name"] == "decide").unwrap();
        assert!(d["inputSchema"].is_object());
        assert!(
            d["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|req| req == "intent")
        );
        assert!(d["description"].as_str().unwrap().contains("coherent"));
    }

    #[test]
    fn decision_request_parses_args_and_defaults_evidence() {
        let (intent, ev, model) = decision_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"decide","arguments":{"intent":"OCR these images"}}}"#,
        )
        .unwrap();
        assert_eq!(intent, "OCR these images");
        assert_eq!(ev, "any");
        assert!(model.is_none());
        // Explicit evidence + model honored; non-matching method -> None.
        let (_, ev, model) = decision_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"decide","arguments":{"intent":"chat","evidence":"verified","model":"qwen.gguf"}}}"#,
        )
        .unwrap();
        assert_eq!(ev, "verified");
        assert_eq!(model.as_deref(), Some("qwen.gguf"));
        assert!(decision_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#).is_none());
    }

    #[test]
    fn decide_returns_precomputed_decision() {
        // The protocol layer returns the HTTP-precomputed unified decision.
        let r = call(
            r#"{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"decide","arguments":{"intent":"ocr"}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("\"request\":\"ocr\""));
        assert!(content.contains("\"capabilities\":[]"));
        assert!(content.contains("\"why\":[]"));
    }

    #[test]
    fn tools_list_exposes_execute_decision() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let d = tools
            .iter()
            .find(|t| t["name"] == "execute_decision")
            .unwrap();
        let required = d["inputSchema"]["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "confirm"));
        assert!(required.iter().any(|r| r == "prompt"));
        assert!(d["description"].as_str().unwrap().contains("MUTATING"));
    }

    #[test]
    fn execution_request_parses_args_and_requires_intent_prompt() {
        let args = execution_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute_decision","arguments":{"intent":"ocr","prompt":"read","confirm":true}}}"#,
        )
        .unwrap();
        assert_eq!(args["intent"], "ocr");
        assert_eq!(args["prompt"], "read");
        assert_eq!(args["confirm"], true);
        // Missing prompt -> None.
        assert!(execution_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute_decision","arguments":{"intent":"ocr","confirm":true}}}"#
        )
        .is_none());
        // Capability-only is accepted (intent OR capability required).
        let args = execution_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute_decision","arguments":{"capability":"ocr","prompt":"read","confirm":true}}}"#,
        )
        .unwrap();
        assert_eq!(args["capability"], "ocr");
        // Neither intent nor capability -> None.
        assert!(execution_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute_decision","arguments":{"prompt":"read","confirm":true}}}"#
        )
        .is_none());
        assert!(execution_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#).is_none());
    }

    #[test]
    fn execute_decision_returns_precomputed_execution() {
        // The protocol layer returns the HTTP-precomputed execution snapshot.
        let mut c = ctx();
        c.execution = json!({ "status": 422, "ok": false, "body": { "error": { "message": "no runnable decision" } } });
        let r = handle_message(&c, r#"{"jsonrpc":"2.0","id":22,"method":"tools/call","params":{"name":"execute_decision","arguments":{"intent":"ocr","prompt":"read","confirm":true}}}"#)
            .unwrap();
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("\"status\":422"));
        assert!(content.contains("no runnable decision"));
    }

    #[test]
    fn tools_list_exposes_list_sessions() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "list_sessions"));
    }

    #[test]
    fn sessions_request_matches_only_the_tool() {
        assert!(sessions_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#
        ));
        assert!(!sessions_request(
            r#"{"jsonrpc":"2.0","method":"tools/list"}"#
        ));
        assert!(!sessions_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_status","arguments":{}}}"#
        ));
    }

    #[test]
    fn list_sessions_returns_precomputed_snapshot() {
        let r = call(
            r#"{"jsonrpc":"2.0","id":23,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("\"sessions_active\":0"));
        assert!(content.contains("\"sessions\":[]"));
    }

    #[test]
    fn tools_list_exposes_get_quota() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "get_quota"));
    }

    #[test]
    fn quota_request_matches_only_the_tool() {
        assert!(quota_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_quota","arguments":{}}}"#
        ));
        assert!(!quota_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#));
        assert!(!quota_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_status","arguments":{}}}"#
        ));
    }

    #[test]
    fn get_quota_returns_precomputed_snapshot() {
        let r = call(
            r#"{"jsonrpc":"2.0","id":24,"method":"tools/call","params":{"name":"get_quota","arguments":{}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("\"total_earned\":0"));
        assert!(content.contains("\"policy_version\":1"));
    }

    #[test]
    fn tools_list_exposes_list_consumer_keys() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "list_consumer_keys"));
    }

    #[test]
    fn consumer_keys_request_matches_only_the_tool() {
        assert!(consumer_keys_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_consumer_keys","arguments":{}}}"#
        ));
        assert!(!consumer_keys_request(
            r#"{"jsonrpc":"2.0","method":"tools/list"}"#
        ));
        assert!(!consumer_keys_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_quota","arguments":{}}}"#
        ));
    }

    #[test]
    fn compensation_request_matches_only_the_tool() {
        assert!(compensation_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_compensation","arguments":{}}}"#
        ));
        assert!(!compensation_request(
            r#"{"jsonrpc":"2.0","method":"tools/list"}"#
        ));
        assert!(!compensation_request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_quota","arguments":{}}}"#
        ));
    }

    #[test]
    fn get_compensation_returns_precomputed_snapshot() {
        let r = call(
            r#"{"jsonrpc":"2.0","id":26,"method":"tools/call","params":{"name":"get_compensation","arguments":{}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("\"total_earned\":0"));
        assert!(content.contains("\"recent_events\":[]"));
        assert!(content.contains("\"policy\":null"));
    }

    #[test]
    fn tools_list_exposes_get_compensation() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "get_compensation"));
    }

    #[test]
    fn list_consumer_keys_returns_precomputed_metadata() {
        let r = call(
            r#"{"jsonrpc":"2.0","id":25,"method":"tools/call","params":{"name":"list_consumer_keys","arguments":{}}}"#,
        );
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        // Default empty projection; never leaks a secret.
        assert!(content.contains("\"keys\":[]"));
        assert!(
            !content.contains("dca_"),
            "metadata must not leak the secret prefix value"
        );
    }
}
