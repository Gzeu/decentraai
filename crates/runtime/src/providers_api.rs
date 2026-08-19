//! Provider control plane API handlers (Model Fabric).
//!
//! Master-gated admin endpoints for provider CRUD, model connection,
//! discovery, health probing and sharing. Never exposes credentials:
//! responses carry only masked fingerprints / handle metadata.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{body::Bytes, http::HeaderMap};
use futures::StreamExt;
use serde_json::json;

use crate::api::ApiState;

fn forbidden(msg: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json!({ "error": { "message": msg, "type": "forbidden" } }).to_string(),
    )
        .into_response()
}

fn ok_json(value: serde_json::Value) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        value.to_string(),
    )
        .into_response()
}

fn bad(msg: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json!({ "error": { "message": msg, "type": "bad_request" } }).to_string(),
    )
        .into_response()
}

fn not_found(msg: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json!({ "error": { "message": msg, "type": "not_found" } }).to_string(),
    )
        .into_response()
}

/// GET /v1/providers — readable summary list (open/master/operator).
/// Returns summaries with masked credential fingerprints, never secrets.
pub async fn providers_list_handler(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    // Operators + admin can read provider metadata.
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return ok_json(json!({ "providers": [], "error": "provider plane not attached" }));
    };
    let mgr = providers.lock().await;
    // Safe view: masked fingerprints only, plus each provider's connected
    // models (never credentials). Serialize the models explicitly so the
    // dashboard can render catalog rows without a second round-trip.
    let summaries = mgr.list_provider_summaries();
    let providers_with_models: Vec<serde_json::Value> = summaries
        .iter()
        .map(|s| {
            let models = mgr
                .provider(&s.provider_id)
                .map(|p| p.models.clone())
                .unwrap_or_default();
            json!({
                "summary": s,
                "models": models,
            })
        })
        .collect();
    ok_json(json!({ "providers": providers_with_models }))
}

/// POST /api/admin/providers — create a provider.
/// Body: { "kind": "openrouter"|..., "name": "...", "base_url": "...",
///         "api_key": "sk-..." }
pub async fn providers_create_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return forbidden("provider plane not attached");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return bad("invalid JSON"),
    };
    let kind_str = req.get("kind").and_then(|k| k.as_str()).unwrap_or("");
    let Some(kind) = decentraai_providers::ProviderKind::parse(kind_str) else {
        return bad(
            "unknown provider kind (use openrouter|openai|groq|together|fireworks|generic_openai_compatible)",
        );
    };
    let name = req
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();
    if name.trim().is_empty() {
        return bad("missing provider name");
    }
    let base_url = req
        .get("base_url")
        .and_then(|b| b.as_str())
        .map(str::to_string)
        .or_else(|| kind.default_base_url().map(str::to_string))
        .unwrap_or_default();
    if base_url.trim().is_empty() {
        return bad("missing base_url (no default for this provider kind)");
    }
    let api_key = req
        .get("api_key")
        .and_then(|k| k.as_str())
        .unwrap_or("")
        .to_string();
    if api_key.trim().is_empty() {
        return bad("missing api_key");
    }

    let mut mgr = providers.lock().await;
    match mgr.add_provider(kind, name.clone(), base_url, api_key) {
        Ok(provider_id) => {
            let summary = mgr.provider(&provider_id).map(|p| {
                p.summary(
                    mgr.credential_store()
                        .lock()
                        .map(|g| g.fingerprint(&p.credential_ref))
                        .unwrap_or_default(),
                )
            });
            record_provider_audit(
                &state,
                "provider_created",
                json!({ "provider_id": provider_id, "kind": kind.as_str(), "name": name }),
            );
            ok_json(json!({ "success": true, "provider_id": provider_id, "provider": summary }))
        }
        Err(e) => bad(&e.to_string()),
    }
}

/// POST /api/admin/providers/{id}/test — test the provider credential.
pub async fn providers_test_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return forbidden("provider plane not attached");
    };
    let mut mgr = providers.lock().await;
    match mgr.test_connection(&provider_id).await {
        Ok((latency_ms, model_count)) => {
            record_provider_audit(
                &state,
                "provider_tested",
                json!({ "provider_id": provider_id, "ok": true }),
            );
            ok_json(
                json!({ "success": true, "provider_id": provider_id, "latency_ms": latency_ms, "model_count": model_count }),
            )
        }
        Err(e) => {
            record_provider_audit(
                &state,
                "provider_tested",
                json!({ "provider_id": provider_id, "ok": false, "error_class": provider_conn_class(&e) }),
            );
            (
                StatusCode::BAD_GATEWAY,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                json!({ "success": false, "provider_id": provider_id, "error": e.to_string(), "error_class": provider_conn_class(&e) }).to_string(),
            )
                .into_response()
        }
    }
}

/// POST /api/admin/providers/{id}/discover — discover available models.
pub async fn providers_discover_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return forbidden("provider plane not attached");
    };
    let mgr = providers.lock().await;
    let models = mgr.discover_models(&provider_id).await;
    match models {
        Ok(models) => {
            ok_json(json!({ "success": true, "provider_id": provider_id, "models": models }))
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json!({ "success": false, "provider_id": provider_id, "error": e.to_string(), "error_class": provider_conn_class(&e) }).to_string(),
        )
            .into_response(),
    }
}

/// POST /api/admin/providers/{id}/models — connect a model.
/// Body: { "upstream_model": "...", "display_name": "..." (optional) }
pub async fn providers_add_model_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return forbidden("provider plane not attached");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return bad("invalid JSON"),
    };
    let Some(upstream) = req.get("upstream_model").and_then(|m| m.as_str()) else {
        return bad("missing upstream_model");
    };
    let display = req
        .get("display_name")
        .and_then(|d| d.as_str())
        .map(str::to_string);
    let mut mgr = providers.lock().await;
    match mgr.connect_model(&provider_id, upstream, display) {
        Ok(model_id) => {
            let handle = mgr
                .model_by_id(&provider_id, &model_id)
                .map(|(_, m)| json!({ "model_id": m.model_id, "upstream_model": m.upstream_model, "symbolic_hash": m.symbolic_hash() }));
            record_provider_audit(
                &state,
                "model_connected",
                json!({ "provider_id": provider_id, "model_id": model_id, "upstream": upstream }),
            );
            ok_json(
                json!({ "success": true, "provider_id": provider_id, "model_id": model_id, "model": handle }),
            )
        }
        Err(e) => bad(&e.to_string()),
    }
}

/// DELETE /api/admin/providers/{id}/models/{model_id} — delete a model.
pub async fn providers_delete_model_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((provider_id, model_id)): Path<(String, String)>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return forbidden("provider plane not attached");
    };
    let mut mgr = providers.lock().await;
    match mgr.delete_model(&provider_id, &model_id) {
        Ok(()) => {
            record_provider_audit(
                &state,
                "model_deleted",
                json!({ "provider_id": provider_id, "model_id": model_id }),
            );
            ok_json(json!({ "success": true }))
        }
        Err(decentraai_providers::ProviderError::NotFound(_)) => not_found("provider not found"),
        Err(decentraai_providers::ProviderError::ModelNotFound(_, _)) => {
            not_found("model not found")
        }
        Err(e) => bad(&e.to_string()),
    }
}

/// POST /api/admin/providers/{id}/models/{model_id}/enable
/// Body: { "enabled": bool }
pub async fn providers_set_enabled_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((provider_id, model_id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return forbidden("provider plane not attached");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return bad("invalid JSON"),
    };
    let enabled = req
        .get("enabled")
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    let mut mgr = providers.lock().await;
    match mgr.set_model_enabled(&provider_id, &model_id, enabled) {
        Ok(()) => {
            record_provider_audit(
                &state,
                "model_enabled",
                json!({ "provider_id": provider_id, "model_id": model_id, "enabled": enabled }),
            );
            ok_json(json!({ "success": true, "enabled": enabled }))
        }
        Err(decentraai_providers::ProviderError::NotFound(_)) => not_found("provider not found"),
        Err(decentraai_providers::ProviderError::ModelNotFound(_, _)) => {
            not_found("model not found")
        }
        Err(e) => bad(&e.to_string()),
    }
}

/// POST /api/admin/providers/{id}/models/{model_id}/sharing
/// Body: full or partial SharingPolicy merge:
/// { "enabled": bool, "allowed_peers": [...], "required_trust_level": N,
///   "max_concurrency": N, "requests_per_minute": N, "daily_token_limit": N,
///   "daily_cost_limit": f64, "expires_at_ms": N, "require_authentication": bool,
///   "require_trusted_peer": bool }
pub async fn providers_sharing_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path((provider_id, model_id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return forbidden("provider plane not attached");
    };
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return bad("invalid JSON"),
    };
    let mut mgr = providers.lock().await;
    let Some((_, model)) = mgr.model_by_id(&provider_id, &model_id) else {
        return not_found("provider or model not found");
    };
    let mut policy = model.sharing.clone();
    if let Some(v) = req.get("enabled").and_then(|v| v.as_bool()) {
        policy.enabled = v;
    }
    if let Some(v) = req.get("allowed_peers").and_then(|v| v.as_array()) {
        policy.allowed_peers = v
            .iter()
            .filter_map(|p| p.as_str().map(str::to_string))
            .collect();
    }
    if let Some(v) = req.get("required_trust_level").and_then(|v| v.as_u64()) {
        policy.required_trust_level = v as u8;
    }
    if let Some(v) = req.get("max_concurrency").and_then(|v| v.as_u64()) {
        policy.max_concurrency = v as u32;
    }
    if let Some(v) = req.get("requests_per_minute").and_then(|v| v.as_u64()) {
        policy.requests_per_minute = v as u32;
    }
    if let Some(v) = req.get("daily_token_limit").and_then(|v| v.as_u64()) {
        policy.daily_token_limit = v;
    }
    if let Some(v) = req.get("daily_cost_limit").and_then(|v| v.as_f64()) {
        policy.daily_cost_limit = v;
    }
    if let Some(v) = req.get("expires_at_ms").and_then(|v| v.as_u64()) {
        policy.expires_at_ms = Some(v);
    }
    if let Some(v) = req.get("require_authentication").and_then(|v| v.as_bool()) {
        policy.require_authentication = v;
    }
    if let Some(v) = req.get("require_trusted_peer").and_then(|v| v.as_bool()) {
        policy.require_trusted_peer = v;
    }

    match mgr.set_sharing(&provider_id, &model_id, policy.clone()) {
        Ok(()) => {
            record_provider_audit(
                &state,
                "sharing_updated",
                json!({ "provider_id": provider_id, "model_id": model_id, "enabled": policy.enabled }),
            );
            ok_json(json!({ "success": true, "sharing": policy }))
        }
        Err(e) => bad(&e.to_string()),
    }
}

/// DELETE /api/admin/providers/{id} — remove a provider + its models.
pub async fn providers_delete_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    let Some(providers) = &state.providers else {
        return forbidden("provider plane not attached");
    };
    let mut mgr = providers.lock().await;
    match mgr.remove_provider(&provider_id) {
        Ok(()) => {
            record_provider_audit(
                &state,
                "provider_deleted",
                json!({ "provider_id": provider_id }),
            );
            ok_json(json!({ "success": true }))
        }
        Err(decentraai_providers::ProviderError::NotFound(_)) => not_found("provider not found"),
        Err(e) => bad(&e.to_string()),
    }
}

fn provider_conn_class(e: &decentraai_providers::adapter::ProviderConnError) -> &'static str {
    match e {
        decentraai_providers::adapter::ProviderConnError::InvalidCredentials(_) => "auth",
        decentraai_providers::adapter::ProviderConnError::Network(_) => "network",
        decentraai_providers::adapter::ProviderConnError::Transport(_) => "transport",
        decentraai_providers::adapter::ProviderConnError::Timeout(_) => "timeout",
        decentraai_providers::adapter::ProviderConnError::Protocol(_) => "protocol",
        decentraai_providers::adapter::ProviderConnError::HttpError { error_class, .. } => {
            error_class.clone().as_str()
        }
    }
}

fn record_provider_audit(state: &ApiState, event: &str, details: serde_json::Value) {
    let a = state.info.repo_root.join("logs/audit.jsonl");
    let _ = decentraai_audit::record(a.parent().unwrap_or(&state.info.repo_root), event, details);
}

// ---------------------------------------------------------------------------
// Provider-backed inference routing (Model Fabric)
// ---------------------------------------------------------------------------

/// Try to serve a `/v1/chat/completions` request from a connected provider
/// model. Returns `Some(response)` when the requested model resolves to a
/// provider model (symbolic hash `prov-…`, provider handle
/// `provider:{provider_id}:{model_id}`, or the raw upstream model name), and
/// `None` when the model is local/fabric/unknown (proxy continues as usual).
///
/// Never sends credentials anywhere except the configured provider base URL;
/// the request body is forwarded as-is (OpenAI-compatible contract).
pub async fn resolve_provider_model(state: &ApiState, outgoing: &[u8]) -> Option<Response> {
    let Some(providers) = &state.providers else {
        return None;
    };
    let body_val: serde_json::Value = serde_json::from_slice(outgoing).ok()?;
    let model = body_val.get("model").and_then(|m| m.as_str())?;
    if model.is_empty() {
        return None;
    }
    let prompt = body_val
        .get("messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|msg| msg.get("content").and_then(|c| c.as_str()))
                .collect::<Vec<_>>()
                .join("\n")
                .into()
        })
        .unwrap_or_default();
    let max_tokens = body_val
        .get("max_tokens")
        .and_then(|m| m.as_u64())
        .unwrap_or(1024) as u32;
    let temperature = body_val
        .get("temperature")
        .and_then(|t| t.as_f64())
        .unwrap_or(0.7) as f32;
    let wants_stream = body_val
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    // Resolve the model to a connected provider model. Only enabled models
    // are reachable. An explicit provider handle wins over name matching;
    // otherwise match the symbolic hash or the raw upstream model name.
    let (provider, model_entry) = {
        let mgr = providers.lock().await;
        let found = mgr.providers().iter().find_map(|p| {
            p.models.iter().find_map(|m| {
                let handle = format!("provider:{}:{}", p.provider_id, m.model_id);
                let hash = m.symbolic_hash();
                let upstream = m.upstream_model.clone();
                let name_matches = m.display_name == model;
                if handle == model || hash == model || upstream == model || name_matches {
                    Some((p.clone(), m.clone()))
                } else {
                    None
                }
            })
        });
        found?
    };
    if !model_entry.enabled {
        return Some(bad("model is disabled"));
    }

    let credential_store = {
        let mgr = providers.lock().await;
        mgr.credential_store().clone()
    };
    let adapter = decentraai_providers::adapter::ModelAdapter::new(
        provider.base_url.clone(),
        model_entry.upstream_model.clone(),
        credential_store,
        provider.credential_ref.clone(),
    );
    let request = decentraai_inference_adapter::BackendRequest {
        request_id: format!("prov-{}", model_entry.model_id),
        prompt,
        max_tokens,
        temperature,
        top_p: 0.9,
    };

    if wants_stream {
        match adapter.stream(&request).await {
            Ok(mut token_stream) => {
                let (tx, rx) = tokio::sync::mpsc::channel(16);
                let model_name = model.to_string();
                tokio::spawn(async move {
                    let mut seq: u64 = 0;
                    while let Some(chunk) = token_stream.next().await {
                        match chunk {
                            Ok(c) => {
                                let event = format!(
                                    "data: {{\"id\":\"{}\",\"object\":\"chat.completion.chunk\",\"created\":{},\"model\":{},\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{}}},\"finish_reason\":{}}}]}}\n\n",
                                    c.request_id,
                                    chrono::Utc::now().timestamp(),
                                    serde_json::to_string(&model_name)
                                        .unwrap_or_else(|_| "\"\"".to_string()),
                                    serde_json::to_string(&c.text)
                                        .unwrap_or_else(|_| "\"\"".to_string()),
                                    c.finish_reason
                                        .as_ref()
                                        .map(|_| "\"stop\"".to_string())
                                        .unwrap_or_else(|| "null".to_string()),
                                );
                                let _ = tx
                                    .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(event)))
                                    .await;
                                seq += 1;
                            }
                            Err(_) => break,
                        }
                    }
                    let _ = tx
                        .send(Ok::<Bytes, std::convert::Infallible>(Bytes::from(
                            "data: [DONE]\n\n".to_string(),
                        )))
                        .await;
                    drop(tx);
                    let _ = seq;
                });
                let body = axum::body::Body::from_stream(futures::stream::unfold(
                    rx,
                    |mut rx| async move { rx.recv().await.map(|item| (item, rx)) },
                ));
                let mut response = (StatusCode::OK, body).into_response();
                response.headers_mut().insert(
                    axum::http::header::CONTENT_TYPE,
                    axum::http::HeaderValue::from_static("text/event-stream"),
                );
                Some(response)
            }
            Err(e) => Some(provider_error_response(&e)),
        }
    } else {
        match adapter.complete(&request).await {
            Ok(resp) => {
                let created = chrono::Utc::now().timestamp();
                let json_body = json!({
                    "id": format!("chatcmpl-prov-{}", model_entry.model_id),
                    "object": "chat.completion",
                    "created": created,
                    "model": model,
                    "choices": [{
                        "index": 0,
                        "message": { "role": "assistant", "content": resp.output },
                        "finish_reason": resp.finish_reason.unwrap_or_else(|| "stop".to_string()),
                    }],
                    "usage": {
                        "prompt_tokens": 0,
                        "completion_tokens": resp.tokens_used.unwrap_or(0),
                        "total_tokens": resp.tokens_used.unwrap_or(0),
                    }
                });
                Some(
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        json_body.to_string(),
                    )
                        .into_response(),
                )
            }
            Err(e) => Some(provider_error_response(&e)),
        }
    }
}

fn provider_error_response(e: &decentraai_providers::adapter::ProviderInferError) -> Response {
    let (status, err_type, message) = match e {
        decentraai_providers::adapter::ProviderInferError::Timeout => {
            (StatusCode::GATEWAY_TIMEOUT, "timeout_error", e.to_string())
        }
        decentraai_providers::adapter::ProviderInferError::PromptTooLarge
        | decentraai_providers::adapter::ProviderInferError::OutputLimitExceeded => (
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            e.to_string(),
        ),
        decentraai_providers::adapter::ProviderInferError::CredentialNotFound(_)
        | decentraai_providers::adapter::ProviderInferError::CredentialLock
        | decentraai_providers::adapter::ProviderInferError::ProviderError(
            decentraai_providers::ProviderErrorClass::Auth,
            _,
        ) => (
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            e.to_string(),
        ),
        _ => (StatusCode::BAD_GATEWAY, "server_error", e.to_string()),
    };
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json!({ "error": { "message": message, "type": err_type } }).to_string(),
    )
        .into_response()
}
