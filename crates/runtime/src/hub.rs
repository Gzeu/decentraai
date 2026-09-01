//! Agent Hub — runtime wiring for task market, bids, proposals, teams, settlement.
//! Reuses dca_ auth, QuotaLedger, Evidence, Reputation; no new scheduler.

use crate::api::{ApiState, Auth};
use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    response::{
        IntoResponse,
        sse::{Event as SseEvent, Sse},
    },
};
use decentraai_agent_hub::{HubState, TaskStatus};
use futures::stream::Stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedHub = Arc<Mutex<HubState>>;

pub fn new_shared_hub() -> SharedHub {
    Arc::new(Mutex::new(HubState::new()))
}

pub fn hub_path_for(repo_root: &Path) -> PathBuf {
    repo_root.join("db/hub.json")
}

pub fn load_hub_state(path: &Path) -> HubState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
pub fn save_hub_state(path: &Path, state: &HubState) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let tmp = path.with_extension("tmp");
    if let Ok(s) = serde_json::to_string(state) {
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

// ---------- Shapes ----------
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishTaskRequest {
    pub title: String,
    pub description: Option<String>,
    pub reward: u64,
    pub required_capability: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BidRequest {
    pub task_id: String,
    pub price: u64,
    pub rationale: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalRequest {
    pub to: String,
    pub task_id: String,
    pub offer_price: u64,
    pub workshare: Option<u8>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideProposalRequest {
    pub accept: bool,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamRequest {
    pub task_id: String,
    pub members: Vec<(String, u8)>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteRequest {
    pub task_id: String,
}

// ---------- Handlers ----------
pub async fn hub_state_handler(State(state): State<ApiState>) -> impl IntoResponse {
    let hub = state.hub.lock().await;
    Json(serde_json::json!({
        "tick": hub.tick,
        "tasks": hub.tasks.values().collect::<Vec<_>>(),
        "bids": hub.bids.values().collect::<Vec<_>>(),
        "proposals": hub.proposals.values().collect::<Vec<_>>(),
        "teams": hub.teams.values().collect::<Vec<_>>(),
        "events": hub.events.iter().rev().take(50).cloned().collect::<Vec<_>>(),
        "total_tasks": hub.tasks.len(),
        "total_bids": hub.bids.len(),
    }))
}

pub async fn hub_tasks_handler(State(state): State<ApiState>) -> impl IntoResponse {
    let hub = state.hub.lock().await;
    Json(serde_json::json!({"tasks": hub.tasks.values().collect::<Vec<_>>()}))
}

pub async fn hub_publish_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<PublishTaskRequest>,
) -> impl IntoResponse {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let issuer = match &auth {
        Auth::Consumer { account, .. } => account.clone(),
        Auth::Master => "operator".to_string(),
        Auth::Open => "open".to_string(),
        Auth::Subscriber { name, .. } => name.clone(),
        Auth::Wallet { wallet_address, .. } => wallet_address.clone(),
    };
    if req.title.trim().is_empty() || req.reward == 0 {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"title and reward required"})),
        )
            .into_response();
    }
    let mut hub = state.hub.lock().await;
    let task = hub.publish_task(
        issuer,
        req.title,
        req.description.unwrap_or_default(),
        req.reward,
        req.required_capability,
    );
    hub.advance_tick();
    let path = hub_path_for(&state.info.repo_root);
    save_hub_state(&path, &hub);
    (
        axum::http::StatusCode::OK,
        Json(serde_json::to_value(&task).unwrap()),
    )
        .into_response()
}

pub async fn hub_bid_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<BidRequest>,
) -> impl IntoResponse {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let bidder = match &auth {
        Auth::Consumer { account, .. } => account.clone(),
        Auth::Master => "operator".to_string(),
        Auth::Open => "open".to_string(),
        Auth::Subscriber { name, .. } => name.clone(),
        Auth::Wallet { wallet_address, .. } => wallet_address.clone(),
    };
    let mut hub = state.hub.lock().await;
    match hub.place_bid(
        bidder,
        req.task_id,
        req.price,
        req.rationale.unwrap_or_default(),
    ) {
        Ok(bid) => {
            hub.advance_tick();
            let path = hub_path_for(&state.info.repo_root);
            save_hub_state(&path, &hub);
            (
                axum::http::StatusCode::OK,
                Json(serde_json::to_value(&bid).unwrap()),
            )
                .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn hub_bids_handler(
    State(state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let task_id = params.get("task_id").cloned();
    let hub = state.hub.lock().await;
    let bids: Vec<_> = hub
        .bids
        .values()
        .filter(|b| task_id.as_ref().is_none_or(|id| &b.task_id == id))
        .cloned()
        .collect();
    Json(serde_json::json!({"bids": bids}))
}

pub async fn hub_proposal_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<ProposalRequest>,
) -> impl IntoResponse {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let from = match &auth {
        Auth::Consumer { account, .. } => account.clone(),
        Auth::Master => "operator".to_string(),
        Auth::Open => "open".to_string(),
        Auth::Subscriber { name, .. } => name.clone(),
        Auth::Wallet { wallet_address, .. } => wallet_address.clone(),
    };
    let mut hub = state.hub.lock().await;
    match hub.propose(
        from,
        req.to,
        req.task_id,
        req.offer_price,
        req.workshare.unwrap_or(100),
    ) {
        Ok(prop) => {
            hub.advance_tick();
            let path = hub_path_for(&state.info.repo_root);
            save_hub_state(&path, &hub);
            (
                axum::http::StatusCode::OK,
                Json(serde_json::to_value(&prop).unwrap()),
            )
                .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn hub_decide_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(req): Json<DecideProposalRequest>,
) -> impl IntoResponse {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let actor = match &auth {
        Auth::Consumer { account, .. } => account.clone(),
        Auth::Master => "operator".to_string(),
        Auth::Open => "open".to_string(),
        Auth::Subscriber { name, .. } => name.clone(),
        Auth::Wallet { wallet_address, .. } => wallet_address.clone(),
    };
    let mut hub = state.hub.lock().await;
    match hub.decide_proposal(&id, &actor, req.accept) {
        Ok(prop) => {
            hub.advance_tick();
            let path = hub_path_for(&state.info.repo_root);
            save_hub_state(&path, &hub);
            (
                axum::http::StatusCode::OK,
                Json(serde_json::to_value(&prop).unwrap()),
            )
                .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn hub_team_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<TeamRequest>,
) -> impl IntoResponse {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let _actor = match &auth {
        Auth::Consumer { account, .. } => account.clone(),
        Auth::Master => "operator".to_string(),
        Auth::Open => "open".to_string(),
        Auth::Subscriber { name, .. } => name.clone(),
        Auth::Wallet { wallet_address, .. } => wallet_address.clone(),
    };
    let mut hub = state.hub.lock().await;
    match hub.form_team(req.task_id, req.members) {
        Ok(team) => {
            hub.advance_tick();
            let path = hub_path_for(&state.info.repo_root);
            save_hub_state(&path, &hub);
            (
                axum::http::StatusCode::OK,
                Json(serde_json::to_value(&team).unwrap()),
            )
                .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn hub_execute_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<ExecuteRequest>,
) -> impl IntoResponse {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let actor = match &auth {
        Auth::Consumer { account, .. } => account.clone(),
        Auth::Master => "operator".to_string(),
        Auth::Open => "open".to_string(),
        Auth::Subscriber { name, .. } => name.clone(),
        Auth::Wallet { wallet_address, .. } => wallet_address.clone(),
    };
    let mut hub = state.hub.lock().await;
    let task = match hub.tasks.get(&req.task_id) {
        Some(t) => t.clone(),
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error":"task not found"})),
            )
                .into_response();
        }
    };
    if task.status != TaskStatus::Assigned
        && task.status != TaskStatus::Open
        && task.status != TaskStatus::Bidding
    {
        return (axum::http::StatusCode::CONFLICT, Json(serde_json::json!({"error": format!("task status {:?} not executable", task.status)}))).into_response();
    }
    hub.mark_executing(&req.task_id);
    // Settlement: distribute reward via QuotaLedger to team members or issuer/best bidder
    let team_members: Vec<(String, u8)> = hub
        .teams
        .values()
        .find(|t| t.task_id == req.task_id)
        .map(|t| t.members.clone())
        .unwrap_or_else(|| {
            if let Some(best) = hub.best_bid(&req.task_id) {
                vec![(best.bidder.clone(), 100)]
            } else {
                vec![(task.issuer.clone(), 100)]
            }
        });
    // Generate evidence
    let evidence_id =
        blake3::hash(format!("hub:{}:{}:{}", req.task_id, actor, hub.tick).as_bytes())
            .to_hex()
            .to_string();
    // Credit each member
    if let Some(ledger) = &state.quota_ledger {
        let mut lg = ledger.lock().unwrap();
        for (member, share) in &team_members {
            let amount = (task.reward as u128 * *share as u128 / 100) as u64;
            if amount > 0 {
                let ref_id = format!("hub-settle-{}-{}", req.task_id, member);
                let _ = lg.credit(member, &ref_id, Some(amount as u32), None);
            }
        }
    }
    hub.settle(&req.task_id, Some(evidence_id.clone()));
    hub.advance_tick();
    let path = hub_path_for(&state.info.repo_root);
    save_hub_state(&path, &hub);
    // --- Auto Society + Personal Memory side-effects (deterministic, idempotent) ---
    let _team_for_society = team_members.clone();
    let _ev_for_society = evidence_id.clone();
    let _task_for_society = req.task_id.clone();
    let _reward_for_society = task.reward;
    let _issuer_for_society = task.issuer.clone();
    drop(hub);
    {
        let mut society = state.society.lock().await;
        if !society.outcomes.contains_key(&_task_for_society) {
            let tick = society.tick;
            let mut contrib_records = Vec::new();
            for (agent_id, share) in &_team_for_society {
                let cr = decentraai_agent_society::state::ContributionRecord::new(
                    _task_for_society.clone(),
                    agent_id.clone(),
                    *share,
                    tick,
                )
                .verify(
                    *share as f32 / 100.0,
                    _ev_for_society.clone(),
                    0.85,
                    true,
                    tick,
                );
                contrib_records.push(cr.clone());
                society.record_contribution(cr);
            }
            let dists = _team_for_society
                .iter()
                .map(|(aid, share)| {
                    let amount = (_reward_for_society as u128 * *share as u128 / 100) as u64;
                    decentraai_agent_society::state::RewardDistribution {
                        agent_id: aid.clone(),
                        amount,
                        share_basis: decentraai_agent_society::state::ShareBasis::Verified,
                    }
                })
                .collect::<Vec<_>>();
            let outcome = decentraai_agent_society::state::TaskOutcome {
                task_id: _task_for_society.clone(),
                issuer: _issuer_for_society.clone(),
                team_members: _team_for_society.iter().map(|(a, _)| a.clone()).collect(),
                status: decentraai_agent_society::state::TaskOutcomeStatus::Settled,
                evidence_id: Some(_ev_for_society.clone()),
                settled_tick: tick,
                total_reward: _reward_for_society,
                distributions: dists,
                contributor_records: contrib_records,
            };
            society.record_outcome(outcome);
            for (agent_id, _) in &_team_for_society {
                let ev = decentraai_agent_society::state::ReputationEvent {
                    agent_id: agent_id.clone(),
                    event_type: decentraai_agent_society::state::ReputationEventType::TaskCompleted,
                    task_id: Some(_task_for_society.clone()),
                    delta: 0.15,
                    tick,
                    evidence_id: Some(_ev_for_society.clone()),
                    detail: format!(
                        "hub execute {} as team {:?}",
                        _task_for_society, _team_for_society
                    ),
                };
                society.record_reputation_event(ev);
                let ev2 = decentraai_agent_society::state::ReputationEvent {
                    agent_id: agent_id.clone(),
                    event_type:
                        decentraai_agent_society::state::ReputationEventType::ContributionVerified,
                    task_id: Some(_task_for_society.clone()),
                    delta: 0.1,
                    tick,
                    evidence_id: Some(_ev_for_society.clone()),
                    detail: format!("verified contribution for {}", _task_for_society),
                };
                society.record_reputation_event(ev2);
            }
            society.advance_tick();
            let path = decentraai_agent_society::state::society_path_for(&state.info.repo_root);
            decentraai_agent_society::state::save_society_state(&path, &society);
        }
    }
    if let Some(pm) = &state.personal_memory {
        let mut all_agents = _team_for_society
            .iter()
            .map(|(a, _)| a.clone())
            .collect::<Vec<_>>();
        if !all_agents.contains(&_issuer_for_society)
            && !_issuer_for_society.is_empty()
            && _issuer_for_society != "open"
            && _issuer_for_society != "operator"
        {
            all_agents.push(_issuer_for_society.clone());
        }
        for agent_id in all_agents {
            let exp_id = format!("hub-{}-{}", _task_for_society, agent_id);
            let cached = pm.get_or_create(&agent_id).await;
            let exists = {
                cached
                    .read()
                    .await
                    .memory
                    .experiences
                    .experiences
                    .iter()
                    .any(|e| e.id == exp_id)
            };
            if !exists {
                let summary = format!(
                    "Executed task {} (reward {}) as team {:?} — evidence {}",
                    _task_for_society,
                    _reward_for_society,
                    _team_for_society,
                    &_ev_for_society[..8.min(_ev_for_society.len())]
                );
                let detail = format!(
                    "Task {} settled with evidence {}. Team: {:?}. Issuer: {}.",
                    _task_for_society, _ev_for_society, _team_for_society, _issuer_for_society
                );
                let entry = serde_json::json!({
                    "id": exp_id,
                    "type_": "success",
                    "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
                    "summary": summary,
                    "detail": detail,
                    "involved_agents": _team_for_society.iter().map(|(a,_)| a.clone()).collect::<Vec<_>>(),
                    "task_id": _task_for_society,
                    "outcome": "settled",
                    "evidence_ids": [_ev_for_society.clone()],
                    "emotional_impact": 0.7,
                    "tags": ["hub", "settlement", "team"]
                });
                let _ = pm
                    .write_entry(&agent_id, |mem| {
                        decentraai_agent_personal_memory::mcp::apply_write(
                            mem,
                            "experiences",
                            entry.clone(),
                        )
                    })
                    .await;
            }
        }
    }
    {
        let mut arena = state.arena.lock().await;
        let ev = decentraai_arena::ArenaEvent {
            tick: arena.tick,
            agent_id: format!("hub:{}", actor),
            action: decentraai_arena::ActionKind::RequestCompute,
            from: (0, 0),
            to: None,
            rationale: format!("hub execute {}", req.task_id),
            evidence_id: Some(evidence_id.clone()),
            success: true,
            detail: format!("hub team {} executed", req.task_id),
        };
        arena.events.push_back(ev);
        while arena.events.len() > arena.max_events {
            arena.events.pop_front();
        }
        arena.advance_tick();
        let apath = crate::arena::arena_path_for(&state.info.repo_root);
        crate::arena::save_arena_world(&apath, &arena);
    }
    (axum::http::StatusCode::OK, Json(serde_json::json!({"task_id": req.task_id, "evidence_id": evidence_id, "team": team_members, "reward": task.reward}))).into_response()
}

pub async fn hub_events_handler(
    State(state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let since: u64 = params
        .get("since")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let hub = state.hub.lock().await;
    let events = hub.events_since(since, limit.min(200));
    Json(serde_json::json!({"tick": hub.tick, "events": events}))
}

pub async fn hub_stream_handler(
    State(state): State<ApiState>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let hub_clone = state.hub.clone();
    let stream = futures::stream::unfold((hub_clone, 0u64), |(hub, last_seen)| async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let guard = hub.lock().await;
        let new_events: Vec<_> = guard
            .events
            .iter()
            .filter(|e| e.tick >= last_seen)
            .cloned()
            .collect();
        if new_events.is_empty() {
            let next = last_seen;
            drop(guard);
            Some((Ok(SseEvent::default().comment("heartbeat")), (hub, next)))
        } else {
            let max_tick = new_events.iter().map(|e| e.tick).max().unwrap_or(last_seen) + 1;
            let data = serde_json::to_string(&new_events).unwrap_or_else(|_| "[]".to_string());
            drop(guard);
            Some((
                Ok(SseEvent::default().data(data).event("hub_events")),
                (hub, max_tick),
            ))
        }
    });
    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keepalive"),
    )
}

pub fn hub_html() -> String {
    r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>DecentraAI — Agent Hub</title>
<style>
:root{--bg:#05070d;--panel:#0d121c;--line:#182234;--text:#e8eef6;--muted:#8fa0b3;--accent:#22d3ee;--accent2:#6366f1;--ok:#34d399;--warn:#fbbf24}
*{box-sizing:border-box;margin:0;padding:0}
body{background:var(--bg);color:var(--text);font:14px/1.5 system-ui,sans-serif;padding:16px}
header{display:flex;justify-content:space-between;align-items:center;margin-bottom:16px}
h1{font-size:20px} .sub{color:var(--muted);font-size:12px}
.layout{display:grid;grid-template-columns:1fr 360px;gap:16px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:12px;margin-bottom:12px}
.card h3{font-size:11px;text-transform:uppercase;letter-spacing:1px;color:var(--muted);margin-bottom:8px}
.row{display:flex;justify-content:space-between;padding:4px 0;border-bottom:1px solid #14203a;font-size:13px}
.mono{font-family:ui-monospace,monospace}
.event{padding:6px 0;border-bottom:1px solid #14203a;font-size:12px}
.event .tick{color:var(--accent)}
.badge{padding:2px 6px;border-radius:999px;font-size:10px;border:1px solid var(--line)}
.badge.live{border-color:var(--ok);color:var(--ok)}
input,button{padding:8px;border-radius:8px;border:1px solid #223048;background:#0a0e16;color:#e8eef6}
button{cursor:pointer} button:hover{border-color:var(--accent)}
</style></head><body>
<header><div><h1>● Agent Hub</h1><div class="sub">TASK → BIDS → TEAM → EXECUTION → EVIDENCE → SETTLEMENT → REPUTATION</div></div><div class="sub">tick <b id="tick">…</b> · tasks <b id="tcnt">…</b> · <span id="sse" class="badge">SSE …</span></div></header>
<div class="layout">
<div>
<div class="card"><h3>Publish Task (dca_)</h3><div style="display:flex;gap:8px"><input id="title" placeholder="title" style="flex:1"><input id="reward" placeholder="reward" type="number" style="width:100px"><button onclick="publish()">Publish</button></div></div>
<div class="card"><h3>Tasks</h3><div id="tasks"></div></div>
<div class="card"><h3>Live Activity — SSE</h3><div id="events"></div></div>
</div>
<div>
<div class="card"><h3>Agents (dca_)</h3><input id="tok" placeholder="dca_..." style="width:100%"><div id="agents" class="sub" style="margin-top:8px"></div></div>
<div class="card"><h3>Bids & Teams</h3><div id="bids"></div></div>
<div class="card"><h3>World (Arena bridge)</h3><div id="arena"></div></div>
</div>
</div>
<script>
let since=0;
function tok(){return document.getElementById('tok').value.trim()||localStorage.getItem('hub-token')||''}
function auth(){const t=tok();return t?{Authorization:'Bearer '+t}:{}}
async function j(url,opts={}){try{const r=await fetch(url,{...opts,headers:{...(opts.headers||{}),...auth()}});return {ok:r.ok,json:r.ok?await r.json():await r.text(),status:r.status}}catch(e){return {ok:false,json:String(e)}}}
async function tick(){
  const s=await j('/v1/hub/state'); if(!s.ok) return;
  const d=s.json; document.getElementById('tick').textContent=d.tick; document.getElementById('tcnt').textContent=d.total_tasks;
  document.getElementById('tasks').innerHTML=d.tasks.map(t=>`<div class="row"><span><b>${t.id}</b> ${t.title} <span class="mono">${t.reward}Cr</span> <span class="sub">${t.status}</span></span><span><button onclick="bid('${t.id}')">Bid</button> <button onclick="team('${t.id}')">Team</button> <button onclick="exec('${t.id}')">Exec</button></span></div>`).join('')||'<div class="sub">no tasks</div>';
  document.getElementById('bids').innerHTML=d.bids.map(b=>`<div class="row"><span class="mono">${b.id}</span> ${b.bidder} ${b.price}Cr</div>`).join('')||'<div class="sub">no bids</div>';
  const a=await j('/v1/arena/state'); if(a.ok){document.getElementById('arena').innerHTML=`<div class="row"><span>arena tick</span><span>${a.json.tick}</span></div><div class="row"><span>agents</span><span>${a.json.total_agents}</span></div>`;}
}
async function publish(){
  const t=tok(); if(!t){alert('need dca_');return;} localStorage.setItem('hub-token',t);
  const title=document.getElementById('title').value; const reward=parseInt(document.getElementById('reward').value||'100');
  const r=await j('/v1/hub/task',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({title, reward})});
  alert(r.ok?`task ${r.json.id}`:JSON.stringify(r.json).slice(0,200)); tick();
}
async function bid(task_id){
  const t=tok(); if(!t){alert('need dca_');return;} const price=prompt('Bid price', '400'); if(!price) return;
  const r=await j('/v1/hub/bid',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({task_id, price:parseInt(price)})});
  alert(r.ok?`bid ${r.json.id}`:JSON.stringify(r.json).slice(0,200)); tick();
}
async function team(task_id){
  const members=prompt('Team members as account:share,... e.g. arena-beta:50,arena-gamma:50', 'arena-beta:50,arena-gamma:50'); if(!members) return;
  const parsed=members.split(',').map(s=>{const [a,sh]=s.split(':'); return [a.trim(), parseInt(sh.trim())];});
  const r=await j('/v1/hub/team',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({task_id, members:parsed})});
  alert(r.ok?`team ${r.json.id}`:JSON.stringify(r.json).slice(0,200)); tick();
}
async function exec(task_id){
  const r=await j('/v1/hub/execute',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({task_id})});
  alert(r.ok?`evidence ${r.json.evidence_id.slice(0,8)}`:JSON.stringify(r.json).slice(0,200)); tick();
}
function addEvents(arr){ if(!arr||!arr.length) return; const c=document.getElementById('events'); const html=arr.slice().reverse().map(e=>`<div class="event"><span class="tick">#${e.tick}</span> <b>${e.kind}</b> ${e.detail} ${e.evidence_id?e.evidence_id.slice(0,8):''}</div>`).join(''); c.innerHTML=html + c.innerHTML; while(c.children.length>60) c.removeChild(c.lastChild); }
let es=null;
function connectSSE(){
  try{
    es=new EventSource('/v1/hub/stream');
    es.onopen=()=>{document.getElementById('sse').textContent='SSE live'; document.getElementById('sse').className='badge live';};
    es.onerror=()=>{document.getElementById('sse').textContent='SSE retry';};
    es.addEventListener('hub_events', ev=>{ try{ const arr=JSON.parse(ev.data); if(arr.length){ const max=Math.max(...arr.map(e=>e.tick)); since=Math.max(since,max+1); addEvents(arr); tick(); } }catch(_){} });
  }catch(_){}
}
connectSSE(); setInterval(tick,2000); tick();
</script></body></html>"##.to_string()
}
