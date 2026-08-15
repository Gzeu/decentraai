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
}
