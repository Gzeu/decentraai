//! Fabric flow — an animated, live visualization of the DecentraAI compute
//! fabric: Agents → Governor → Model Colony → Resource Decision → Compute
//! Pool → Evidence → Economy. It polls the same real read-only endpoints as
//! the fabric dashboard and renders the live pipeline as an animated flow
//! (SVG), with live metrics below. Never proxies inference endpoints.

/// The animated fabric-flow HTML (SVG pipeline + live metrics).
pub const FABRIC_FLOW_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>DecentraAI — Fabric Flow</title>
<style>
:root{--bg:#060a12;--panel:#0c1322;--line:#1a2740;--text:#e8eef9;--muted:#7c8faa;--accent:#22d3ee;--accent2:#818cf8;--ok:#34d399;--warn:#fbbf24;}
*{box-sizing:border-box;margin:0;padding:0}
body{background:radial-gradient(1100px 620px at 15% -5%,#0d1e3a 0%,#060a12 60%);color:var(--text);font:14px/1.5 ui-sans-serif,system-ui,sans-serif;min-height:100vh;padding:24px}
.wrap{max-width:1240px;margin:0 auto}
header{display:flex;justify-content:space-between;align-items:center;gap:16px;flex-wrap:wrap;margin-bottom:18px}
h1{font-size:21px;font-weight:700}
h1 .dot{color:var(--accent)}
.sub{color:var(--muted);font-size:12.5px}
.pill{border:1px solid var(--line);background:var(--panel);border-radius:999px;padding:6px 14px;font-size:12px;color:var(--muted)}
.pill b{color:var(--ok)}
.pipeline{background:var(--panel);border:1px solid var(--line);border-radius:18px;padding:18px;margin-bottom:16px}
svg{width:100%;height:auto;display:block}
.stage{fill:var(--panel);stroke:var(--line);stroke-width:1.5;rx:12}
.stage-label{fill:var(--muted);font-size:12px;font-weight:600}
.stage-sub{fill:var(--muted);font-size:10px;opacity:.8}
.flow{stroke:var(--accent2);stroke-width:2.5;fill:none;stroke-dasharray:6 6;opacity:.0}
.flow.on{opacity:.95;animation:dash 1.4s linear infinite}
@keyframes dash{to{stroke-dashoffset:-24}}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(250px,1fr));gap:14px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:14px;padding:15px}
.card h3{font-size:11px;text-transform:uppercase;letter-spacing:1.2px;color:var(--muted);margin-bottom:10px}
.row{display:flex;justify-content:space-between;padding:5px 0;border-bottom:1px solid #14203a;font-size:13px}
.row:last-child{border-bottom:none}
.row .l{color:var(--muted)} .mono{font-family:ui-monospace,monospace}
.ok{color:var(--ok)} .acc{color:var(--accent)} .warn{color:var(--warn)}
.big{font-size:26px;font-weight:700;color:var(--accent)}
.foot{color:var(--muted);font-size:11px;margin-top:20px;text-align:center}
</style>
</head>
<body>
<div class="wrap">
  <header>
    <div><h1>DecentraAI <span class="dot">●</span> Fabric Flow</h1><div class="sub">Agents → Governor → Model Colony → Compute → Evidence → Economy — live.</div></div>
    <div class="pill">pipeline: <b id="pl">…</b></div>
  </header>

  <div class="pipeline" id="pipe">
    <svg viewBox="0 0 1200 190" xmlns="http://www.w3.org/2000/svg">
      <g id="stages"></g>
      <g id="flows"></g>
    </svg>
  </div>

  <div class="grid">
    <div class="card"><h3>Compute Pool</h3><div id="pool">…</div></div>
    <div class="card"><h3>Governor · Recent Jobs</h3><div id="jobs">…</div></div>
    <div class="card"><h3>Model Colony</h3><div id="models">…</div></div>
    <div class="card"><h3>Evidence</h3><div id="evidence">…</div></div>
    <div class="card"><h3>Economy · Credit</h3><div id="economy">…</div></div>
  </div>
  <div class="foot">Read-only live view. Agents: <span style="color:var(--accent)">/v1/governor/execute</span> · <span style="color:var(--accent)">/v1/pool/bench</span> · <span style="color:var(--accent)">/v1/agents/workflow</span></div>
</div>
<script>
let autoToken="";(async()=>{try{autoToken=(await(await fetch('/v1/token')).text()).trim();}catch(_){}})();
const auth=()=>({headers:{Authorization:`Bearer ${autoToken}`}});
const $=id=>document.getElementById(id);
async function j(url){try{const r=await fetch(url,auth());return r.ok?await r.json():null;}catch(_){return null;}}

// Pipeline stage coordinates + ids
const STAGES=[
 {id:'agents',x:40,y:55,label:'AGENTS',sub:'human · dca_ keys'},
 {id:'governor',x:250,y:55,label:'GOVERNOR',sub:'LOCAL / DISTRIBUTED'},
 {id:'models',x:460,y:55,label:'MODEL COLONY',sub:'capability · RAM · evidence'},
 {id:'pool',x:670,y:55,label:'COMPUTE POOL',sub:'VPS · Desktop · Laptop'},
 {id:'evidence',x:880,y:55,label:'EVIDENCE',sub:'signed Ed25519'},
 {id:'economy',x:1090,y:55,label:'ECONOMY',sub:'verified credit'},
];
const stageDef=()=>{
  let s='';
  STAGES.forEach(st=>{s+=`<rect class="stage" x="${st.x}" y="30" width="130" height="70"/><text x="${st.x+65}" y="${st.y}" text-anchor="middle" class="stage-label">${st.label}</text><text x="${st.x+65}" y="${st.y+16}" text-anchor="middle" class="stage-sub">${st.sub}</text>`;});
  $('stages').innerHTML=s;
  let f='';
  for(let i=0;i<STAGES.length-1;i++){
    const a=STAGES[i],b=STAGES[i+1];
    const x1=a.x+130,y=65, x2=b.x;
    f+=`<path class="flow" id="flow-${i}" d="M${x1} ${y} L${x2} ${y}"/>`;
  }
  $('flows').innerHTML=f;
  // direct human->governor + agent->governor highlight handled below
};

function pulse(){ for(let i=0;i<STAGES.length-1;i++){const f=$('flow-'+i); if(f) f.classList.add('on');} }
function idle(){ for(let i=0;i<STAGES.length-1;i++){const f=$('flow-'+i); if(f) f.classList.remove('on');} }

async function tick(){
  // pipeline status
  try{
    const s=await (await fetch('/status')).json();
    $('pl').textContent=s.model_loaded?'live · flowing':'degraded';
  }catch(_){}

  // Compute pool
  try{
    const s=await (await fetch('/status')).json();
    const peers=(await j('/v1/peers'))||[];
    let rows=`<div class="row"><span class="l">nodes</span><span class="v acc">${1+peers.length}</span></div>`;
    peers.slice(0,5).forEach(p=>{rows+=`<div class="row"><span class="l mono">${String(p.peer_id||'').slice(0,12)}…</span><span class="v ${p.banned?'warn':'ok'}">${p.banned?'banned':'ok'}</span></div>`;});
    $('pool').innerHTML=rows;
  }catch(_){}

  // Recent jobs (evidence)
  try{
    const ev=await j('/v1/evidence');
    const jobs=(ev&&ev.recent||[]).filter(e=>String(e.id||'').startsWith('gov:'));
    $('jobs').innerHTML=jobs.slice(0,6).map(e=>`<div class="row"><span class="l mono">${(e.id||'').slice(8,26)}</span><span class="v">${(e.text||'').slice(0,22)}</span></div>`).join('')+(jobs.length?'':'<div class="row"><span class="l">no jobs yet</span></div>');
    if(jobs.length) pulse(); else idle();
  }catch(_){}

  // Model colony
  try{
    const fg=await j('/v1/fabric');
    const m=(fg&&fg.models)||[];
    $('models').innerHTML=m.slice(0,5).map(x=>`<div class="row"><span class="l mono">${(x.model_id||x.model||'?').slice(0,22)}</span><span class="v">${(x.quantization||'')}</span></div>`).join('')+'';
  }catch(_){}

  // Evidence
  try{
    const ev=await j('/v1/evidence');
    $('evidence').innerHTML=`<div class="row"><span class="l">total</span><span class="v acc">${ev?ev.total:0}</span></div>`+(ev&&ev.counts?Object.entries(ev.counts).map(([k,v])=>`<div class="row"><span class="l">${k}</span><span class="v">${v}</span></div>`).join(''):'');
  }catch(_){}

  // Economy
  try{
    const bal=await j('/v1/credits/balance');
    const a=(bal&&bal.accounts)||{};
    const total=Object.values(a).reduce((s,x)=>s+(x.balance||0),0);
    $('economy').innerHTML=`<div class="row"><span class="l">total credit</span><span class="v big">${total}</span></div>`+Object.entries(a).slice(0,3).map(([k,v])=>`<div class="row"><span class="l mono">${k.slice(0,12)}…</span><span class="v ok">${v.balance??0}</span></div>`).join('');
  }catch(_){}
}
stageDef(); setInterval(tick,4000); tick();
</script>
</body>
</html>"#;

/// Renders the animated fabric-flow HTML.
pub fn fabric_flow_html() -> String {
    FABRIC_FLOW_HTML.to_string()
}
