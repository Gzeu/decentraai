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
}

/// A single MCP tool definition (name + description + JSON-Schema input).
struct ToolDef {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
}

fn all_tools() -> Vec<ToolDef> {
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
                    "intent": { "type": "string", "description": "A natural-language intent, e.g. 'OCR these images'." },
                    "prompt": { "type": "string", "description": "The actual prompt to run." },
                    "max_tokens": { "type": "integer", "description": "Max tokens to generate (default 1024, cap 4096)." },
                    "stream": { "type": "boolean", "description": "Whether to stream (default false)." },
                    "model": { "type": "string", "description": "Optional: narrow to a specific model file." },
                    "evidence": { "type": "string", "enum": ["any", "verified"], "description": "Evidence filter for the decision." },
                    "session_id": { "type": "string", "description": "Optional: links this run to an earlier one for KV-cache locality (continuation)." },
                    "dry_run": { "type": "boolean", "description": "If true, preview what would be reserved/routed WITHOUT executing (no request, no reservation). Requires confirm:true too." },
                    "confirm": { "type": "boolean", "description": "MUST be true to execute (mutation safety)." },
                },
                "required": ["intent", "prompt", "confirm"],
                "additionalProperties": false,
            }),
        },
        ToolDef {
            name: "list_sessions",
            description: "Coordinator-tracked KV/session residency (KV locality): which worker currently holds each session's KV prefix, the model, accounted tokens used, capacity, and KV headroom. Read-only; real accounted state (empty when no sessions/compute). Useful to know why a continuation would be steered to a specific worker.",
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
    let args = msg
        .get("params")
        .and_then(|p| p.get("arguments"))?;
    let capability = args.get("capability").and_then(|c| c.as_str())?.to_string();
    if capability.is_empty() {
        return None;
    }
    let query = args.get("query").and_then(|q| q.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
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
    let evidence = if evidence == "verified" { "verified" } else { "any" }.to_string();
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
    let evidence = if evidence == "verified" { "verified" } else { "any" }.to_string();
    Some((model, capability, evidence))
}

/// Extract the parameters of a `resolve_intent` call, if the incoming message
/// is one. Pure — lets the HTTP layer precompute the resolution into
/// [`McpContext::intent_resolution`]. Returns `(intent, evidence)` where
/// evidence defaults to "any".
pub fn intent_request(raw: &str) -> Option<(String, String)> {    let msg: Value = serde_json::from_str(raw).ok()?;
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
    let evidence = if evidence == "verified" { "verified" } else { "any" }.to_string();
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
    let evidence = if evidence == "verified" { "verified" } else { "any" }.to_string();
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
    let evidence = if evidence == "verified" { "verified" } else { "any" }.to_string();
    let model = args.get("model").and_then(|m| m.as_str()).map(str::to_string);
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
    if args.get("intent").and_then(|i| i.as_str()).unwrap_or("").is_empty()
        || args.get("prompt").and_then(|p| p.as_str()).unwrap_or("").is_empty()
    {
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
            return Some(error_response(id, -32600, "Invalid Request: missing method"));
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
        _ => return Some(error_response(id, -32601, format!("Method not found: {method}"))),
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
        "list_sessions" => &ctx.sessions,
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
        }
    }

    fn call(msg: &str) -> Value {
        handle_message(&ctx(), msg).unwrap()
    }

    #[test]
    fn initialize_negotiates_protocol() {
        let r = call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#);
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
        let r = call(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_workers","arguments":{}}}"#);
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("w1"), "must return the supplied snapshot");
    }

    #[test]
    fn capability_search_returns_the_precomputed_hub_result() {
        // The HTTP layer precomputes the Hub search into `capability_search`;
        // the protocol layer returns it unchanged (no I/O here).
        let r = call(r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"search_models_by_capability","arguments":{"capability":"vision"}}}"#);
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("org/vision"), "must return the supplied snapshot");
        assert!(content.contains("\"matched\":1"));
    }

    #[test]
    fn local_capability_search_returns_precomputed_local_filter() {
        // Same pattern: HTTP layer precomputes the local-claims filter; the
        // protocol layer returns it unchanged.
        let r = call(r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"find_local_models_by_capability","arguments":{"capability":"ocr","evidence":"verified"}}}"#);
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("local.gguf"), "must return the supplied snapshot");
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
        assert!(local_capability_search_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#).is_none());
    }

    #[test]
    fn get_worker_capability_returns_precomputed_verdict() {
        // HTTP layer precomputes the per-worker verdict; protocol returns it.
        let r = call(r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"get_worker_capability","arguments":{"model":"qwen.gguf","capability":"ocr","evidence":"verified"}}}"#);
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("CAN_RUN"), "must return the supplied snapshot");
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
        let r = call(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope","arguments":{}}}"#);
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
            handle_message(&ctx(), r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
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
        let resolve = tools.iter().find(|t| t["name"] == "resolve_intent").unwrap();
        assert!(resolve["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "intent"));
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
        assert_eq!(out["matching_local_models"]["ocr"][0]["id"], "local-ocr.gguf");
        assert_eq!(out["matching_local_models"]["ocr"][0]["evidence"], "inferred");
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
        assert_eq!(out["matching_local_models"]["ocr"][0]["id"], "local-ocr.gguf");
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
        let fit = tools.iter().find(|t| t["name"] == "resolve_intent_with_fit").unwrap();
        assert!(fit["inputSchema"].is_object());
        assert!(fit["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|req| req == "intent"));
        assert!(fit["inputSchema"]["additionalProperties"] == json!(false));
        assert!(fit["description"]
            .as_str()
            .unwrap()
            .contains("capabilities"));
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
        let g = tools.iter().find(|t| t["name"] == "get_fabric_graph").unwrap();
        assert!(g["inputSchema"].is_object());
        assert!(g["description"]
            .as_str()
            .unwrap()
            .contains("fabric graph"));
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
        let r = call(r#"{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"get_fabric_graph","arguments":{}}}"#);
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
        assert!(d["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|req| req == "intent"));
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
        let r = call(r#"{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"decide","arguments":{"intent":"ocr"}}}"#);
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("\"request\":\"ocr\""));
        assert!(content.contains("\"capabilities\":[]"));
        assert!(content.contains("\"why\":[]"));
    }

    #[test]
    fn tools_list_exposes_execute_decision() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        let d = tools.iter().find(|t| t["name"] == "execute_decision").unwrap();
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
        assert!(!sessions_request(r#"{"jsonrpc":"2.0","method":"tools/list"}"#));
        assert!(!sessions_request(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_status","arguments":{}}}"#));
    }

    #[test]
    fn list_sessions_returns_precomputed_snapshot() {
        let r = call(r#"{"jsonrpc":"2.0","id":23,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#);
        let content = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(content.contains("\"sessions_active\":0"));
        assert!(content.contains("\"sessions\":[]"));
    }
}
