//! M18 — MultiversX Trust & Economic Layer API handlers.
//!
//! Exposes the contract, escrow, and trust anchor primitives via REST API
//! and manages their persistence in `db/`.

use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use crate::api::ApiState;
use decentraai_economy::contract::{self, AgentContract, ContractTerms, ServiceDescriptor};
use decentraai_economy::escrow::EscrowRecord;
use decentraai_economy::trust_anchor::{AnchorParams, TrustAnchor, TrustStore};

/// Shared M18 economic state, attached to ApiState.
pub struct M18State {
    pub contracts: StdMutex<BTreeMap<String, AgentContract>>,
    pub escrow: StdMutex<decentraai_economy::escrow::EscrowLedger>,
    pub trust: StdMutex<TrustStore>,
    pub contracts_path: PathBuf,
    pub escrow_path: PathBuf,
    pub trust_path: PathBuf,
}

impl M18State {
    pub fn load(data_dir: &Path) -> Self {
        let contracts_path = data_dir.join("db/contracts.json");
        let escrow_path = data_dir.join("db/escrow.json");
        let trust_path = data_dir.join("db/trust.json");
        let contracts = load_json(&contracts_path).unwrap_or_default();
        let escrow = load_json(&escrow_path).unwrap_or_default();
        let trust = load_json(&trust_path).unwrap_or_default();
        Self {
            contracts: StdMutex::new(contracts),
            escrow: StdMutex::new(escrow),
            trust: StdMutex::new(trust),
            contracts_path,
            escrow_path,
            trust_path,
        }
    }
    pub fn save_contracts(&self) -> Result<(), String> {
        let contracts = self.contracts.lock().map_err(|e| e.to_string())?;
        save_json(&self.contracts_path, &*contracts)
    }
    pub fn save_escrow(&self) -> Result<(), String> {
        let escrow = self.escrow.lock().map_err(|e| e.to_string())?;
        save_json(&self.escrow_path, &*escrow)
    }
    pub fn save_trust(&self) -> Result<(), String> {
        let trust = self.trust.lock().map_err(|e| e.to_string())?;
        save_json(&self.trust_path, &*trust)
    }
}

fn load_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Option<T> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

fn save_json<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    let payload = serde_json::to_vec_pretty(data).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, &payload).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

fn m18_opt(state: &ApiState) -> Option<Arc<M18State>> {
    state.m18.clone()
}
#[allow(clippy::result_large_err)]
fn require_m18(state: &ApiState) -> Result<Arc<M18State>, Response> {
    m18_opt(state).ok_or_else(|| error_response("M18 economic layer not attached"))
}

// ---------------------------------------------------------------------------
// Contract handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ProposeContractRequest {
    pub provider_wallet: String,
    pub consumer_wallet: String,
    pub capability: String,
    pub description: String,
    pub price_micro_cu: u64,
    pub max_duration_secs: u64,
    pub min_quality_percent: u8,
    pub escrow_required: bool,
}

pub async fn contract_propose_handler(
    State(state): State<ApiState>,
    _headers: HeaderMap,
    Json(req): Json<ProposeContractRequest>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let now = now_secs();
    let service = ServiceDescriptor {
        capability: req.capability,
        description: req.description,
        model_requirement: None,
        estimated_input_size: None,
    };
    let terms = ContractTerms {
        price_micro_cu: req.price_micro_cu,
        max_duration_secs: req.max_duration_secs,
        min_quality_percent: req.min_quality_percent,
        escrow_required: req.escrow_required,
    };
    match contract::propose_contract(
        &req.provider_wallet,
        &req.consumer_wallet,
        service,
        terms,
        now,
    ) {
        Ok(c) => {
            m18.contracts
                .lock()
                .unwrap()
                .insert(c.contract_id.clone(), c.clone());
            let _ = m18.save_contracts();
            json_response(&c)
        }
        Err(e) => error_response(e.to_string()),
    }
}

pub async fn contract_list_handler(State(state): State<ApiState>) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let contracts = m18.contracts.lock().unwrap();
    let list: Vec<&AgentContract> = contracts.values().collect();
    json_response(&list)
}

pub async fn contract_get_handler(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let contracts = m18.contracts.lock().unwrap();
    match contracts.get(&id) {
        Some(c) => json_response(c),
        None => error_response("contract not found"),
    }
}

#[derive(Deserialize)]
pub struct ContractActionRequest {
    pub caller_wallet: String,
}

macro_rules! contract_action {
    ($name:ident, $fn:ident) => {
        pub async fn $name(
            State(state): State<ApiState>,
            AxumPath(id): AxumPath<String>,
            Json(req): Json<ContractActionRequest>,
        ) -> Response {
            let m18 = match require_m18(&state) {
                Ok(m) => m,
                Err(e) => return e,
            };
            let mut contracts = m18.contracts.lock().unwrap();
            match contracts.get_mut(&id) {
                Some(c) => {
                    let now = now_secs();
                    match contract::$fn(c, &req.caller_wallet, now) {
                        Ok(()) => {
                            drop(contracts);
                            let _ = m18.save_contracts();
                            let c2 = m18.contracts.lock().unwrap();
                            json_response(c2.get(&id).unwrap())
                        }
                        Err(e) => error_response(e.to_string()),
                    }
                }
                None => error_response("contract not found"),
            }
        }
    };
}
contract_action!(contract_accept_handler, accept_contract);
contract_action!(contract_start_handler, start_execution);
contract_action!(contract_complete_handler, complete_contract);
contract_action!(contract_cancel_handler, cancel_contract);

// ---------------------------------------------------------------------------
// Escrow handlers
// ---------------------------------------------------------------------------

pub async fn escrow_list_handler(State(state): State<ApiState>) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let escrow = m18.escrow.lock().unwrap();
    let list: Vec<&EscrowRecord> = escrow.records.values().collect();
    json_response(&list)
}

pub async fn escrow_get_handler(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let escrow = m18.escrow.lock().unwrap();
    match escrow.get_escrow(&id) {
        Some(e) => {
            let r = e.clone();
            drop(escrow);
            json_response(&r)
        }
        None => error_response("escrow not found"),
    }
}

pub async fn escrow_create_handler(
    State(state): State<ApiState>,
    AxumPath(contract_id): AxumPath<String>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let contract = {
        let c = m18.contracts.lock().unwrap();
        match c.get(&contract_id) {
            Some(x) => x.clone(),
            None => return error_response("contract not found"),
        }
    };
    let now = now_secs();
    let mut escrow = m18.escrow.lock().unwrap();
    match escrow.create_escrow(&contract, now) {
        Ok(e) => {
            let r = e.clone();
            drop(escrow);
            let _ = m18.save_escrow();
            json_response(&r)
        }
        Err(e) => error_response(e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct EscrowSettleRequest {
    pub evidence_hash: String,
    pub amount_micro_cu: u64,
    pub tx_hash: String,
}

pub async fn escrow_settle_handler(
    State(state): State<ApiState>,
    AxumPath(escrow_id): AxumPath<String>,
    Json(req): Json<EscrowSettleRequest>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let now = now_secs();
    let mut escrow = m18.escrow.lock().unwrap();
    if let Err(e) = escrow.release_escrow(&escrow_id, &req.evidence_hash, now) {
        return error_response(e.to_string());
    }
    match escrow.settle_escrow(&escrow_id, &req.tx_hash, req.amount_micro_cu, now) {
        Ok(()) => {
            let r = escrow.get_escrow(&escrow_id).unwrap().clone();
            drop(escrow);
            let _ = m18.save_escrow();
            json_response(&r)
        }
        Err(e) => error_response(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Trust anchor handlers
// ---------------------------------------------------------------------------

pub async fn trust_list_handler(State(state): State<ApiState>) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let trust = m18.trust.lock().unwrap();
    let list: Vec<&TrustAnchor> = trust.anchors.values().collect();
    json_response(&list)
}

pub async fn trust_get_handler(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let trust = m18.trust.lock().unwrap();
    match trust.anchors.get(&id) {
        Some(a) => {
            let r = a.clone();
            drop(trust);
            json_response(&r)
        }
        None => error_response("trust anchor not found"),
    }
}

#[derive(Deserialize)]
pub struct TrustRecordRequest {
    pub agent_wallet: String,
    pub evidence_hash: String,
    pub capability: String,
    pub quality_score: u8,
    pub verified: bool,
    pub micro_cu: u64,
    pub contract_id: Option<String>,
}

pub async fn trust_record_handler(
    State(state): State<ApiState>,
    Json(req): Json<TrustRecordRequest>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let now = now_secs();
    let params = AnchorParams {
        agent_wallet: req.agent_wallet,
        evidence_hash: req.evidence_hash,
        capability: req.capability,
        quality_score: req.quality_score,
        verified: req.verified,
        micro_cu: req.micro_cu,
        contract_id: req.contract_id,
    };
    let mut trust = m18.trust.lock().unwrap();
    match trust.record_anchor(&params, now) {
        Ok(a) => {
            let r = a.clone();
            drop(trust);
            let _ = m18.save_trust();
            json_response(&r)
        }
        Err(e) => error_response(e.to_string()),
    }
}

pub async fn trust_verify_handler(
    State(state): State<ApiState>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let trust = m18.trust.lock().unwrap();
    match trust.anchors.get(&id) {
        Some(a) => match trust.verify_anchor(a) {
            Ok(()) => json_response(
                &serde_json::json!({ "valid": true, "anchor_id": a.anchor_id, "agent_wallet": a.agent_wallet }),
            ),
            Err(e) => error_response(e.to_string()),
        },
        None => error_response("trust anchor not found"),
    }
}

pub async fn trust_score_handler(
    State(state): State<ApiState>,
    AxumPath(wallet): AxumPath<String>,
) -> Response {
    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };
    let trust = m18.trust.lock().unwrap();
    let score = trust.trust_score(&wallet);
    let anchors = trust.anchors_for_wallet(&wallet);
    json_response(
        &serde_json::json!({ "wallet": wallet, "score": score, "anchor_count": anchors.len(), "verified_count": anchors.iter().filter(|a| a.verified).count() }),
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn json_response<T: Serialize>(data: &T) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(data).unwrap_or_default(),
    )
        .into_response()
}

fn error_response(msg: impl Into<String>) -> Response {
    let body = serde_json::json!({ "error": msg.into() });
    (
        axum::http::StatusCode::BAD_REQUEST,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(&body).unwrap_or_default(),
    )
        .into_response()
}
