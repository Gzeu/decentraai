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

/// An economic action recorded during a tick (for audit trail).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum M18Action {
    ProposeContract {
        contract_id: String,
        consumer: String,
    },
    AcceptContract {
        contract_id: String,
        provider: String,
    },
    StartExecution {
        contract_id: String,
    },
    CompleteContract {
        contract_id: String,
    },
    SettleContract {
        contract_id: String,
    },
    CancelContract {
        contract_id: String,
    },
}

/// Shared M18 economic state, attached to ApiState.
pub struct M18State {
    pub contracts: StdMutex<BTreeMap<String, AgentContract>>,
    pub escrow: StdMutex<decentraai_economy::escrow::EscrowLedger>,
    pub trust: StdMutex<TrustStore>,
    pub actions: StdMutex<Vec<M18Action>>,
    pub tick: StdMutex<u64>,
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
            actions: StdMutex::new(Vec::new()),
            tick: StdMutex::new(0),
            contracts_path,
            escrow_path,
            trust_path,
        }
    }

    /// Test-only default (empty state, unique temp file paths per call).
    #[cfg(test)]
    pub fn test_default() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("m18-test-{}-{:05}", std::process::id(), n));
        let _ = std::fs::create_dir_all(&base);
        Self {
            contracts: StdMutex::new(BTreeMap::new()),
            escrow: StdMutex::new(decentraai_economy::escrow::EscrowLedger::default()),
            trust: StdMutex::new(TrustStore::default()),
            actions: StdMutex::new(Vec::new()),
            tick: StdMutex::new(1),
            contracts_path: base.join("contracts.json"),
            escrow_path: base.join("escrow.json"),
            trust_path: base.join("trust.json"),
        }
    }

    /// Current tick (read from atomic).
    pub fn current_tick(&self) -> u64 {
        *self.tick.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Advance tick by 1.
    pub fn advance_tick(&self) {
        if let Ok(mut t) = self.tick.lock() {
            *t += 1;
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

/// Current unix time, seconds. Public for the World settlement path
/// (trade timestamps share the same clock as the M18 handlers).
pub fn now_secs_public() -> u64 {
    now_secs()
}

fn m18_opt(state: &ApiState) -> Option<Arc<M18State>> {
    state.m18.clone()
}
#[allow(clippy::result_large_err)]
fn require_m18(state: &ApiState) -> Result<Arc<M18State>, Response> {
    m18_opt(state).ok_or_else(|| error_response("M18 economic layer not attached"))
}

// ---------------------------------------------------------------------------
// World trade anchoring: contract → escrow → release → settle
// ---------------------------------------------------------------------------

/// Map a World entity wallet to an M18-valid wallet: `erd1…` passes
/// through, local identities (`npc:…`, plain ids) become `agent:{id}` —
/// the documented local-network form. Pure, no I/O.
pub fn world_m18_wallet(entity_wallet: &str, entity_id: &str) -> String {
    if entity_wallet.starts_with("erd1") {
        entity_wallet.to_string()
    } else {
        format!("agent:{entity_id}")
    }
}

/// Record a COMPLETED World sale through the full M18 path:
///
/// propose → accept → persist → escrow create → persist →
/// release(evidence) → persist. Returns `(contract_id, escrow_id)`.
///
/// Amount unit: World credits pass 1:1 as micro-CU for the anchor record.
/// The World ledger stays authoritative for balances; the M18 reward
/// formula is NOT invoked here — this anchors an agreed trade, it does
/// not mint contribution value.
#[allow(clippy::too_many_arguments)]
pub fn record_world_sale(
    m18: &M18State,
    provider: (&str, &str),
    consumer: (&str, &str),
    capability: &str,
    description: &str,
    price_credits: u64,
    evidence_hash: &str,
    now: u64,
) -> Result<(String, String), String> {
    let (provider_id, provider_wallet_raw) = provider;
    let (consumer_id, consumer_wallet_raw) = consumer;
    let provider_wallet = world_m18_wallet(provider_wallet_raw, provider_id);
    let consumer_wallet = world_m18_wallet(consumer_wallet_raw, consumer_id);

    let mut c = contract::propose_contract(
        &provider_wallet,
        &consumer_wallet,
        ServiceDescriptor {
            capability: capability.to_string(),
            description: description.chars().take(280).collect(),
            model_requirement: None,
            estimated_input_size: None,
        },
        ContractTerms {
            price_micro_cu: price_credits,
            max_duration_secs: 3_600,
            min_quality_percent: 0,
            escrow_required: true,
        },
        now,
    )
    .map_err(|e| format!("m18 propose failed: {e}"))?;
    contract::accept_contract(&mut c, &provider_wallet, now)
        .map_err(|e| format!("m18 accept failed: {e}"))?;
    let contract_id = c.contract_id.clone();
    {
        let mut contracts = m18.contracts.lock().map_err(|e| e.to_string())?;
        contracts.insert(contract_id.clone(), c.clone());
    }
    let _ = m18.save_contracts();
    {
        let mut escrow = m18.escrow.lock().map_err(|e| e.to_string())?;
        escrow
            .create_escrow(&c, now)
            .map_err(|e| format!("m18 escrow create failed: {e}"))?;
        escrow
            .release_escrow(&contract_id, evidence_hash, now)
            .map_err(|e| format!("m18 escrow release failed: {e}"))?;
    }
    let _ = m18.save_escrow();
    Ok((contract_id.clone(), contract_id))
}

/// Settle a released World-sale escrow with the chain tx hash.
/// Best-effort after broadcast: failure here never un-records the proof.
/// When the escrow already settled on a tx that later proved dead
/// (dropped from the mempool), the anchor is CORRECTED to the live hash.
pub fn settle_world_sale(
    m18: &M18State,
    escrow_id: &str,
    tx_hash: &str,
    amount_credits: u64,
    now: u64,
) -> Result<(), String> {
    {
        let mut escrow = m18.escrow.lock().map_err(|e| e.to_string())?;
        match escrow.settle_escrow(escrow_id, tx_hash, amount_credits, now) {
            Ok(()) => {}
            Err(decentraai_economy::escrow::EscrowError::InvalidTransition(
                decentraai_economy::escrow::EscrowStatus::Settled,
                _,
            )) => {
                escrow
                    .reanchor_escrow(escrow_id, tx_hash, now)
                    .map_err(|e| format!("m18 escrow reanchor failed: {e}"))?;
            }
            Err(e) => return Err(format!("m18 escrow settle failed: {e}")),
        }
    }
    let _ = m18.save_escrow();
    Ok(())
}

/// Find the escrow holding a given evidence hash — the proof ↔ escrow link.
/// Used by the sweep/check paths to settle escrows created before a restart.
pub fn escrow_for_evidence(m18: &M18State, evidence_hash: &str) -> Option<String> {
    let escrow = m18.escrow.lock().ok()?;
    escrow.records.values().find_map(|r| {
        if r.evidence_hash.as_deref() == Some(evidence_hash) {
            Some(r.escrow_id.clone())
        } else {
            None
        }
    })
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
// Economic tick handler — runs autonomous agent economic behavior
// ---------------------------------------------------------------------------

/// Runs one economic tick: every World agent evaluates its needs and
/// applies its chosen action (bid, propose, accept, execute, complete).
/// Operator/master only; wallet/consumer are rejected before any mutation.
pub async fn economic_tick_handler(State(state): State<ApiState>, headers: HeaderMap) -> Response {
    use crate::api::Auth;

    let m18 = match require_m18(&state) {
        Ok(m) => m,
        Err(e) => return e,
    };

    // Operator/master only — deterministic economic decisions affect state.
    let operator_ok = match state.classify(&headers) {
        Ok(Auth::Master) => true,
        Ok(Auth::Subscriber { role, .. }) => {
            matches!(role, decentraai_tokens::Role::Operator)
        }
        Ok(_) => false,
        Err(e) => return e.into_response(),
    };
    if !operator_ok {
        return error_response("economic tick requires operator/master");
    }

    // Snapshot World agents.
    let agents = {
        let world = state.world.lock().await;
        world.agents.clone()
    };

    let result = crate::world_economics::run_world_economic_tick(
        &agents,
        &state.hub,
        &m18,
        &state.info.repo_root,
        &state.quota_ledger,
    )
    .await;

    m18.advance_tick();
    json_response(&result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn json_response<T: Serialize>(data: &T) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        serde_json::to_string(data).unwrap_or_default(),
    )
        .into_response()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_m18() -> M18State {
        M18State::test_default()
    }

    #[test]
    fn wallet_mapping_keeps_erd1_and_namespaces_locals() {
        assert_eq!(
            world_m18_wallet("erd1abc", "x"),
            "erd1abc".to_string()
        );
        assert_eq!(
            world_m18_wallet("npc:npc-smith", "npc-smith"),
            "agent:npc-smith".to_string()
        );
    }

    #[test]
    fn world_sale_full_path_propose_to_settle() {
        let m18 = test_m18();
        let now = 1_700_000_000;
        let (cid, eid) = record_world_sale(
            &m18,
            ("npc-smith", "npc:npc-smith"),
            ("agent-1", "erd1buyerbuyerbuyerbuyerbuyerbuyerbuyerbuyerbuye"),
            "coding",
            "agent-1 purchased coding from npc-smith for 10Cr",
            10,
            "aa".repeat(32).as_str(),
            now,
        )
        .unwrap();
        assert_eq!(cid, eid);
        // Released with evidence attached.
        {
            let escrow = m18.escrow.lock().unwrap();
            let r = escrow.records.get(&eid).unwrap();
            assert_eq!(
                r.status,
                decentraai_economy::escrow::EscrowStatus::Released
            );
            assert_eq!(r.evidence_hash.as_deref(), Some("aa".repeat(32).as_str()));
            assert_eq!(r.provider_wallet, "agent:npc-smith".to_string());
        }
        // Proof ↔ escrow link resolves.
        assert_eq!(
            escrow_for_evidence(&m18, &"aa".repeat(32)),
            Some(eid.clone())
        );
        // Chain settle finalizes.
        settle_world_sale(&m18, &eid, "deadbeef01", 10, now + 1).unwrap();
        let escrow = m18.escrow.lock().unwrap();
        let r = escrow.records.get(&eid).unwrap();
        assert_eq!(r.status, decentraai_economy::escrow::EscrowStatus::Settled);
        assert_eq!(r.tx_hash.as_deref(), Some("deadbeef01"));
    }
}
