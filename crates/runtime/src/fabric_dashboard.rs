//! Fabric dashboard — a live view of the DecentraAI compute fabric, positioned
//! agent-first: humans and agents enter through the same fabric. Purely a
//! read-only projection over the real endpoints (fabric graph, peers, agents,
//! evidence, credits, status). It never polls proxied inference endpoints, so
//! watching it cannot reset the engine idle clock.

/// The fabric dashboard HTML. Injects the runtime JS which polls the real
/// read-only endpoints and renders the live fabric.
pub const FABRIC_DASHBOARD_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>DecentraAI — Compute Fabric</title>
<style>
:root{--bg:#070b12;--panel:#0d1420;--panel2:#111a2a;--line:#1c2a40;--text:#e6eef8;--muted:#7d92ab;--accent:#22d3ee;--accent2:#818cf8;--ok:#34d399;--warn:#fbbf24;}
*{box-sizing:border-box;margin:0;padding:0}
body{background:radial-gradient(1200px 600px at 20% -10%,#0d1b33 0%,#070b12 60%);color:var(--text);font:14px/1.5 ui-sans-serif,system-ui,-apple-system,sans-serif;min-height:100vh;padding:28px}
.wrap{max-width:1240px;margin:0 auto}
header{display:flex;align-items:center;justify-content:space-between;gap:20px;flex-wrap:wrap;margin-bottom:8px}
h1{font-size:22px;font-weight:700;letter-spacing:.3px}
h1 .dot{color:var(--accent)}
.sub{color:var(--muted);font-size:13px;margin-top:2px}
.pill{border:1px solid var(--line);background:var(--panel);border-radius:999px;padding:6px 14px;font-size:12px;color:var(--muted)}
.pill b{color:var(--ok)}
.actions{display:flex;gap:10px;margin:16px 0}
.enter{flex:1;border-radius:12px;padding:14px 18px;border:1px solid var(--line);background:linear-gradient(135deg,var(--panel),var(--panel2));text-decoration:none;color:var(--text);transition:.2s}
.enter:hover{border-color:var(--accent);transform:translateY(-1px)}
.enter .t{font-weight:700;font-size:15px;display:flex;align-items:center;gap:8px}
.enter .d{color:var(--muted);font-size:12px;margin-top:4px}
.enter .k{display:inline-block;background:#0a1a2e;border:1px solid var(--line);border-radius:6px;padding:1px 6px;font-family:ui-monospace,monospace;font-size:11px;color:var(--accent)}
.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:14px;margin-top:8px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:14px;padding:16px;overflow:hidden;transition:.18s}
.card:hover{border-color:var(--accent);transform:translateY(-1px);box-shadow:0 8px 30px rgba(34,211,238,.08)}
.card h3{font-size:12px;text-transform:uppercase;letter-spacing:1px;color:var(--muted);margin-bottom:12px;display:flex;align-items:center;gap:8px}
.card h3 .n{color:var(--accent2)}
.mono{font-family:ui-monospace,SFMono-Regular,monospace}
.row{display:flex;justify-content:space-between;align-items:center;padding:6px 0;border-bottom:1px solid #16233a;font-size:13px}
.row:last-child{border-bottom:none}
.row .l{color:var(--muted)}
.row .v{font-weight:600}
.ok{color:var(--ok)} .warn{color:var(--warn)} .acc{color:var(--accent)}
.node{display:flex;align-items:center;gap:8px;padding:5px 0;font-size:13px}
.node .bar{height:6px;border-radius:3px;background:#0c1424;flex:1;overflow:hidden}
.node .bar i{display:block;height:100%;background:linear-gradient(90deg,var(--accent),var(--accent2))}
.statelist{max-height:220px;overflow:auto}
.foot{color:var(--muted);font-size:11px;margin-top:22px;text-align:center}
.tag{font-size:10px;border:1px solid var(--line);border-radius:4px;padding:1px 5px;color:var(--muted)}
.dot.pulse{animation:blink 2s infinite}
@keyframes blink{0%,100%{opacity:1}50%{opacity:.3}}
.bignum{font-size:28px;font-weight:700;color:var(--accent)}
.cpubar{display:flex;align-items:center;gap:8px}
.cpubar .fill{height:8px;border-radius:4px;background:linear-gradient(90deg,var(--accent),var(--accent2));flex:1}
</style>
</head>
<body>
<div class="wrap">
  <header>
    <div>
      <h1>DecentraAI <span class="dot" id="livedot">●</span> Compute Fabric</h1>
      <div class="sub">An autonomous compute fabric where humans and AI agents discover, request, contribute and orchestrate compute.</div>
    </div>
    <div class="pill">Fabric status: <b id="fab-status">…</b></div>
  </header>

  <div class="actions">
    <a class="enter" href="/">
      <div class="t">🧑 Enter as <b>Human</b></div>
      <div class="d">Node dashboard — models, chat, RAG, workers.</div>
    </a>
    <a class="enter" href="/v1/token" onclick="agentGuide(event)">
      <div class="t">🤖 Enter as <b>Agent</b></div>
      <div class="d">Scoped <span class="k">dca_…</span> key → <span class="k">/v1/governor/execute</span>, MCP, BYOA.</div>
    </a>
  </div>

  <div class="grid">
    <div class="card"><h3><span class="n">01</span> Nodes · CPU</h3><div id="nodes">loading…</div></div>
    <div class="card"><h3><span class="n">02</span> Models (Model Colony)</h3><div id="models">loading…</div></div>
    <div class="card"><h3><span class="n">03</span> Agents</h3><div id="agents">loading…</div></div>
    <div class="card"><h3><span class="n">04</span> Governor · Jobs</h3><div class="statelist" id="jobs">loading…</div></div>
    <div class="card"><h3><span class="n">05</span> Evidence</h3><div id="evidence">loading…</div></div>
    <div class="card"><h3><span class="n">06</span> Economy · Credit</h3><div id="economy">loading…</div></div>
  </div>

  <div class="foot">Read-only live projection of the fabric. Agents: <span class="tag">/v1/governor/execute</span> <span class="tag">/v1/intel/assist</span> <span class="tag">/v1/pool/bench</span> <span class="tag">/v1/agents/workflow</span> · evidence <span class="tag">/v1/evidence</span> · economy <span class="tag">/v1/credits</span></div>
</div>
<script>
let autoToken="";
(async()=>{try{autoToken=(await(await fetch('/v1/token')).text()).trim();}catch(_){}})();
const auth=()=>({headers:{Authorization:`Bearer ${autoToken}`}});
const $=id=>document.getElementById(id);
async function j(url){try{const r=await fetch(url,auth());return r.ok?await r.json():null;}catch(_){return null;}}

async function tick(){
  // Nodes + CPU
  try{
    const s=await (await fetch('/status')).json();
    $('fab-status').textContent = s.model_loaded?`live · ${s.model_name||'model'}`:'degraded'; const ld=$('livedot'); if(ld){ld.classList.toggle('pulse',s.model_loaded);}
    const peers=(await j('/v1/peers'))||[];
    const cpuPct=Math.round((s.cpu_percent??0));
    let nodes=`<div class="cpubar"><span class="l">this node CPU</span><div class="fill" style="width:${Math.max(6,cpuPct)}%"></div><span class="v">${cpuPct}%</span></div><div class="row"><span class="l">this node</span><span class="v acc">${s.model_loaded?'online':'…'}</span></div>`;
    (peers.length?peers:[]).slice(0,6).forEach(p=>{
      nodes+=`<div class="row"><span class="l mono">${String(p.peer_id||'').slice(0,14)}…</span><span class="v ${p.banned?'warn':'ok'}">${p.banned?'banned':'ok'}</span></div>`;
    });
    $('nodes').innerHTML=nodes+(peers.length===0?'<div class="row"><span class="l">no peers</span></div>':'');
  }catch(_){}

  // Models — from fabric graph (best effort)
  try{
    const fg=await j('/v1/fabric');
    const models=(fg&&fg.models)||[];
    if(models.length){
      let html='';
      models.slice(0,6).forEach(m=>{
        const ram=m.ram_required_mb?`<span class="tag">${Math.round(m.ram_required_mb/1024)}GB</span>`:'';
        html+=`<div class="row"><span class="l mono">${(m.model_id||m.model||'?').slice(0,30)}</span><span class="v">${ram||''}</span></div>`;
      });
      $('models').innerHTML=html;
    } else {
      $('models').innerHTML='<div class="row"><span class="l">no fabric models</span></div>';
    }
  }catch(_){}

  // Agents
  try{
    const ag=(await j('/v1/agents'))||{};
    const list=ag.agents||[];
    $('agents').innerHTML=`<div class="row"><span class="l">agents</span><span class="v acc">${ag.total_count??list.length}</span></div>`+
      list.slice(0,5).map(a=>`<div class="row"><span class="l">${a.name||a.agent_id}</span><span class="v">${a.role||''}</span></div>`).join('')+
      (list.length===0?'<div class="row"><span class="l">none</span></div>':'');
  }catch(_){}

  // Jobs — recent evidence entries (governor executions)
  try{
    const ev=await j('/v1/evidence');
    const recent=(ev&&ev.recent)||[];
    const jobs=recent.filter(e=>String(e.id||'').startsWith('gov:')||String(e.text||'').includes('governor'));
    $('jobs').innerHTML=jobs.slice(0,8).map(e=>`<div class="row"><span class="l mono" title="${e.text||''}">${(e.id||'').slice(0,26)}</span><span class="v">${String(e.text||'').slice(0,24)}</span></div>`).join('')+
      (jobs.length===0?'<div class="row"><span class="l">no jobs yet</span></div>':'');
  }catch(_){}

  // Evidence totals
  try{
    const ev=await j('/v1/evidence');
    const c=(ev&&ev.counts)||{};
    const total=ev&&ev.total||0;
    $('evidence').innerHTML=`<div class="row"><span class="l">total entries</span><span class="v acc">${total}</span></div>`+
      Object.entries(c).map(([k,v])=>`<div class="row"><span class="l">${k}</span><span class="v">${v}</span></div>`).join('');
  }catch(_){}

  // Economy
  try{
    const bal=await j('/v1/credits/balance');
    const accts=(bal&&bal.accounts)||{};
    const rows=Object.entries(accts).map(([k,v])=>`<div class="row"><span class="l mono">${k.slice(0,16)}…</span><span class="v ok">${v.balance??0}</span></div>`);
    $('economy').innerHTML=`<div class="bignum">${Object.values(accts).reduce((a,b)=>a+(b.balance||0),0)}</div><div class="row"><span class="l">workers</span><span class="v">${Object.keys(accts).length}</span></div>`+rows.join('');
  }catch(_){}
}
setInterval(tick,4000); tick();
function agentGuide(e){
  // Show how an agent enters: dca_ key + governor_execute.
  e.preventDefault();
  alert('Agent entry:\n1) issue a scoped key: decentraai consumer-key create --account my-agent --quota-ceiling 5000 --scopes inference\n2) fund it: POST /api/admin/quota/grant {account, amount}\n3) drive the fabric: POST /v1/governor/execute with Authorization: Bearer dca_…\nOr use MCP on /mcp.');
}
</script>
</body>
</html>"#;

/// The agent-first landing hero: a compact, premium page that frames the
/// fabric and links to the live views.
pub const FABRIC_LANDING_HTML: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>DecentraAI — Compute Fabric</title><style>
:root{--bg:#05080f;--panel:#0b111f;--line:#16233c;--text:#e8eef9;--muted:#7c8faa;--accent:#22d3ee;--accent2:#818cf8;--ok:#34d399}
*{box-sizing:border-box;margin:0;padding:0}
body{background:radial-gradient(1100px 600px at 18% -5%,#0e2340 0%,#05080f 60%);color:var(--text);font:15px/1.6 ui-sans-serif,system-ui,sans-serif;min-height:100vh;display:flex;align-items:center;justify-content:center;padding:28px}
.hero{max-width:880px;text-align:center}
.badge{display:inline-block;border:1px solid var(--line);background:var(--panel);border-radius:999px;padding:6px 14px;font-size:12px;color:var(--muted);margin-bottom:22px;letter-spacing:1px}
h1{font-size:44px;font-weight:800;letter-spacing:-1px;line-height:1.1}
h1 .grad{background:linear-gradient(90deg,var(--accent),var(--accent2));-webkit-background-clip:text;background-clip:text;color:transparent}
.tagline{color:var(--muted);font-size:17px;max-width:640px;margin:18px auto 30px}
.flow{display:flex;flex-wrap:wrap;justify-content:center;gap:8px;margin-bottom:34px;color:var(--muted);font-size:12.5px;letter-spacing:.5px}
.flow span{background:var(--panel);border:1px solid var(--line);border-radius:8px;padding:6px 12px}
.flow b{color:var(--accent)}
.cta{display:flex;gap:14px;justify-content:center;flex-wrap:wrap}
.btn{display:inline-block;padding:13px 26px;border-radius:12px;text-decoration:none;font-weight:700;font-size:15px;transition:.2s}
.btn.human{background:linear-gradient(135deg,var(--accent),var(--accent2));color:#04101f}
.btn.agent{background:var(--panel);border:1px solid var(--line);color:var(--text)}
.btn:hover{transform:translateY(-2px)}
.btn small{display:block;font-weight:500;font-size:11px;opacity:.8;margin-top:3px}
.docs{margin-top:40px;color:var(--muted);font-size:12px}
.docs a{color:var(--accent);text-decoration:none;margin:0 6px}
</style></head><body><div class="hero">
<div class="badge">● LIVE COMPUTE FABRIC</div>
<h1>DecentraAI — <span class="grad">compute fabric natively agentic</span></h1>
<div class="tagline">An autonomous fabric where humans and AI agents discover, request, contribute and orchestrate compute — through the same door.</div>
<div class="flow"><span>Agents</span><b>→</b><span>Governor</span><b>→</b><span>Models</span><b>→</b><span>Nodes · CPU</span><b>→</b><span>Evidence</span><b>→</b><span>Economy</span></div>
<div class="cta">
<a class="btn human" href="/flow">Watch it live<small>animated fabric pipeline</small></a>
<a class="btn human" href="/fabric">Fabric dashboard<small>live panels</small></a>
<a class="btn agent" href="/v1/token" onclick="agentGuide(event)">Enter as an Agent<small>scoped dca_ key · /v1/governor/execute</small></a>
<a class="btn agent" href="/">Operator console<small>node dashboard</small></a>
</div>
<div class="docs">Docs: <a href="/docs/PRODUCT.md">Product</a> · <a href="/docs/API.md">API</a> · <a href="/docs/DEPLOYMENT.md">Deploy</a> · <a href="/docs/BENCHMARKS.md">Benchmarks</a></div>
</div>
<script>function agentGuide(e){e.preventDefault();alert('Agent entry:\n1) decentraai consumer-key create --account my-agent --quota-ceiling 5000 --scopes inference\n2) POST /api/admin/quota/grant {account, amount}\n3) POST /v1/governor/execute with Bearer dca_…\nOr MCP on /mcp.');}</script>
</body></html>"#;

/// Renders the agent-first landing hero.
pub fn fabric_landing_html() -> String {
    FABRIC_LANDING_HTML.to_string()
}

/// Renders the fabric dashboard HTML (no-store).
pub fn fabric_dashboard_html() -> (String, bool) {
    (FABRIC_DASHBOARD_HTML.to_string(), true)
}
