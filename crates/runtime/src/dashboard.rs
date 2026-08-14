//! The Command Deck — the node's single control-plane UI.
//!
//! One embedded HTML/JS surface served by the node from `crates/runtime` (no
//! separate frontend build, single-binary constraint). Every number rendered
//! here is **real runtime state**: the page reads `/status`, `/v1/peers`,
//! `/v1/compute`, `/v1/network` and `/v1/execution` (operator/admin views) and
//! the master-gated admin JSON endpoints. It never calls the proxied inference
//! endpoints on its own, so watching the page neither inflates the request
//! counter nor resets the idle-unload clock.
//!
//! Open WebUI remains the primary Chat; the deck keeps a compact quick-chat
//! (streamed `/v1/chat/completions`) so the control plane can also talk to the
//! node directly. The dashboard is the technical control plane; it is not
//! replaced by Open WebUI.
//!
//! # Views (all from real state, never mocks)
//!
//! - **Overview** — model, inference metrics (requests/tokens/latency/success
//!   rate/tok/s/uptime/idle), quick chat, fair queue, recent calls, system.
//! - **Fabric** — live topology: the local node at the center, every advertised
//!   worker on a ring, edges colored by the **measured** M19 RTT, node rings
//!   colored by real worker health, load arcs from the compute registry.
//! - **Decisions** — the M23 autonomous execution decisions: workload class,
//!   priority, every candidate with its constraint breaches + score breakdown,
//!   the selected worker, expected mode, network cost, KV affinity, reservation,
//!   outcome and the full lifecycle trace.
//! - **Execution** — the correlated `ExecutedPlan` rows (plan/reservation/
//!   worker/RTT/KV-headroom/outcome/reasoning) from `/v1/execution`.
//! - **Workers** — the compute registry with load/queue/tok-s/latency/RAM/VRAM,
//!   in-flight and reserved capacity, trust state and master-gated
//!   Approve/Revoke.
//! - **Network** — measured per-peer links (RTT, bandwidth, transfer cost,
//!   locality), connected peers and the reputation store (`/v1/peers`).
//! - **Models** — served models (engine, context, RAM/VRAM, active) + the local
//!   registry index.
//! - **Observability** — latency/tok-s sparklines from real recent requests,
//!   p50/p95/p99, success rate, lifetime totals, Prometheus endpoint note.
//! - **Recovery** — M24 resilience signals: engine auto-restarts, KV sessions,
//!   offline/stale workers, connection state.
//! - **Diagnostics** — health checks (node, engine, P2P, workers, sessions)
//!   + recent audit events.
//! - **Security** — full audit events + token admin (create/list/revoke,
//!   tier + role) + worker trust actions.
//! - **Settings** — node config, resources, generation defaults and tier
//!   policies surfaced from `/status`.

/// The Command Deck HTML shell. All dynamic data is fetched by the module JS;
/// the shell itself contains no node data (invariant: watching the page never
/// touches the inference backend). `/*__JS__*/` and `__API_PORT__` are filled
/// by `api.rs` at serve time.
pub const DASHBOARD_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>DecentraAI dashboard</title>
<style>
:root{
  --bg:#070a10; --bg-2:#0b0f17; --panel:#10161f; --panel-2:#0d131c;
  --line:#1c2634; --line-2:#273244;
  --text:#e6edf3; --muted:#8fa0b3; --faint:#5c6c80;
  --accent:#22d3ee; --accent-2:#6366f1; --accent-soft:rgba(34,211,238,.12);
  --ok:#34d399; --warn:#fbbf24; --bad:#f87171;
  --mono:ui-monospace,"SF Mono",SFMono-Regular,Menlo,Consolas,monospace;
  --sans:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Inter,Helvetica,Arial,sans-serif;
  --radius:14px; --radius-sm:9px;
  --shadow:0 10px 30px rgba(0,0,0,.35);
}
*{box-sizing:border-box;margin:0;padding:0}
html,body{height:100%}
body{background:radial-gradient(1200px 800px at 80% -10%,#101b2e 0%,var(--bg) 45%);color:var(--text);font:14px/1.55 var(--sans);-webkit-font-smoothing:antialiased}
a{color:var(--accent);text-decoration:none}
code{font-family:var(--mono);font-size:12px;background:rgba(255,255,255,.045);border:1px solid var(--line);border-radius:6px;padding:1px 6px}
.mono{font-family:var(--mono)}
button{font:inherit;color:inherit;background:var(--panel);border:1px solid var(--line-2);border-radius:var(--radius-sm);padding:7px 12px;cursor:pointer;transition:border-color .15s,background .15s,transform .1s}
button:hover{border-color:var(--accent);background:var(--accent-soft)}
button:active{transform:translateY(1px)}
button:disabled{opacity:.45;cursor:not-allowed}
button.primary{background:linear-gradient(135deg,var(--accent),var(--accent-2));border:0;color:#04121a;font-weight:600}
button.primary:hover{filter:brightness(1.1)}
button.danger{border-color:rgba(248,113,113,.5);color:#fda4af}
button.danger:hover{background:rgba(248,113,113,.12)}
input,select,textarea{font:inherit;color:var(--text);background:var(--bg-2);border:1px solid var(--line-2);border-radius:var(--radius-sm);padding:8px 10px;outline:none}
input:focus,select:focus,textarea:focus{border-color:var(--accent)}
.layout{display:grid;grid-template-columns:224px 1fr;min-height:100vh}
/* ---------- sidebar ---------- */
.rail{position:sticky;top:0;height:100vh;display:flex;flex-direction:column;background:rgba(11,15,23,.85);border-right:1px solid var(--line);padding:18px 12px;gap:4px;backdrop-filter:blur(10px)}
.brand{display:flex;align-items:center;gap:10px;padding:4px 8px 16px}
.brand-mark{width:30px;height:30px;border-radius:9px;background:linear-gradient(135deg,var(--accent),var(--accent-2));display:grid;place-items:center;font-weight:800;color:#04121a;font-size:15px}
.brand-name{font-weight:700;letter-spacing:.02em}
.brand-sub{font-size:11px;color:var(--faint);text-transform:uppercase;letter-spacing:.14em}
.rail-label{font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.12em;padding:14px 10px 4px}
.nav-item{display:flex;align-items:center;gap:10px;width:100%;text-align:left;background:transparent;border:1px solid transparent;color:var(--muted);padding:8px 10px;border-radius:10px;font-size:13px}
.nav-item:hover{background:rgba(255,255,255,.04);color:var(--text)}
.nav-item.active{background:var(--accent-soft);border-color:rgba(34,211,238,.28);color:var(--accent);font-weight:600}
.nav-item .ic{width:16px;text-align:center;font-size:13px;opacity:.85}
.rail-foot{margin-top:auto;display:flex;flex-direction:column;gap:8px;padding-top:12px;border-top:1px solid var(--line)}
.rail-live{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--muted);padding:2px 8px}
/* ---------- main ---------- */
.main{padding:22px 26px 60px;max-width:1320px;width:100%}
.topbar{display:flex;align-items:center;justify-content:space-between;gap:16px;margin-bottom:18px}
.topbar h1{font-size:19px;font-weight:700;letter-spacing:-.01em}
.topbar .crumb{color:var(--faint);font-size:12px}
.top-right{display:flex;align-items:center;gap:10px}
.live-pill{display:inline-flex;align-items:center;gap:7px;background:var(--panel);border:1px solid var(--line-2);border-radius:999px;padding:5px 12px;font-size:12px;color:var(--muted)}
.dot{width:8px;height:8px;border-radius:50%;display:inline-block}
.dot.ok{background:var(--ok);box-shadow:0 0 8px var(--ok)}
.dot.warn{background:var(--warn);box-shadow:0 0 8px var(--warn)}
.dot.bad{background:var(--bad);box-shadow:0 0 8px var(--bad)}
.dot.off{background:var(--faint)}
.pulse{animation:pulse 2s infinite}
@keyframes pulse{0%,100%{opacity:1}50%{opacity:.35}}
.view{display:none;animation:fade .25s ease}
.view.active{display:block}
@keyframes fade{from{opacity:0;transform:translateY(4px)}to{opacity:1;transform:none}}
.grid{display:grid;gap:14px}
.grid.cols-2{grid-template-columns:repeat(auto-fit,minmax(320px,1fr))}
.grid.cols-3{grid-template-columns:repeat(auto-fit,minmax(230px,1fr))}
.card{background:linear-gradient(180deg,var(--panel),var(--panel-2));border:1px solid var(--line);border-radius:var(--radius);padding:16px 18px;box-shadow:var(--shadow)}
.card h2{font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.14em;color:var(--faint);margin-bottom:12px;display:flex;align-items:center;gap:8px}
.card h2 .count{font-size:11px;color:var(--accent)}
.metric{background:var(--bg-2);border:1px solid var(--line);border-radius:12px;padding:12px 14px;min-width:0}
.metric .label{font-size:10.5px;text-transform:uppercase;letter-spacing:.1em;color:var(--faint);margin-bottom:4px}
.metric .value{font-family:var(--mono);font-size:22px;font-weight:600;letter-spacing:-.02em;line-height:1.2;white-space:nowrap}
.metric .sub{font-size:11.5px;color:var(--muted);margin-top:2px;font-family:var(--mono)}
.metric.ok .value{color:var(--ok)} .metric.warn .value{color:var(--warn)} .metric.bad .value{color:var(--bad)} .metric.accent .value{color:var(--accent)}
table{width:100%;border-collapse:collapse;font-size:12.5px}
th{font-size:10.5px;text-transform:uppercase;letter-spacing:.09em;color:var(--faint);text-align:left;padding:6px 8px;border-bottom:1px solid var(--line)}
td{padding:7px 8px;border-bottom:1px solid rgba(28,38,52,.6);vertical-align:top}
tr:last-child td{border-bottom:0}
td.num,th.num{text-align:right;font-family:var(--mono)}
.badge{display:inline-flex;align-items:center;gap:5px;border-radius:999px;padding:2px 9px;font-size:11px;font-weight:600;white-space:nowrap}
.badge.ok{background:rgba(52,211,153,.12);color:var(--ok)}
.badge.warn{background:rgba(251,191,36,.12);color:var(--warn)}
.badge.bad{background:rgba(248,113,113,.12);color:var(--bad)}
.badge.accent{background:var(--accent-soft);color:var(--accent)}
.badge.faint{background:rgba(255,255,255,.05);color:var(--muted)}
.bar{height:5px;background:var(--bg-2);border-radius:99px;overflow:hidden;margin-top:5px}
.bar>i{display:block;height:100%;border-radius:99px;background:linear-gradient(90deg,var(--accent),var(--accent-2));transition:width .5s ease}
.bar.warn>i{background:linear-gradient(90deg,#fbbf24,#f59e0b)}
.bar.bad>i{background:linear-gradient(90deg,#f87171,#ef4444)}
.score-bars{display:grid;grid-template-columns:repeat(4,1fr);gap:6px;margin-top:8px}
.score-bars .sb{font-size:10px;color:var(--faint)}
.score-bars .sb b{font-family:var(--mono);color:var(--text);font-weight:600}
.score-bars .bar{height:3px;margin-top:2px}
/* chat */
.chat-box{display:flex;flex-direction:column;height:300px}
#chat-history{flex:1;overflow-y:auto;background:var(--bg-2);border:1px solid var(--line);border-radius:12px;padding:12px;display:flex;flex-direction:column;gap:8px;margin-bottom:10px}
.chat-msg{max-width:85%;padding:8px 12px;border-radius:12px;font-size:13px;white-space:pre-wrap;word-break:break-word}
.chat-msg.user{align-self:flex-end;background:linear-gradient(135deg,rgba(34,211,238,.16),rgba(99,102,241,.16));border:1px solid rgba(34,211,238,.25)}
.chat-msg.node{align-self:flex-start;background:var(--panel);border:1px solid var(--line)}
.chat-msg .who{font-size:10px;text-transform:uppercase;letter-spacing:.1em;color:var(--faint);margin-bottom:3px}
.chat-controls{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
.chat-controls textarea{flex:1;min-width:200px;resize:none}
.chat-status{font-size:11.5px;color:var(--muted);font-family:var(--mono)}
/* topology */
.topo-wrap{background:var(--bg-2);border:1px solid var(--line);border-radius:12px;overflow:hidden}
.topo-wrap svg{display:block;width:100%;height:auto}
.topo-legend{display:flex;gap:16px;flex-wrap:wrap;padding:10px 14px;border-top:1px solid var(--line);font-size:11.5px;color:var(--muted)}
.topo-legend span{display:inline-flex;align-items:center;gap:6px}
/* timeline */
.trace{list-style:none;display:flex;flex-direction:column;gap:0;padding-left:0}
.trace li{display:flex;gap:10px;position:relative;padding-bottom:10px}
.trace li::before{content:"";position:absolute;left:5px;top:16px;bottom:0;width:1px;background:var(--line)}
.trace li:last-child::before{display:none}
.trace .tk{width:11px;height:11px;border-radius:50%;background:var(--accent);flex-shrink:0;margin-top:3px;box-shadow:0 0 6px var(--accent)}
.trace .tk.warn{background:var(--warn);box-shadow:0 0 6px var(--warn)}
.trace .tk.bad{background:var(--bad);box-shadow:0 0 6px var(--bad)}
.trace .tk.off{background:var(--faint);box-shadow:none}
.trace .tl{font-size:12.5px;color:var(--muted)}
.trace .tl b{color:var(--text);font-weight:600}
.trace .tl code{font-size:11px}
/* decision card */
.decision{border-left:3px solid var(--accent)}
.decision.failed{border-left-color:var(--bad)}
.decision.succeeded{border-left-color:var(--ok)}
.decision-head{display:flex;gap:8px;align-items:center;flex-wrap:wrap;margin-bottom:8px}
.cand{background:var(--bg-2);border:1px solid var(--line);border-radius:10px;padding:10px 12px;margin-top:8px}
.cand.breached{opacity:.65}
.breach{display:inline-block;font-size:10px;color:var(--bad);border:1px solid rgba(248,113,113,.4);border-radius:6px;padding:1px 6px;margin:2px 4px 0 0}
/* notifications */
#toast{position:fixed;right:18px;bottom:18px;z-index:50;display:flex;flex-direction:column;gap:8px}
.toast{background:var(--panel);border:1px solid var(--line-2);border-left:3px solid var(--accent);border-radius:10px;padding:10px 14px;font-size:13px;box-shadow:var(--shadow);animation:fade .2s ease;max-width:340px}
.toast.bad{border-left-color:var(--bad)}
/* command palette */
#palette{position:fixed;inset:0;background:rgba(4,7,12,.7);backdrop-filter:blur(6px);display:none;align-items:flex-start;justify-content:center;z-index:100;padding-top:14vh}
#palette.open{display:flex}
.pal-box{width:min(560px,92vw);background:var(--panel);border:1px solid var(--line-2);border-radius:16px;box-shadow:0 24px 70px rgba(0,0,0,.6);overflow:hidden}
.pal-input{width:100%;border:0;border-bottom:1px solid var(--line);background:transparent;padding:16px 18px;font-size:15px;border-radius:0}
.pal-list{max-height:380px;overflow-y:auto;padding:6px}
.pal-item{display:flex;gap:10px;align-items:center;width:100%;text-align:left;background:transparent;border:0;border-radius:10px;padding:9px 12px;font-size:13.5px}
.pal-item:hover,.pal-item.sel{background:var(--accent-soft)}
.pal-item .k{font-family:var(--mono);font-size:11px;color:var(--faint);width:16px;text-align:center}
.pal-item .d{margin-left:auto;font-size:11px;color:var(--faint)}
.pal-empty{padding:20px;color:var(--faint);text-align:center;font-size:13px}
kbd{font-family:var(--mono);font-size:11px;background:var(--bg-2);border:1px solid var(--line-2);border-radius:5px;padding:1px 6px;color:var(--muted)}
.empty{color:var(--faint);font-size:12.5px;padding:10px 4px}
.two-col{display:grid;grid-template-columns:1fr 1fr;gap:14px}
@media(max-width:900px){.two-col{grid-template-columns:1fr}.layout{grid-template-columns:64px 1fr}.rail{padding:14px 8px}.brand-name,.brand-sub,.rail-label,.nav-item span:not(.ic),.rail-live span:last-child{display:none}.rail-live{justify-content:center}}
</style>
</head>
<body>
<div class="layout">
  <!-- ======================= RAIL ======================= -->
  <aside class="rail">
    <div class="brand"><div class="brand-mark">◆</div><div><div class="brand-name">DecentraAI</div><div class="brand-sub">command deck</div></div></div>

    <div class="rail-label">Navigate</div>
    <button class="nav-item" data-view="overview"><span class="ic">◉</span><span>Overview</span></button>
    <button class="nav-item" data-view="chat"><span class="ic">✎</span><span>Chat</span></button>

    <div class="rail-label">Fabric</div>
    <button class="nav-item" data-view="fabric"><span class="ic">◈</span><span>Topology</span></button>
    <button class="nav-item" data-view="decisions"><span class="ic">✦</span><span>Decisions</span></button>
    <button class="nav-item" data-view="execution"><span class="ic">⇄</span><span>Execution</span></button>

    <div class="rail-label">Mesh</div>
    <button class="nav-item" data-view="workers"><span class="ic">▤</span><span>Workers</span></button>
    <button class="nav-item" data-view="network"><span class="ic">⬡</span><span>Network</span></button>
    <button class="nav-item" data-view="models"><span class="ic">▦</span><span>Models</span></button>

    <div class="rail-label">Ops</div>
    <button class="nav-item" data-view="observability"><span class="ic">◔</span><span>Observability</span></button>
    <button class="nav-item" data-view="recovery"><span class="ic">↻</span><span>Recovery</span></button>
    <button class="nav-item" data-view="diag"><span class="ic">✚</span><span>Diag</span></button>
    <button class="nav-item" data-view="security"><span class="ic">⚿</span><span>Security</span></button>
    <button class="nav-item" data-view="settings"><span class="ic">⚙</span><span>Settings</span></button>

    <div class="rail-foot">
      <button id="adv-toggle" title="Show or hide the advanced fabric views">Show advanced</button>
      <div class="rail-live"><span class="dot off" id="rail-dot"></span><span id="rail-live">connecting…</span></div>
    </div>
  </aside>

  <!-- ======================= MAIN ======================= -->
  <main class="main">
    <div class="topbar">
      <div><div class="crumb">DecentraAI · control plane</div><h1 id="page-title">Overview</h1></div>
      <div class="top-right">
        <span class="live-pill"><span class="dot ok pulse" id="live-dot"></span><span id="live-text">model loaded</span></span>
        <button id="palette-open" title="Command palette (Ctrl+K)">⌘K</button>
      </div>
    </div>

    <!-- ================= OVERVIEW (normal user) ================= -->
    <section class="view active" id="view-overview">
      <div class="grid cols-3">
        <div class="card">
          <h2>Model</h2>
          <div class="metric accent" style="margin-bottom:8px"><div class="label">Active model</div><div class="value" id="model-name" style="font-size:17px">&hellip;</div></div>
          <div class="metric"><div class="label">File</div><div class="value" id="model-size" style="font-size:15px">&mdash;</div></div>
          <div class="metric" style="margin-top:8px"><div class="label">Status</div><div class="value" id="model-status" style="font-size:15px">loading&hellip;</div></div>
          <div style="margin-top:10px;font-size:12px;color:var(--muted)" id="also-models"></div>
        </div>
        <div class="card">
          <h2>Inference</h2>
          <div class="grid" style="grid-template-columns:1fr 1fr;gap:8px">
            <div class="metric"><div class="label">Requests</div><div class="value" id="requests">0</div></div>
            <div class="metric"><div class="label">Tokens generated</div><div class="value" id="tokens">0</div></div>
            <div class="metric"><div class="label">Latency p50</div><div class="value" id="latency">&mdash;</div><div class="sub" id="latency-sub"></div></div>
            <div class="metric"><div class="label">Success rate</div><div class="value" id="successrate">&mdash;</div></div>
            <div class="metric"><div class="label">Last speed</div><div class="value" id="toksec">&mdash;</div></div>
            <div class="metric"><div class="label">Uptime</div><div class="value" id="uptime">&mdash;</div><div class="sub" id="idle">idle &mdash;</div></div>
          </div>
          <div style="margin-top:12px;font-size:12px;color:var(--muted)">Backend <code id="backend">&mdash;</code><br>API <code>http://127.0.0.1:__API_PORT__/v1</code> <span class="mono">(OpenAI-compatible)</span></div>
        </div>
        <div class="card">
          <h2>Queue</h2>
          <table><tbody>
            <tr><td>Serving now</td><td class="num" id="queue-serving"><span class="badge faint">idle</span></td></tr>
            <tr><td>Waiting</td><td class="num" id="queue-waiting"><span class="badge faint">nobody</span></td></tr>
          </tbody></table>
          <div style="margin-top:12px;font-size:12px;color:var(--muted)">System: <span id="ram">&mdash;</span> RAM · <span id="cpu">&mdash;</span> · <span id="gpu">&mdash;</span></div>
        </div>
      </div>

      <div class="card" style="margin-top:14px">
        <h2>Recent inference calls</h2>
        <table><thead><tr><th>Time</th><th>Endpoint</th><th class="num">Prompt tok</th><th class="num">Gen tok</th><th class="num">ms</th><th class="num">tok/s</th></tr></thead>
        <tbody id="recent"><tr><td colspan="6" class="empty">loading&hellip;</td></tr></tbody></table>
      </div>

      <div class="card" style="margin-top:14px">
        <h2>Share a model with another machine</h2>
        <div id="share"></div>
      </div>
    </section>

    <!-- ================= CHAT (quick chat; Open WebUI is primary) ================= -->
    <section class="view" id="view-chat">
      <div class="card" style="max-width:860px">
        <h2>Quick chat <span class="count">primary Chat: Open WebUI</span></h2>
        <div class="chat-box">
          <div id="chat-history"><div class="chat-msg node"><div class="who">node</div>Ask the node something. Streamed from the fabric route path.</div></div>
          <div class="chat-controls">
            <textarea id="chat-input" rows="2" placeholder="Type a message&hellip;"></textarea>
            <button id="chat-send" class="primary">Send</button>
            <button id="chat-stop" class="danger" style="display:none">Stop</button>
            <button id="chat-retry" disabled>Retry</button>
          </div>
          <div class="chat-controls" style="margin-top:8px">
            <select id="chat-model" title="Model for chat" style="min-width:180px"></select>
            <label style="display:flex;align-items:center;gap:6px;font-size:12px;color:var(--muted)"><input id="chat-stream" type="checkbox" checked> stream</label>
            <span class="chat-status" id="chat-status">ready</span>
          </div>
        </div>
      </div>
    </section>

    <!-- ================= ADVANCED (fabric/mesh/ops — hidden until toggled) ================= -->
    <div id="advanced" hidden>
      <!-- FABRIC / TOPOLOGY -->
      <section class="view" id="view-fabric">
        <div class="card">
          <h2>Live fabric · topology <span class="count" id="topo-count"></span></h2>
          <div class="topo-wrap">
            <svg id="topology" viewBox="0 0 900 520" preserveAspectRatio="xMidYMid meet"></svg>
            <div class="topo-legend">
              <span><span class="dot ok"></span>ready</span>
              <span><span class="dot warn"></span>degraded/busy</span>
              <span><span class="dot bad"></span>unhealthy/offline</span>
              <span><span class="dot accent" style="background:var(--accent)"></span>local node</span>
              <span style="margin-left:auto">edge = measured RTT (M19) · ring = health</span>
            </div>
          </div>
        </div>
        <div class="grid cols-3" style="margin-top:14px">
          <div class="card"><h2>Fabric summary</h2>
            <div class="metric"><div class="label">Local peer</div><div class="value" id="local-peer" style="font-size:13px">&mdash;</div></div>
          </div>
          <div class="card"><h2>Mesh</h2>
            <div class="grid" style="grid-template-columns:1fr 1fr;gap:8px">
              <div class="metric"><div class="label">Workers</div><div class="value" id="fabric-workers">&mdash;</div></div>
              <div class="metric"><div class="label">Connected</div><div class="value" id="fabric-connected">&mdash;</div></div>
              <div class="metric"><div class="label">Links measured</div><div class="value" id="fabric-links">&mdash;</div></div>
              <div class="metric"><div class="label">KV sessions</div><div class="value" id="fabric-sessions">&mdash;</div></div>
            </div>
          </div>
          <div class="card"><h2>Planner state</h2>
            <div class="metric"><div class="label">Last decision</div><div class="value" id="fabric-last" style="font-size:13px">&mdash;</div><div class="sub" id="fabric-last-sub"></div></div>
          </div>
        </div>
      </section>

      <!-- DECISIONS -->
      <section class="view" id="view-decisions">
        <div class="card">
          <h2>Autonomous decisions <span class="count">M23 · real planner traces</span></h2>
          <div id="decisions"></div>
        </div>
      </section>

      <!-- EXECUTION -->
      <section class="view" id="view-execution">
        <div class="card">
          <h2>Execution (planner decisions) <span class="count" id="exec-count"></span></h2>
          <table><thead><tr><th>Req</th><th>Worker</th><th class="num">Score</th><th class="num">Stages</th><th>Cont</th><th class="num">RTT</th><th>KV</th><th>Outcome</th><th>Reasoning</th></tr></thead>
          <tbody id="execution"><tr><td colspan="9" class="empty">no executions yet</td></tr></tbody></table>
        </div>
      </section>

      <!-- WORKERS -->
      <section class="view" id="view-workers">
        <div class="card">
          <h2>Workers (compute registry) <span class="count" id="workers-count"></span></h2>
          <table><thead><tr><th>Worker</th><th>Node</th><th>Status</th><th class="num">Load</th><th class="num">Queue</th><th class="num">tok/s</th><th class="num">Latency</th><th class="num">RAM free</th><th class="num">In-flight</th><th>Trust</th><th>Action</th></tr></thead>
          <tbody id="workers"><tr><td colspan="11" class="empty">no workers yet (compute not attached)</td></tr></tbody></table>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Contributions <span class="count">M17 · real served work</span></h2>
          <table><thead><tr><th>Worker</th><th class="num">CPU</th><th class="num">RAM</th><th class="num">Online</th><th class="num">Verified</th><th class="num">Failed</th><th class="num">Score</th><th>Tier</th><th class="num">Reward</th></tr></thead>
          <tbody id="contributions"><tr><td colspan="9" class="empty">no contribution ledger yet</td></tr></tbody></table>
        </div>
      </section>

      <!-- NETWORK -->
      <section class="view" id="view-network">
        <div class="grid cols-2">
          <div class="card">
            <h2>Measured links <span class="count">M19</span></h2>
            <table><thead><tr><th>Peer</th><th class="num">RTT</th><th class="num">BW</th><th class="num">ms/MiB</th><th>Locality</th></tr></thead>
            <tbody id="network"><tr><td colspan="5" class="empty">no measured links yet</td></tr></tbody></table>
          </div>
          <div class="card">
            <h2>Connected peers</h2>
            <div id="connected" class="empty">no connected peers</div>
          </div>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Tracked peers (reputation)</h2>
          <table><thead><tr><th>Peer</th><th class="num">Verified chunks</th><th class="num">Failed</th><th class="num">Score</th><th>Status</th></tr></thead>
          <tbody id="peers"><tr><td colspan="5" class="empty">no peers tracked yet</td></tr></tbody></table>
        </div>
      </section>

      <!-- MODELS -->
      <section class="view" id="view-models">
        <div class="card">
          <h2>Served models <span class="count" id="models-count"></span></h2>
          <table><thead><tr><th>Model</th><th>Engine</th><th class="num">Context</th><th class="num">RAM</th><th class="num">VRAM</th><th>Active</th></tr></thead>
          <tbody id="models"><tr><td colspan="6" class="empty">no served models advertised</td></tr></tbody></table>
          <div style="margin-top:10px;font-size:12px;color:var(--muted)" id="models-status"></div>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Local registry</h2>
          <table><thead><tr><th>Model</th><th class="num">Size</th></tr></thead>
          <tbody id="registry-models"><tr><td colspan="2" class="empty">no indexed models</td></tr></tbody></table>
        </div>
      </section>

      <!-- OBSERVABILITY -->
      <section class="view" id="view-observability">
        <div class="grid cols-3">
          <div class="card"><h2>Latency sparkline</h2><svg id="spark-latency" viewBox="0 0 300 70" preserveAspectRatio="none" style="width:100%;height:70px"></svg><div class="sub mono" id="obs-lat"></div></div>
          <div class="card"><h2>Tokens / sec sparkline</h2><svg id="spark-tps" viewBox="0 0 300 70" preserveAspectRatio="none" style="width:100%;height:70px"></svg><div class="sub mono" id="obs-tps"></div></div>
          <div class="card"><h2>Lifetime totals</h2>
            <div class="metric"><div class="label">Requests (mesh)</div><div class="value" id="obs-total-req">&mdash;</div></div>
            <div class="metric" style="margin-top:8px"><div class="label">Tokens (mesh)</div><div class="value" id="obs-total-tok">&mdash;</div></div>
            <div class="metric" style="margin-top:8px"><div class="label">Failed</div><div class="value" id="obs-total-fail">&mdash;</div></div>
          </div>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Prometheus</h2>
          <div class="mono" style="font-size:12px;color:var(--muted)">Scrape <code>/metrics</code> on this port for counters &amp; gauges: <code>http://127.0.0.1:__API_PORT__/metrics</code></div>
        </div>
      </section>

      <!-- RECOVERY -->
      <section class="view" id="view-recovery">
        <div class="grid cols-3">
          <div class="card"><h2>Engine</h2>
            <div class="metric"><div class="label">Auto-restarts (M24)</div><div class="value" id="rec-respawns">&mdash;</div></div>
            <div class="metric" style="margin-top:8px"><div class="label">Backend</div><div class="value" id="rec-backend" style="font-size:13px">&mdash;</div></div>
            <div class="metric" style="margin-top:8px"><div class="label">Idle</div><div class="value" id="rec-idle" style="font-size:15px">&mdash;</div></div>
          </div>
          <div class="card"><h2>Sessions</h2>
            <div class="metric"><div class="label">Active KV sessions</div><div class="value" id="rec-sessions">&mdash;</div></div>
            <div class="metric" style="margin-top:8px"><div class="label">Worker health</div><div class="value" id="rec-health" style="font-size:15px">&mdash;</div></div>
          </div>
          <div class="card"><h2>Connectivity</h2>
            <div class="metric"><div class="label">Connected peers</div><div class="value" id="rec-connected">&mdash;</div></div>
            <div class="metric" style="margin-top:8px"><div class="label">Measured links</div><div class="value" id="rec-links">&mdash;</div></div>
            <div class="metric" style="margin-top:8px"><div class="label">Reconnect loop</div><div class="value" id="rec-reconnect" style="font-size:13px">&mdash;</div></div>
          </div>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Resilience events</h2>
          <div id="rec-events" class="empty">no recovery events yet</div>
        </div>
      </section>

      <!-- DIAGNOSTICS -->
      <section class="view" id="view-diag">
        <div class="card">
          <h2>Diagnostics</h2>
          <table><tbody>
            <tr><td>Node health</td><td class="num" id="diag-health">&mdash;</td></tr>
            <tr><td>Engine</td><td class="num" id="diag-engine">&mdash;</td></tr>
            <tr><td>P2P / network</td><td class="num" id="diag-p2p">&mdash;</td></tr>
            <tr><td>Workers</td><td class="num" id="diag-workers">&mdash;</td></tr>
            <tr><td>Engine restarts (recovery)</td><td class="num" id="diag-restarts">&mdash;</td></tr>
            <tr><td>Active sessions (KV)</td><td class="num" id="diag-sessions">&mdash;</td></tr>
          </tbody></table>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Recent security events</h2>
          <table><thead><tr><th>Time</th><th>Event</th><th>Details</th></tr></thead>
          <tbody id="events"><tr><td colspan="3" class="empty">no security events yet</td></tr></tbody></table>
        </div>
      </section>

      <!-- SECURITY / ADMIN -->
      <section class="view" id="view-security">
        <div class="grid cols-2">
          <div class="card">
            <h2>Token admin <span class="count">master-gated</span></h2>
            <div class="grid" style="grid-template-columns:1fr 110px 120px auto;gap:8px;align-items:end">
              <div><div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">Name</div><input id="tok-name" placeholder="alice"></div>
              <div><div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">Tier</div><select id="tok-tier"><option value="1">Guest</option><option value="2">Contributor</option><option value="3">Core</option></select></div>
              <div><div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">Role</div><select id="tok-role"><option value="client">Client</option><option value="operator">Operator</option></select></div>
              <button id="tok-create" class="primary">Create</button>
            </div>
            <div id="tok-result" style="margin-top:10px;font-size:12px"></div>
            <table style="margin-top:12px"><thead><tr><th>Name</th><th>Tier</th><th>Role</th><th>Status</th><th></th></tr></thead>
            <tbody id="tok-list"><tr><td colspan="5" class="empty">loading tokens&hellip;</td></tr></tbody></table>
          </div>
          <div class="card">
            <h2>Security events <span class="count">audit log</span></h2>
            <div id="audit-list" class="empty">loading&hellip;</div>
          </div>
        </div>
      </section>

      <!-- SETTINGS -->
      <section class="view" id="view-settings">
        <div class="grid cols-2">
          <div class="card">
            <h2>Node</h2>
            <table><tbody>
              <tr><td>Node name</td><td class="num" id="set-name">&mdash;</td></tr>
              <tr><td>Dashboard port</td><td class="num" id="set-port">&mdash;</td></tr>
              <tr><td>Discovery</td><td class="num" id="set-discovery">&mdash;</td></tr>
              <tr><td>Trusted workers</td><td class="num" id="set-trust">&mdash;</td></tr>
              <tr><td>Model / engine</td><td class="num" id="set-model">&mdash;</td></tr>
            </tbody></table>
          </div>
          <div class="card">
            <h2>Resources (admission guards)</h2>
            <table><tbody>
              <tr><td>CPU</td><td class="num" id="set-cpu">&mdash;</td></tr>
              <tr><td>RAM</td><td class="num" id="set-ram">&mdash;</td></tr>
              <tr><td>GPU</td><td class="num" id="set-gpu">&mdash;</td></tr>
            </tbody></table>
          </div>
        </div>
        <div class="grid cols-2" style="margin-top:14px">
          <div class="card">
            <h2>Generation defaults</h2>
            <div id="set-generation" class="empty">&mdash;</div>
          </div>
          <div class="card">
            <h2>Tier policies</h2>
            <div id="set-tiers" class="empty">tiers disabled (admin-token-only)</div>
          </div>
        </div>
      </section>
    </div>
  </main>
</div>

<!-- command palette -->
<div id="palette">
  <div class="pal-box">
    <input class="pal-input" id="palette-input" placeholder="Type a command…  (↑↓ navigate, Enter run, Esc close)" autocomplete="off" spellcheck="false">
    <div class="pal-list" id="palette-list"></div>
  </div>
</div>
<div id="toast"></div>
<script type="module">
/*__JS__*/
</script>
</body>
</html>
"##;

/// The Command Deck JavaScript. Pure client-side rendering from the real JSON
/// views; the only mutation of runtime state happens when the user explicitly
/// sends a chat message or performs an admin action. `__SHARE__` and
/// `__MODEL__` are filled at serve time by `api.rs`.
pub const JS_TEMPLATE: &str = r##"
// ---- helpers ---------------------------------------------------------------
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const $ = id => document.getElementById(id);
const fmtUptime = s => { const h = Math.floor(s/3600), m = Math.floor((s%3600)/60); return h>0 ? h+'h '+m+'m' : (m>0 ? m+'m '+(s%60)+'s' : s+'s'); };
const short = (s, n=14) => (s && s.length > n) ? s.slice(0, n)+'…' : (s || '—');
const tstr = ts => ts ? new Date(ts*1000).toLocaleTimeString() : '—';
const fmtMB = mb => mb ? (mb/1024).toFixed(1)+' GiB' : '—';
function toast(msg, bad=false){ const t=document.createElement('div'); t.className='toast'+(bad?' bad':''); t.textContent=msg; $('toast').appendChild(t); setTimeout(()=>t.remove(), 3600); }

// ---- auth ------------------------------------------------------------------
let token = '';
try { token = await (await fetch('/v1/token')).text(); } catch (e) {}
const headers = token ? { 'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json' } : { 'Content-Type': 'application/json' };
const isAdmin = !!token;

// ---- navigation ------------------------------------------------------------
const VIEWS = ['overview','chat','fabric','decisions','execution','workers','network','models','observability','recovery','diag','security','settings'];
const TITLES = { overview:'Overview', chat:'Chat', fabric:'Fabric · Topology', decisions:'Autonomous decisions', execution:'Execution lifecycle', workers:'Workers', network:'Network', models:'Models', observability:'Observability', recovery:'Recovery', diag:'Diagnostics', security:'Security · Admin', settings:'Settings' };
let current = 'overview';
function show(view){
  current = view;
  document.querySelectorAll('.view').forEach(v => v.classList.toggle('active', v.id === 'view-' + view));
  document.querySelectorAll('.nav-item').forEach(b => b.classList.toggle('active', b.dataset.view === view));
  $('page-title').textContent = TITLES[view] || view;
}
document.querySelectorAll('.nav-item').forEach(b => b.addEventListener('click', () => show(b.dataset.view)));
document.addEventListener('keydown', e => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); openPalette(); }
});

// ---- advanced toggle -------------------------------------------------------
const advEl = document.getElementById('advanced');
const advBtn = $('adv-toggle');
const setAdv = show => {
  advEl.hidden = !show;
  advBtn.textContent = show ? 'Hide advanced' : 'Show advanced';
};
setAdv((localStorage.getItem('decentraai.advanced') || '0') === '1');
advBtn.addEventListener('click', () => {
  const show = advEl.hidden;
  try { localStorage.setItem('decentraai.advanced', show ? '1' : '0'); } catch (e) {}
  setAdv(show);
});

// ---- command palette -------------------------------------------------------
const pal = $('palette'), palInput = $('palette-input'), palList = $('palette-list');
let palSel = 0, palCmds = [];
function openPalette(){
  pal.classList.add('open'); palInput.value = ''; renderPalette(''); palInput.focus();
  document.addEventListener('keydown', palKey);
}
function closePalette(){ pal.classList.remove('open'); document.removeEventListener('keydown', palKey); }
function palKey(e){
  if (e.key === 'Escape') closePalette();
  else if (e.key === 'ArrowDown'){ e.preventDefault(); palSel = Math.min(palSel+1, palCmds.length-1); markPal(); }
  else if (e.key === 'ArrowUp'){ e.preventDefault(); palSel = Math.max(palSel-1, 0); markPal(); }
  else if (e.key === 'Enter'){ e.preventDefault(); if (palCmds[palSel]) { const c = palCmds[palSel]; closePalette(); c.run(); } }
}
function markPal(){ palList.querySelectorAll('.pal-item').forEach((el,i) => el.classList.toggle('sel', i===palSel)); }
function renderPalette(q){
  const all = [
    ...VIEWS.map(v => ({ k:'↳', label:TITLES[v], d:'view', run:()=>show(v) })),
    { k:'⇧', label:'Toggle advanced views', d:'action', run:()=>advBtn.click() },
    { k:'↻', label:'Refresh now', d:'action', run:refresh },
    { k:'✎', label:'Quick chat — focus', d:'action', run:()=>{ show('chat'); $('chat-input').focus(); } },
  ];
  palCmds = all.filter(c => !q || (c.label+' '+c.d).toLowerCase().includes(q));
  palSel = 0;
  palList.innerHTML = palCmds.length ? palCmds.map((c,i) =>
    '<button class="pal-item'+(i===0?' sel':'')+'" data-i="'+i+'"><span class="k">'+c.k+'</span><span>'+esc(c.label)+'</span><span class="d">'+c.d+'</span></button>'
  ).join('') : '<div class="pal-empty">no commands</div>';
  palList.querySelectorAll('.pal-item').forEach(el => el.addEventListener('click', () => { const c = palCmds[+el.dataset.i]; if (c) { closePalette(); c.run(); } }));
}
palInput.addEventListener('input', () => renderPalette(palInput.value.trim().toLowerCase()));
$('palette-open').addEventListener('click', openPalette);

// ---- share guide -----------------------------------------------------------
$('share').innerHTML = "__SHARE__";
const activeModel = "__MODEL__";

// ---- chat (quick chat; Open WebUI is the primary Chat) ----------------------
const HIST_KEY = 'decentraai.chat.history';
let hist = [];
try { hist = JSON.parse(localStorage.getItem(HIST_KEY)) || []; } catch (e) {}
const chatbox = $('chat-history'), chatModel = $('chat-model'), chatInput = $('chat-input');
const currentModel = () => chatModel.value || activeModel;
const addMsg = (role, text) => {
  const div = document.createElement('div');
  div.className = 'chat-msg ' + role;
  div.innerHTML = '<div class="who">' + (role === 'user' ? 'you' : 'node') + '</div><div>' + esc(text) + '</div>';
  chatbox.appendChild(div);
  chatbox.scrollTop = chatbox.scrollHeight;
  return div;
};
const saveHist = () => { try { localStorage.setItem(HIST_KEY, JSON.stringify(hist.slice(-24))); } catch (e) {} };
hist.forEach(m => addMsg(m.role === 'assistant' ? 'node' : 'user', m.content || '(empty)'));
const readSse = async (resp) => {
  const reader = resp.body.getReader(), dec = new TextDecoder();
  let buffer = '', text = '', tokens = null;
  const msgNode = addMsg('node', '');
  const bodyEl = msgNode.querySelector(':scope > div:nth-child(2)');
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += dec.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';
      for (const raw of lines) {
        const line = raw.trim();
        if (!line.startsWith('data:')) continue;
        const payload = line.slice(5).trim();
        if (payload === '[DONE]') continue;
        let ev; try { ev = JSON.parse(payload); } catch (e) { continue; }
        const delta = ev.choices && ev.choices[0] && ev.choices[0].delta && ev.choices[0].delta.content;
        if (delta) { text += delta; bodyEl.textContent = text; }
        if (ev.usage) tokens = ev.usage.completion_tokens;
      }
      chatbox.scrollTop = chatbox.scrollHeight;
    }
  } finally { reader.releaseLock(); }
  return { text, tokens };
};
const chatStatus = $('chat-status'), chatStopBtn = $('chat-stop'), chatRetryBtn = $('chat-retry'), chatSendBtn = $('chat-send');
let currentController = null, lastUserPrompt = null;
const setStreamingUI = on => { chatStopBtn.style.display = on ? 'inline-block' : 'none'; chatSendBtn.disabled = on; chatRetryBtn.disabled = on || !lastUserPrompt; };
const sendChat = async (prompt) => {
  prompt = (prompt || '').trim();
  if (!prompt) return;
  lastUserPrompt = prompt; chatInput.value = '';
  addMsg('user', prompt); hist.push({ role: 'user', content: prompt });
  const stream = $('chat-stream').checked;
  const controller = new AbortController();
  currentController = controller; setStreamingUI(true);
  chatStatus.textContent = 'routing & generating…';
  const t0 = performance.now();
  try {
    const body = JSON.stringify({ model: currentModel(), messages: hist, stream });
    const r = await fetch('/v1/chat/completions', { method: 'POST', headers, body, signal: controller.signal });
    let answer = '', tokens = null;
    if (stream && r.ok && r.body) { const out = await readSse(r); answer = out.text; tokens = out.tokens; }
    else {
      const j = await r.json();
      answer = (j && j.choices && j.choices[0] && j.choices[0].message && j.choices[0].message.content) || (j && j.error ? ('error: ' + (j.error.message || '')) : '');
    }
    if (controller.signal.aborted) return;
    addMsg('node', answer || '(empty response)');
    hist.push({ role: 'assistant', content: answer || '' });
    if (hist.length > 24) hist.splice(0, hist.length - 24);
    saveHist();
    const dt = Math.round(performance.now() - t0);
    chatStatus.textContent = (r.ok ? 'done' : 'error') + ' in ' + dt + ' ms' + (tokens != null ? ' · ' + tokens + ' tokens' : '');
  } catch (e) {
    if (controller.signal.aborted) chatStatus.textContent = 'stopped';
    else { addMsg('node', 'request failed: ' + e); chatStatus.textContent = 'failed'; }
  } finally {
    if (currentController === controller) currentController = null;
    setStreamingUI(false);
  }
};
chatSendBtn.addEventListener('click', () => { if (currentController) return; sendChat(chatInput.value); });
chatStopBtn.addEventListener('click', () => { if (currentController) currentController.abort(); });
chatRetryBtn.addEventListener('click', () => { if (currentController || !lastUserPrompt) return; sendChat(lastUserPrompt); });
setStreamingUI(false);
chatInput.addEventListener('keydown', e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); chatSendBtn.click(); } });

// ---- worker trust actions (master-gated) -----------------------------------
const workerAct = async (action, peerId) => {
  if (!peerId || !isAdmin) return;
  const endpoint = action === 'trust' ? '/api/admin/worker/trust' : '/api/admin/worker/revoke';
  try {
    const r = await fetch(endpoint, { method: 'POST', headers, body: JSON.stringify({ peer_id: peerId }) });
    const d = await r.json().catch(() => ({}));
    if (!r.ok) toast((d.error && d.error.message) || 'worker ' + action + ' failed', true);
    else toast('worker ' + action + ' ok');
  } catch (e) { toast('worker ' + action + ' failed: ' + e, true); }
  refresh();
};
window.trustWorker = e => workerAct('trust', e.target.dataset.p);
window.revokeWorker = e => workerAct('revoke', e.target.dataset.p);

// ---- sparklines ------------------------------------------------------------
function spark(id, values, color){
  const svg = $(id); if (!svg) return;
  const v = (values || []).slice(0, 24).reverse();
  const w = 300, h = 70;
  if (!v.length) { svg.innerHTML = ''; return; }
  const max = Math.max(...v, 1);
  const pts = v.map((x,i) => [ (i/(v.length-1))*w, h - 6 - (x/max)*(h-14) ]);
  const line = pts.map(p => p[0].toFixed(1)+','+p[1].toFixed(1)).join(' ');
  const area = '0,'+h+' ' + line + ' '+w+','+h;
  svg.innerHTML = '<defs><linearGradient id="g-'+id+'" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stop-color="'+color+'" stop-opacity=".35"/><stop offset="100%" stop-color="'+color+'" stop-opacity="0"/></linearGradient></defs>'
    + '<polygon points="'+area+'" fill="url(#g-'+id+')"/>'
    + '<polyline points="'+line+'" fill="none" stroke="'+color+'" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>';
}

// ---- topology --------------------------------------------------------------
function renderTopology(c, n){
  const svg = $('topology'); if (!svg) return;
  const W = 900, H = 520, cx = W/2, cy = H/2, R = 200;
  const workers = (c && c.workers) || [];
  const links = (n && n.links) || [];
  const rtt = {}; links.forEach(l => rtt[l.peer] = l.rtt_ms);
  const local = (c && c.local_peer) || (n && n.local_peer) || 'local';
  $('local-peer').textContent = short(local, 24);
  $('fabric-workers').textContent = workers.length;
  $('fabric-connected').textContent = ((n && n.connected) || []).length;
  $('fabric-links').textContent = links.length;
  $('fabric-sessions').textContent = (c && c.sessions) || 0;
  $('topo-count').textContent = workers.length + ' workers · ' + links.length + ' links';

  const colors = { Ready:'#34d399', Busy:'#22d3ee', Degraded:'#fbbf24', Unhealthy:'#f87171', Offline:'#5c6c80' };
  const stateOf = w => (w && w.status) || 'Offline';
  const edgeColor = rtt_ms => rtt_ms === undefined ? 'rgba(140,160,180,.25)' : rtt_ms < 5 ? 'rgba(52,211,153,.7)' : rtt_ms < 25 ? 'rgba(34,211,238,.65)' : rtt_ms < 100 ? 'rgba(251,191,36,.7)' : 'rgba(248,113,113,.75)';

  let s = '<defs><filter id="glow" x="-50%" y="-50%" width="200%" height="200%"><feGaussianBlur stdDeviation="4" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter></defs>';
  // ring guides
  for (let rr = R; rr >= 120; rr -= 40) s += '<circle cx="'+cx+'" cy="'+cy+'" r="'+rr+'" fill="none" stroke="rgba(255,255,255,.04)"/>';
  // edges (measured RTT)
  workers.forEach((w, i) => {
    const ang = -Math.PI/2 + (i / Math.max(workers.length,1)) * Math.PI * 2;
    const x = cx + R*Math.cos(ang), y = cy + R*Math.sin(ang);
    const rtt_ms = rtt[w.peer_id];
    s += '<line x1="'+cx+'" y1="'+cy+'" x2="'+x+'" y2="'+y+'" stroke="'+edgeColor(rtt_ms)+'" stroke-width="'+(rtt_ms ? 2.2 : 1)+'" stroke-dasharray="'+(rtt_ms ? '' : '3 5')+'"/>';
    if (rtt_ms !== undefined) s += '<text x="'+((cx+x)/2)+'" y="'+((cy+y)/2-6)+'" fill="'+edgeColor(rtt_ms)+'" font-size="11" text-anchor="middle" font-family="var(--mono)">'+rtt_ms+'ms</text>';
  });
  // workers
  workers.forEach((w, i) => {
    const ang = -Math.PI/2 + (i / Math.max(workers.length,1)) * Math.PI * 2;
    const x = cx + R*Math.cos(ang), y = cy + R*Math.sin(ang);
    const col = colors[stateOf(w)] || '#5c6c80';
    const load = (w.load_percent || 0) / 100;
    s += '<circle cx="'+x+'" cy="'+y+'" r="26" fill="rgba(0,0,0,.35)"/>';
    s += '<circle cx="'+x+'" cy="'+y+'" r="22" fill="none" stroke="'+col+'" stroke-width="2.5" opacity=".35"/>';
    s += '<circle cx="'+x+'" cy="'+y+'" r="14" fill="'+col+'" opacity="'+(stateOf(w)==='Offline' ? .25 : .9)+'" filter="url(#glow)"/>';
    // load arc
    if (load > 0.02) {
      const circ = 2*Math.PI*22;
      s += '<circle cx="'+x+'" cy="'+y+'" r="22" fill="none" stroke="'+col+'" stroke-width="3" stroke-dasharray="'+(circ*load)+' '+(circ)+'" transform="rotate(-90 '+x+' '+y+')"/>';
    }
    if (w.in_flight > 0) s += '<circle cx="'+x+'" cy="'+y+'" r="26" fill="none" stroke="'+col+'" stroke-width="1.5" stroke-dasharray="4 6" class="pulse"/>';
    const label = (w.node_name || short(w.peer_id, 10));
    s += '<text x="'+x+'" y="'+y+34+'" fill="'+(stateOf(w)==='Offline' ? '#5c6c80' : '#e6edf3')+'" font-size="11.5" text-anchor="middle" font-weight="600">'+esc(label)+'</text>';
    s += '<text x="'+x+'" y="'+y+48+'" fill="'+(w.trusted ? '#34d399' : '#fbbf24')+'" font-size="9.5" text-anchor="middle" font-family="var(--mono)">'+(w.trusted ? 'TRUSTED' : 'UNTRUSTED')+'</text>';
    s += '<text x="'+x+'" y="'+y-28+'" fill="#8fa0b3" font-size="10" text-anchor="middle" font-family="var(--mono)">'+(stateOf(w))+' · '+(w.load_percent||0)+'%</text>';
  });
  // local node
  s += '<circle cx="'+cx+'" cy="'+cy+'" r="44" fill="rgba(34,211,238,.08)"/>';
  s += '<circle cx="'+cx+'" cy="'+cy+'" r="30" fill="none" stroke="#22d3ee" stroke-width="1" stroke-dasharray="3 5" class="pulse"/>';
  s += '<circle cx="'+cx+'" cy="'+cy+'" r="18" fill="#22d3ee" filter="url(#glow)"/>';
  s += '<text x="'+cx+'" y="'+cy+26+'" fill="#8fa0b3" font-size="11" text-anchor="middle">you · this node</text>';
  s += '<text x="'+cx+'" y="'+cy-30+'" fill="#22d3ee" font-size="10.5" text-anchor="middle" font-family="var(--mono)">'+short(local, 16)+'</text>';
  svg.innerHTML = s;
}

// ---- renderers -------------------------------------------------------------
function renderWorkers(c){
  const rows = (c && c.workers || []).map(w => {
    const action = w.trusted
      ? (isAdmin ? '<button data-p="'+w.peer_id+'" onclick="revokeWorker(event)" class="danger">Revoke</button>' : '<span class="badge ok">trusted</span>')
      : (isAdmin ? '<button data-p="'+w.peer_id+'" onclick="trustWorker(event)">Approve</button>' : '<button disabled>Approve</button>');
    const status = w.status || '';
    const badge = status === 'Ready' ? '<span class="badge ok">ready</span>' : status === 'Offline' ? '<span class="badge bad">offline</span>' : status ? '<span class="badge warn">'+esc(status)+'</span>' : '<span class="badge faint">—</span>';
    return '<tr><td><code>'+short(w.peer_id)+'</code></td><td>'+esc(w.node_name || '')+'</td><td>'+badge+'</td>'+
      '<td class="num"><span>'+w.load_percent+'%</span><div class="bar '+(w.load_percent>80?'bad':w.load_percent>60?'warn':'')+'"><i style="width:'+Math.min(w.load_percent||0,100)+'%"></i></div></td>'+
      '<td class="num">'+w.queue_depth+'</td><td class="num">'+w.tokens_per_second+'</td><td class="num">'+w.current_latency_ms+'ms</td>'+
      '<td class="num">'+fmtMB(w.available_ram_mb)+'</td><td class="num">'+w.in_flight+'</td>'+
      '<td>'+(w.trusted ? '<span class="badge ok">yes</span>' : '<span class="badge faint">no</span>')+'</td><td>'+action+'</td></tr>';
  }).join('');
  $('workers').innerHTML = rows || '<tr><td colspan="11" class="empty">no workers yet (compute not attached)</td></tr>';
  $('workers-count').textContent = (c && c.workers || []).length + ' advertised';
  $('diag-workers').innerHTML = ((c && c.workers || []).length) + ' worker(s)';
  $('diag-sessions').innerHTML = (c && c.sessions) + ' KV session(s)';
  $('set-trust').textContent = ((c && c.workers || []).filter(w => w.trusted).length) + ' trusted of ' + (c && c.workers || []).length;
  // contributions
  const crel = (c && c.contributions || []).map(r =>
    '<tr><td>'+esc(r.node_name || short(r.peer_id))+'</td><td class="num">'+r.cpu_cores+'</td><td class="num">'+fmtMB(r.ram_mb)+'</td><td class="num">'+fmtUptime(r.online_seconds)+'</td>'+
    '<td class="num">'+r.verified_requests+'</td><td class="num">'+r.failed_requests+'</td><td class="num">'+r.score.toFixed(2)+'</td>'+
    '<td><span class="badge '+(r.suggested_tier===3?'ok':r.suggested_tier===2?'warn':'faint')+'">T'+r.suggested_tier+'</span></td><td class="num">'+r.reward_tokens+'</td></tr>'
  ).join('');
  $('contributions').innerHTML = crel || '<tr><td colspan="9" class="empty">no contribution ledger yet</td></tr>';
}
function renderNetwork(n){
  const links = (n && n.links || []).map(l =>
    '<tr><td><code>'+short(l.peer)+'</code></td><td class="num">'+l.rtt_ms+' ms</td><td class="num">'+(l.bandwidth_mbps || '—')+'</td><td class="num">'+(l.transfer_ms_per_mib || '—')+'</td><td><span class="badge '+(l.locality==='Lan'?'ok':l.locality==='Remote'?'warn':'accent')+'">'+esc(l.locality||'')+'</span></td></tr>'
  ).join('');
  $('network').innerHTML = links || '<tr><td colspan="5" class="empty">no measured links yet</td></tr>';
  const conn = (n && n.connected || []);
  $('connected').innerHTML = conn.length ? conn.map(p => '<code style="display:inline-block;margin:2px">'+esc(p)+'</code>').join(' ') : 'no connected peers';
  $('diag-p2p').innerHTML = conn.length + ' connected, ' + (n && n.links || []).length + ' measured link(s)';
  $('rec-connected').textContent = conn.length;
  $('rec-links').textContent = (n && n.links || []).length;
}
function renderPeers(p){
  const rows = (p || []).map(peer =>
    '<tr><td><code>'+short(peer.peer_id)+'</code></td><td class="num">'+peer.verified+'</td><td class="num">'+peer.failed+'</td><td class="num">'+peer.score.toFixed(1)+'</td><td>'+(peer.banned ? '<span class="badge bad">banned</span>' : '<span class="badge ok">ok</span>')+'</td></tr>'
  ).join('');
  $('peers').innerHTML = rows || '<tr><td colspan="5" class="empty">no peers tracked yet</td></tr>';
}
function renderExecutions(x){
  const ex = (x && x.executions || []).slice(0, 14);
  const rows = ex.map(e =>
    '<tr><td><code>'+short(e.request_id, 8)+'</code></td><td><code>'+short(e.selected_worker, 10)+'</code></td><td class="num">'+e.score.toFixed(2)+'</td><td class="num">'+e.stages+'</td>'+
    '<td>'+(e.is_continuation ? '<span class="badge accent">cont</span>' : '<span class="badge faint">cold</span>')+'</td><td class="num">'+e.network_rtt_ms+'ms</td>'+
    '<td class="mono">'+esc(e.kv_headroom || '—')+'</td><td>'+outcomeBadge(e.outcome)+'</td><td class="mono" style="font-size:11px">'+esc(e.reasoning || '')+'</td></tr>'
  ).join('');
  $('execution').innerHTML = rows || '<tr><td colspan="9" class="empty">no executions yet</td></tr>';
  $('exec-count').textContent = (x && x.executions || []).length;
  const last = (x && x.decisions || [])[0];
  if (last) { $('fabric-last').textContent = short(last.selected_worker || 'no worker', 22); $('fabric-last-sub').textContent = (last.workload_class || '') + ' · ' + (last.expected_mode || '') + ' · ' + (last.network_cost_ms || 0) + 'ms'; }
  else { $('fabric-last').textContent = '—'; $('fabric-last-sub').textContent = ''; }
}
function outcomeBadge(o){
  if (o === 'succeeded') return '<span class="badge ok">succeeded</span>';
  if (o === 'failed') return '<span class="badge bad">failed</span>';
  return '<span class="badge accent">'+esc(o || 'in flight')+'</span>';
}
function workloadBadge(cls){
  const map = { streaming_chat:'Streaming', continuation:'Continuation', batch:'Batch', completion:'Completion' };
  return '<span class="badge accent">'+esc(map[cls] || cls || 'unknown')+'</span>';
}
function renderDecisions(x){
  const ds = (x && x.decisions || []).slice(0, 10);
  if (!ds.length) { $('decisions').innerHTML = '<div class="empty">no autonomous decisions yet — run a routed request to see the M23 trace</div>'; return; }
  const cards = ds.map(d => {
    const cands = (d.candidates || []).map(c => {
      const breaches = (c.constraints && c.constraints.breaches || []);
      const sc = c.score || {};
      const bars = [['total', sc.total], ['tps', sc.tps], ['lat', sc.latency], ['load', sc.load], ['queue', sc.queue], ['head', sc.headroom], ['net', sc.net], ['kv', sc.kv]]
        .filter(([,v]) => v !== undefined && v !== null)
        .map(([k,v]) => '<div class="sb">'+k+' <b>'+v.toFixed(2)+'</b><div class="bar"><i style="width:'+Math.max(2, Math.min(100, v*100))+'%"></i></div></div>').join('');
      const br = breaches.map(b => '<span class="breach">'+esc(b)+'</span>').join('');
      const sel = d.selected_worker === c.peer_id;
      return '<div class="cand '+(breaches.length?'breached':'')+'">'+
        '<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">'+
          '<code>'+short(c.peer_id, 12)+'</code>'+
          (c.kv_prefix_resident ? '<span class="badge ok">KV resident</span>' : '')+
          '<span class="mono" style="font-size:11px;color:var(--muted)">net '+c.network_cost_ms+'ms · '+esc(c.engine||'')+'</span>'+
          (sel ? '<span class="badge accent" style="margin-left:auto">SELECTED</span>' : '')+
        '</div>'+(br ? '<div>'+br+'</div>' : '')+
        (sc.total !== undefined ? '<div class="score-bars">'+bars+'</div>' : '')+
      '</div>';
    }).join('');
    const trace = (d.trace || []).map(t => {
      const name = (t.event || '').replace(/_/g,' ');
      const cls = t.event === 'failed' ? 'bad' : t.event === 'recovering' || t.event === 'replanning' ? 'warn' : (t.event === 'completed' || t.event === 'released' || t.event === 'reserved' ? 'ok' : '');
      const extra = t.worker ? ' <code>'+short(t.worker,10)+'</code>' : t.workers !== undefined ? ' <code>'+t.workers+'</code>' : '';
      return '<li><span class="tk '+cls+'"></span><span class="tl"><b>'+esc(name)+'</b>'+extra+'</span></li>';
    }).join('');
    return '<div class="card decision '+(d.outcome||'in_flight')+'" style="margin-top:12px">'+
      '<div class="decision-head">'+
        '<code>'+short(d.request_id, 10)+'</code>'+ workloadBadge(d.workload_class)+
        '<span class="badge faint">priority '+d.priority+'</span>'+
        '<span class="badge accent">'+esc(d.expected_mode||'')+'</span>'+
        (d.selected_worker ? '<span class="badge ok">→ '+short(d.selected_worker,12)+'</span>' : '<span class="badge bad">no worker</span>')+
        outcomeBadge(d.outcome)+
        '<span class="mono" style="margin-left:auto;font-size:11px;color:var(--faint)">'+tstr(d.ts)+'</span>'+
      '</div>'+
      '<div class="mono" style="font-size:11.5px;color:var(--muted);margin-bottom:8px">net '+d.network_cost_ms+'ms · kv '+esc(d.kv_affinity||'')+(d.reservation_id ? ' · res '+short(d.reservation_id,8) : '')+'</div>'+
      (d.reasoning ? '<div class="mono" style="font-size:11.5px;color:var(--muted);margin-bottom:8px">'+esc(d.reasoning)+'</div>' : '')+
      '<div>'+cands+'</div>'+
      '<ul class="trace" style="margin-top:10px">'+trace+'</ul>'+
    '</div>';
  }).join('');
  $('decisions').innerHTML = cards;
}
function renderModels(s){
  const served = (s && s.node && s.node.served_models || []);
  const rows = served.map(m =>
    '<tr><td>'+esc(m.name||'')+'</td><td>'+esc((s && s.node && s.node.engine)||'')+'</td><td class="num">'+(m.context_tokens||'—')+'</td><td class="num">'+fmtMB(m.est_ram_mb)+'</td><td class="num">'+(m.est_vram_mb?fmtMB(m.est_vram_mb):'—')+'</td><td>'+(s.model===m.name?'<span class="badge ok">loaded</span>':'<span class="badge faint">-</span>')+'</td></tr>'
  ).join('');
  $('models').innerHTML = rows || '<tr><td colspan="6" class="empty">no served models advertised</td></tr>';
  $('models-count').textContent = served.length;
  $('models-status').innerHTML = 'active model: '+esc(s.model||'')+(s.model_loaded?' · <span class="badge ok">loaded</span>':' · <span class="badge faint">unloaded</span>');
  const reg = (s && s.available_models || []);
  $('registry-models').innerHTML = reg.map(m =>
    '<tr><td>'+esc(m.name)+'</td><td class="num">'+(m.size_bytes/1073741824).toFixed(2)+' GiB</td></tr>'
  ).join('') || '<tr><td colspan="2" class="empty">no indexed models</td></tr>';
}
function renderObservability(s, c){
  const lat = (s && s.recent_requests || []).map(r => r.duration_ms);
  const tps = (s && s.recent_requests || []).map(r => r.tokens_per_second);
  spark('spark-latency', lat, '#22d3ee');
  spark('spark-tps', tps, '#6366f1');
  const lm = (s && s.latency_ms) || {};
  $('obs-lat').textContent = 'p50 '+lm.p50+'ms · p95 '+lm.p95+'ms · p99 '+lm.p99+'ms · last '+((lat[0]||0))+'ms';
  $('obs-tps').textContent = 'last '+(tps[0]||0).toFixed(1)+' tok/s';
  const t = (c && c.totals) || {};
  $('obs-total-req').textContent = t.requests_completed || 0;
  $('obs-total-tok').textContent = t.tokens_total || 0;
  $('obs-total-fail').textContent = t.requests_failed || 0;
}
function renderRecovery(s, c, x){
  const respawns = (s && s.engine_respawns) || 0;
  $('rec-respawns').innerHTML = respawns > 0 ? '<span class="badge warn">'+respawns+'</span>' : '<span class="badge ok">0</span>';
  $('rec-backend').textContent = short((s && s.backend) || 'none', 30);
  $('rec-idle').textContent = Math.floor((s && s.idle_for_secs || 0)/60) + ' min';
  $('rec-sessions').textContent = (c && c.sessions) || 0;
  const ws = (c && c.workers || []);
  const ok = ws.filter(w => w.status === 'Ready').length;
  const off = ws.filter(w => w.status === 'Offline').length;
  $('rec-health').innerHTML = ws.length ? (ok+' ready · '+off+' offline') : 'no workers';
  $('rec-reconnect').textContent = 'bounded reconnect loop (M24) active';
  $('diag-restarts').innerHTML = respawns > 0 ? '<span class="badge warn">'+respawns+' auto-restart(s)</span>' : '<span class="badge ok">0</span>';
  // resilience events from the audit log
  const evs = (s && s.recent_events || []).filter(e => /restart|recover|evict|offline|reconnect|respawn|release|reservation|health/i.test(e.event || ''));
  $('rec-events').innerHTML = evs.length ? evs.map(e => '<div class="mono" style="font-size:11.5px;margin-bottom:4px"><span style="color:var(--faint)">'+tstr(e.timestamp)+'</span> <b>'+esc(e.event)+'</b> <span style="color:var(--muted)">'+esc(JSON.stringify(e.details||{}))+'</span></div>').join('') : '<div class="empty">no recovery events yet</div>';
}
function renderDiag(s, c, n){
  $('diag-health').innerHTML = s && s.model_loaded ? '<span class="badge ok">node up · model loaded</span>' : '<span class="badge warn">model not loaded</span>';
  $('diag-engine').innerHTML = s && s.backend ? '<code>'+esc(s.backend)+'</code>' : '<span class="badge faint">none</span>';
  $('set-name').textContent = (s && s.node && s.node.name) || esc(s.model || '') || '—';
  $('set-port').textContent = (s && s.api_port) || '—';
  $('set-discovery').textContent = 'mDNS / LAN (auto)';
  $('set-model').textContent = (s ? esc(s.model) : '—') + ' / ' + (s && s.node ? esc(s.node.engine) : '—');
}
function renderSettings(s){
  const r = (s && s.resources) || {};
  $('set-cpu').textContent = (s && s.system && s.system.cpu_threads ? s.system.cpu_threads+' threads' : '—') + ' · reserve '+r.reserve_cpu_cores+' core(s)';
  $('set-ram').textContent = (s && s.system ? Math.round(s.system.ram_total_gib)+' GiB total' : '—') + ' · reserve '+(Math.round((r.reserve_ram_mb||0)/1024))+' GiB';
  $('set-gpu').textContent = (r.gpu_enabled || 'auto') + (r.gpu_max_vram_percent ? ' (vram cap '+r.gpu_max_vram_percent+'%)' : '') + (r.reserve_vram_mb ? ' · reserve '+Math.round((r.reserve_vram_mb||0)/1024)+' GiB' : '');
  const g = (s && s.generation) || {};
  if (g && g.temperature !== undefined) {
    $('set-generation').innerHTML = '<div class="mono" style="font-size:12px;color:var(--muted)">'+
      'temperature <b>'+g.temperature+'</b> · top_p <b>'+g.top_p+'</b> · top_k <b>'+g.top_k+'</b> · repeat_penalty <b>'+g.repeat_penalty+'</b>'+
      (g.system_prompt ? '<div style="margin-top:6px">system prompt: <code>'+esc(g.system_prompt)+'</code></div>' : '')+'</div>';
  } else $('set-generation').innerHTML = '<div class="empty">—</div>';
  const tiers = (s && s.tiers);
  if (tiers) {
    const t = [['T1', tiers.tier1], ['T2', tiers.tier2], ['T3', tiers.tier3]].map(([k,p]) => {
      const pv = p || {};
      return '<div style="margin-bottom:8px"><div style="display:flex;gap:8px;align-items:center"><span class="badge accent">'+k+'</span><span class="mono" style="font-size:12px">'+(pv.rate_limit_per_minute||0)+' req/min</span></div>'+
        '<div class="mono" style="font-size:11px;color:var(--muted)">models: '+(pv.models && pv.models.length ? esc(pv.models.join(', ')) : 'all')+'</div></div>';
    }).join('');
    $('set-tiers').innerHTML = t;
  } else $('set-tiers').innerHTML = '<div class="empty">tiers disabled (admin-token-only)</div>';
}
function renderSecurity(){
  // audit events
  fetch('/api/admin/events', { headers }).then(r => r.json()).then(d => {
    const evs = (d && d.events || []).slice(0, 30);
    const html = evs.map(e => '<div class="mono" style="font-size:11.5px;margin-bottom:5px"><span style="color:var(--faint)">'+tstr(e.timestamp)+'</span> <b>'+esc(e.event||'')+'</b> <span style="color:var(--muted)">'+esc(JSON.stringify(e.details||{}))+'</span></div>').join('');
    $('audit-list').innerHTML = html || '<div class="empty">no security events yet</div>';
  }).catch(() => { $('audit-list').innerHTML = '<div class="empty">master token required</div>'; });
  // tokens
  fetch('/api/admin/token/list', { headers }).then(r => r.json()).then(d => {
    const toks = (d && d.tokens || []);
    const rows = toks.map(t =>
      '<tr><td>'+esc(t.name)+'</td><td><span class="badge '+(t.tier===3?'ok':t.tier===2?'warn':'faint')+'">T'+t.tier+'</span></td><td>'+esc(t.role||'client')+'</td><td>'+(t.revoked ? '<span class="badge bad">revoked</span>' : '<span class="badge ok">active</span>')+'</td>'+
      '<td>'+(isAdmin && !t.revoked ? '<button class="danger" data-n="'+t.name+'" onclick="revokeToken(event)">Revoke</button>' : '')+'</td></tr>'
    ).join('');
    $('tok-list').innerHTML = rows || '<tr><td colspan="5" class="empty">no tokens issued</td></tr>';
  }).catch(() => { $('tok-list').innerHTML = '<tr><td colspan="5" class="empty">master token required</td></tr>'; });
}
window.revokeToken = async e => {
  const name = e.target.dataset.n;
  const r = await fetch('/api/admin/token/revoke', { method:'POST', headers, body: JSON.stringify({ name }) });
  if (r.ok) toast('token revoked'); else { const d = await r.json().catch(()=>({})); toast((d.error&&d.error.message)||'revoke failed', true); }
  renderSecurity();
};
$('tok-create').addEventListener('click', async () => {
  const name = $('tok-name').value.trim(), tier = +$('tok-tier').value, role = $('tok-role').value;
  if (!name) { toast('token name required', true); return; }
  const r = await fetch('/api/admin/token/create', { method:'POST', headers, body: JSON.stringify({ name, tier, role }) });
  const d = await r.json().catch(()=>({}));
  if (r.ok) {
    $('tok-result').innerHTML = '<div class="badge ok" style="margin-bottom:6px">created — copy now, shown once:</div><code style="display:block;word-break:break-all">'+esc(d.token)+'</code>';
    $('tok-name').value = ''; toast('token created');
  } else $('tok-result').innerHTML = '<span class="badge bad">' + esc((d.error&&d.error.message)||'create failed') + '</span>';
  renderSecurity();
});

// ---- main refresh (every 3s, real data only) -------------------------------
async function refresh(){
  let s = null, c = null, n = null, x = null;
  try { s = await (await fetch('/status')).json(); } catch (e) {}
  if (s) {
    $('model-name').textContent = s.model || '—';
    $('model-size').textContent = s.model_size_bytes > 0 ? (s.model_size_bytes/1073741824).toFixed(2)+' GiB' : '—';
    $('model-status').innerHTML = s.model_loaded ? '<span class="badge ok">● loaded</span>' : '<span class="badge warn">○ unloaded (idle timeout)</span>';
    $('live-dot').className = 'dot ' + (s.model_loaded ? 'ok pulse' : 'warn pulse');
    $('rail-dot').className = 'dot ' + (s.model_loaded ? 'ok pulse' : 'warn pulse');
    $('live-text').textContent = s.model_loaded ? 'model loaded' : 'model unloaded';
    $('rail-live').textContent = (s.node && s.node.name) || s.model || 'node';
    const others = (s.available_models||[]).filter(m => m.name !== s.model);
    $('also-models').textContent = others.length ? 'also indexed: '+others.map(m=>esc(m.name)+' ('+(m.size_bytes/1073741824).toFixed(2)+' GiB)').join(', ') : '';
    $('requests').textContent = s.requests_served;
    $('tokens').textContent = s.tokens_generated;
    const lm = s.latency_ms || {};
    $('latency').textContent = (lm.p50 !== undefined && lm.p50 > 0) ? lm.p50+'ms' : '—';
    $('latency-sub').textContent = (lm.p95 !== undefined && lm.p95 > 0) ? 'p95 '+lm.p95+' · p99 '+lm.p99 : '';
    $('successrate').textContent = (s.success_rate_percent !== undefined) ? s.success_rate_percent.toFixed(1)+'%' : '—';
    const last = (s.recent_requests||[])[0];
    $('toksec').textContent = last ? last.tokens_per_second.toFixed(1)+' tok/s' : '—';
    $('uptime').textContent = fmtUptime(s.uptime_secs);
    $('idle').textContent = 'idle '+Math.floor((s.idle_for_secs||0)/60)+' min';
    $('backend').textContent = s.backend;
    const q = s.queue || {};
    $('queue-serving').innerHTML = q.serving
      ? '<span class="badge ok">●</span> <code>'+esc(q.serving.who)+'</code> · '+esc(q.serving.endpoint.replace('/v1/',''))+' ('+q.serving.elapsed_secs+'s)'
      : '<span class="badge faint">idle</span>';
    $('queue-waiting').innerHTML = (q.waiting||[]).length
      ? (q.waiting||[]).map((w,i)=>'#'+(i+1)+' <code>'+esc(w.who)+'</code> ('+w.waited_secs+'s)').join(' · ')
      : '<span class="badge faint">nobody</span>';
    $('recent').innerHTML = (s.recent_requests||[]).map(r =>
      '<tr><td>'+tstr(r.timestamp)+'</td><td><code>'+esc(r.endpoint.replace('/v1/',''))+'</code></td><td class="num">'+r.prompt_tokens+'</td><td class="num">'+r.completion_tokens+'</td><td class="num">'+r.duration_ms+'</td><td class="num">'+r.tokens_per_second.toFixed(1)+'</td></tr>'
    ).join('') || '<tr><td colspan="6" class="empty">no inference calls yet</td></tr>';
    $('ram').textContent = (s.system && s.system.ram_available_gib !== undefined) ? s.system.ram_available_gib.toFixed(1)+' / '+s.system.ram_total_gib.toFixed(1)+' GiB' : '—';
    $('cpu').textContent = (s.system && s.system.cpu_threads) ? s.system.cpu_threads+' threads' : '—';
    $('gpu').innerHTML = (s.system && s.system.gpu) ? esc(s.system.gpu.name)+' · '+s.system.gpu.temperature_c+'°C · '+s.system.gpu.free_vram_mib+' MiB free · '+s.system.gpu.utilization_percent+'%' : '<span class="badge faint">none detected</span>';
    $('events').innerHTML = (s.recent_events||[]).map(e =>
      '<tr><td>'+tstr(e.timestamp)+'</td><td><code>'+esc(e.event)+'</code></td><td class="mono" style="font-size:11px">'+esc(JSON.stringify(e.details||{}))+'</td></tr>'
    ).join('') || '<tr><td colspan="3" class="empty">no security events yet</td></tr>';
    // populate chat model selector once
    if (chatModel.options.length === 0) {
      const names = new Set([activeModel]);
      (s.available_models||[]).forEach(m => { if (m && m.name) names.add(m.name); });
      names.forEach(name => { const opt = document.createElement('option'); opt.value = name; opt.textContent = name; chatModel.appendChild(opt); });
      chatModel.value = activeModel;
    }
    renderModels(s);
    renderDiag(s, null, null);
    renderSettings(s);
    renderObservability(s, null);
    renderRecovery(s, null, null);
  }
  try { const p = await (await fetch('/v1/peers', { headers })).json(); renderPeers(p); } catch (e) {}
  try { c = await (await fetch('/v1/compute', { headers })).json(); renderWorkers(c); renderObservability(s, c); renderRecovery(s, c, null); } catch (e) {}
  try { n = await (await fetch('/v1/network', { headers })).json(); renderNetwork(n); } catch (e) {}
  try { x = await (await fetch('/v1/execution', { headers })).json(); renderExecutions(x); renderDecisions(x); } catch (e) {}
  if (c || n) renderTopology(c, n);
  if (s) renderDiag(s, c, n);
  if (s) renderRecovery(s, c, x);
}
refresh(); setInterval(refresh, 3000);
"##;