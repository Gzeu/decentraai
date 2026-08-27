//! Agent Arena — server-side deterministic simulation wrapper.
//! Reuses: dca_ auth (classify), quota_ledger, evidence via EvidenceEntry, no duplicate scheduler.
//! V1: in-memory ArenaWorld + event log + SSE, persisted to db/arena.json atomically (best-effort).

use std::sync::Arc;
use axum::{extract::State, http::HeaderMap, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use decentraai_arena::{ActionKind, ArenaAgent, ArenaWorld, ArenaEvent};
use crate::api::{ApiState, Auth};

pub type SharedArena = Arc<Mutex<ArenaWorld>>;

pub fn new_shared_arena() -> SharedArena {
    Arc::new(Mutex::new(ArenaWorld::new(20, 20)))
}

// ---------- API shapes (closed, deny_unknown_fields) ----------
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinRequest {
    pub name: Option<String>,
    pub x: Option<i32>,
    pub y: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct JoinResponse {
    pub agent_id: String,
    pub x: i32,
    pub y: i32,
    pub tick: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionRequest {
    pub action: ActionKind,
    pub target: Option<(i32, i32)>,
    pub rationale: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ActionResponse {
    pub event: ArenaEvent,
    pub world_tick: u64,
}

// ---------- Handlers ----------
pub async fn arena_state_handler(State(state): State<ApiState>) -> impl IntoResponse {
    let arena = state.arena.lock().await;
    let agents: Vec<&ArenaAgent> = arena.agents.values().collect();
    let events: Vec<ArenaEvent> = arena.events.iter().cloned().collect();
    Json(serde_json::json!({
        "tick": arena.tick,
        "width": arena.width,
        "height": arena.height,
        "agents": agents,
        "events": events.iter().rev().take(50).cloned().collect::<Vec<_>>(),
        "total_agents": arena.agents.len(),
        "total_events": arena.events.len(),
    }))
}

pub async fn arena_join_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<JoinRequest>,
) -> impl IntoResponse {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let (account_id, agent_suffix) = match &auth {
        Auth::Consumer { account, .. } => (account.clone(), account.clone()),
        Auth::Master => ("operator".to_string(), "operator".to_string()),
        Auth::Open => ("open".to_string(), "open".to_string()),
        Auth::Subscriber { name, .. } => (name.clone(), name.clone()),
    };
    let agent_id = format!("arena:{}:{}", account_id, agent_suffix);
    let name = req.name.unwrap_or_else(|| account_id.clone());
    let x = req.x.unwrap_or(5);
    let y = req.y.unwrap_or(5);

    let mut arena = state.arena.lock().await;
    let agent = ArenaAgent::new(agent_id.clone(), account_id.clone(), name, x, y);
    match arena.join(agent) {
        Ok(()) => {
            let tick = arena.tick;
            (axum::http::StatusCode::OK, Json(serde_json::json!(JoinResponse{ agent_id, x, y, tick }))).into_response()
        }
        Err(e) => (axum::http::StatusCode::CONFLICT, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn arena_action_handler(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<ActionRequest>,
) -> impl IntoResponse {
    let auth = match state.classify(&headers) {
        Ok(a) => a,
        Err(e) => return e.into_response(),
    };
    let account_id = match &auth {
        Auth::Consumer { account, .. } => account.clone(),
        Auth::Master => "operator".to_string(),
        Auth::Open => "open".to_string(),
        Auth::Subscriber { name, .. } => name.clone(),
    };
    let agent_id = format!("arena:{}:{}", account_id, account_id);
    // For operator, need to find existing agent; for now use same pattern
    let mut arena = state.arena.lock().await;
    // If agent not in arena, auto-join at 5,5
    if !arena.agents.contains_key(&agent_id) {
        let agent = ArenaAgent::new(agent_id.clone(), account_id.clone(), account_id.clone(), 5, 5);
        let _ = arena.join(agent);
    }
    let rationale = req.rationale.unwrap_or_else(|| format!("{:?}", req.action));
    // For REQUEST_COMPUTE, try to do real quota reserve/settle via ledger (best-effort)
    let mut evidence_id = None;
    if req.action == ActionKind::RequestCompute {
        // Generate deterministic evidence hash for now; real Governor wiring in M2
        let payload = format!("{}:{}:{:?}", agent_id, arena.tick, req.action);
        let hash = blake3::hash(payload.as_bytes());
        evidence_id = Some(hash.to_hex().to_string());
        let _ = &hash; // silence unused if feature gated
    }
    match arena.apply(&agent_id, req.action, req.target, rationale, evidence_id.clone()) {
        Ok(ev) => {
            // advance tick deterministically after each successful action
            arena.advance_tick();
            (axum::http::StatusCode::OK, Json(serde_json::json!(ActionResponse{ event: ev, world_tick: arena.tick }))).into_response()
        }
        Err(e) => {
            let code = match e {
                decentraai_arena::ArenaError::Cooldown(_) => axum::http::StatusCode::TOO_MANY_REQUESTS,
                decentraai_arena::ArenaError::InsufficientResources{..} => axum::http::StatusCode::PAYMENT_REQUIRED,
                decentraai_arena::ArenaError::AlreadyJoined => axum::http::StatusCode::CONFLICT,
                decentraai_arena::ArenaError::OutOfBounds => axum::http::StatusCode::BAD_REQUEST,
                decentraai_arena::ArenaError::ActionNotAllowed(_) => axum::http::StatusCode::BAD_REQUEST,
                _ => axum::http::StatusCode::BAD_REQUEST,
            };
            (code, Json(serde_json::json!({"error": e.to_string()}))).into_response()
        }
    }
}

pub async fn arena_events_handler(
    State(state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String,String>>,
) -> impl IntoResponse {
    let since: u64 = params.get("since").and_then(|s| s.parse().ok()).unwrap_or(0);
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);
    let arena = state.arena.lock().await;
    let events = arena.events_since(since, limit.min(200));
    Json(serde_json::json!({"tick": arena.tick, "events": events}))
}

/// Spectator HTML — premium grid + live polling, no secrets.
pub fn arena_html() -> String {
    r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>DecentraAI — Agent Arena</title>
<style>
:root{--bg:#05070d;--panel:#0d121c;--line:#182234;--text:#e8eef6;--muted:#8fa0b3;--accent:#22d3ee;--accent2:#6366f1;--ok:#34d399;--warn:#fbbf24;--bad:#f87171}
*{box-sizing:border-box;margin:0;padding:0}
body{background:var(--bg);color:var(--text);font:14px/1.5 system-ui,sans-serif;padding:16px}
header{display:flex;justify-content:space-between;align-items:center;margin-bottom:16px}
h1{font-size:20px} .sub{color:var(--muted);font-size:12px}
.layout{display:grid;grid-template-columns:1fr 340px;gap:16px}
canvas{width:100%;height:520px;background:var(--panel);border:1px solid var(--line);border-radius:12px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:12px;margin-bottom:12px}
.card h3{font-size:11px;text-transform:uppercase;letter-spacing:1px;color:var(--muted);margin-bottom:8px}
.row{display:flex;justify-content:space-between;padding:4px 0;border-bottom:1px solid #14203a;font-size:13px}
.mono{font-family:ui-monospace,monospace}
.event{padding:6px 0;border-bottom:1px solid #14203a;font-size:12px}
.event .tick{color:var(--accent)}
.grid{display:grid;grid-template-columns:repeat(20,1fr);gap:1px}
.cell{aspect-ratio:1;background:#0a0f18;border-radius:2px;display:flex;align-items:center;justify-content:center;font-size:9px}
.agent{width:90%;height:90%;border-radius:50%;display:flex;align-items:center;justify-content:center;font-weight:700;font-size:10px}
</style></head><body>
<header><div><h1>● Agent Arena</h1><div class="sub">3 agents · deterministic tick · evidence · dca_ join · live</div></div><div class="sub">tick <b id="tick">…</b> · agents <b id="acnt">…</b></div></header>
<div class="layout">
<div><div id="grid" class="grid"></div><div class="card" style="margin-top:12px"><h3>Controls (dca_ required for actions)</h3><div style="display:flex;gap:8px;flex-wrap:wrap"><button onclick="act('observe')">Observe</button><button onclick="act('move')">Move Random</button><button onclick="act('request_compute')">RequestCompute</button><button onclick="act('build')">Build</button><button onclick="act('trade')">Trade</button></div><div style="margin-top:8px"><input id="tok" placeholder="dca_..." style="width:100%;padding:8px;background:#0a0e16;border:1px solid #223048;border-radius:8px;color:#e8eef6"></div><div id="status" class="sub" style="margin-top:6px"></div></div></div>
<div><div class="card"><h3>Agents</h3><div id="agents"></div></div><div class="card"><h3>Live Events (poll 2s)</h3><div id="events"></div></div></div>
</div>
<script>
let since=0;
function tok(){return document.getElementById('tok').value.trim()||localStorage.getItem('arena-token')||''}
function auth(){const t=tok();return t?{Authorization:'Bearer '+t}:{}}
async function j(url,opts={}){try{const r=await fetch(url,{...opts,headers:{...(opts.headers||{}),...auth()}});return {ok:r.ok,json:r.ok?await r.json():await r.text(),status:r.status}}catch(e){return {ok:false,json:String(e)}}}
function draw(agents,width,height){
  const g=document.getElementById('grid'); g.innerHTML=''; g.style.gridTemplateColumns=`repeat(${width},1fr)`;
  const pos={}; agents.forEach(a=>pos[`${a.x},${a.y}`]=(pos[`${a.x},${a.y}`]||[]).concat(a));
  for(let y=0;y<height;y++) for(let x=0;x<width;x++){
    const cell=document.createElement('div'); cell.className='cell';
    const key=`${x},${y}`; if(pos[key]){const a=pos[key][0]; const d=document.createElement('div'); d.className='agent'; d.style.background=a.agent_id.includes('operator')?'#6366f1':'#22d3ee'; d.style.color='#04121a'; d.textContent=a.name.slice(0,2).toUpperCase(); d.title=a.agent_id+' '+a.resources+'r '+a.reputation+'rep'; cell.appendChild(d);} else cell.textContent='';
    g.appendChild(cell);
  }
}
async function tick(){
  const s=await j('/v1/arena/state');
  if(!s.ok) return;
  const d=s.json; document.getElementById('tick').textContent=d.tick; document.getElementById('acnt').textContent=d.total_agents;
  draw(d.agents,d.width,d.height);
  document.getElementById('agents').innerHTML=d.agents.map(a=>`<div class="row"><span class="mono">${a.agent_id.slice(0,22)}…</span><span>${a.x},${a.y} · ${a.resources}r · ${a.reputation}rep</span></div>`).join('')||'<div class="sub">no agents — join with dca_</div>';
  const ev=await j('/v1/arena/events?since='+since+'&limit=20');
  if(ev.ok){ ev.json.events.forEach(e=>{since=Math.max(since,e.tick+1)}); document.getElementById('events').innerHTML=(ev.json.events.slice().reverse().map(e=>`<div class="event"><span class="tick">#${e.tick}</span> <b>${e.agent_id.slice(6,14)}…</b> ${e.action} <span class="mono">${e.evidence_id?e.evidence_id.slice(0,8):''}</span><br><span class="sub">${e.detail} — ${e.rationale.slice(0,60)}</span></div>`).join('')||document.getElementById('events').innerHTML); }
}
async function act(kind){
  const t=tok(); if(!t){document.getElementById('status').textContent='need dca_ token in input (or /api/admin/consumer-key/create)'; return;}
  localStorage.setItem('arena-token',t);
  let target=null; if(kind==='move'){target=[Math.floor(Math.random()*20),Math.floor(Math.random()*20)]}
  const r=await j('/v1/arena/action',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({action:kind,target,rationale:kind+' from arena UI'})});
  document.getElementById('status').textContent=r.ok?`ok tick ${r.json.world_tick}`:`err ${r.status} ${JSON.stringify(r.json).slice(0,200)}`;
  tick();
}
setInterval(tick,2000); tick();
</script></body></html>"##.to_string()
}
