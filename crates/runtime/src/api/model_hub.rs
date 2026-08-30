//! Auto-extracted model_hub module from api/mod.rs.
//! Re-exported via `pub(crate) use model_hub::*` in mod.rs.

use super::*;

pub(crate) fn load_model_intel_registry(
    path: &std::path::Path,
) -> decentraai_hub::model_intel::ModelIntelRegistry {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_else(decentraai_hub::model_intel::seed_model_colony)
}
pub(crate) fn save_model_intel_registry(
    path: &std::path::Path,
    registry: &decentraai_hub::model_intel::ModelIntelRegistry,
) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(registry) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}
pub(crate) fn ram_pressure_percent() -> u8 {
    let snap = decentraai_system_probe::SystemSnapshot::collect();
    if snap.total_memory_bytes == 0 {
        return 0;
    }
    let used = snap
        .total_memory_bytes
        .saturating_sub(snap.available_memory_bytes);
    ((used * 100) / snap.total_memory_bytes).min(100) as u8
}
/// GET /v1/models/intel — the Model Colony view (operator+): seeded
/// registry facts (governance, claims, hardware) joined with runtime
/// availability (which model is actually loaded) and VERIFIED performance
/// observations from Collective Memory. Read-only; nothing here promotes
pub(crate) async fn models_intel_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let Some(shared) = &state.model_intel else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "model intelligence not attached"})),
        )
            .into_response();
    };
    let registry_snapshot = shared.read().expect("model_intel lock").clone();
    let pressure = ram_pressure_percent();
    let active = state.active_model.read().await.clone();
    let mut rows = Vec::new();
    for record in registry_snapshot.all() {
        // Honest availability: we only KNOW about the loaded engine.
        let availability =
            if normalize_model_name(&active) == normalize_model_name(&record.model_id) {
                "available"
            } else {
                "unavailable"
            };
        let observed = state.memory.as_ref().and_then(|m| {
            decentraai_distributed::model_performance::aggregate_model(m, &record.model_id).ok()
        });
        let mut v = record.summary();
        v["availability"] = serde_json::json!(availability);
        v["ram_pressure_percent"] = serde_json::json!(pressure);
        v["observed"] = match observed {
            Some(o) => serde_json::json!({
                "samples": o.samples,
                "success_percent": o.success_percent,
                "mean_latency_ms": o.mean_latency_ms,
            }),
            None => serde_json::json!(null),
        };
        rows.push(v);
    }
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({
            "models": rows,
            "advisory": true,
            "invariant": "AI proposes -> deterministic policy decides -> workers execute",
        })),
    )
        .into_response()
}
pub(crate) fn normalize_model_name(name: &str) -> String {
    name.to_lowercase()
        .replace(['.', '_', ' '], "-")
        .trim_matches('-')
        .to_string()
}
/// POST /v1/models/route — DRY-RUN routing projection (operator+).
/// Body: {"capability":"reasoning","min_context_tokens":4096,
///        "traffic":"production"|"shadow"|"benchmark"}.
/// Deterministic policy output: selected + ordered fallbacks + every hard-gate
/// rejection with its reason. ADVISORY ONLY — actual serving still goes
pub(crate) async fn models_route_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: axum::Json<serde_json::Value>,
) -> Response {
    use decentraai_fabric::model_routing::{
        ObservedPerformance, RouteNeed, RoutedCandidate, TrafficClass, route,
    };
    use decentraai_hub::model_intel::AvailabilityState;
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let Some(cap_str) = body.0.get("capability").and_then(|v| v.as_str()) else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": "capability is required"
            })),
        )
            .into_response();
    };
    let Some(required) = serde_json::from_value::<decentraai_hub::capability::CapabilityKind>(
        serde_json::Value::String(cap_str.to_string()),
    )
    .ok() else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": format!("unknown capability '{cap_str}'; see hub taxonomy")
            })),
        )
            .into_response();
    };
    let traffic =
        match body
            .0
            .get("traffic")
            .and_then(|v| v.as_str())
            .unwrap_or("production")
        {
            "production" => TrafficClass::Production,
            "shadow" => TrafficClass::Shadow,
            "benchmark" => TrafficClass::Benchmark,
            other => return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "error": format!("traffic must be production|shadow|benchmark, got '{other}'")
                })),
            )
                .into_response(),
        };
    let need = RouteNeed {
        required,
        min_context_tokens: body
            .0
            .get("min_context_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(4096)
            .min(u32::MAX as u64) as u32,
        traffic,
    };

    let Some(shared) = &state.model_intel else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "model intelligence not attached"})),
        )
            .into_response();
    };
    let registry = shared.read().expect("model_intel lock").clone();
    let pressure = ram_pressure_percent();
    let active = state.active_model.read().await.clone();
    let candidates_owned: Vec<decentraai_hub::model_intel::ModelIntelRecord> =
        registry.all().into_iter().cloned().collect();
    let candidates: Vec<RoutedCandidate<'_>> = candidates_owned
        .iter()
        .map(|record| {
            let availability =
                if normalize_model_name(&active) == normalize_model_name(&record.model_id) {
                    AvailabilityState::Available
                } else {
                    AvailabilityState::Unavailable
                };
            let observed = state.memory.as_ref().and_then(|m| {
                decentraai_distributed::model_performance::aggregate_model(m, &record.model_id)
                    .ok()
                    .filter(|o| o.samples > 0)
            });
            RoutedCandidate {
                record,
                availability,
                observed: observed.map(|o| ObservedPerformance {
                    success_percent: o.success_percent.min(255) as u8,
                    mean_latency_ms: o.mean_latency_ms,
                }),
                ram_pressure_percent: pressure,
            }
        })
        .collect();

    let decision = route(&candidates, &need);
    let payload = serde_json::json!({
        "need": {
            "capability": required,
            "min_context_tokens": need.min_context_tokens,
            "traffic": match traffic {
                TrafficClass::Production => "production",
                TrafficClass::Shadow => "shadow",
                TrafficClass::Benchmark => "benchmark",
            },
        },
        "selected": decision.selected,
        "fallbacks": decision.fallbacks,
        "rejections": decision.rejections.iter().map(|r| serde_json::json!({
            "model_id": r.model_id, "reason": r.reason,
        })).collect::<Vec<_>>(),
        "advisory": true,
        "note": "dry-run projection — the deterministic planner still owns real placement",
    });
    (StatusCode::OK, axum::Json(payload)).into_response()
}
/// POST /v1/models/governance — apply a gated lifecycle transition to a
/// colony model (operator+). Body: {"model_id":"…","to":"shadow"|"candidate"|
/// "approved"|"rejected"}. The state machine validates the jump; the new
/// stage persists to db/model_intel.json (tmp+rename); audited. This is the
/// ONLY path from shadow recommendation to approved — evidence first,
pub(crate) async fn models_governance_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: axum::Json<serde_json::Value>,
) -> Response {
    use decentraai_hub::model_intel::GovernanceStage;
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let Some(shared) = &state.model_intel else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "model intelligence not attached"})),
        )
            .into_response();
    };
    let Some(model_id) = body.0.get("model_id").and_then(|v| v.as_str()) else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "model_id is required"})),
        )
            .into_response();
    };
    let Some(to_raw) = body.0.get("to").and_then(|v| v.as_str()) else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "to (stage) is required"})),
        )
            .into_response();
    };
    let Ok(to) =
        serde_json::from_value::<GovernanceStage>(serde_json::Value::String(to_raw.to_string()))
    else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": "invalid stage; expected experimental|shadow|candidate|approved|rejected"
            })),
        )
            .into_response();
    };
    let mut registry = shared.write().expect("model_intel lock");
    match registry.transition_governance(model_id, to) {
        Ok(applied) => {
            state.save_model_intel(&registry);
            decentraai_audit::record_best_effort(
                &state.info.repo_root.join("logs"),
                "model_governance_transition",
                serde_json::json!({ "model_id": model_id, "to": applied }),
            );
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "ok": true, "model_id": model_id, "governance": applied,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::CONFLICT,
            axum::Json(serde_json::json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}
pub(crate) async fn fabric_model_list(state: &ApiState) -> Option<serde_json::Value> {
    let compute = state.compute.as_ref()?;
    let local_peer = compute.local_peer();
    let workers = compute.workers().await;
    // Best-effort local registry: source of persisted capability claims for
    // the local model entries (no Hub round-trip). A failure to load simply
    // omits the field — the model list must never break on registry trouble.
    let registry =
        decentraai_registry::ModelRegistry::load(&state.info.repo_root.join("db/registry.json"))
            .ok();
    // id (file name) → (owned_by, is_local)
    let mut seen: std::collections::BTreeMap<String, (String, bool)> =
        std::collections::BTreeMap::new();
    for w in &workers {
        let is_local = w.peer_id == local_peer;
        if !is_local && !w.accepts_remote_inference {
            continue;
        }
        let owned_by = if is_local {
            "local".to_string()
        } else {
            w.node_id.clone()
        };
        for m in w
            .capability
            .served_models
            .iter()
            .chain(w.capability.available_models.iter())
        {
            match seen.get(&m.file_name) {
                Some((_, true)) => {} // local already wins
                Some(_) if is_local => {
                    seen.insert(m.file_name.clone(), (owned_by.clone(), true));
                }
                None => {
                    seen.insert(m.file_name.clone(), (owned_by.clone(), is_local));
                }
                _ => {}
            }
        }
    }
    let data: Vec<serde_json::Value> = seen
        .into_iter()
        .map(|(id, (owned_by, is_local))| {
            let mut entry = serde_json::json!({
                "id": id,
                "object": "model",
                "created": 0,
                "owned_by": owned_by,
            });
            // Local models only: attach real persisted capability data when it
            // exists. Absent means UNKNOWN — never force an empty list.
            if is_local {
                if let Some(reg) = &registry {
                    let claims = claims_for_file_name(reg, &id);
                    if !claims.is_empty() {
                        entry["capability_claims"] =
                            serde_json::to_value(claims).unwrap_or(serde_json::Value::Null);
                    }
                }
            }
            entry
        })
        .collect();
    Some(serde_json::json!({ "object": "list", "data": data }))
}
/// `GET /v1/models` — OpenAI model list. With the fabric attached this is the
/// fabric-wide view (models served *or* available on disk across all trusted
/// workers); without the fabric it is the plain backend passthrough, so a
pub(crate) async fn models_handler(
    State(state): State<ApiState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let auth = match state.classify(&headers) {
        Ok(auth) => auth,
        Err(e) => return e.into_response(),
    };
    if state.compute.is_some() {
        if let Some(list) = fabric_model_list(&state).await {
            return (
                [(header::CONTENT_TYPE, "application/json")],
                list.to_string(),
            )
                .into_response();
        }
    }
    // Fall back to the backend passthrough (standalone node, no fabric).
    proxy_with_auth(State(state), method, uri, headers, body, auth).await
}
/// `GET /v1/models/{id}` — OpenAI single-model view over the fabric list.
pub(crate) async fn model_detail_handler(
    State(state): State<ApiState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    AxumPath(model_id): AxumPath<String>,
    body: Bytes,
) -> Response {
    let auth = match state.classify(&headers) {
        Ok(auth) => auth,
        Err(e) => return e.into_response(),
    };
    if let Some(list) = fabric_model_list(&state).await {
        let found = list["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|m| m["id"].as_str() == Some(model_id.as_str()))
            .cloned();
        if let Some(entry) = found {
            return (
                [(header::CONTENT_TYPE, "application/json")],
                entry.to_string(),
            )
                .into_response();
        }
        let body = serde_json::json!({
            "error": {"message": format!("model '{model_id}' not found on the fabric"), "type": "not_found"}
        });
        return (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            body.to_string(),
        )
            .into_response();
    }
    proxy_with_auth(State(state), method, uri, headers, body, auth).await
}
