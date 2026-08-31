//! Agent World — persistent projection over Hub/Society/EventBus.
//!
//! WorldState is **NOT** a second source of truth. It is a thin
//! projection persisted as `db/world.json` that only stores
//! `world_id + mission.task_id + rooms + agents`.
//! All ledger/quota/placement/Hub/Society remain canonical.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldRoom {
    pub id: String,
    pub label: String,
    pub capability_filter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldAgent {
    pub agent_id: String,
    pub key_id: String,
    pub account: String,
    pub declared_capabilities: Vec<String>,
    pub room_id: String,
    pub joined_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    pub world_id: String,
    pub mission_task_id: Option<String>,
    pub rooms: Vec<WorldRoom>,
    pub agents: Vec<WorldAgent>,
    pub tick: u64,
}

impl Default for WorldState {
    fn default() -> Self {
        Self {
            world_id: "world-build-x".to_string(),
            mission_task_id: None,
            rooms: vec![
                WorldRoom {
                    id: "research-lab".to_string(),
                    label: "Research Lab".to_string(),
                    capability_filter: "research".to_string(),
                },
                WorldRoom {
                    id: "coding-lab".to_string(),
                    label: "Coding Lab".to_string(),
                    capability_filter: "coding".to_string(),
                },
            ],
            agents: vec![],
            tick: 0,
        }
    }
}

impl WorldState {
    pub fn advance_tick(&mut self) {
        self.tick += 1;
    }

    pub fn room_for_capabilities(&self, caps: &[String]) -> String {
        let lower: Vec<String> = caps.iter().map(|c| c.to_lowercase()).collect();
        for room in &self.rooms {
            let f = room.capability_filter.to_lowercase();
            if lower
                .iter()
                .any(|c| c.contains(&f) || f.contains(c.as_str()))
            {
                return room.id.clone();
            }
        }
        // default: first room
        self.rooms
            .first()
            .map(|r| r.id.clone())
            .unwrap_or_else(|| "research-lab".to_string())
    }
}

pub fn world_path_for(repo_root: &Path) -> PathBuf {
    repo_root.join("db/world.json")
}

pub fn load_world_state(path: &Path) -> WorldState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_world_state(path: &Path, state: &WorldState) {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let tmp = path.with_extension("tmp");
    if let Ok(s) = serde_json::to_string_pretty(state) {
        if std::fs::write(&tmp, &s).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

pub fn world_html() -> String {
    r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>DecentraAI — World</title>
<style>
:root{--bg:#070a12;--panel:#111827;--line:#1f2a44;--text:#e6eef8;--muted:#8aa0b8;--accent:#22d3ee;--accent2:#a78bfa;--ok:#34d399;--warn:#fbbf24;--bad:#f87171}
*{box-sizing:border-box;margin:0;padding:0}
body{background:radial-gradient(1200px 600px at 20% -10%, #1a2540 0%, transparent 60%), var(--bg);color:var(--text);font:14px/1.5 system-ui,sans-serif;padding:18px}
header{display:flex;justify-content:space-between;align-items:center;margin-bottom:14px;gap:12px;flex-wrap:wrap}
h1{font-size:22px;letter-spacing:0.2px} h1 span{color:var(--accent)}
.sub{color:var(--muted);font-size:12px}
.badge{padding:3px 8px;border-radius:999px;border:1px solid var(--line);font-size:11px;color:var(--muted)}
.badge.live{border-color:var(--ok);color:var(--ok);box-shadow:0 0 8px #34d39955}
.layout{display:grid;grid-template-columns:1.1fr 0.9fr;gap:14px}
@media(max-width:900px){.layout{grid-template-columns:1fr}}
.card{background:linear-gradient(180deg, #0f172a 0%, #0b1222 100%);border:1px solid var(--line);border-radius:14px;padding:12px;box-shadow:0 6px 20px #0006}
.card h3{font-size:11px;text-transform:uppercase;letter-spacing:1.2px;color:var(--muted);margin-bottom:8px;display:flex;justify-content:space-between;align-items:center}
.world-grid{display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-bottom:12px}
.room{border:1px solid var(--line);border-radius:12px;padding:10px;min-height:220px;position:relative;overflow:hidden;background:linear-gradient(180deg,#0d1426 0%,#0a1020 100%)}
.room h4{font-size:12px;letter-spacing:1px;text-transform:uppercase;color:var(--accent);margin-bottom:8px;display:flex;justify-content:space-between}
.room .filter{font-size:10px;color:var(--muted);border:1px solid var(--line);padding:1px 6px;border-radius:999px}
.agent{border:1px solid #22304a;background:#0e1a30;border-radius:10px;padding:8px 9px;margin-bottom:8px;display:flex;justify-content:space-between;align-items:center;transition:transform .25s ease, border-color .25s ease, box-shadow .25s ease}
.agent.bidding{border-color:var(--warn);box-shadow:0 0 10px #fbbf2440;transform:translateY(-1px)}
.agent.placed{border-color:var(--accent);box-shadow:0 0 10px #22d3ee40;transform:translateY(-1px)}
.agent.settled{border-color:var(--ok);box-shadow:0 0 12px #34d39955;transform:translateY(-1px) scale(1.01)}
.agent .name{font-weight:600;font-size:13px}
.agent .meta{font-size:11px;color:var(--muted)}
.dot{width:8px;height:8px;border-radius:50%;background:var(--muted);box-shadow:0 0 6px #fff2}
.dot.bidding{background:var(--warn)} .dot.placed{background:var(--accent)} .dot.settled{background:var(--ok)}
.mission{border-left:3px solid var(--accent);padding-left:10px}
.mission .status{font-size:11px;padding:2px 7px;border-radius:999px;border:1px solid var(--line);color:var(--muted)}
.mission .status.open{color:var(--muted)} .mission .status.bidding{color:var(--warn);border-color:var(--warn)} .mission .status.assigned{color:var(--accent);border-color:var(--accent)} .mission .status.settled{color:var(--ok);border-color:var(--ok)}
.evidence{font-family:ui-monospace,monospace;font-size:11px;color:var(--accent2);word-break:break-all}
.events{max-height:320px;overflow:auto}
.event{padding:6px 0;border-bottom:1px solid #14203a;font-size:12px;display:flex;gap:8px}
.event .tick{color:var(--accent);font-family:ui-monospace,monospace}
.join{display:flex;gap:8px;flex-wrap:wrap;margin-top:8px}
.join input, .join button, .join select{padding:8px 10px;border-radius:10px;border:1px solid #22304a;background:#0a0e16;color:var(--text);font-size:13px}
.join button{cursor:pointer;background:linear-gradient(180deg,#1a2a4a,#12203a);border-color:#2a3a5e}
.join button:hover{border-color:var(--accent)}
#agentsCount{color:var(--accent)}
</style></head><body>
<header><div><h1>● DecentraAI <span>World</span> — Build X</h1><div class="sub">Research Lab + Coding Lab · MISSION live din HubState · agenți reali cu dca_ · fiecare mișcare = event real</div></div><div><span id="sse" class="badge">SSE …</span> <span class="badge">tick <b id="tick">…</b></span> <span class="badge"><span id="agentsCount">0</span> agents</span></div></header>

<div class="world-grid" id="rooms"></div>

<div class="layout">
<div>
<div class="card"><h3>Mission <span id="missionStatus" class="status open">…</span></h3><div id="mission" class="mission">loading…</div></div>
 <div class="card"><h3>Join World <span class="sub">dca_ + capability → cameră</span></h3>
<div class="sub" style="margin-bottom:6px">1) <a href="/world/join" style="color:var(--accent)">Creează cont instant</a> — fără comandă, primești <code>dca_</code> și intri direct. Sau 2) Onboard clasic <code>POST /v1/agents/onboard</code> cu master.</div>
<div class="join"><input id="tok" placeholder="dca_..." style="flex:1;min-width:220px"><select id="cap"><option value="research">research</option><option value="coding">coding</option></select><button onclick="join()">Join World</button></div>
<div class="join"><input id="missionTitle" placeholder="Mission title (master sau dca_ hub)" style="flex:1"><input id="missionReward" placeholder="reward" type="number" value="500" style="width:110px"><button onclick="createMission()">Create Mission</button></div>
<div id="joinMsg" class="sub" style="margin-top:6px"></div></div>
</div>
<div>
<div class="card"><h3>Live Events <span class="sub">gateway + hub + society</span></h3><div id="events" class="events"><div class="sub">waiting…</div></div></div>
<div class="card"><h3>World Snapshot <span class="sub">GET /v1/world</span></h3><pre id="snapshot" style="font-size:11px;color:var(--muted);max-height:220px;overflow:auto;white-space:pre-wrap"></pre></div>
</div>
</div>

<script>
let since=0;
function tok(){return document.getElementById('tok').value.trim()||localStorage.getItem('world-token')||''}
function auth(){const t=tok();return t?{Authorization:'Bearer '+t}:{}}
async function j(url,opts={}){try{const r=await fetch(url,{...opts,headers:{...(opts.headers||{}),...auth()}});const text=await r.text();let js;try{js=JSON.parse(text)}catch(_){js=text}return {ok:r.ok, js, status:r.status}}catch(e){return {ok:false, js:String(e)}}}

function renderWorld(w){
 document.getElementById('tick').textContent=w.tick
 document.getElementById('agentsCount').textContent=w.agents.length
 document.getElementById('snapshot').textContent=JSON.stringify(w,null,2)
 const roomsEl=document.getElementById('rooms')
 roomsEl.innerHTML=w.rooms.map(r=>{
  const agents=w.agents.filter(a=>a.room_id===r.id)
  const cards=agents.map(a=>{
   const st=a.status||'idle'
   return `<div class="agent ${st}"><div><div class="name">${a.agent_id}</div><div class="meta">${a.account} · ${a.declared_capabilities.join(', ')} · ${st} · rep ${a.reputation?.toFixed?.(2)??'0.00'}</div></div><div class="dot ${st}"></div></div>`
  }).join('')||'<div class="sub" style="padding:8px;border:1px dashed var(--line);border-radius:8px">no agents yet — join with dca_</div>'
  return `<div class="room"><h4>${r.label} <span class="filter">${r.capability_filter}</span></h4>${cards}</div>`
 }).join('')
 const m=w.mission
 const holder=document.getElementById('mission')
 const stEl=document.getElementById('missionStatus')
 if(!m || !m.task){
  holder.innerHTML='<div class="sub">no mission yet — create one</div>'
  stEl.textContent='none'; stEl.className='status open'
 } else {
  const t=m.task; const status=(t.status||'open'); stEl.textContent=status; stEl.className='status '+status
  holder.innerHTML=`<div style="font-weight:600">${t.title} <span style="font-weight:400;color:var(--muted)">#${t.id} · ${t.reward}Cr</span></div>
   <div class="sub">issuer ${t.issuer} · cap ${t.required_capability||'—'}</div>
   ${m.evidence_id?`<div class="evidence" style="margin-top:6px">evidence ${m.evidence_id.slice(0,16)}…</div>`:''}
   ${m.team && m.team.length? `<div class="sub" style="margin-top:6px">team: ${m.team.map(([a,s])=>a+':'+s+'%').join(', ')}</div>`:''}
   ${m.bids && m.bids.length? `<div class="sub" style="margin-top:4px">bids: ${m.bids.map(b=>b.bidder+':'+b.price).join(' · ')}</div>`:''}
   `
 }
}

async function tick(){
 const s=await j('/v1/world'); if(!s.ok) return; renderWorld(s.js)
}
async function join(){
 const t=tok(); if(!t){alert('need dca_');return}
 localStorage.setItem('world-token', t)
 const cap=document.getElementById('cap').value
 const r=await j('/v1/world/join',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({declared_capabilities:[cap]})})
 document.getElementById('joinMsg').textContent=r.ok? `joined ${r.js.room_id} as ${r.js.agent_id}` : JSON.stringify(r.js).slice(0,300)
 tick()
}
async function createMission(){
 const title=document.getElementById('missionTitle').value.trim(); if(!title){alert('need title');return}
 const reward=parseInt(document.getElementById('missionReward').value||'500')
 const r=await j('/v1/world/mission',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({title, reward, required_capability:'research'})})
 document.getElementById('joinMsg').textContent=r.ok? `mission ${r.js.task_id} ${r.js.status}` : JSON.stringify(r.js).slice(0,300)
 tick()
}
function addEvents(arr){
 if(!arr||!arr.length) return;
 const c=document.getElementById('events')
 const html=arr.slice().reverse().map(e=>`<div class="event"><span class="tick">#${e.tick}</span><div><b>${e.kind}</b> ${e.detail||''} ${e.evidence_id?'<span class="evidence">'+e.evidence_id.slice(0,8)+'</span>':''}</div></div>`).join('')
 c.innerHTML=html + c.innerHTML
 while(c.children.length>80) c.removeChild(c.lastChild)
}
let es=null;
function connectSSE(){
 try{
  es=new EventSource('/v1/world/stream')
  es.onopen=()=>{document.getElementById('sse').textContent='SSE live';document.getElementById('sse').className='badge live'}
  es.onerror=()=>{document.getElementById('sse').textContent='SSE retry'}
  es.addEventListener('world', ev=>{ try{const arr=JSON.parse(ev.data); addEvents(arr); tick()}catch(_){}})
  es.addEventListener('hub_events', ev=>{ try{const arr=JSON.parse(ev.data); addEvents(arr); tick()}catch(_){}})
 }catch(_){}
}
connectSSE(); setInterval(tick, 2000); tick()
</script></body></html>"##.to_string()
}

pub fn world_skill_md() -> &'static str {
    include_str!("../../../.agents/skills/world.md")
}
