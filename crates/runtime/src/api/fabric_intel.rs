//! Auto-extracted fabric_intel module from api/mod.rs.
//! Re-exported via `pub(crate) use fabric_intel::*` in mod.rs.

use super::*;

pub(crate) async fn mcp_capability_search(
    query: &str,
    limit: usize,
    capability: decentraai_hub::CapabilityKind,
) -> serde_json::Value {
    let catalog = decentraai_hub::HubCatalog::new();
    match catalog.search(query, limit).await {
        Ok(models) => hub_search_body(query, &models, Some(capability)),
        Err(e) => serde_json::json!({
            "error": e.to_string(),
            "matched": 0,
            "models": [],
        }),
    }
}
pub(crate) async fn mcp_local_capability_search(
    state: &ApiState,
    capability: &str,
    evidence: &str,
) -> serde_json::Value {
    let Some(list) = fabric_model_list(state).await else {
        return serde_json::json!({ "matched": 0, "models": [] });
    };
    filter_local_models_by_capability(&list, capability, evidence)
}
pub(crate) fn filter_local_models_by_capability(
    list: &serde_json::Value,
    capability: &str,
    evidence: &str,
) -> serde_json::Value {
    let require_verified = evidence == "verified";
    let mut matched = Vec::new();
    for m in list["data"].as_array().cloned().unwrap_or_default() {
        let Some(claims) = m["capability_claims"].as_array() else {
            continue; // no persisted claims -> UNKNOWN, not a match
        };
        let hit = claims.iter().find(|c| {
            let cap = c["capability"].as_str().unwrap_or("");
            let prov = c["provenance"].as_str().unwrap_or("");
            cap.eq_ignore_ascii_case(capability)
                && (!require_verified || prov.eq_ignore_ascii_case("verified"))
        });
        if let Some(hit) = hit {
            matched.push(serde_json::json!({
                "id": m["id"],
                "evidence": hit["provenance"],
            }));
        }
    }
    serde_json::json!({
        "capability": capability,
        "evidence": if require_verified { "verified" } else { "any" },
        "matched": matched.len(),
        "models": matched,
    })
}
pub(crate) async fn mcp_worker_capability(
    state: &ApiState,
    model: &str,
    capability: &str,
    evidence: &str,
) -> serde_json::Value {
    // Best-effort registry load for persisted claims (absent => UNKNOWN).
    let registry_path = state.info.repo_root.join("db/registry.json");
    let registry = decentraai_registry::ModelRegistry::load(&registry_path).ok();
    let claims: Vec<(String, String)> = registry
        .as_ref()
        .map(|reg| {
            claims_for_file_name(reg, model)
                .into_iter()
                .map(|c| (c.capability, c.provenance))
                .collect()
        })
        .unwrap_or_default();

    // Fetch the worker set once and reuse it for the requested model and for
    // every on-disk variant below (workers()/is_trusted are async I/O).
    let mut workers: Vec<(decentraai_compute::ComputeAdvertisement, bool)> = Vec::new();
    let mut local_peer: Option<String> = None;
    if let Some(cm) = &state.compute {
        local_peer = Some(cm.local_peer().to_string());
        for adv in cm.workers().await {
            let trusted = cm.is_trusted(&adv.peer_id).await;
            workers.push((adv, trusted));
        }
    }

    let mut results: Vec<WorkerCapResult> = Vec::new();
    for (adv, trusted) in &workers {
        let is_local = local_peer.as_deref() == Some(&adv.peer_id.to_string());
        let accepts_remote_work = is_local || adv.accepts_remote_inference;
        results.push(worker_capability_verdict_with_policy(
            adv,
            *trusted,
            model,
            capability,
            evidence,
            &claims,
            accepts_remote_work,
        ));
    }
    let fit = aggregate_can_i_run(&results);
    let workers_json: Vec<serde_json::Value> = results.iter().map(|r| r.to_json()).collect();

    // Honest model metadata: quantization is INFERRED from the requested model
    // string when it carries a recognized marker, else null (UNKNOWN);
    // available_workers counts workers that actually hold the model (served or
    // on-disk), derived from the real per-worker verdicts above.
    let quantization = variant_quantization_from_file_name(model);
    let available_workers = results
        .iter()
        .filter(|r| r.model_availability != "unavailable")
        .count();

    // On-disk GGUF variants of this model from the REAL local registry (never
    // invented). Each variant is evaluated by the SAME per-worker pipeline as
    // the requested model, so a variant with no matching worker honestly
    // resolves to CANNOT_RUN/UNKNOWN via the existing aggregate.
    let variants: Vec<serde_json::Value> = registry
        .as_ref()
        .map(|reg| {
            registry_variants_for_model(reg, model)
                .into_iter()
                .map(|(file, size_bytes)| {
                    let v_claims: Vec<(String, String)> = claims_for_file_name(reg, &file)
                        .into_iter()
                        .map(|c| (c.capability, c.provenance))
                        .collect();
                    let mut v_results: Vec<WorkerCapResult> = Vec::new();
                    for (adv, trusted) in &workers {
                        let is_local = local_peer.as_deref() == Some(&adv.peer_id.to_string());
                        let accepts_remote_work = is_local || adv.accepts_remote_inference;
                        v_results.push(worker_capability_verdict_with_policy(
                            adv,
                            *trusted,
                            &file,
                            capability,
                            evidence,
                            &v_claims,
                            accepts_remote_work,
                        ));
                    }
                    let v_fit = aggregate_can_i_run(&v_results);
                    serde_json::json!({
                        "file": file,
                        "quantization": variant_quantization_from_file_name(&file),
                        "size_bytes": size_bytes,
                        "fit": v_fit.to_json(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Best on-disk variant to deploy on THIS fabric: the first variant whose
    // fit is CAN_RUN (variants are in deterministic file-name order), else None
    // (honest: no variant is confirmed runnable).
    let best_variant = variants
        .iter()
        .find(|v| v["fit"]["verdict"] == "CAN_RUN")
        .and_then(|v| v["file"].as_str().map(str::to_string));

    serde_json::json!({
        "model": model,
        "capability": capability,
        "evidence": evidence,
        "model_info": {
            "model": model,
            "quantization": quantization,
            "available_workers": available_workers,
            "best_variant": best_variant,
        },
        "fit": fit.to_json(),
        "worker_count": workers_json.len(),
        "workers": workers_json,
        "variants": variants,
    })
}
/// Composed intent → capability → fabric-fit resolution for the MCP
/// `resolve_intent_with_fit` tool. Closes the Intent Planner loop: a
/// natural-language intent maps (deterministically) to capabilities, and for
/// each capability a real matching local model is found from the persisted
/// registry claims, then evaluated against the fabric via the SAME per-worker
/// verdict + aggregate pipeline. Read-only; never triggers execution.
///
/// Honest by construction: a capability with no matching local model reports
/// fit = UNKNOWN ("no local model"); a capability that resolves to a model with
pub(crate) async fn mcp_intent_with_fit(
    state: &ApiState,
    intent: &str,
    evidence: &str,
) -> serde_json::Value {
    let registry_path = state.info.repo_root.join("db/registry.json");
    let registry = decentraai_registry::ModelRegistry::load(&registry_path).ok();
    let require_verified = evidence == "verified";

    let capabilities = decentraai_hub::intent::capabilities_for_intent(intent);

    let mut capabilities_out = Vec::new();
    for cap in capabilities {
        let cap_str = serde_json::to_string(&cap)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_default();

        // Find a real local model with a persisted claim for this capability.
        let mut candidate: Option<(String, String)> = None; // (file, provenance)
        if let Some(reg) = &registry {
            if let Some(m) = reg
                .models_with_capability(&cap_str, require_verified)
                .into_iter()
                .next()
            {
                candidate = Some((m.0.to_string(), m.2.to_string()));
            }
        }

        let (fit_json, model_used) = match candidate {
            Some((file, prov)) => {
                let claims = vec![(cap_str.clone(), prov)];
                let mut results = Vec::new();
                if let Some(cm) = &state.compute {
                    let local_peer = cm.local_peer().to_string();
                    for adv in cm.workers().await {
                        let trusted = cm.is_trusted(&adv.peer_id).await;
                        let accepts_remote_work =
                            local_peer == adv.peer_id.to_string() || adv.accepts_remote_inference;
                        results.push(worker_capability_verdict_with_policy(
                            &adv,
                            trusted,
                            &file,
                            &cap_str,
                            evidence,
                            &claims,
                            accepts_remote_work,
                        ));
                    }
                }
                let fit = aggregate_can_i_run(&results);
                (fit.to_json(), Some(file))
            }
            None => (
                serde_json::json!({
                    "verdict": "UNKNOWN",
                    "counts": { "can_run": 0, "cannot_run": 0, "unknown": 0 },
                    "chosen_worker": null,
                    "reasons": ["no local model with a claim for this capability"],
                }),
                None,
            ),
        };

        capabilities_out.push(serde_json::json!({
            "capability": cap_str,
            "label": cap.label(),
            "evidence": if require_verified { "verified" } else { "any" },
            "model": model_used,
            "fit": fit_json,
        }));
    }

    serde_json::json!({
        "intent": intent,
        "capabilities": capabilities_out,
        "note": "intent-to-capability is INFERRED from keywords; fit reflects real local models + fabric state.",
    })
}
pub(crate) fn fabric_fit_for_model(
    model_file: &str,
    capability: &str,
    evidence: &str,
    claims: &[(String, String)],
    workers: &[(decentraai_compute::ComputeAdvertisement, bool)],
    local_peer: &str,
) -> Vec<WorkerCapResult> {
    let mut results = Vec::new();
    for (adv, trusted) in workers {
        let accepts_remote_work =
            *local_peer == adv.peer_id.to_string() || adv.accepts_remote_inference;
        results.push(worker_capability_verdict_with_policy(
            adv,
            *trusted,
            model_file,
            capability,
            evidence,
            claims,
            accepts_remote_work,
        ));
    }
    results
}
/// Suggested share (%) of a request-level workload each CAN_RUN worker could
/// absorb, based on real advertised capacity — throughput × idle headroom ×
/// adaptive contribution factor (thermal/battery/GPU-util pressure). Pure,
/// INFERRED, and
/// advisory only — it never changes scheduling. Uses the authoritative pure
/// distribution [`decentraai_compute::adaptive_load_shares`]; normalized so
pub(crate) fn load_balance_for_workers(
    workers: &[(decentraai_compute::ComputeAdvertisement, bool)],
    can_run_peer_ids: &std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    let eligible: Vec<(String, String, decentraai_compute::ComputeAvailability)> = workers
        .iter()
        .filter(|(w, _)| can_run_peer_ids.contains(&w.peer_id.to_string()))
        .map(|(w, _)| {
            (
                w.peer_id.to_string(),
                w.node_id.clone(),
                w.availability.clone(),
            )
        })
        .collect();
    if eligible.is_empty() {
        return Vec::new();
    }
    let shares = decentraai_compute::adaptive_load_shares(&eligible);
    shares
        .into_iter()
        .map(|s| {
            let w = workers
                .iter()
                .find(|(w, _)| w.peer_id.to_string() == s.peer_id);
            let (tps, load, trusted, node_name, device) = match w {
                Some((w, trusted)) => (
                    w.availability.tokens_per_second,
                    w.availability.load_percent,
                    *trusted,
                    w.node_name.clone(),
                    device_class(&w.capability),
                ),
                None => (0, 0, false, String::new(), ""),
            };
            serde_json::json!({
                "peer_id": s.peer_id,
                "node_id": s.node_id,
                "node_name": node_name,
                "trusted": trusted,
                "device_class": device,
                "tokens_per_second": tps,
                "load_percent": load,
                "adaptive_contribution": s.adaptive_factor,
                "suggested_share_pct": (s.share * 100.0).round() as u32,
            })
        })
        .collect()
}
/// ONE coherent, explainable fabric decision (Phase 1 — Unified Decision).
///
/// Combines intent → capabilities → model options → per-variant fabric fit →
/// chosen decision → why, by REUSING the existing capability resolver, the
/// per-worker verdict, the aggregate, and the registry claims. It is a
/// read-only projection, NOT a new planner or scoring system — the "best"
/// choice is the first CAN_RUN (deterministic order), and every reason comes
/// from the real per-worker checks. No fabricated telemetry.
///
/// `explicit_model` (optional) narrows the model options to that model file;
pub(crate) async fn unified_fabric_decision(
    state: &ApiState,
    intent: &str,
    evidence: &str,
    explicit_model: Option<&str>,
) -> serde_json::Value {
    let registry_path = state.info.repo_root.join("db/registry.json");
    let registry = decentraai_registry::ModelRegistry::load(&registry_path).ok();
    let require_verified = evidence == "verified";

    // Live worker set + local peer (I/O once).
    let mut workers: Vec<(decentraai_compute::ComputeAdvertisement, bool)> = Vec::new();
    let mut local_peer = String::new();
    // Historical measured execution statistics (Phase 2): real aggregates only,
    // UNKNOWN when insufficient.
    let mut historical: serde_json::Value = serde_json::json!({ "records": 0 });
    // Recent recovery timeline (Phase 5): what happened when something failed —
    // projected from the real decisions' trace using the existing vocabulary.
    let mut recovery: Vec<serde_json::Value> = Vec::new();
    if let Some(cm) = &state.compute {
        local_peer = cm.local_peer().to_string();
        for adv in cm.workers().await {
            let trusted = cm.is_trusted(&adv.peer_id).await;
            workers.push((adv, trusted));
        }
        historical = decentraai_distributed::execution_statistics(&cm.executions());
        recovery = cm
            .decisions()
            .iter()
            .take(5)
            .map(|d| {
                let mut r = decentraai_fabric::recovery_timeline(d);
                r["request_id"] = serde_json::json!(d.request_id);
                r
            })
            .collect();
    }

    let capabilities = decentraai_hub::intent::capabilities_for_intent(intent);

    // capabilities → model_options → best decision.
    let mut capabilities_out = Vec::new();
    let mut best: Option<(String, String, String)> = None; // (cap, model, worker)
    let mut best_why: Vec<String> = Vec::new();

    for cap in capabilities {
        let cap_str = serde_json::to_string(&cap)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_default();

        // Candidate models: explicit one (if given) else all local models with
        // a persisted claim for this capability.
        let mut model_files: Vec<String> = Vec::new();
        if let Some(explicit) = explicit_model {
            model_files.push(explicit.to_string());
        } else if let Some(reg) = &registry {
            for m in reg.models_with_capability(&cap_str, require_verified) {
                model_files.push(m.0.to_string());
            }
        }

        let mut model_options = Vec::new();
        for model_file in &model_files {
            let claims: Vec<(String, String)> = registry
                .as_ref()
                .map(|reg| {
                    claims_for_file_name(reg, model_file)
                        .into_iter()
                        .map(|c| (c.capability, c.provenance))
                        .collect()
                })
                .unwrap_or_default();
            let results = fabric_fit_for_model(
                model_file,
                &cap_str,
                evidence,
                &claims,
                &workers,
                &local_peer,
            );
            let fit = aggregate_can_i_run(&results);
            let verdict = match fit.verdict {
                WorkerCapVerdict::CanRun => "CAN_RUN",
                WorkerCapVerdict::CannotRun => "CANNOT_RUN",
                WorkerCapVerdict::Unknown => "UNKNOWN",
            };
            let chosen = fit.chosen_worker.clone();
            // Record the first CAN_RUN as the fabric-wide best for this capability.
            if best.is_none() && fit.verdict == WorkerCapVerdict::CanRun {
                best = chosen
                    .clone()
                    .map(|w| (cap_str.clone(), model_file.clone(), w));
                // Aggregate the best model's passing checks as the "why".
                best_why = results
                    .iter()
                    .filter(|r| r.verdict == WorkerCapVerdict::CanRun)
                    .flat_map(|r| r.checks.iter())
                    .filter(|c| c.pass)
                    .map(|c| format!("✓ {} — {}", c.check, c.state))
                    .collect::<Vec<_>>();
            }
            let can_run_peer_ids: std::collections::HashSet<String> = results
                .iter()
                .filter(|r| r.verdict == WorkerCapVerdict::CanRun)
                .map(|r| r.peer_id.clone())
                .collect();
            model_options.push(serde_json::json!({
                "model": model_file,
                "quantization": variant_quantization_from_file_name(model_file),
                "verdict": verdict,
                "fit": fit.to_json(),
                "can_run_workers": results
                    .iter()
                    .filter(|r| r.verdict == WorkerCapVerdict::CanRun)
                    .map(|r| serde_json::json!({
                        "peer_id": r.peer_id, "node_id": r.node_id, "node_name": r.node_name,
                        "trusted": r.trusted, "engine": r.engine_compat,
                        "ram_sufficient": r.ram_sufficient, "vram_sufficient": r.vram_sufficient,
                    }))
                    .collect::<Vec<_>>(),
                // Adaptive fan-out advisory: suggested request-level share ({%})
                // per CAN_RUN worker (capacity x idle headroom), advisory only.
                "load_balance": load_balance_for_workers(&workers, &can_run_peer_ids),
            }));
        }

        capabilities_out.push(serde_json::json!({
            "capability": cap_str,
            "label": cap.label(),
            "evidence": if require_verified { "verified" } else { "any" },
            "model_options": model_options,
        }));
    }

    serde_json::json!({
        "request": intent,
        "capabilities": capabilities_out,
        "decision": match &best {
            Some((cap, model, worker)) => serde_json::json!({
                "capability": cap,
                "model": model,
                "worker": worker,
            }),
            None => serde_json::Value::Null,
        },
        "why": best_why,
        "historical": historical,
        "recent_recovery": recovery,
        "note": "coherent read-only projection of real fabric state; decision = first CAN_RUN (deterministic); reasons from real per-worker checks.",
    })
}
/// Fabric graph projection for the MCP `get_fabric_graph` tool (Phase C). Same
/// pure aggregation as `GET /v1/fabric`, read-only, no execution. Real state
pub(crate) async fn mcp_fabric_graph(state: &ApiState) -> serde_json::Value {
    let registry =
        decentraai_registry::ModelRegistry::load(&state.info.repo_root.join("db/registry.json"))
            .ok();
    let mut workers: Vec<(decentraai_distributed::ComputeAdvertisement, bool)> = Vec::new();
    let mut decisions: Vec<decentraai_fabric::ExecutionDecision> = Vec::new();
    let mut network = decentraai_fabric::NetworkGraph::new();
    let mut sessions_active = 0usize;
    let mut coordinator_version = String::new();
    if let Some(compute) = &state.compute {
        coordinator_version = compute.node_version().to_string();
        for adv in compute.workers().await {
            let trusted = compute.is_trusted(&adv.peer_id).await;
            workers.push((adv, trusted));
        }
        decisions = compute.decisions();
        sessions_active = compute.session_count();
        network = compute.network_graph();
    }
    fabric_graph_aggregate(
        &workers,
        registry.as_ref(),
        &decisions,
        &network,
        sessions_active,
        &coordinator_version,
    )
}
/// Resolve a BLAKE3 `model_hash` for a model file name from the live fabric
/// advertisements (served or on-disk). `None` when no worker advertises the
pub(crate) async fn resolve_model_hash(state: &ApiState, file_name: &str) -> Option<String> {
    let cm = state.compute.as_ref()?;
    let workers = cm.workers().await;
    for adv in workers {
        for m in adv
            .capability
            .served_models
            .iter()
            .chain(adv.capability.available_models.iter())
        {
            let f = m.file_name.to_lowercase();
            let target = file_name.to_lowercase();
            if f == target || f.ends_with(&target) || target.ends_with(&f) {
                return Some(m.model_hash.clone());
            }
        }
    }
    None
}
pub(crate) struct ResourceFit {
    pub(crate) ram_sufficient: bool,
    pub(crate) vram_sufficient: bool,
    pub(crate) local_fit: bool,
    pub(crate) trusted_worker_can_run: bool,
    pub(crate) classification: &'static str,
}
/// Pure resource-fit decision for the Model Hub "Models I can run" view.
///
/// Separated from I/O (per AGENTS.md) so the honesty invariants are driven by
/// synthetic inputs in tests, not by live hardware. `est_ram_mb`/`est_vram_mb`
pub(crate) fn resource_fit(
    local_avail_ram_mb: u64,
    local_free_vram_mb: Option<u64>,
    est_ram_mb: u64,
    est_vram_mb: u64,
    trusted_worker_count: usize,
) -> ResourceFit {
    let ram_sufficient = local_avail_ram_mb >= est_ram_mb;
    let vram_sufficient = match local_free_vram_mb {
        Some(v) => v >= est_vram_mb,
        None => false,
    };
    // A node can run the model if either resource it could use is sufficient;
    // the per-resource checks above already compared each against its OWN
    // estimate, so this OR is not a resource-mix.
    let local_fit = ram_sufficient || vram_sufficient;
    let trusted_worker_can_run = trusted_worker_count > 0;
    let classification = if local_fit && trusted_worker_can_run {
        "BEST FIT"
    } else if trusted_worker_can_run {
        "GOOD FIT"
    } else if local_fit {
        "LIMITED"
    } else {
        "NOT AVAILABLE"
    };
    ResourceFit {
        ram_sufficient,
        vram_sufficient,
        local_fit,
        trusted_worker_can_run,
        classification,
    }
}
/// Final verdict for a worker's capability query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerCapVerdict {
    CanRun,
    CannotRun,
    Unknown,
}
/// One explainable check contributing to a worker verdict.
#[derive(Debug, Clone)]
pub(crate) struct WorkerCheck {
    pub(crate) check: &'static str,
    pub(crate) pass: bool,
    pub(crate) state: String,
    pub(crate) reason: String,
}
/// The pure result of evaluating one worker against a capability query.
#[derive(Debug, Clone)]
pub(crate) struct WorkerCapResult {
    pub(crate) peer_id: String,
    pub(crate) node_id: String,
    pub(crate) node_name: String,
    pub(crate) verdict: WorkerCapVerdict,
    pub(crate) checks: Vec<WorkerCheck>,
    pub(crate) model_availability: &'static str,
    pub(crate) trusted: bool,
    pub(crate) ram_sufficient: bool,
    pub(crate) vram_sufficient: bool,
    pub(crate) est_ram_mb: u64,
    pub(crate) est_vram_mb: u64,
    pub(crate) engine_compat: &'static str,
    /// Quantization label INFERRED from the matched model file name (None =
    /// unknown). Never VERIFIED.
    pub(crate) quantization: Option<String>,
}
impl WorkerCapResult {
    /// Serialize to the MCP-facing projection (real identity kept separate).
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "worker": {
                "peer_id": self.peer_id,
                "node_id": self.node_id,
                "node_name": self.node_name,
            },
            "verdict": match self.verdict {
                WorkerCapVerdict::CanRun => "CAN_RUN",
                WorkerCapVerdict::CannotRun => "CANNOT_RUN",
                WorkerCapVerdict::Unknown => "UNKNOWN",
            },
            "model_availability": self.model_availability,
            "quantization": self.quantization,
            "trusted": self.trusted,
            "engine": self.engine_compat,
            "resource_fit": {
                "ram_sufficient": self.ram_sufficient,
                "vram_sufficient": self.vram_sufficient,
                "est_ram_mb": self.est_ram_mb,
                "est_vram_mb": self.est_vram_mb,
            },
            "checks": self.checks.iter().map(|c| serde_json::json!({
                "check": c.check,
                "pass": c.pass,
                "state": c.state,
                "reason": c.reason,
            })).collect::<Vec<_>>(),
        })
    }
}

/// The unified fabric-wide "CAN I RUN THIS?" answer, aggregated from per-worker
/// verdicts. Explainable: no opaque score — it derives from real per-worker
/// checks and reuses the exact same capability/resource/trust vocabulary.
pub(crate) struct FabricCapFit {
    /// Overall: CAN_RUN if any worker can; CANNOT_RUN if workers exist but none
    /// can and at least one hard-fails; else UNKNOWN (no workers / all unknown).
    pub(crate) verdict: WorkerCapVerdict,
    pub(crate) can_run_count: usize,
    pub(crate) cannot_run_count: usize,
    pub(crate) unknown_count: usize,
    /// The chosen worker for "which worker should I use?" — the first CAN_RUN
    /// result (deterministic input order), else None.
    pub(crate) chosen_worker: Option<String>,
    /// Human reasons behind the overall verdict (aggregated from workers).
    pub(crate) reasons: Vec<String>,
}
impl FabricCapFit {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "verdict": match self.verdict {
                WorkerCapVerdict::CanRun => "CAN_RUN",
                WorkerCapVerdict::CannotRun => "CANNOT_RUN",
                WorkerCapVerdict::Unknown => "UNKNOWN",
            },
            "counts": {
                "can_run": self.can_run_count,
                "cannot_run": self.cannot_run_count,
                "unknown": self.unknown_count,
            },
            "chosen_worker": self.chosen_worker,
            "reasons": self.reasons,
        })
    }
}

/// Pure aggregation of per-worker capability verdicts into a fabric-wide answer.
///
/// Rules (honest, no invented state):
/// - zero workers → UNKNOWN ("no compatible worker"), never CANNOT_RUN without
///   a real worker to blame.
/// - any worker CAN_RUN → overall CAN_RUN; chosen_worker is the first CAN_RUN
///   (deterministic given the caller's sorted worker order).
/// - no CAN_RUN but at least one CANNOT_RUN → CANNOT_RUN.
pub(crate) fn aggregate_can_i_run(results: &[WorkerCapResult]) -> FabricCapFit {
    if results.is_empty() {
        return FabricCapFit {
            verdict: WorkerCapVerdict::Unknown,
            can_run_count: 0,
            cannot_run_count: 0,
            unknown_count: 0,
            chosen_worker: None,
            reasons: vec!["no compatible worker on the fabric".to_string()],
        };
    }

    let can_run: Vec<&WorkerCapResult> = results
        .iter()
        .filter(|r| r.verdict == WorkerCapVerdict::CanRun)
        .collect();
    let cannot_run = results
        .iter()
        .filter(|r| r.verdict == WorkerCapVerdict::CannotRun)
        .count();
    let unknown = results
        .iter()
        .filter(|r| r.verdict == WorkerCapVerdict::Unknown)
        .count();

    let chosen_worker = can_run.first().map(|r| r.peer_id.clone());
    let verdict = if !can_run.is_empty() {
        WorkerCapVerdict::CanRun
    } else if cannot_run > 0 {
        WorkerCapVerdict::CannotRun
    } else {
        WorkerCapVerdict::Unknown
    };

    // Aggregate a small set of human reasons: the first CAN_RUN worker's
    // capability + resource evidence (when present), or representative blockers.
    let mut reasons = Vec::new();
    match verdict {
        WorkerCapVerdict::CanRun => {
            if let Some(best) = can_run.first() {
                reasons.push(format!(
                    "{} (node {} / {}) can run it",
                    best.peer_id, best.node_id, best.node_name
                ));
                let cap = best.checks.iter().find(|c| c.check == "capability");
                if let Some(cap) = cap {
                    reasons.push(format!(
                        "capability {} — {} evidence",
                        cap.state,
                        if cap.pass {
                            "satisfied"
                        } else {
                            "insufficient"
                        }
                    ));
                }
                reasons.push(format!(
                    "RAM {} · VRAM {} ({} CAN_RUN workers)",
                    if best.ram_sufficient {
                        "sufficient"
                    } else {
                        "insufficient"
                    },
                    if best.vram_sufficient {
                        "sufficient"
                    } else {
                        "insufficient"
                    },
                    can_run.len()
                ));
            }
        }
        WorkerCapVerdict::CannotRun => {
            // Report the first few distinct blockers across CANNOT_RUN workers.
            let mut seen = std::collections::BTreeSet::new();
            for r in results
                .iter()
                .filter(|r| r.verdict == WorkerCapVerdict::CannotRun)
            {
                for c in &r.checks {
                    if !c.pass {
                        let key = format!("{}:{}", c.check, c.state);
                        if seen.insert(key) {
                            reasons.push(format!(
                                "{} ({} / {}): {} — {}",
                                r.node_name, r.node_id, r.peer_id, c.check, c.state
                            ));
                        }
                    }
                }
            }
        }
        WorkerCapVerdict::Unknown => {
            reasons.push(
                "no worker can be confirmed to run it (evidence/telemetry unknown)".to_string(),
            );
        }
    }

    FabricCapFit {
        verdict,
        can_run_count: can_run.len(),
        cannot_run_count: cannot_run,
        unknown_count: unknown,
        chosen_worker,
        reasons,
    }
}
/// Resolve the engine-compatibility state for a worker advertising `engine`
/// that holds `model`.
///
/// DecentraAI only spawns llama-server (a GGUF-serving OpenAI-compatible
/// engine), and a worker advertises a model as *served* only when its engine
/// actually runs it — so a served model is engine-compatible by construction.
/// A model that is merely on disk (not served) still requires the engine to
/// serve GGUF; the bundled engine does, so on-disk is compatible. An
/// unknown/unparsed engine with a model on disk cannot be confirmed, so it is
/// `unknown`. A worker that does not hold the model cannot claim a compatible
pub(crate) fn worker_engine_compat(
    engine: &str,
    model_served: bool,
    model_on_disk: bool,
) -> &'static str {
    if model_served {
        return "compatible"; // the engine is demonstrably running this model
    }
    let kind = decentraai_fabric::EngineKind::parse(engine);
    match kind {
        decentraai_fabric::EngineKind::LlamaServer
        | decentraai_fabric::EngineKind::Vllm
        | decentraai_fabric::EngineKind::Sglang
        | decentraai_fabric::EngineKind::Ollama => {
            if model_on_disk {
                "compatible" // known GGUF-serving engine can be swapped to this model
            } else {
                "unknown" // does not hold the model; cannot confirm a path to it
            }
        }
        decentraai_fabric::EngineKind::RemoteOpenAI => "unknown", // unprobed generic endpoint
    }
}
/// Pure per-worker capability verdict. A thin projection reusing the existing
/// capability resolver, resource-fit vocabulary and the authoritative
/// advertisement (peer_id / node_id / node_name are never conflated).
///
/// `model` is matched against the worker's served/available models by file
/// name (suffix-safe). `claims` are the model's persisted capability claims
/// (`(capability, provenance)`), `evidence` is "any" or "verified".
/// Derive a variant's quantization label from a GGUF file name using ONLY
/// conservative heuristics. The label is INFERRED from the file name — the file
/// name is not authoritative metadata, so callers must never present it as
/// VERIFIED. When no recognized quant marker is present, return `None`
/// (UNKNOWN); never guess a quantization that isn't in the name.
///
/// Recognized markers (case-insensitive):
/// - `q2_k` -> "Q2", `q3_k` -> "Q3", `q4_k_m`/`q4_0` -> "Q4"
/// - `q5_1` -> "Q5", `q6_k` -> "Q6", `q8_0` -> "Q8"
pub(crate) fn variant_quantization_from_file_name(file_name: &str) -> Option<String> {
    let lower = file_name.to_lowercase();
    if lower.contains("fp16") || lower.contains("f16") {
        return Some("FP16".to_string());
    }
    // Longest markers first so e.g. `q4_k_m` is not swallowed by `q4`.
    for (marker, label) in [
        ("q8_0", "Q8"),
        ("q6_k", "Q6"),
        ("q5_1", "Q5"),
        ("q4_k_m", "Q4"),
        ("q4_0", "Q4"),
        ("q3_k", "Q3"),
        ("q2_k", "Q2"),
    ] {
        if lower.contains(marker) {
            return Some(label.to_string());
        }
    }
    None
}
/// Policy-aware per-worker capability verdict (Phase M foundation). `accepts_remote_work`
/// is true for the LOCAL node (which always serves its own work) or a remote
/// worker that opted into remote inference (`accepts_remote_inference`). A
/// remote worker that did NOT opt in cannot run this fabric's request — a
pub(crate) fn worker_capability_verdict_with_policy(
    adv: &decentraai_compute::ComputeAdvertisement,
    trusted: bool,
    model: &str,
    capability: &str,
    evidence: &str,
    claims: &[(String, String)],
    accepts_remote_work: bool,
) -> WorkerCapResult {
    let model_lower = model.to_lowercase();
    let matches_model = |m: &decentraai_compute::ServedModel| {
        let f = m.file_name.to_lowercase();
        f == model_lower || f.ends_with(&model_lower) || model_lower.ends_with(&f)
    };

    let served = adv.capability.served_models.iter().any(matches_model);
    let on_disk = adv.capability.available_models.iter().any(matches_model);
    let model_entry = adv
        .capability
        .served_models
        .iter()
        .find(|m| matches_model(m))
        .or_else(|| {
            adv.capability
                .available_models
                .iter()
                .find(|m| matches_model(m))
        });

    let model_availability = if served {
        "served"
    } else if on_disk {
        "local_on_disk"
    } else {
        "unavailable"
    };

    let engine_compat = worker_engine_compat(&adv.capability.engine, served, on_disk);
    let quantization = model_entry.and_then(|m| variant_quantization_from_file_name(&m.file_name));
    let est_ram_mb = model_entry.map(|m| m.est_ram_mb).unwrap_or(0);
    let est_vram_mb = model_entry.map(|m| m.est_vram_mb).unwrap_or(0);

    // RAM/VRAM fit from the model's own estimates vs the worker's advertised
    // availability. Missing telemetry must stay UNKNOWN, not a false pass.
    let avail_ram = adv.availability.available_ram_mb;
    let avail_vram = adv.availability.available_vram_mb;
    let ram_known = model_entry.is_some() && est_ram_mb > 0;
    let ram_sufficient = ram_known && avail_ram >= est_ram_mb;
    let vram_known = model_entry.is_some() && est_vram_mb > 0 && avail_vram.is_some();
    let vram_sufficient = vram_known && avail_vram.is_some_and(|v| v >= est_vram_mb);

    let mut checks: Vec<WorkerCheck> = Vec::new();

    // Capability verdict via the existing resolver (honest provenance).
    // When there is NO capability data at all (empty claims), the honest state
    // is UNKNOWN — the resolver would report MISSING, but "no data" is distinct
    // from "claims exist and none match". Never convert UNKNOWN into success or
    // failure.
    let cap_view = if claims.is_empty() {
        let label = capability.replace('_', " ");
        decentraai_fabric::planner::CapabilityRequirementView {
            capability: capability.to_string(),
            label,
            satisfied: false,
            evidence: "UNKNOWN".to_string(),
        }
    } else {
        let claim_refs: Vec<(&str, &str)> = claims
            .iter()
            .map(|(c, p)| (c.as_str(), p.as_str()))
            .collect();
        decentraai_fabric::planner::resolve_capability_requirement(capability, &claim_refs)
    };
    let cap_pass = cap_view.satisfied;
    checks.push(WorkerCheck {
        check: "capability",
        pass: cap_pass,
        state: cap_view.evidence.clone(),
        reason: if cap_pass {
            format!("{} — {} evidence", cap_view.label, cap_view.evidence)
        } else {
            format!(
                "{} — {} (insufficient provenance for evidence='{evidence}')",
                cap_view.label, cap_view.evidence
            )
        },
    });

    // Model availability.
    let avail_pass = model_availability != "unavailable";
    checks.push(WorkerCheck {
        check: "model_available",
        pass: avail_pass,
        state: model_availability.to_string(),
        reason: match model_availability {
            "served" => "model is currently served by this worker".into(),
            "local_on_disk" => "model is on disk (not loaded); engine can be swapped".into(),
            _ => "model is not on this worker".into(),
        },
    });

    // Trust.
    checks.push(WorkerCheck {
        check: "trusted",
        pass: trusted,
        state: if trusted { "trusted" } else { "not_trusted" }.into(),
        reason: if trusted {
            "worker is trusted by this coordinator".into()
        } else {
            "worker is not trusted".into()
        },
    });

    // Policy (Phase M): a remote worker that has not opted into remote
    // inference cannot serve a request from this fabric. The local node is
    // always allowed its own work. This is a definitive policy gate, not a
    // capability/telemetry guess.
    let policy_pass = accepts_remote_work;
    checks.push(WorkerCheck {
        check: "policy",
        pass: policy_pass,
        state: if policy_pass {
            "allowed"
        } else {
            "remote_not_accepted"
        }
        .into(),
        reason: if policy_pass {
            "worker may serve this fabric's request (local or remote-opt-in)".into()
        } else {
            "worker does not accept remote inference (policy)".into()
        },
    });

    // Engine compatibility.
    let engine_pass = engine_compat == "compatible";
    checks.push(WorkerCheck {
        check: "engine",
        pass: engine_pass,
        state: engine_compat.to_string(),
        reason: match engine_compat {
            "compatible" => format!("engine '{}' can serve this model", adv.capability.engine),
            "unknown" => format!(
                "engine '{}' compatibility unknown for this model",
                adv.capability.engine
            ),
            _ => format!(
                "engine '{}' incompatible with this model",
                adv.capability.engine
            ),
        },
    });

    // RAM.
    if model_entry.is_none() {
        checks.push(WorkerCheck {
            check: "ram",
            pass: false,
            state: "unknown".into(),
            reason: "no model footprint on this worker; cannot estimate RAM".into(),
        });
    } else if !ram_known {
        checks.push(WorkerCheck {
            check: "ram",
            pass: false,
            state: "unknown".into(),
            reason: "model RAM estimate unavailable (UNKNOWN)".into(),
        });
    } else {
        checks.push(WorkerCheck {
            check: "ram",
            pass: ram_sufficient,
            state: if ram_sufficient {
                "sufficient"
            } else {
                "insufficient"
            }
            .into(),
            reason: format!(
                "available RAM {} MiB vs estimated {} MiB",
                avail_ram, est_ram_mb
            ),
        });
    }

    // VRAM (separate dimension; CPU-only model => trivially satisfied).
    if est_vram_mb == 0 {
        checks.push(WorkerCheck {
            check: "vram",
            pass: true,
            state: "not_applicable".into(),
            reason: "CPU-only model requires no VRAM".into(),
        });
    } else if !vram_known {
        checks.push(WorkerCheck {
            check: "vram",
            pass: false,
            state: "unknown".into(),
            reason: "GPU/VRAM telemetry unavailable (UNKNOWN)".into(),
        });
    } else {
        checks.push(WorkerCheck {
            check: "vram",
            pass: vram_sufficient,
            state: if vram_sufficient {
                "sufficient"
            } else {
                "insufficient"
            }
            .into(),
            reason: format!(
                "available VRAM {} MiB vs estimated {} MiB",
                avail_vram.unwrap_or(0),
                est_vram_mb
            ),
        });
    }

    // Combine into a verdict. A definitive hard failure => CANNOT_RUN; any
    // UNKNOWN component with no hard failure => UNKNOWN; else CAN_RUN.
    // A capability with no evidence (UNKNOWN) is not a hard failure — it is an
    // unknown that must NOT be converted into success OR failure.
    let cap_hard_fail = !cap_pass && cap_view.evidence != "UNKNOWN";
    let has_hard_fail = cap_hard_fail
        || !avail_pass
        || !trusted
        || !policy_pass
        || !engine_pass
        || (ram_known && !ram_sufficient)
        || (est_vram_mb > 0 && vram_known && !vram_sufficient);
    let has_unknown = model_entry.is_none()
        || !ram_known
        || (est_vram_mb > 0 && !vram_known)
        || engine_compat == "unknown"
        || cap_view.evidence == "UNKNOWN";

    let verdict = if has_hard_fail {
        WorkerCapVerdict::CannotRun
    } else if has_unknown {
        WorkerCapVerdict::Unknown
    } else {
        WorkerCapVerdict::CanRun
    };

    WorkerCapResult {
        peer_id: adv.peer_id.to_string(),
        node_id: adv.node_id.clone(),
        node_name: adv.node_name.clone(),
        verdict,
        checks,
        model_availability,
        trusted,
        ram_sufficient,
        vram_sufficient,
        est_ram_mb,
        est_vram_mb,
        engine_compat,
        quantization,
    }
}
pub(crate) async fn shadow_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let Some(compute) = &state.compute else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"error": "compute manager not attached"}).to_string(),
        )
            .into_response();
    };
    let body: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": "invalid JSON body"}).to_string(),
            )
                .into_response();
        }
    };
    match body.get("enabled").and_then(|v| v.as_bool()) {
        Some(enabled) => {
            compute.set_shadow_enabled(enabled);
            (
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"enabled": enabled, "shadow_mode": "observe-only"}).to_string(),
            )
                .into_response()
        }
        None => (
            StatusCode::BAD_REQUEST,
            serde_json::json!({"error": "'enabled' (bool) is required"}).to_string(),
        )
            .into_response(),
    }
}
pub(crate) async fn placement_plan_handler(
    State(state): State<ApiState>,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let Some(compute) = &state.compute else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"error": "compute manager not attached"}).to_string(),
        )
            .into_response();
    };
    // Parse requirements from query params; missing values become defaults.
    let q = query.0;
    let model_id = q.get("model_id").cloned().unwrap_or_default();
    let min_vram_mb = q
        .get("min_vram_mb")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let min_ram_mb = q
        .get("min_ram_mb")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let min_gpu_count = q
        .get("min_gpu_count")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1u32);
    let context_tokens = q
        .get("context_tokens")
        .and_then(|s| s.parse().ok())
        .unwrap_or(4096u32);
    let allow_distributed = q
        .get("distributed")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(true);
    let requirements = decentraai_compute::ModelRequirements {
        model_id: model_id.clone(),
        min_gpu_count,
        min_vram_mb,
        min_ram_mb,
        context_tokens,
        local_peer: Some(compute.local_peer().to_string()),
        ..Default::default()
    };
    // Build the live fabric graph and run the deterministic placement engine.
    let graph = compute.fabric_graph().await;
    let engine = decentraai_compute::PlacementEngine {
        allow_distributed,
        ..Default::default()
    };
    let plan = engine.plan(&requirements, &graph);
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&plan).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}
pub(crate) async fn fabric_graphs_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let Some(compute) = &state.compute else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::json!({"error": "compute manager not attached"}).to_string(),
        )
            .into_response();
    };
    let graph = compute.fabric_graph().await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&graph).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}
pub(crate) async fn bench_shadow_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: axum::Json<serde_json::Value>,
) -> Response {
    use decentraai_hub::model_intel::GovernanceStage;
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let Some(bench) = &state.benchmark else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "benchmark manager not attached (no inference executor)"})),
        ).into_response();
    };
    let Some(memory) = &state.memory else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "memory store not attached"})),
        )
            .into_response();
    };
    // The executing model is whatever this node actually serves.
    let active_raw = state.active_model.read().await.clone();
    let active_norm = normalize_model_name(&active_raw);
    let shared = state.model_intel.as_ref();
    let model_id = shared.and_then(|r| {
        r.read()
            .expect("model_intel lock")
            .all()
            .into_iter()
            .map(|rec| rec.model_id.clone())
            .find(|id| normalize_model_name(id) == active_norm)
    });
    let Some(model_id) = model_id else {
        return (
            StatusCode::CONFLICT,
            axum::Json(serde_json::json!({
                "error": format!("active model '{active_raw}' is not a registered colony member"),
            })),
        )
            .into_response();
    };
    // Governance gate: benchmark traffic requires may_benchmark().
    if let Some(reg) = shared {
        let stage = reg
            .read()
            .expect("model_intel lock")
            .get(&model_id)
            .map(|m| m.governance);
        if stage.is_some_and(|s| !s.may_benchmark()) {
            return (
                StatusCode::CONFLICT,
                axum::Json(serde_json::json!({
                    "error": "model governance stage does not allow benchmarking"
                })),
            )
                .into_response();
        }
    }
    let _ = GovernanceStage::Experimental;
    let limit = body
        .0
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .min(24) as usize;
    match bench.run_intel_suite(memory, &model_id, limit).await {
        Ok(report) => {
            let summary =
                decentraai_distributed::model_performance::aggregate_model(memory, &model_id).ok();
            decentraai_audit::record_best_effort(
                &state.info.repo_root.join("logs"),
                "bench_shadow_suite",
                serde_json::json!({
                    "model_id": model_id,
                    "attempted": report.attempted,
                    "correct": report.correct,
                    "recorded": report.recorded,
                }),
            );
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "report": report,
                    "performance": summary,
                    "advisory": true,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
pub(crate) fn fabric_graph_aggregate(
    workers: &[(decentraai_distributed::ComputeAdvertisement, bool)],
    registry: Option<&decentraai_registry::ModelRegistry>,
    decisions: &[decentraai_fabric::ExecutionDecision],
    network: &decentraai_fabric::NetworkGraph,
    sessions_active: usize,
    coordinator_version: &str,
) -> serde_json::Value {
    // NODE -> WORKER: one node per real advertisement; identity fields stay
    // separate (peer_id / node_id / node_name) and trust comes from the
    // coordinator's real trust decision.
    let nodes: Vec<serde_json::Value> = workers
        .iter()
        .map(|(w, trusted)| {
            let served: Vec<String> = w
                .capability
                .served_models
                .iter()
                .map(|m| m.file_name.clone())
                .collect();
            let available: Vec<String> = w
                .capability
                .available_models
                .iter()
                .map(|m| m.file_name.clone())
                .collect();
            serde_json::json!({
                "peer_id": w.peer_id.to_string(),
                "node_id": w.node_id,
                "node_name": w.node_name,
                "trusted": *trusted,
                "device_class": device_class(&w.capability),
                "node_version": w.node_version,
                "version_status": version_status(coordinator_version, &w.node_version),
                "outdated": version_status(coordinator_version, &w.node_version) == "OUTDATED",
                "lifecycle": node_lifecycle(
                    *trusted,
                    w.availability.healthy(),
                    version_status(coordinator_version, &w.node_version),
                ),
                "gpu": {
                    "temperature_celsius": w.availability.gpu_temperature_celsius,
                    "utilization_percent": w.availability.gpu_utilization_percent,
                },
                "capacity": w.availability.capacity_state(),
                "adaptive_contribution": w.availability.adaptive_contribution_factor(),
                "battery_percent": w.availability.battery_percent,
                "load_percent": w.availability.load_percent,
                "available_ram_mb": w.availability.available_ram_mb,
                "engine": w.capability.engine,
                "health": format!("{:?}", w.availability.status),
                "served_models": served,
                "available_models": available,
            })
        })
        .collect();

    // MODEL: aggregate distinct file names across all workers (served +
    // available), deduplicated; each maps to the set of node_ids holding it.
    let mut models: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for (w, _) in workers {
        // Legacy workers may advertise an empty node_id; fall back to the
        // peer id so identity is never an empty string.
        let node = if w.node_id.is_empty() {
            w.peer_id.to_string()
        } else {
            w.node_id.clone()
        };
        for m in w
            .capability
            .served_models
            .iter()
            .chain(w.capability.available_models.iter())
        {
            models
                .entry(m.file_name.clone())
                .or_default()
                .insert(node.clone());
        }
    }

    // CAPABILITY: distinct capability names from real persisted claims only
    // (the local registry). capability -> (model files, node ids holding them).
    let mut caps: std::collections::BTreeMap<
        String,
        (
            std::collections::BTreeSet<String>,
            std::collections::BTreeSet<String>,
        ),
    > = std::collections::BTreeMap::new();
    for (file, node_ids) in &models {
        let claims = registry
            .map(|reg| claims_for_file_name(reg, file))
            .unwrap_or_default();
        for claim in &claims {
            let (model_set, node_set) = caps.entry(claim.capability.clone()).or_default();
            model_set.insert(file.clone());
            for n in node_ids {
                node_set.insert(n.clone());
            }
        }
    }

    let models_json: Vec<serde_json::Value> = models
        .iter()
        .map(|(file, node_ids)| {
            // Empty array = UNKNOWN capability data (never fabricated).
            let capabilities: Vec<serde_json::Value> = registry
                .map(|reg| claims_for_file_name(reg, file))
                .unwrap_or_default()
                .into_iter()
                .map(|c| {
                    serde_json::json!({ "capability": c.capability, "provenance": c.provenance })
                })
                .collect();
            let nodes: Vec<String> = node_ids.iter().cloned().collect();
            serde_json::json!({
                "file": file,
                "quantization": variant_quantization_from_file_name(file),
                "capabilities": capabilities,
                "nodes": nodes,
            })
        })
        .collect();

    let caps_json: Vec<serde_json::Value> = caps
        .iter()
        .map(|(name, (model_set, node_set))| {
            serde_json::json!({
                "capability": name,
                "models": model_set.iter().cloned().collect::<Vec<_>>(),
                "nodes": node_set.iter().cloned().collect::<Vec<_>>(),
            })
        })
        .collect();

    // EXECUTION: projected from the real recorded decisions (request_id /
    // model_hash / selected_worker / outcome / ts / capability_requirement)
    // plus the pure recovery timeline — mirroring execution_handler.
    let executions: Vec<serde_json::Value> = decisions
        .iter()
        .map(|d| {
            let recovery = decentraai_fabric::recovery_timeline(d);
            let mut v = serde_json::json!({
                "request_id": d.request_id,
                "model_hash": d.model_hash,
                "selected_worker": d.selected_worker,
                "outcome": d.outcome,
                "ts": d.ts,
            });
            if let Some(cr) = &d.capability_requirement {
                v["capability_requirement"] =
                    serde_json::to_value(cr).unwrap_or(serde_json::Value::Null);
            }
            v["recovery"] = recovery;
            v
        })
        .collect();

    // NETWORK: measured links back to this coordinator (RTT / bandwidth /
    // locality) — real only.
    let network: Vec<serde_json::Value> = network
        .peers()
        .map(|(peer, link)| {
            serde_json::json!({
                "peer": peer,
                "rtt_ms": link.rtt_us / 1000,
                "bandwidth_mbps": link.bandwidth_mbps,
                "locality": format!("{:?}", link.locality),
            })
        })
        .collect();

    serde_json::json!({
        "coordinator": { "version": coordinator_version },
        "nodes": nodes,
        "models": models_json,
        "capabilities": caps_json,
        "executions": executions,
        "network": network,
        "kv": { "sessions_active": sessions_active },
        "note": "Projection of real fabric state (NODE -> WORKER -> ENGINE -> MODEL -> CAPABILITY -> EXECUTION). Empty arrays are honest: absent data is never fabricated.",
    })
}
/// `GET /v1/fabric` (Phase C — Fabric Graph / Digital Twin). A read-only
/// projection of the conceptual fabric graph from authoritative live state.
pub(crate) async fn fabric_graph_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    // Best-effort local registry: source of persisted capability claims. A
    // failure to load simply yields no claims (models become UNKNOWN), never a
    // fabricated capability — mirroring fabric_model_list.
    let registry =
        decentraai_registry::ModelRegistry::load(&state.info.repo_root.join("db/registry.json"))
            .ok();
    let mut workers: Vec<(decentraai_distributed::ComputeAdvertisement, bool)> = Vec::new();
    let mut decisions: Vec<decentraai_fabric::ExecutionDecision> = Vec::new();
    let mut network = decentraai_fabric::NetworkGraph::new();
    let mut sessions_active = 0usize;
    let mut coordinator_version = String::new();
    if let Some(compute) = &state.compute {
        coordinator_version = compute.node_version().to_string();
        for adv in compute.workers().await {
            let trusted = compute.is_trusted(&adv.peer_id).await;
            workers.push((adv, trusted));
        }
        decisions = compute.decisions();
        sessions_active = compute.session_count();
        network = compute.network_graph();
    }
    let body = fabric_graph_aggregate(
        &workers,
        registry.as_ref(),
        &decisions,
        &network,
        sessions_active,
        &coordinator_version,
    );
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}
pub(crate) async fn can_run_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let model = query.get("model").cloned().unwrap_or_default();
    let capability = query.get("capability").cloned().unwrap_or_default();
    if model.trim().is_empty() || capability.trim().is_empty() {
        return forbidden("missing model and/or capability");
    }
    let evidence = query.get("evidence").map(String::as_str).unwrap_or("any");
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    };
    let body = mcp_worker_capability(&state, &model, &capability, evidence).await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}
/// `GET /v1/decision?intent=...&evidence=any|verified&model=...` — the ONE
/// coherent fabric decision (Phase 1): intent → capabilities → model options →
/// fabric fit → chosen decision → why. Reuses the existing capability resolver,
pub(crate) async fn decision_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(e) = state.require_operator_or_admin(&headers) {
        return e.into_response();
    }
    let intent = query.get("intent").cloned().unwrap_or_default();
    if intent.trim().is_empty() {
        return forbidden("missing intent");
    }
    let evidence = query.get("evidence").map(String::as_str).unwrap_or("any");
    let evidence = if evidence == "verified" {
        "verified"
    } else {
        "any"
    };
    let model = query.get("model").map(String::as_str);
    let body = unified_fabric_decision(&state, &intent, evidence, model).await;
    (
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string()),
    )
        .into_response()
}

/// Test-only convenience wrapper: default policy gate on.
#[cfg(test)]
pub(crate) fn worker_capability_verdict(
    adv: &decentraai_compute::ComputeAdvertisement,
    trusted: bool,
    model: &str,
    capability: &str,
    evidence: &str,
    claims: &[(String, String)],
) -> WorkerCapResult {
    worker_capability_verdict_with_policy(adv, trusted, model, capability, evidence, claims, true)
}
