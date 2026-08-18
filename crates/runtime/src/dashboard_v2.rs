//! Visual-refresh dashboard shell — DecentraAI Execution Fabric.
//!
//! This is deliberately independent from `dashboard.rs`: v1 remains a stable
//! fallback while operators evaluate v2 at `/ui2`. Dynamic values are fetched
//! from the node's public status views, never from the llama-server backend.
//!
//! Design language: a mission-control / distributed-computing control plane
//! (dark navy, cyan/teal infrastructure glow, purple network energy, dense
//! operational information). Every value rendered comes from the live runtime
//! — no mock data, no invented metrics.

pub const DASHBOARD_V2_HTML: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>DecentraAI · Node</title><style>
/* ---- Design tokens: Execution Fabric control plane ---- */
:root{
  --bg:#020611;--bg2:#050a14;--bg3:#07101f;--bg4:#0b1222;
  --panel:rgba(5,12,25,.82);--panel-2:rgba(7,16,31,.72);
  --line:rgba(0,229,255,.12);--line-2:rgba(0,229,255,.22);
  --cyan:#00E5FF;--teal:#00FFC6;--blue:#2563FF;--indigo:#6366F1;--purple:#8B5CF6;
  --green:#00F5A0;--warn:#ffb657;--danger:#ff8491;
  --ink:#e6edf7;--muted:#8fa3bf;--dim:#5b6b85;
  --glow:0 0 0 1px rgba(0,229,255,.05),0 8px 30px rgba(0,10,30,.45);
  --mono:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;
  --sans:ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",Inter,sans-serif;
}
*{box-sizing:border-box}
html,body{margin:0;padding:0}
body{background:var(--bg);color:var(--ink);font:14px/1.45 var(--sans);min-height:100vh;
  background-image:radial-gradient(1100px 500px at 85% -10%,rgba(99,102,241,.10),transparent 60%),
  radial-gradient(900px 480px at -10% 30%,rgba(0,229,255,.06),transparent 55%),
  radial-gradient(700px 400px at 50% 110%,rgba(139,92,246,.07),transparent 60%)}
::selection{background:rgba(0,229,255,.25)}
.mono{font-family:var(--mono)}
/* ---- Shell layout ---- */
.shell{display:grid;grid-template-columns:236px minmax(0,1fr);min-height:100vh}
/* ---- Rail ---- */
.rail{background:linear-gradient(180deg,rgba(5,10,20,.9),rgba(3,7,16,.95));border-right:1px solid var(--line);
  padding:18px 12px;position:sticky;top:0;height:100vh;display:flex;flex-direction:column;gap:4px;overflow-y:auto}
.brand{display:flex;align-items:center;gap:9px;padding:2px 10px 16px;border-bottom:1px solid var(--line);margin-bottom:10px}
.brand-mark{width:26px;height:26px;border-radius:7px;display:grid;place-items:center;font-weight:800;font-size:15px;
  background:linear-gradient(135deg,var(--cyan),var(--indigo));color:#021018;box-shadow:0 0 18px rgba(0,229,255,.35)}
.brand-name{font-weight:800;letter-spacing:-.03em;font-size:16px}
.brand-sub{font-size:9px;letter-spacing:.18em;color:var(--cyan);text-transform:uppercase;opacity:.75}
.nav-group{font-size:9px;letter-spacing:.16em;color:var(--dim);text-transform:uppercase;padding:14px 10px 4px}
.nav button{border:0;background:transparent;color:var(--muted);text-align:left;border-radius:10px;padding:8px 10px;
  font:inherit;cursor:pointer;display:flex;align-items:center;gap:9px;width:100%;transition:background .12s,color .12s}
.nav button .ico{width:16px;text-align:center;font-size:12px;opacity:.85}
.nav button:hover{background:rgba(0,229,255,.06);color:var(--ink)}
.nav button.active{background:rgba(0,229,255,.10);color:var(--cyan);border:1px solid rgba(0,229,255,.16);
  box-shadow:inset 0 0 18px rgba(0,229,255,.05)}
.rail-bottom{margin-top:auto;padding:12px 4px 2px;border-top:1px solid var(--line)}
/* ---- Main ---- */
.main{max-width:1560px;width:100%;margin:auto;padding:20px 26px 40px}
/* ---- Top header ---- */
.topbar{display:flex;align-items:center;justify-content:space-between;gap:16px;margin-bottom:16px;flex-wrap:wrap}
.topbar h1{font-size:22px;letter-spacing:-.03em;margin:0;font-weight:800}
.topbar .sub{color:var(--muted);font-size:11px;letter-spacing:.06em;text-transform:uppercase;margin-top:2px}
.live-badge{display:inline-flex;align-items:center;gap:7px;border:1px solid rgba(0,229,255,.25);border-radius:99px;
  padding:5px 12px;font-size:10px;letter-spacing:.14em;color:var(--cyan);background:rgba(0,229,255,.06)}
.live-dot{width:7px;height:7px;border-radius:9px;background:var(--cyan);box-shadow:0 0 8px var(--cyan);animation:pulse 2.4s infinite}
@keyframes pulse{0%,100%{opacity:1}50%{opacity:.45}}
/* ---- Status bar ---- */
.statusbar{display:flex;align-items:center;gap:10px;border:1px solid var(--line);border-radius:12px;
  padding:8px 14px;background:var(--panel-2);margin-bottom:18px;flex-wrap:wrap}
.statusbar .now{font-size:9px;letter-spacing:.18em;color:var(--dim);text-transform:uppercase}
.status-flow{display:flex;align-items:center;gap:6px;flex-wrap:wrap;font-family:var(--mono);font-size:11px;color:var(--dim)}
.status-flow .step{display:inline-flex;align-items:center;gap:5px;padding:2px 8px;border-radius:6px;border:1px solid transparent}
.status-flow .step.active{color:var(--cyan);border-color:rgba(0,229,255,.3);background:rgba(0,229,255,.07)}
.status-flow .arrow{color:var(--dim);opacity:.5}
.state-badge{font-family:var(--mono);font-size:11px;letter-spacing:.1em;padding:4px 12px;border-radius:8px;
  border:1px solid rgba(0,245,160,.3);color:var(--green);background:rgba(0,245,160,.07)}
.state-badge.warn{color:var(--warn);border-color:rgba(255,182,87,.3);background:rgba(255,182,87,.07)}
.state-badge.err{color:var(--danger);border-color:rgba(255,132,145,.3);background:rgba(255,132,145,.07)}
/* ---- Views ---- */
.view{display:none}.view.active{display:block}
/* ---- Cards / grids ---- */
.grid{display:grid;gap:14px}
.kpis{grid-template-columns:repeat(6,minmax(0,1fr));margin-bottom:14px}
.kpis-4{grid-template-columns:repeat(4,minmax(0,1fr));margin-bottom:14px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:16px;padding:16px;
  box-shadow:var(--glow);transition:border-color .15s}
.card:hover{border-color:rgba(0,229,255,.2)}
.card h2,.card h3{font-size:10px;letter-spacing:.16em;text-transform:uppercase;color:var(--dim);margin:0 0 12px;font-weight:600}
.card h2 .live{color:var(--green);letter-spacing:.1em}
.kpi .value{font-size:26px;font-weight:800;letter-spacing:-.04em;font-family:var(--mono)}
.kpi .label{color:var(--muted);font-size:10px;letter-spacing:.14em;text-transform:uppercase;margin-top:4px}
.kpi .trend{font-family:var(--mono);font-size:10px;color:var(--green);margin-top:5px}
.kpi .trend.dim{color:var(--dim)}
.kpi{position:relative;overflow:hidden}
.kpi::before{content:"";position:absolute;top:0;left:0;right:0;height:2px;
  background:linear-gradient(90deg,transparent,var(--cyan),transparent);opacity:.5}
.split{grid-template-columns:1.2fr .8fr;margin-top:14px}
.split-3{grid-template-columns:1fr 1fr 1fr;margin-top:14px}
.stack{display:grid;gap:14px}
.row{display:flex;justify-content:space-between;gap:12px;padding:7px 0;border-bottom:1px solid rgba(0,229,255,.07);font-size:13px}
.row:last-child{border-bottom:0}
.row b{color:var(--ink);font-weight:600}
.row span{color:var(--muted);text-align:right;overflow-wrap:anywhere;font-family:var(--mono);font-size:12px}
.hint{color:var(--muted);font-size:12px}
.status{display:inline-flex;align-items:center;gap:6px;border-radius:99px;padding:3px 9px;background:rgba(0,245,160,.08);
  font-size:10px;letter-spacing:.08em;color:var(--green);font-family:var(--mono);text-transform:uppercase}
.status.off{color:var(--danger);background:rgba(255,132,145,.08)}
.status.idle{color:var(--dim);background:rgba(91,107,133,.1)}
/* ---- Topology ---- */
.topology{position:relative;min-height:230px;background:var(--panel-2);border:1px solid var(--line);border-radius:16px;
  overflow:hidden;display:flex;align-items:center;justify-content:center}
.topology::before{content:"";position:absolute;inset:0;background:
  radial-gradient(500px 300px at 50% 50%,rgba(99,102,241,.08),transparent 70%)}
.topo-node{position:absolute;display:flex;flex-direction:column;align-items:center;gap:5px;z-index:2;cursor:default}
.topo-node .orb{width:52px;height:52px;border-radius:50%;display:grid;place-items:center;position:relative;
  background:radial-gradient(circle at 32% 28%,rgba(0,229,255,.25),rgba(5,12,25,.9));
  border:1px solid rgba(0,229,255,.4);box-shadow:0 0 22px rgba(0,229,255,.25)}
.topo-node.local .orb{border-color:rgba(0,245,160,.6);box-shadow:0 0 26px rgba(0,245,160,.3)}
.topo-node.off .orb{border-color:rgba(255,132,145,.5);box-shadow:0 0 18px rgba(255,132,145,.2);opacity:.7}
.topo-node .ring{position:absolute;inset:-6px;border-radius:50%;border:1px solid rgba(0,229,255,.25);animation:pulse 3.4s infinite}
.topo-node .orb i{font-style:normal;font-family:var(--mono);font-size:13px;color:var(--cyan)}
.topo-node .nm{font-family:var(--mono);font-size:11px;color:var(--ink);max-width:130px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.topo-node .tag{font-family:var(--mono);font-size:9px;letter-spacing:.1em;text-transform:uppercase}
.topo-node.local .tag{color:var(--green)}.topo-node.remote .tag{color:var(--cyan)}.topo-node.off .tag{color:var(--danger)}
.topo-node .meta{font-family:var(--mono);font-size:9px;color:var(--dim)}
.topo-line{position:absolute;height:1px;background:linear-gradient(90deg,transparent,rgba(99,102,241,.5),transparent);
  transform-origin:0 50%;z-index:1}
.topo-legend{position:absolute;bottom:10px;left:14px;display:flex;gap:14px;font-family:var(--mono);font-size:9px;color:var(--dim);z-index:3}
.topo-legend b{color:var(--cyan);font-weight:500}
/* ---- Pipeline ---- */
.pipeline{display:flex;align-items:center;gap:2px;flex-wrap:wrap;margin-top:14px}
.pipe-step{display:flex;align-items:center;gap:6px;padding:7px 11px;border:1px solid var(--line);border-radius:9px;
  font-family:var(--mono);font-size:10px;letter-spacing:.06em;color:var(--dim);text-transform:uppercase;background:var(--panel-2)}
.pipe-step .pi{font-size:12px}
.pipe-step.active{color:var(--cyan);border-color:rgba(0,229,255,.35);background:rgba(0,229,255,.08);box-shadow:0 0 14px rgba(0,229,255,.12)}
.pipe-step.done{color:var(--green);border-color:rgba(0,245,160,.25)}
.pipe-arrow{color:var(--dim);font-family:var(--mono);font-size:11px;opacity:.6}
/* ---- Tables ---- */
table{width:100%;border-collapse:collapse;font-size:12px}
th{font-size:9px;letter-spacing:.14em;text-transform:uppercase;color:var(--dim);text-align:left;padding:6px 8px;border-bottom:1px solid var(--line);font-weight:600}
td{padding:7px 8px;border-bottom:1px solid rgba(0,229,255,.06);font-family:var(--mono);font-size:11px}
td .dot{display:inline-block;width:6px;height:6px;border-radius:9px;margin-right:6px;background:var(--green);vertical-align:middle}
td .dot.off{background:var(--danger)}td .dot.idle{background:var(--dim)}
tr:last-child td{border-bottom:0}
/* ---- Chat ---- */
.chat{min-height:300px;max-height:54vh;overflow:auto;background:var(--bg2);border:1px solid var(--line);border-radius:12px;padding:14px}
.msg{padding:10px 13px;border-radius:10px;max-width:85%;margin:7px 0;white-space:pre-wrap;font-size:13px;line-height:1.5}
.msg.user{background:linear-gradient(135deg,rgba(0,229,255,.16),rgba(37,99,255,.16));color:var(--ink);margin-left:auto;border:1px solid rgba(0,229,255,.25)}
.msg.assistant{background:var(--panel);border:1px solid var(--line);color:var(--ink)}
textarea{width:100%;min-height:85px;padding:11px;resize:vertical;font:inherit;color:var(--ink);
  background:var(--bg2);border:1px solid var(--line);border-radius:10px}
textarea:focus,input:focus,select:focus{outline:none;border-color:rgba(0,229,255,.45)}
.chatbar{display:grid;grid-template-columns:1fr auto;gap:10px;margin-top:11px}
.button,input,select{font:inherit;border:1px solid var(--line);border-radius:9px;background:var(--panel-2);color:var(--ink)}
.button{padding:8px 13px;cursor:pointer;transition:border-color .12s,color .12s}
.button:hover{border-color:rgba(0,229,255,.4);color:var(--cyan)}
.button.primary{background:linear-gradient(135deg,var(--cyan),var(--blue));color:#021018;font-weight:700;border:0}
.button.primary:hover{color:#021018;filter:brightness(1.1)}
input[type=password],select{padding:7px 10px;color:var(--ink);background:var(--panel-2)}
.actions{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
.advanced[hidden]{display:none}
.token{width:180px}
.empty{color:var(--muted);padding:12px 0;font-size:13px}
/* ---- Lifecycle ---- */
.lifecycle{display:flex;align-items:center;gap:4px;flex-wrap:wrap;font-family:var(--mono);font-size:9px;letter-spacing:.08em;color:var(--dim);text-transform:uppercase;margin-top:8px}
.lc-step{padding:3px 8px;border-radius:6px;border:1px solid var(--line)}
.lc-step.on{color:var(--green);border-color:rgba(0,245,160,.4);background:rgba(0,245,160,.08)}
.lc-arrow{opacity:.5}
/* ---- Skills pipeline ---- */
.skill-pipe{display:flex;align-items:stretch;gap:8px;flex-wrap:wrap;margin-top:6px}
.skill-box{flex:1;min-width:120px;border:1px solid var(--line);border-radius:12px;padding:12px;background:var(--panel-2)}
.skill-box h4{font-size:9px;letter-spacing:.16em;text-transform:uppercase;color:var(--dim);margin:0 0 8px}
.skill-box .val{font-family:var(--mono);font-size:12px;color:var(--ink);word-break:break-word}
.skill-box.hot{border-color:rgba(0,229,255,.35);box-shadow:0 0 16px rgba(0,229,255,.10)}
.skill-box.hot h4{color:var(--cyan)}
.skill-arrow{align-self:center;color:var(--cyan);font-family:var(--mono);font-size:16px;opacity:.8}
/* ---- Provenance & trust ---- */
.trust-ring{display:flex;align-items:center;gap:16px}
.trust-score{font-family:var(--mono);font-size:34px;font-weight:800;color:var(--green);letter-spacing:-.04em}
.trust-label{font-size:9px;letter-spacing:.16em;color:var(--dim);text-transform:uppercase}
.trust-checks{display:grid;gap:6px;font-family:var(--mono);font-size:11px;color:var(--muted)}
.trust-checks .ok{color:var(--green)}
/* ---- Workload chart (CSS-only spark) ---- */
.spark{display:flex;align-items:flex-end;gap:3px;height:44px;margin-top:10px}
.spark i{flex:1;background:linear-gradient(180deg,var(--cyan),rgba(0,229,255,.08));border-radius:2px 2px 0 0;min-height:2px;opacity:.85}
/* ---- Responsive ---- */
@media(max-width:900px){.shell{display:block}.rail{position:static;height:auto;border-right:0;overflow:visible}
  .nav{display:grid;grid-template-columns:repeat(3,1fr)}.nav-group,.rail-bottom{display:none}
  .main{padding:16px}.kpis{grid-template-columns:repeat(3,minmax(0,1fr))}
  .kpis-4{grid-template-columns:repeat(2,minmax(0,1fr))}.split,.split-3{grid-template-columns:1fr}}
@media(max-width:520px){.kpis,.kpis-4{grid-template-columns:repeat(2,minmax(0,1fr))}
  .chatbar{grid-template-columns:1fr}.actions{flex-wrap:wrap}}
</style></head><body><div class="shell">
<aside class="rail">
  <div class="brand"><div class="brand-mark">◈</div><div><div class="brand-name">DecentraAI</div><div class="brand-sub">Execution Fabric</div></div></div>
  <div class="nav-group">Navigate</div>
  <nav class="nav" id="nav">
    <button class="active" data-view="overview"><span class="ico">◉</span>Overview</button>
    <button data-view="chat"><span class="ico">⌁</span>Chat</button>
    <div class="nav-group">Fabric</div>
    <button data-view="agents"><span class="ico">◎</span>Agents</button>
    <button data-view="skills"><span class="ico">⚡</span>Skills</button>
    <button data-view="workers"><span class="ico">▤</span>Workers</button>
    <button data-view="network"><span class="ico">○</span>Network</button>
    <button data-view="execution"><span class="ico">⇄</span>Execution</button>
    <button data-view="models"><span class="ico">▦</span>Models</button>
    <div class="nav-group">Ops</div>
    <button data-view="settings"><span class="ico">⚙</span>Settings</button>
    <button data-view="diagnostics"><span class="ico">⌖</span>Diagnostics</button>
  </nav>
  <div class="rail-bottom"><button class="quiet" id="advanced-toggle">Show advanced</button></div>
</aside>
<main class="main">
  <header class="topbar">
    <div><h1 id="title">EXECUTION FABRIC</h1><div class="sub" id="node-line">Connecting to local node…</div></div>
    <div class="actions">
      <span class="live-badge"><span class="live-dot"></span><span id="live-text">LIVE FABRIC</span></span>
      <input class="token" id="token" type="password" autocomplete="off" placeholder="API token (optional)">
      <button class="button" id="refresh">Refresh</button>
    </div>
  </header>
  <div class="statusbar"><span class="now">Now</span>
    <div class="status-flow">
      <span class="step" data-state="ready">READY</span><span class="arrow">→</span>
      <span class="step" data-state="planning">PLANNING</span><span class="arrow">→</span>
      <span class="step" data-state="executing">EXECUTING</span><span class="arrow">→</span>
      <span class="step" data-state="learning">LEARNING</span>
    </div>
    <span class="state-badge" id="state-badge">READY</span>
  </div>

  <section class="view active" id="view-overview">
    <div class="grid kpis" id="kpi-row"></div>
    <div class="card"><h2>Fabric Topology <span class="live">● Live</span></h2>
      <div class="topology" id="topology"><div class="topo-legend">LOCAL <b>●</b>&nbsp; REMOTE <b>◉</b>&nbsp; OFFLINE <b>○</b>&nbsp; latency ms</div></div>
      <div class="pipeline" id="pipeline"></div>
    </div>
    <div class="grid split">
      <div class="card"><h2>Active Workers <span class="live">● Live</span></h2><div id="workers-table"><div class="empty">Loading worker registry…</div></div></div>
      <div class="stack">
        <div class="card"><h2>Model Status</h2><div id="model-card" class="list"></div></div>
        <div class="card"><h2>Inference</h2><div id="inference-card" class="list"></div></div>
      </div>
    </div>
    <div class="grid split-3">
      <div class="card"><h2>Queue <span class="live">● Live</span></h2><div id="queue-card" class="list"></div></div>
      <div class="card"><h2>Fabric Workload</h2><div id="workload-card" class="list"></div><div class="spark" id="workload-spark"></div></div>
      <div class="card"><h2>P2P Fabric</h2><div id="p2p-card" class="list"></div></div>
    </div>
    <div class="grid split">
      <div class="card"><h2>Recent Events <span class="live">● Live</span></h2><div id="recent-events" class="list"></div></div>
      <div class="card"><h2>Provenance &amp; Trust</h2><div id="trust-card"></div></div>
    </div>
  </section>

  <section class="view" id="view-chat"><article class="card"><h2>Chat with this node</h2>
    <p class="hint">Your token stays in this browser. Replies stream directly from the node API.</p>
    <div class="chat" id="chat"><div class="empty">Start a conversation.</div></div>
    <div class="chatbar"><textarea id="prompt" placeholder="Ask the currently served model…"></textarea>
      <div class="stack"><select id="chat-model"><option value="">Current model</option></select>
        <label class="hint"><input id="stream" type="checkbox" checked> Stream response</label>
        <button class="button primary" id="send">Send</button></div>
    </div><div class="hint" id="chat-status"></div></article>
  </section>

  <div id="advanced" class="advanced" hidden>
    <section class="view" id="view-agents"><div class="card"><h2>Collective Agents <span class="live">● Live</span></h2><div id="agents-grid" class="grid kpis-4"></div><div id="agents-list" class="stack" style="margin-top:12px"></div></div></section>
    <section class="view" id="view-skills"><div class="card"><h2>Skills <span class="live">● Live</span> · P8 Dataset/Skill</h2><div id="skills-kpis" class="grid kpis-4"></div><div class="skill-pipe" id="skill-pipe" style="margin-top:14px"></div><div id="skills-list" class="stack" style="margin-top:14px"></div></div></section>
    <section class="view" id="view-workers"><div class="card"><h2>Workers — Distributed Compute Registry <span class="live">● Live</span></h2><div id="workers-detail"></div></div></section>
    <section class="view" id="view-network"><div class="card"><h2>Network</h2><pre id="network" class="mono"></pre></div></section>
    <section class="view" id="view-execution"><div class="card"><h2>Execution — Planner Decisions</h2><pre id="execution" class="mono"></pre></div></section>
    <section class="view" id="view-models"><div class="card"><h2>Models</h2><div id="models" class="list"></div></div></section>
    <section class="view" id="view-settings"><div class="card"><h2>Settings</h2><pre id="settings" class="mono"></pre></div></section>
    <section class="view" id="view-diagnostics"><div class="card"><h2>Diagnostics</h2><pre id="diagnostics" class="mono"></pre></div></section>
  </div>
</main></div><script>/*__JS__*/</script></body></html>"##;

pub const JS_V2_TEMPLATE: &str = r##"
const $ = id => document.getElementById(id);
const esc = v => String(v ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const fmt = (n,d=1) => Number.isFinite(n) ? n.toFixed(d) : '—';
const fmtBytes = b => { if (!Number.isFinite(b)) return '—'; const u=['B','KiB','MiB','GiB','TiB']; let i=0; while(b>=1024&&i<u.length-1){b/=1024;i++} return b.toFixed(i?1:0)+' '+u[i]; };
const auth = () => $('token').value.trim() ? {Authorization:'Bearer '+$('token').value.trim()} : (autoToken ? {Authorization:'Bearer '+autoToken} : {});
const tokenKey = 'decentraai.dashboard-v2.token';
try { $('token').value = localStorage.getItem(tokenKey) || ''; } catch (_) {}
$('token').addEventListener('change', () => { try { localStorage.setItem(tokenKey, $('token').value.trim()); } catch (_) {} });
// Loopback convenience: the node serves its own operator token at /v1/token
// (same mechanism as the v1 dashboard), so the fabric views work without
// typing the token manually. A token typed in the field always wins.
let autoToken = '';
(async () => { try { autoToken = (await (await fetch('/v1/token')).text()).trim(); } catch (_) {} })();

const title = {overview:'EXECUTION FABRIC',chat:'Chat',agents:'Collective Agents',skills:'Skills',workers:'Workers',network:'Network',execution:'Execution',models:'Models',settings:'Settings',diagnostics:'Diagnostics'};
let currentView = 'overview', lastStatus = null;
function show(view) {
  currentView = view;
  document.querySelectorAll('.view').forEach(el => el.classList.toggle('active', el.id === 'view-'+view));
  document.querySelectorAll('[data-view]').forEach(el => el.classList.toggle('active', el.dataset.view === view));
  $('title').textContent = title[view] || view;
  if (!['overview','chat','models'].includes(view)) loadAdvanced(view);
}
document.querySelectorAll('[data-view]').forEach(button => button.addEventListener('click', () => show(button.dataset.view)));
const advanced = $('advanced');
function setAdvanced(on) { advanced.hidden = !on; $('advanced-toggle').textContent = on ? 'Hide advanced' : 'Show advanced'; if (!on && !['overview','chat'].includes(currentView)) show('overview'); }
setAdvanced((localStorage.getItem('decentraai.dashboard-v2.advanced') || '0') === '1');
$('advanced-toggle').addEventListener('click', () => { const on = advanced.hidden; try { localStorage.setItem('decentraai.dashboard-v2.advanced', on ? '1' : '0'); } catch (_) {} setAdvanced(on); });

function valueRows(values) { return Object.entries(values).map(([k,v]) => '<div class="row"><b>'+esc(k)+'</b><span>'+esc(v)+'</span></div>').join(''); }
function kpi(label,value,trend,trendDim) { return '<article class="card kpi"><div class="value">'+esc(value)+'</div><div class="label">'+esc(label)+'</div>'+(trend?'<div class="trend'+(trendDim?' dim':'')+'">'+esc(trend)+'</div>':'' )+'</article>'; }
function stateFromStatus(s) {
  if (!s.model_loaded) return {txt:'RECOVERING',cls:'warn'};
  const q = s.queue || {}, waiting = (q.waiting || []).length;
  if (waiting > 0 || (s.requests_served || 0) > (s.recent_requests?.length || 0)) return {txt:'EXECUTING',cls:''};
  return {txt:'READY',cls:''};
}
function renderStatus(s) {
  lastStatus = s;
  const sys = s.system || {}, gpu = sys.gpu;
  const st = stateFromStatus(s);
  const badge = $('state-badge'); badge.textContent = st.txt; badge.className = 'state-badge'+(st.cls?' '+st.cls:'');
  const steps = {ready:['ready'],planning:['ready','planning'],executing:['ready','planning','executing']};
  const activeSet = steps[st.txt.toLowerCase()] || ['ready'];
  document.querySelectorAll('.status-flow .step').forEach(el => { el.classList.toggle('active', activeSet.includes(el.dataset.state)); });
  $('live-text').textContent = st.txt === 'READY' ? 'LIVE FABRIC' : st.txt;
  $('node-line').textContent = (s.node?.name || s.p2p_peer_id || 'Local node')+' · '+(s.model_loaded ? 'ready' : 'engine not loaded');

  // KPI strip — real metrics only
  const workers = (lastCompute?.workers || []).length;
  const agents = lastAgents?.total_count ?? '—';
  const peers = lastPeers?.length ?? '—';
  $('kpi-row').innerHTML =
    kpi('Requests', s.requests_served ?? 0, (s.recent_requests?.length||0)+' recent') +
    kpi('Tokens', s.tokens_generated ?? 0, fmt(s.tokens_generated/(Math.max(s.uptime_secs,1)),0)+'/s avg') +
    kpi('Success', fmt(s.success_rate_percent ?? 0)+'%', s.requests_failed ? (s.requests_failed+' failed') : '0 failed', !!s.requests_failed) +
    kpi('Workers', workers || '—', lastCompute ? workers+' advertised' : '') +
    kpi('Agents', agents, (lastAgents?.local_count ?? 0)+' local') +
    kpi('Uptime', (s.uptime_secs ?? 0)+'s', fmt(s.idle_for_secs ?? 0,0)+'s idle');

  // Model status card
  const served = (s.node?.served_models || [])[0];
  $('model-card').innerHTML = valueRows({
    Model: s.model || '—',
    Loaded: s.model_loaded ? 'ACTIVE' : 'UNLOADED',
    Size: fmtBytes(s.model_size_bytes || (served?.size_mb||0)*1048576),
    Context: served ? (served.context_tokens||'—')+' tokens' : '—',
    EstRAM: served?.est_ram_mb ? fmtBytes(served.est_ram_mb*1048576) : '—',
    Backend: s.backend || '—',
    Respawns: s.engine_respawns ?? 0,
  });

  // Inference card
  const lat = s.latency_ms || {};
  $('inference-card').innerHTML = valueRows({
    'Latency p50': lat.p50 ? lat.p50+' ms' : '—',
    'Latency p95': lat.p95 ? lat.p95+' ms' : '—',
    'Latency p99': lat.p99 ? lat.p99+' ms' : '—',
    'RAM free': sys.ram_available_gib !== undefined ? fmt(sys.ram_available_gib)+' GiB' : '—',
    'CPU': sys.cpu_usage_percent !== undefined ? fmt(sys.cpu_usage_percent)+'%' : '—',
    'GPU': gpu ? fmt(gpu.utilization_percent)+'%' : 'none',
  });

  // Queue card
  const q = s.queue || {};
  $('queue-card').innerHTML = valueRows({
    Serving: q.serving?.who || 'none',
    Waiting: (q.waiting || []).length,
    'Requests failed': s.requests_failed ?? 0,
  });

  // Fabric workload — real local perf + spark history
  const perf = lastCompute?.local_perf || {};
  const spark = sparkHistory.length ? sparkHistory.map(v => Math.max(2, Math.min(44, (v||0)*10))).join(' ') : '';
  $('workload-card').innerHTML = valueRows({
    'Tokens/sec': perf.tokens_per_second ? fmt(perf.tokens_per_second) : '—',
    Latency: perf.current_latency_ms ? fmt(perf.current_latency_ms)+' ms' : '—',
    'Queue depth': perf.queue_depth ?? 0,
  });
  $('workload-spark').innerHTML = spark ? sparkHistory.map((v,i) => '<i style="height:'+Math.max(3,Math.min(44,(v||0)*10))+'px"></i>').join('') : '<span class="empty">No live throughput yet.</span>';

  // P2P fabric
  const links = lastNetwork?.links || [];
  $('p2p-card').innerHTML = valueRows({
    Connected: (lastNetwork?.connected?.length ?? 0),
    'LAN links': links.length,
    'Bootstrap': (lastNetwork?.bootstrap_peers?.length ?? 0),
    'DHT': lastNetwork?.dht_enabled ? 'on' : 'off',
    'Relay': lastNetwork?.relay_enabled ? 'on' : 'off',
  });

  // Recent events — real audit/event stream
  const evs = (s.recent_events || []).slice(0,12);
  $('recent-events').innerHTML = evs.length ? evs.map(e => {
    const t = new Date((e.timestamp||0)*1000).toLocaleTimeString();
    return '<div class="row"><b>'+esc(t)+'</b><span>'+esc(e.event)+(e.details?.node_name?' · '+esc(e.details.node_name):'')+'</span></div>';
  }).join('') : '<div class="empty">No recent security/ops events.</div>';

  renderTrust();

  // Models select in chat
  const select = $('chat-model'), chosen = select.value;
  select.innerHTML = '<option value="">Current model</option>'+(s.available_models || []).map(m => '<option value="'+esc(m)+'">'+esc(m)+'</option>').join('');
  select.value = chosen;

  // Models view (advanced)
  $('models').innerHTML = (s.available_models || []).map(m => '<div class="row"><b>'+esc(m)+'</b><span>'+'served</span></div>').join('') || '<div class="empty">No indexed models.</div>';
}
function renderTopology(s) {
  const workers = lastCompute?.workers || [];
  const links = lastNetwork?.links || [];
  const localPeer = lastCompute?.local_peer || (s.node && s.node.peer_id) || '';
  const nodes = workers.map(w => ({peer:w.peer_id, name:w.node_name || w.node_id, remote: w.peer_id !== localPeer, load:w.load_percent, lat:(links.find(l=>l.peer===w.peer_id)||{}).rtt_ms, status:w.status, lastSeen:w.last_seen_secs, trusted:w.trusted, models:(w.available_models||[]).length, reachable:w.reachable}));
  const el = $('topology');
  const W = el.clientWidth || 700, H = el.clientHeight || 230;
  const spots = nodes.length ? nodes : [{peer:localPeer,name:(s.node?.name||'local'),remote:false,load:0,lat:0,status:'Ready',lastSeen:0,trusted:true,models:0,reachable:true}];
  const positions = spots.map((n,i) => {
    if (spots.length === 1) return {x:W/2, y:H/2};
    const ang = (i / spots.length) * Math.PI * 2 - Math.PI/2;
    return {x:W/2 + Math.cos(ang)*(W/2 - 100), y:H/2 + Math.sin(ang)*(H/2 - 55)};
  });
  let html = '<div class="topo-legend">LOCAL <b>●</b>&nbsp; REMOTE <b>◉</b>&nbsp; OFFLINE <b>○</b>&nbsp; latency ms</div>';
  // connecting lines between adjacent ring positions
  const lines = [];
  for (let i=0;i<positions.length;i++){ const a=positions[i], b=positions[(i+1)%positions.length];
    const cx=(a.x+b.x)/2, cy=(a.y+b.y)/2, dx=b.x-a.x, dy=b.y-a.y, len=Math.hypot(dx,dy)||1, ang=Math.atan2(dy,dx)*180/Math.PI;
    lines.push('<div class="topo-line" style="left:'+cx+'px;top:'+cy+'px;width:'+len+'px;transform:rotate('+ang+'deg)"></div>'); }
  html += lines.join('');
  spots.forEach((n,i) => {
    const off = !n.reachable && n.lastSeen > 30;
    const cls = n.remote ? (off?'off':'remote') : 'local';
    const meta = (n.lat ? n.lat+'ms' : '—') + ' · ' + (n.trusted ? 'trusted' : 'untrusted');
    html += '<div class="topo-node '+cls+'" style="left:'+(positions[i].x-34)+'px;top:'+(positions[i].y-42)+'px">'+
      '<div class="orb"><i>'+(n.remote?'◉':'●')+'</i>'+(off?'':'<div class="ring"></div>')+'</div>'+
      '<div class="nm">'+esc(n.name||n.peer.slice(0,16))+'</div>'+
      '<div class="tag">'+(off?'OFFLINE':(n.status||'READY'))+'</div>'+
      '<div class="meta">'+meta+'</div></div>';
  });
  el.innerHTML = html;
}
function renderPipeline() {
  const stages = ['USER','REQUEST','PLANNER','RESERVATION','FABRIC','WORKER','ENGINE','STREAM','RESULT'];
  const decisions = lastExec?.decisions || [], execs = lastExec?.executions || [];
  let active = 0;
  if (execs.length) active = execs.length ? 6 : 0;
  const html = stages.map((st,i) => {
    const act = i === active && execs.length;
    const done = i < active;
    return '<span class="pipe-step'+(act?' active':done?' done':'')+'"><span class="pi">'+(act?'◉':done?'✓':'·')+'</span>'+st+'</span>'+(i<stages.length-1?'<span class="pipe-arrow">→</span>':'');
  }).join('');
  $('pipeline').innerHTML = html + (decisions.length ? '<span class="hint" style="margin-left:12px">'+decisions.length+' planner decisions recorded</span>' : '');
}
function renderPeers(peers) {
  lastPeers = peers;
  const healthy = (peers || []).filter(p => !p.banned).length;
  const totalV = (peers||[]).reduce((a,p)=>a+(p.verified||0),0);
  const info = $('health');
  if (info) info.insertAdjacentHTML('beforeend', '<div class="row"><b>Peers</b><span>'+healthy+' healthy / '+(peers || []).length+' known</span></div>');
  renderTrust();
}
function renderTrust() {
  // Trust & provenance — real values only
  const peersArr = lastPeers || [];
  const totalV = peersArr.reduce((a,p)=>a+(p.verified||0),0), totalF = peersArr.reduce((a,p)=>a+(p.failed||0),0);
  const trustPct = (totalV+totalF) > 0 ? Math.round(100*totalV/(totalV+totalF)) : null;
  $('trust-card').innerHTML =
    '<div class="trust-ring"><div><div class="trust-score">'+(trustPct===null?'—':trustPct+'%')+'</div><div class="trust-label">Trust</div></div>'+
    '<div class="trust-checks">'+
    '<span class="ok">✓ '+esc(totalV)+' verified responses</span>'+
    '<span>✗ '+esc(totalF)+' failed</span>'+
    '<span>'+esc(peersArr.length)+' peers scored</span>'+
    '<span class="ok">✓ Signed capability claims</span>'+
    '<span class="ok">✓ Per-chunk BLAKE3 verified</span>'+
    '</div></div>';
}
function renderWorkers() {
  const workers = lastCompute?.workers || [];
  const rows = workers.map(w => '<tr><td>'+(w.trusted?'<span class="dot"></span>':'<span class="dot idle"></span>')+esc(w.node_name||w.node_id||w.peer_id.slice(0,12))+'</td>'+
    '<td>'+(w.reachable===false?'<span class="dot off"></span>':'<span class="dot"></span>')+esc(w.status||'Ready')+'</td>'+
    '<td>'+esc(w.load_percent ?? 0)+'%</td>'+
    '<td>'+fmtBytes((w.ram_mb||0)*1048576)+'</td>'+
    '<td>'+esc(w.cpu_cores ?? '—')+'</td>'+
    '<td>'+((w.available_models||[]).length)+'</td>'+
    '<td>'+(w.current_latency_ms ? fmt(w.current_latency_ms)+'ms' : '—')+'</td></tr>').join('');
  $('workers-table').innerHTML = workers.length
    ? '<table><thead><tr><th>Node</th><th>Status</th><th>Load</th><th>RAM</th><th>CPU</th><th>Models</th><th>Latency</th></tr></thead><tbody>'+rows+'</tbody></table>'
    : '<div class="empty">No workers advertised yet.</div>';
}
async function refresh() {
  try {
    const s = await (await fetch('/status')).json();
    renderStatus(s); renderTopology(s);
  } catch (_) { $('node-line').textContent = 'Status unavailable — is the node running?'; }
  try { const peers = await (await fetch('/v1/peers',{headers:auth()})).json(); renderPeers(peers); } catch (_) {}
  try { lastCompute = await (await fetch('/v1/compute',{headers:auth()})).json(); } catch (_) {}
  try { lastNetwork = await (await fetch('/v1/network',{headers:auth()})).json(); } catch (_) {}
  try { lastExec = await (await fetch('/v1/execution',{headers:auth()})).json(); } catch (_) {}
  try { lastAgents = await (await fetch('/v1/agents',{headers:auth()})).json(); } catch (_) {}
  // Second pass: cards that depend on /v1/* data (workload, p2p, KPI workers,
  // topology rings, pipeline) must reflect the fresh values in the SAME tick,
  // not one poll later.
  if (lastStatus) { renderStatus(lastStatus); renderTopology(lastStatus); renderPipeline(); }
  renderWorkers(); renderAgents();
  if (lastCompute?.local_perf?.tokens_per_second) { sparkHistory.push(lastCompute.local_perf.tokens_per_second); if (sparkHistory.length > 24) sparkHistory.shift(); }
}
$('refresh').addEventListener('click', refresh);
refresh();
// These are the only recurring requests: dashboard observation must never
// invoke the inference proxy or reset the managed engine's idle clock.
setInterval(refresh, 5000);

let lastCompute=null, lastNetwork=null, lastExec=null, lastAgents=null, lastPeers=null, sparkHistory=[];

/* ---- Advanced views ---- */
const advancedEndpoints = {workers:'/v1/compute',network:'/v1/network',execution:'/v1/execution',settings:'/v1/resources',diagnostics:'/v1/fabric'};
const loadedAdvanced = new Set();
async function loadAdvanced(view) {
  if (view==='agents') { renderAgents(); return; }
  if (view==='skills') { renderSkills(); return; }
  if (view==='models') return;
  if (loadedAdvanced.has(view) || !advancedEndpoints[view]) return;
  const target = $(view);
  try { const r = await fetch(advancedEndpoints[view], {headers:auth()}); const j = await r.json(); target.textContent = JSON.stringify(j,null,2); loadedAdvanced.add(view); }
  catch (_) { target.textContent = 'This view needs a valid operator token or the node is unavailable.'; }
}
function renderAgents() {
  const ag = lastAgents || {};
  const agents = ag.agents || [];
  $('agents-grid').innerHTML =
    kpi('Total Agents', ag.total_count ?? '—', '') +
    kpi('Local', ag.local_count ?? '—', '') +
    kpi('Remote Peers', ag.remote_peer_count ?? '—', '') +
    kpi('Capability Claims', agents.reduce((a,x)=>a+((x.semantic_capabilities||[]).length),0), '');
  $('agents-list').innerHTML = agents.map(a => {
    const caps = (a.semantic_capabilities || []).map(c => typeof c === 'string' ? c : (c.capability || '')).filter(Boolean);
    return '<div class="card"><div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:8px">'+
      '<b>'+esc(a.agent_id)+'</b><span class="status'+(a.remote?'':'')+'">'+(a.remote?'REMOTE':'LOCAL')+'</span></div>'+
      '<div class="hint">'+esc(a.role)+' · '+(a.policies?.allow_remote ? 'remote-ok' : 'local-only')+'</div>'+
      (caps.length ? '<div style="margin-top:8px;display:flex;flex-wrap:wrap;gap:5px">'+caps.map(c=>'<span class="status idle" style="text-transform:none;letter-spacing:0">'+esc(c)+'</span>').join('')+'</div>' : '')+
      '</div>';
  }).join('') || '<div class="empty">No agents.</div>';
}
function renderSkills() {
  // Fetch skills live (not part of the recurring refresh to keep it light)
  (async () => {
    let d = null;
    try { d = await (await fetch('/v1/skills',{headers:auth()})).json(); } catch (_) { return; }
    const skills = d.skills || [], datasets = d.datasets || [], model = d.model || {};
    const unlocked = new Set((skills||[]).flatMap(s=>s.unlocked||[]));
    $('skills-kpis').innerHTML =
      kpi('Datasets', datasets.length, '') +
      kpi('Skills', skills.length, '') +
      kpi('Applicable', model.applicable_skills ?? 0, '') +
      kpi('Unlocked Caps', unlocked.size, (d.runtime_evidence ? 'runtime evidence' : 'provenance-aware'));
    const pipe = [
      {t:'Dataset', v: datasets.length ? datasets.map(x=>x.id||x.name||'dataset').join(', ') : '—'},
      {t:'Skill', v: skills.length ? skills.map(x=>x.name||x.id).join(', ') : '—'},
      {t:'Capability', v: [...unlocked].join(', ') || '—'},
      {t:'Talent', v: 'P8 graph'},
      {t:'Agent Power', v: model.base_capabilities?.length ? model.base_capabilities.length+' base caps' : '—'},
    ];
    $('skill-pipe').innerHTML = pipe.map((p,i) => '<div class="skill-box'+(i===2?' hot':'')+'"><h4>'+p.t+'</h4><div class="val">'+esc(p.v)+'</div></div>'+(i<pipe.length-1?'<span class="skill-arrow">→</span>':'')).join('');
    $('skills-list').innerHTML = skills.map(s => '<div class="card"><b>'+esc(s.name||s.id)+'</b><div class="hint">status: '+esc(s.status)+' · develops: '+esc((s.develops||[]).join(', '))+'</div></div>').join('') || '<div class="empty">No skills registered.</div>';
  })();
}
function renderWorkersDetail() {
  const workers = lastCompute?.workers || [];
  $('workers-detail').innerHTML = workers.length ? workers.map(w =>
    '<div class="card" style="margin-bottom:12px"><div style="display:flex;justify-content:space-between;align-items:center">'+
    '<b>'+esc(w.node_name||w.node_id)+'</b><span class="status'+(w.trusted?'':' idle')+'">'+(w.trusted?'WORKER READY':'UNTRUSTED')+'</span></div>'+
    '<div class="lifecycle">DISCOVERED<span class="lc-arrow">→</span>UNTRUSTED<span class="lc-arrow">→</span>APPROVED<span class="lc-arrow">→</span>CONNECTED<span class="lc-arrow">→</span><span class="lc-step'+(w.trusted?' on':'')+'">WORKER READY</span></div>'+
    '<div style="display:grid;grid-template-columns:repeat(4,1fr);gap:8px;margin-top:10px">'+
    '<div class="hint">CPU<br><b class="mono">'+esc(w.cpu_cores??'—')+' cores</b></div>'+
    '<div class="hint">RAM<br><b class="mono">'+fmtBytes((w.ram_mb||0)*1048576)+'</b></div>'+
    '<div class="hint">VRAM<br><b class="mono">'+(w.gpu_vram_mb?fmtBytes(w.gpu_vram_mb*1048576):'—')+'</b></div>'+
    '<div class="hint">LOAD<br><b class="mono">'+esc(w.load_percent??0)+'%</b></div></div>'+
    '<div class="hint" style="margin-top:8px">MODEL: '+esc((w.available_models||[]).map(m=>m.file_name||m).join(', ')||'none')+'</div>'+
    '<div class="hint">last seen '+esc(w.last_seen_secs)+'s ago · errors '+esc(w.connection_errors||0)+' · in-flight '+esc(w.in_flight||0)+'</div>'+
    '</div>').join('') : '<div class="empty">No workers.</div>';
}

/* ---- Chat ---- */
const chat = $('chat'); let history = [];
function addMessage(role, text) { if (chat.querySelector('.empty')) chat.innerHTML = ''; const el = document.createElement('div'); el.className = 'msg '+role; el.textContent = text; chat.appendChild(el); chat.scrollTop = chat.scrollHeight; return el; }
async function readSse(response, node) { const reader = response.body.getReader(), decoder = new TextDecoder(); let buffer = '', output = ''; for (;;) { const {done,value} = await reader.read(); if (done) break; buffer += decoder.decode(value,{stream:true}); const lines = buffer.split('\n'); buffer = lines.pop() || ''; for (const line of lines) { if (!line.startsWith('data:')) continue; const data = line.slice(5).trim(); if (data === '[DONE]') continue; try { const event = JSON.parse(data), delta = event.choices?.[0]?.delta?.content; if (delta) { output += delta; node.textContent = output; chat.scrollTop = chat.scrollHeight; } } catch (_) {} } } return output; }
$('send').addEventListener('click', async () => { const prompt = $('prompt').value.trim(); if (!prompt) return; $('prompt').value = ''; addMessage('user',prompt); history.push({role:'user',content:prompt}); const streaming = $('stream').checked, model = $('chat-model').value || lastStatus?.model || 'auto'; $('chat-status').textContent = 'Generating…'; try { const response = await fetch('/v1/chat/completions',{method:'POST',headers:{'Content-Type':'application/json',...auth()},body:JSON.stringify({model,messages:history,stream:streaming})}); let answer = ''; if (streaming && response.ok && response.body) { answer = await readSse(response,addMessage('assistant','')); } else { const body = await response.json(); answer = body.choices?.[0]?.message?.content || body.error?.message || 'No response'; addMessage('assistant',answer); } history.push({role:'assistant',content:answer}); history = history.slice(-24); $('chat-status').textContent = response.ok ? 'Done' : 'Request failed'; } catch (error) { addMessage('assistant','Request failed: '+error); $('chat-status').textContent = 'Request failed'; } });
"##;
