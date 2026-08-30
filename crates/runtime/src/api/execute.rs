//! Auto-extracted execute module from api/mod.rs.
//! Re-exported via `pub(crate) use execute::*` in mod.rs.

use super::*;

pub(crate) async fn execute_decision_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = state.require_master(&headers) {
        return e.into_response();
    }
    // Phase M LIMITS: mutations are rate-limited per token name (master here,
    // since execute is master-gated) so the fabric cannot be hammered.
    if let Err(e) = state.check_execute_rate_limit("master") {
        return e.into_response();
    }
    let req: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return forbidden("invalid JSON"),
    };
    // STREAM step: when the caller asks for a stream, emit SSE from the
    // fabric router instead of a single buffered JSON body.
    if req.get("stream").and_then(|s| s.as_bool()).unwrap_or(false) {
        return execute_decision_stream(&state, &req).await;
    }
    run_execute_decision(&state, &req).await
}
/// The STREAM step of decide→confirm→reserve→execute: run the decided model on
/// the fabric and stream the output as SSE (like the chat proxy's remote route),
pub(crate) async fn execute_decision_stream(state: &ApiState, req: &serde_json::Value) -> Response {
    // Mutation safety: explicit confirmation is required.
    if req.get("confirm").and_then(|c| c.as_bool()) != Some(true) {
        return forbidden("mutating execution requires \"confirm\": true");
    }
    let prompt = req
        .get("prompt")
        .and_then(|p| p.as_str())
        .unwrap_or_default();
    if prompt.trim().is_empty() {
        return forbidden("missing prompt");
    }
    // Intent OR a direct capability can drive the decision (capability alone
    // lets an operator run a specific capability+model without intent parsing).
    let intent = execute_decision_intent(req);
    if intent.trim().is_empty() {
        return forbidden("missing intent (or a capability to run)");
    }
    let max_tokens = req
        .get("max_tokens")
        .and_then(|m| m.as_u64())
        .unwrap_or(1024)
        .min(4096) as u32;
    let evidence = req
        .get("evidence")
        .and_then(|e| e.as_str())
        .unwrap_or("any");
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    };
    let explicit_model = req.get("model").and_then(|m| m.as_str());

    // decide → chosen model.
    let decision = unified_fabric_decision(state, &intent, evidence, explicit_model).await;
    let Some(model) = decision["decision"]["model"].as_str().map(str::to_string) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": "no runnable decision on the fabric for this intent (nothing to execute)", "type": "unprocessable" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };
    let Some(model_hash) = resolve_model_hash(state, &model).await else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": format!("chosen model '{model}' has no advertised model hash on the fabric"), "type": "unprocessable" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };

    // DRY-RUN: show exactly what would be reserved/routed without executing.
    // Requires the same confirm gate (it is part of the mutation path), but
    // never sends a request or holds a reservation.
    if req
        .get("dry_run")
        .and_then(|d| d.as_bool())
        .unwrap_or(false)
    {
        let prompt_tokens = decentraai_distributed::prompt_token_estimate(prompt);
        let preview = match &state.compute {
            Some(cm) => {
                cm.plan_preview(
                    &model_hash,
                    prompt_tokens,
                    req.get("session_id").and_then(|s| s.as_str()),
                    req.get("priority").and_then(|p| p.as_u64()).unwrap_or(0) as u8,
                )
                .await
            }
            None => None,
        };
        return match preview {
            Some((plan, worker, est_ms)) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "dry_run": true,
                    "decision": decision,
                    "would_execute": {
                        "model": model,
                        "model_hash": model_hash,
                        "worker": worker,
                        "estimated_ms": est_ms,
                        "plan_id": plan.plan_id,
                        "stages": plan.stage_count(),
                    },
                    "note": "dry-run preview only — no request sent, no reservation held",
                })
                .to_string(),
            )
                .into_response(),
            None => (
                StatusCode::UNPROCESSABLE_ENTITY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": { "message": "no eligible worker on the fabric for this model (nothing would be executed)", "type": "unprocessable" },
                    "decision": decision,
                    "dry_run": true,
                })
                .to_string(),
            )
                .into_response(),
        };
    }

    let Some(distributed) = state.distributed.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": "fabric router unavailable", "type": "server_error" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };

    let mut request = decentraai_distributed::InferRequest::new(
        model_hash.clone(),
        prompt.to_string(),
        max_tokens,
    )
    .with_sender(distributed.p2p_node().local_peer_id())
    .with_streaming(true);
    request.timeout_ms = remote_request_timeout_ms();
    if let Some(sid) = req
        .get("session_id")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
    {
        request = request.with_session(sid.to_string());
    }

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let dist = distributed.clone();
    let resp_task =
        tokio::spawn(async move { dist.route_request_streamed(request, progress_tx).await });
    let (body_tx, body_rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(64);
    let state2 = state.clone();
    let started = std::time::Instant::now();
    let model2 = model.clone();
    // Owned copy for the spawned task (the raw `prompt` borrows `req`, which
    // does not live long enough for tokio::spawn).
    let prompt_owned = prompt.to_string();
    tokio::spawn(async move {
        while let Some(chunk) = progress_rx.recv().await {
            if chunk.is_empty() {
                continue;
            }
            let payload = format!(
                "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{}}}}}]}}\n\n",
                serde_json::to_string(&chunk).unwrap_or_else(|_| "\"\"".to_string())
            );
            if body_tx.send(Ok(Bytes::from(payload))).await.is_err() {
                break;
            }
        }
        let final_event = match resp_task.await {
            Ok(Ok(resp)) => {
                // Report the real prompt token estimate (the remote worker does
                // not echo usage through the streamed path); token.input would
                // otherwise read 0 in gen_ai.server.token.input metrics.
                let prompt_tokens = decentraai_distributed::prompt_token_estimate(&prompt_owned);
                let usage = format!(
                    "{{\"usage\":{{\"prompt_tokens\":{},\"completion_tokens\":{}}}}}",
                    prompt_tokens, resp.tokens_used
                );
                state2.record_inference("/v1/execute", started.elapsed(), usage.as_bytes());
                format!(
                    "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}],\"model\":{},\"usage\":{{\"prompt_tokens\":{},\"completion_tokens\":{}}}}}\n\n",
                    serde_json::to_string(&model2).unwrap_or_else(|_| "\"\"".to_string()),
                    prompt_tokens,
                    resp.tokens_used
                )
            }
            Ok(Err(_)) => {
                state2.requests_failed.fetch_add(1, Ordering::SeqCst);
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"error\"}]}\n\n"
                    .to_string()
            }
            Err(_) => String::new(),
        };
        let _ = body_tx.send(Ok(Bytes::from(final_event))).await;
        let _ = body_tx
            .send(Ok(Bytes::from("data: [DONE]\n\n".to_string())))
            .await;
    });
    let body = Body::from_stream(futures::stream::unfold(body_rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }));
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/event-stream"),
    );
    response
}
/// Core decide→reserve→execute logic, shared by the HTTP handler and the MCP
/// `execute_decision` tool. Enforces the mutation-safety confirmation itself
/// (so no caller can bypass it) and requires the node master token (checked by
/// the HTTP layer; MCP runs behind the same master-gated boundary).
/// Derive the intent string that drives the unified decision for an execute
/// call: the explicit `intent` if present, else the `capability` (a snake_case
pub(crate) fn execute_decision_intent(req: &serde_json::Value) -> String {
    if let Some(i) = req
        .get("intent")
        .and_then(|i| i.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        return i.trim().to_string();
    }
    if let Some(c) = req
        .get("capability")
        .and_then(|c| c.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        return c.trim().to_string();
    }
    String::new()
}
pub(crate) async fn run_execute_decision(state: &ApiState, req: &serde_json::Value) -> Response {
    // Mutation safety: explicit confirmation is required.
    if req.get("confirm").and_then(|c| c.as_bool()) != Some(true) {
        return forbidden("mutating execution requires \"confirm\": true");
    }
    let intent = execute_decision_intent(req);
    if intent.trim().is_empty() {
        return forbidden("missing intent (or a capability to run)");
    }
    let prompt = req
        .get("prompt")
        .and_then(|p| p.as_str())
        .unwrap_or_default();
    if prompt.trim().is_empty() {
        return forbidden("missing prompt");
    }
    let max_tokens = req
        .get("max_tokens")
        .and_then(|m| m.as_u64())
        .unwrap_or(1024)
        .min(4096) as u32;
    let stream = req.get("stream").and_then(|s| s.as_bool()).unwrap_or(false);
    let evidence = req
        .get("evidence")
        .and_then(|e| e.as_str())
        .unwrap_or("any");
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    };
    let explicit_model = req.get("model").and_then(|m| m.as_str());

    // decide: pick the first CAN_RUN model/worker from the unified projection.
    let decision = unified_fabric_decision(state, &intent, evidence, explicit_model).await;
    let chosen_model = decision["decision"]["model"].as_str().map(str::to_string);

    // reserve+execute requires a real, advertised model hash.
    let Some(model) = chosen_model else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": "no runnable decision on the fabric for this intent (nothing to execute)", "type": "unprocessable" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };
    let Some(model_hash) = resolve_model_hash(state, &model).await else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::json!({
                "error": { "message": format!("chosen model '{model}' has no advertised model hash on the fabric"), "type": "unprocessable" },
                "decision": decision,
            })
            .to_string(),
        )
            .into_response();
    };

    // Execute through the existing fabric router (reserve + route + audit).
    let distributed = match &state.distributed {
        Some(d) => d.clone(),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": { "message": "fabric router unavailable", "type": "server_error" },
                    "decision": decision,
                })
                .to_string(),
            )
                .into_response();
        }
    };
    let mut request = decentraai_distributed::InferRequest::new(
        model_hash.clone(),
        prompt.to_string(),
        max_tokens,
    )
    .with_sender(distributed.p2p_node().local_peer_id())
    .with_streaming(stream);
    request.timeout_ms = remote_request_timeout_ms();
    // Continuation support (KV locality): an optional session_id links this run
    // to an earlier one, steering the fabric router back to the worker holding
    // the session's KV prefix.
    if let Some(sid) = req
        .get("session_id")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
    {
        request = request.with_session(sid.to_string());
    }
    let started = std::time::Instant::now();
    match distributed.route_request(request).await {
        Ok(resp) => {
            let elapsed = started.elapsed();
            state.record_inference(
                "/v1/execute",
                elapsed,
                format!(
                    "{{\"usage\":{{\"prompt_tokens\":0,\"completion_tokens\":{}}}}}",
                    resp.tokens_used
                )
                .as_bytes(),
            );
            // MEASURE + HISTORY steps: real measured tokens/time/tps, plus the
            // updated historical stats from the execution the router just
            // recorded (UNKNOWN when no compute manager).
            let measured = {
                let secs = (elapsed.as_millis().max(1) as f64) / 1000.0;
                let tps = (f64::from(resp.tokens_used) / secs).round();
                serde_json::json!({
                    "tokens_used": resp.tokens_used,
                    "latency_ms": elapsed.as_millis() as u64,
                    "tokens_per_sec": if resp.tokens_used > 0 { tps } else { 0.0 },
                    "provenance": "MEASURED",
                })
            };
            let historical = state
                .compute
                .as_ref()
                .map(|cm| decentraai_distributed::execution_statistics(&cm.executions()))
                .unwrap_or_else(|| serde_json::json!({ "records": 0 }));
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "decision": decision,
                    "executed": {
                        "model": model,
                        "model_hash": model_hash,
                        "output": resp.output,
                        "tokens_used": resp.tokens_used,
                        "processing_time_ms": resp.processing_time_ms,
                        "worker": resp.worker_peer_id.to_string(),
                    },
                    "measure": measured,
                    "historical": historical,
                })
                .to_string(),
            )
                .into_response()
        }
        Err(e) => {
            // REPLAN advisory (Phase H vocabulary): on a retryable transport
            // failure with remaining eligible workers, advise a retry/replan
            // onto an alternative; otherwise abort. This is advisory-only — the
            // router already retried internally; we never claim an action the
            // runtime did not take.
            let retryable = e.is_retryable();
            let alternatives = decision["capabilities"]
                .as_array()
                .map(|caps| {
                    caps.iter()
                        .flat_map(|c| c["model_options"].as_array().cloned().unwrap_or_default())
                        .filter(|m| m["verdict"] == "CAN_RUN")
                        .count()
                })
                .unwrap_or(0);
            let adv = decentraai_fabric::decision::adapt(
                false,        // outcome_ok
                retryable,    // retryable
                false,        // cancelled
                0,            // tokens_emitted (no output was returned)
                alternatives, // eligible_after_primary
                1,            // replan_budget
                false,        // is_continuation
            );
            let replan = match adv {
                decentraai_fabric::decision::Adaptation::Retry
                | decentraai_fabric::decision::Adaptation::Replan => {
                    if alternatives > 0 {
                        "REPLAN_AVAILABLE"
                    } else {
                        "NO_ALTERNATIVE"
                    }
                }
                decentraai_fabric::decision::Adaptation::Abort => "ABORT",
                decentraai_fabric::decision::Adaptation::Continue => "NO_RETRY_NEEDED",
            };
            (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({
                    "error": { "message": e.to_string(), "type": "execution_error" },
                    "decision": decision,
                    "replan": {
                        "advisory": replan,
                        "retryable": retryable,
                        "eligible_alternatives": alternatives,
                        "note": "advisory only; the router already applied its own retry/fallback",
                    },
                })
                .to_string(),
            )
                .into_response()
        }
    }
}
