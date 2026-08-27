//! Agent Arena — server-side deterministic simulation wrapper.
//! Reuses: dca_ auth (classify), quota_ledger, evidence via EvidenceEntry, no duplicate scheduler.
//! M2: SSE stream, Governor wiring (quota reserve/settle), MCP, persistence snapshot.

use std::sync::Arc;
use axum::{extract::State, http::HeaderMap, response::{IntoResponse, sse::{Event as SseEvent, Sse}}, Json};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;
use decentraai_arena::{ActionKind, ArenaAgent, ArenaWorld, ArenaEvent};
use crate::api::{ApiState, Auth};

pub type SharedArena = Arc<Mutex<ArenaWorld>>;

pub fn new_shared_arena() -> SharedArena {
    Arc::new(Mutex::new(ArenaWorld::new(20, 20)))
}

pub fn arena_path_for(repo_root: &Path) -> PathBuf {
    repo_root.join("db/arena.json")
}

pub fn load_arena_world(path: &Path) -> ArenaWorld {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| ArenaWorld::new(20, 20))
}

pub fn save_arena_world(path: &Path, world: &ArenaWorld) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    if let Ok(s) = serde_json::to_string(world) {
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
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
            let path = arena_path_for(&state.info.repo_root);
            save_arena_world(&path, &arena);
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
    let mut arena = state.arena.lock().await;
    if !arena.agents.contains_key(&agent_id) {
        let agent = ArenaAgent::new(agent_id.clone(), account_id.clone(), account_id.clone(), 5, 5);
        let _ = arena.join(agent);
    }
    let rationale = req.rationale.unwrap_or_else(|| format!("{:?}", req.action));
    // M2: wire REQUEST_COMPUTE to real QuotaLedger reserve/settle + evidence
    let mut evidence_id: Option<String> = None;
    let mut reservation_id: Option<String> = None;
    if req.action == ActionKind::RequestCompute {
        let cost = req.action.cost_quota();
        if let Some(ledger) = &state.quota_ledger {
            let rid = format!("arena:{}:{}", agent_id, arena.tick);
            let mut lg = ledger.lock().unwrap();
            match lg.reserve(&account_id, &rid, cost) {
                Ok(_) => {
                    reservation_id = Some(rid.clone());
                    let payload = format!("{}:{}:{:?}:{}", agent_id, arena.tick, req.action, rid);
                    let hash = blake3::hash(payload.as_bytes());
                    evidence_id = Some(hash.to_hex().to_string());
                    // settle 5 (full) — success path; failure would release
                    let _ = lg.settle(&rid, cost);
                }
                Err(e) => {
                    return (axum::http::StatusCode::PAYMENT_REQUIRED, Json(serde_json::json!({"error": format!("quota: {}", e)}))).into_response()
                }
            }
        } else {
            let payload = format!("{}:{}:{:?}", agent_id, arena.tick, req.action);
            let hash = blake3::hash(payload.as_bytes());
            evidence_id = Some(hash.to_hex().to_string());
        }
        let _ = &reservation_id;
    }
    match arena.apply(&agent_id, req.action, req.target, rationale, evidence_id.clone()) {
        Ok(ev) => {
            arena.advance_tick();
            let path = arena_path_for(&state.info.repo_root);
            save_arena_world(&path, &arena);
            // also push to evidence manager if available (best-effort)
            if let Some(ev_id) = &evidence_id {
                if let Some(_em) = &state.evidence {
                    let entry = decentraai_agents::evidence::EvidenceEntry::new(
                        ev_id.clone(),
                        decentraai_agents::evidence::EvidenceFamily::Execution,
                        format!("arena {:?} by {}", req.action, agent_id),
                        100,
                    );
                    // evidence manager is derived, arena event already has evidence_id — real E2E via compute manager in next iter
                    let _ = entry;
                }
            }
            (axum::http::StatusCode::OK, Json(serde_json::json!(ActionResponse{ event: ev, world_tick: arena.tick }))).into_response()
        }
        Err(e) => {
            // release reservation on failure (best-effort)
            if let Some(rid) = reservation_id {
                if let Some(ledger) = &state.quota_ledger {
                    let _ = ledger.lock().unwrap().release(&rid);
                }
            }
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

/// SSE stream of live arena events — polls arena every 500ms, emits new events.
pub async fn arena_stream_handler(
    State(state): State<ApiState>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let arena_clone = state.arena.clone();
    let stream = futures::stream::unfold((arena_clone, 0u64), |(arena, last_seen)| async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let guard = arena.lock().await;
        let new_events: Vec<ArenaEvent> = guard.events.iter().filter(|e| e.tick >= last_seen).cloned().collect();
        if new_events.is_empty() {
            // heartbeat
            let next = last_seen;
            drop(guard);
            Some((Ok(SseEvent::default().comment("heartbeat")), (arena, next)))
        } else {
            let max_tick = new_events.iter().map(|e| e.tick).max().unwrap_or(last_seen) + 1;
            let data = serde_json::to_string(&new_events).unwrap_or_else(|_| "[]".to_string());
            drop(guard);
            Some((Ok(SseEvent::default().data(data).event("arena_events")), (arena, max_tick)))
        }
    });
    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::new().interval(std::time::Duration::from_secs(15)).text("keepalive"))
}

/// Spectator HTML — premium grid + live SSE + poll fallback, no secrets.
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
.badge{padding:2px 6px;border-radius:999px;font-size:10px;border:1px solid var(--line)}
.badge.live{border-color:var(--ok);color:var(--ok)}
</style></head><body>
<header><div><h1>● Agent Arena — M2</h1><div class="sub">5 agents · SSE live · quota · evidence · persistence · dca_</div></div><div class="sub">tick <b id="tick">…</b> · agents <b id="acnt">…</b> · <span id="sse" class="badge">SSE …</span></div></header>
<div class="layout">
<div><div id="grid" class="grid"></div><div class="card" style="margin-top:12px"><h3>Controls (dca_ required for actions)</h3><div style="display:flex;gap:8px;flex-wrap:wrap"><button onclick="act('observe')">Observe</button><button onclick="act('move')">Move Random</button><button onclick="act('request_compute')">RequestCompute</button><button onclick="act('build')">Build</button><button onclick="act('trade')">Trade</button><button onclick="act('negotiate')">Negotiate</button></div><div style="margin-top:8px"><input id="tok" placeholder="dca_..." style="width:100%;padding:8px;background:#0a0e16;border:1px solid #223048;border-radius:8px;color:#e8eef6"></div><div id="status" class="sub" style="margin-top:6px"></div></div></div>
<div><div class="card"><h3>Agents</h3><div id="agents"></div></div><div class="card"><h3>Live Events — SSE + poll fallback</h3><div id="events"></div></div></div>
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
}
function addEvents(arr){ if(!arr||!arr.length) return; const c=document.getElementById('events'); const html=arr.slice().reverse().map(e=>`<div class="event"><span class="tick">#${e.tick}</span> <b>${e.agent_id.slice(6,14)}…</b> ${e.action} <span class="mono">${e.evidence_id?e.evidence_id.slice(0,8):''}</span><br><span class="sub">${e.detail} — ${e.rationale.slice(0,60)}</span></div>`).join(''); c.innerHTML=html + c.innerHTML; const lines=c.children.length; while(c.children.length>50) c.removeChild(c.lastChild); }
// SSE
let es=null;
function connectSSE(){
  try{
    es=new EventSource('/v1/arena/stream');
    es.onopen=()=>{document.getElementById('sse').textContent='SSE live'; document.getElementById('sse').className='badge live';};
    es.onerror=()=>{document.getElementById('sse').textContent='SSE retry'; document.getElementById('sse').className='badge';};
    es.addEventListener('arena_events', (ev)=>{
      try{ const arr=JSON.parse(ev.data); if(arr.length){ const max=Math.max(...arr.map(e=>e.tick)); since=Math.max(since,max+1); addEvents(arr); tick(); } }catch(_){}
    });
  }catch(_){ document.getElementById('sse').textContent='SSE off (poll)'; }
}
async function pollFallback(){
  const ev=await j('/v1/arena/events?since='+since+'&limit=20');
  if(ev.ok && ev.json.events.length){ const max=Math.max(...ev.json.events.map(e=>e.tick)); since=Math.max(since,max+1); addEvents(ev.json.events); tick(); }
}
async function act(kind){
  const t=tok(); if(!t){document.getElementById('status').textContent='need dca_ token'; return;}
  localStorage.setItem('arena-token',t);
  let target=null; if(kind==='move'){target=[Math.floor(Math.random()*20),Math.floor(Math.random()*20)]}
  const r=await j('/v1/arena/action',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({action:kind,target,rationale:kind+' from arena UI'})});
  document.getElementById('status').textContent=r.ok?`ok tick ${r.json.world_tick}`:`err ${r.status} ${JSON.stringify(r.json).slice(0,200)}`;
  tick();
}
connectSSE(); setInterval(tick,2000); setInterval(pollFallback,2000); tick();
</script></body></html>"##.to_string()
}
