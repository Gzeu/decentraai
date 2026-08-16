//! The DecentraAI Command Deck — a living visual control plane for a
//! distributed AI fabric.
//!
//! Design principle: DecentraAI is a **living distributed AI fabric**, not an
//! admin panel. The primary surface (Overview) answers "what is the system
//! doing right now?" visually: a live canvas stage renders the fabric as
//! entities — the local node, every advertised worker, the planner — with real
//! P2P links and visible execution flow. Statistics, tables and metrics stay
//! available but are **secondary**.
//!
//! Everything drawn is derived from **real runtime state** (`/status`,
//! `/v1/peers`, `/v1/compute`, `/v1/network`, `/v1/execution`). Nothing is
//! faked: when idle the stage is calm and atmospheric; when a real request is
//! being planned, reserved and executed, the planner activates, the selected
//! worker lights up, reservations appear and tokens visibly stream. When the
//! M24 recovery machinery reacts to a failure, the affected worker changes
//! state and the replan becomes part of the story.
//!
//! Views (unchanged from the previous Command Deck, functionality preserved):
//! - **Overview** — the living fabric (primary) + M23 decision strip +
//!   secondary metrics (Model, Inference, Queue, Recent, Share).
//! - **Chat** — quick chat (Open WebUI is the primary Chat).
//! - **Topology** — the same fabric engine on a larger stage.
//! - **Decisions / Execution / Workers / Network / Models / Observability /
//!   Recovery / Diag / Security / Settings** — real data, operator-grade.
//!
//! Single-binary constraint: pure embedded HTML/CSS/JS, no external assets,
//! no CDN. The strongest visualization technology compatible with that
//! constraint is the **Canvas 2D** API (with requestAnimationFrame), which is
//! what the fabric stage uses.
//!
//! Invariant: the page only ever polls read-only control endpoints; a chat
//! POST or an admin action happens exclusively on explicit user intent, so
//! watching the page never touches the inference backend, never inflates the
//! request counter and never resets the idle-unload clock.

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
  --bg:#05070d; --bg-2:#0a0e16; --panel:#0d121c; --panel-2:#0a0f18;
  --line:#182234; --line-2:#223048;
  --text:#e8eef6; --muted:#8fa0b3; --faint:#5a6b80;
  --accent:#22d3ee; --accent-2:#6366f1; --accent-soft:rgba(34,211,238,.1);
  --ok:#34d399; --warn:#fbbf24; --bad:#f87171;
  --mono:ui-monospace,"SF Mono",SFMono-Regular,Menlo,Consolas,monospace;
  --sans:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Inter,Helvetica,Arial,sans-serif;
  --radius:14px; --radius-sm:9px;
  --shadow:0 14px 44px rgba(0,0,0,.45);
}
*{box-sizing:border-box;margin:0;padding:0}
html,body{height:100%}
body{background:radial-gradient(1100px 700px at 78% -12%, rgba(34,211,238,.07) 0%, transparent 55%), radial-gradient(900px 620px at -8% 108%, rgba(99,102,241,.06) 0%, transparent 50%), var(--bg);color:var(--text);font:14px/1.55 var(--sans);-webkit-font-smoothing:antialiased}
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
.rail{position:sticky;top:0;height:100vh;display:flex;flex-direction:column;background:rgba(8,12,20,.82);border-right:1px solid var(--line);padding:18px 12px;gap:4px;backdrop-filter:blur(12px)}
.brand{display:flex;align-items:center;gap:10px;padding:4px 8px 16px}
.brand-mark{width:30px;height:30px;border-radius:9px;background:linear-gradient(135deg,var(--accent),var(--accent-2));display:grid;place-items:center;font-weight:800;color:#04121a;font-size:15px;box-shadow:0 0 18px rgba(34,211,238,.35)}
.brand-name{font-weight:700;letter-spacing:.02em}
.brand-sub{font-size:11px;color:var(--faint);text-transform:uppercase;letter-spacing:.14em}
.rail-label{font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.12em;padding:14px 10px 4px}
.nav-item{display:flex;align-items:center;gap:10px;width:100%;text-align:left;background:transparent;border:1px solid transparent;color:var(--muted);padding:8px 10px;border-radius:10px;font-size:13px;transition:background .15s,color .15s}
.nav-item:hover{background:rgba(255,255,255,.04);color:var(--text)}
.nav-item.active{background:var(--accent-soft);border-color:rgba(34,211,238,.28);color:var(--accent);font-weight:600}
.nav-item .ic{width:16px;text-align:center;font-size:13px;opacity:.85}
.rail-foot{margin-top:auto;display:flex;flex-direction:column;gap:8px;padding-top:12px;border-top:1px solid var(--line)}
.rail-live{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--muted);padding:2px 8px}
/* ---------- main ---------- */
.main{padding:22px 26px 60px;max-width:1360px;width:100%}
.topbar{display:flex;align-items:center;justify-content:space-between;gap:16px;margin-bottom:12px}
.topbar h1{font-size:19px;font-weight:700;letter-spacing:-.01em}
.topbar .crumb{color:var(--faint);font-size:12px}
.top-right{display:flex;align-items:center;gap:10px}
.live-pill{display:inline-flex;align-items:center;gap:7px;background:var(--panel);border:1px solid var(--line-2);border-radius:999px;padding:5px 12px;font-size:12px;color:var(--muted)}
.dot{width:8px;height:8px;border-radius:50%;display:inline-block}
.dot.ok{background:var(--ok);box-shadow:0 0 8px var(--ok)}
.dot.warn{background:var(--warn);box-shadow:0 0 8px var(--warn)}
.dot.bad{background:var(--bad);box-shadow:0 0 8px var(--bad)}
.dot.off{background:var(--faint)}
.dot.accent{background:var(--accent);box-shadow:0 0 8px var(--accent)}
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
.badge.remote{background:rgba(139,92,246,.16);color:#c4b5fd}
.badge.local{background:rgba(52,211,153,.10);color:var(--ok)}
#chat-served{margin-left:auto;min-height:20px}
.bar{height:5px;background:var(--bg-2);border-radius:99px;overflow:hidden;margin-top:5px}
.bar>i{display:block;height:100%;border-radius:99px;background:linear-gradient(90deg,var(--accent),var(--accent-2));transition:width .5s ease}
.bar.warn>i{background:linear-gradient(90deg,#fbbf24,#f59e0b)}
.bar.bad>i{background:linear-gradient(90deg,#f87171,#ef4444)}
.score-bars{display:grid;grid-template-columns:repeat(4,1fr);gap:6px;margin-top:8px}
.score-bars .sb{font-size:10px;color:var(--faint)}
.score-bars .sb b{font-family:var(--mono);color:var(--text);font-weight:600}
.score-bars .bar{height:3px;margin-top:2px}
/* ---------- living fabric stage ---------- */
.now-strip{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-bottom:12px;font-size:12.5px}
.now-label{font-size:10px;font-weight:700;letter-spacing:.18em;color:var(--accent);border:1px solid rgba(34,211,238,.3);border-radius:999px;padding:2px 10px;background:var(--accent-soft)}
#now-state{font-family:var(--mono);font-size:12.5px;color:var(--text)}
.planner-chip{display:inline-flex;align-items:center;gap:7px;font-size:11px;font-family:var(--mono);color:var(--muted);border:1px solid var(--line);border-radius:999px;padding:3px 10px;background:rgba(13,18,28,.7)}
.planner-chip b{color:var(--accent);font-weight:600}
.planner-chip .pd{width:7px;height:7px;border-radius:50%;background:var(--faint)}
.planner-chip .pd.on{background:var(--accent);box-shadow:0 0 8px var(--accent)}
.planner-chip .pd.busy{background:var(--warn);box-shadow:0 0 8px var(--warn)}
.planner-chip .pd.fail{background:var(--bad);box-shadow:0 0 8px var(--bad)}
.stage-card{position:relative;background:radial-gradient(820px 460px at 32% -6%, rgba(34,211,238,.07), transparent 62%), linear-gradient(180deg,#0b101a,#060a11);border:1px solid var(--line);border-radius:var(--radius);overflow:hidden;box-shadow:var(--shadow)}
.fabric-stage{display:block;width:100%;height:520px}
.stage-foot{display:flex;align-items:center;justify-content:space-between;gap:10px;padding:9px 14px;border-top:1px solid var(--line);background:rgba(6,10,17,.72);flex-wrap:wrap}
.stage-foot .sf{font-size:11px;color:var(--faint);font-family:var(--mono)}
.stage-foot .sf b{color:var(--muted);font-weight:600}
/* pipeline: USER -> REQUEST -> PLANNER -> RESERVATION -> FABRIC -> WORKER -> ENGINE -> STREAM -> RESULT */
.pipeline{display:flex;align-items:center;gap:2px;flex-wrap:wrap}
.pipe{display:flex;align-items:center;gap:6px;font-size:10px;letter-spacing:.1em;color:var(--faint);padding:4px 8px;border-radius:8px;border:1px solid transparent;transition:color .45s,border-color .45s,background .45s}
.pipe .pi{font-size:11px;opacity:.85}
.pipe.on{color:var(--accent);border-color:rgba(34,211,238,.32);background:var(--accent-soft)}
.pipe.on .pi{opacity:1;text-shadow:0 0 8px rgba(34,211,238,.75)}
.pipe.done{color:var(--ok)}
.pipe.done .pi{color:var(--ok);text-shadow:0 0 8px rgba(52,211,153,.6)}
.pipe.fail{color:var(--bad);border-color:rgba(248,113,113,.4)}
.pipe.fail .pi{color:var(--bad)}
.pipe-arrow{color:var(--faint);font-size:11px;opacity:.4}
/* M23 decision strip — safe operational facts only, no chain-of-thought */
.decision-strip{margin-top:14px;background:linear-gradient(180deg,var(--panel),var(--panel-2));border:1px solid var(--line);border-radius:var(--radius);padding:12px 16px;box-shadow:var(--shadow)}
.ds-head{font-size:10.5px;font-weight:700;text-transform:uppercase;letter-spacing:.14em;color:var(--faint);margin-bottom:10px;display:flex;align-items:center;gap:8px}
.ds-head .count{color:var(--accent);font-family:var(--mono)}
.ds-row{display:flex;align-items:stretch;gap:0;flex-wrap:wrap}
.ds-step{display:flex;flex-direction:column;gap:2px;padding:7px 12px;border:1px solid var(--line);border-left:0;background:rgba(8,12,19,.6);min-width:108px}
.ds-step:first-child{border-left:1px solid var(--line);border-radius:10px 0 0 10px}
.ds-step:last-child{border-radius:0 10px 10px 0}
.ds-step .k{font-size:9.5px;letter-spacing:.13em;color:var(--faint);text-transform:uppercase}
.ds-step .v{font-family:var(--mono);font-size:12px;color:var(--text);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:150px}
.ds-step.on{border-color:rgba(34,211,238,.35);background:var(--accent-soft)}
.ds-step.on .k{color:var(--accent)}
.ds-step.done{border-color:rgba(52,211,153,.3)}
.ds-step.done .k{color:var(--ok)}
.ds-step.fail{border-color:rgba(248,113,113,.4);background:rgba(248,113,113,.06)}
.ds-step.fail .k{color:var(--bad)}
.ds-arrow{align-self:center;color:var(--faint);padding:0 3px;font-size:12px;opacity:.55}
.ds-empty{font-size:12.5px;color:var(--faint);padding:4px 2px}
/* living fabric: per-node identity/resource view (real registry + network) */
.pipe-name{display:block;font-family:var(--mono);font-size:8.5px;color:var(--accent);letter-spacing:0;text-transform:none;margin-top:1px;max-width:130px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.fabric-nodes{margin-top:14px}
.fabric-nodes-wrap{display:flex;gap:10px;flex-wrap:wrap}
.node-chip{flex:1 1 210px;max-width:340px;background:linear-gradient(180deg,var(--panel),var(--panel-2));border:1px solid var(--line);border-radius:var(--radius);padding:10px 12px;position:relative;overflow:hidden;box-shadow:var(--shadow)}
.node-chip.local{border-color:rgba(34,211,238,.45)}
.node-chip .nc-head{display:flex;align-items:center;gap:7px;font-size:12.5px;font-weight:600;color:var(--text);margin-bottom:6px}
.node-chip .nc-head .dot{width:8px;height:8px;border-radius:50%;flex:none}
.node-chip .nc-tag{font-family:var(--mono);font-size:9px;letter-spacing:.1em;padding:1px 6px;border-radius:999px;border:1px solid var(--line);color:var(--muted);text-transform:uppercase}
.node-chip .nc-tag.local-tag{color:var(--accent);border-color:rgba(34,211,238,.4)}
.node-chip .nc-tag.remote-tag{color:var(--ok);border-color:rgba(52,211,153,.35)}
.node-chip .nc-meta{display:flex;gap:4px 10px;flex-wrap:wrap;font-family:var(--mono);font-size:10px;color:var(--faint);margin-bottom:7px}
.node-chip .nc-meta b{color:var(--muted);font-weight:600}
.node-chip .nc-bars{display:flex;flex-direction:column;gap:4px}
.node-chip .nc-bar{display:flex;align-items:center;gap:7px;font-size:9.5px;color:var(--faint);font-family:var(--mono)}
.node-chip .nc-bar .track{flex:1;height:4px;background:rgba(130,150,180,.12);border-radius:2px;overflow:hidden}
.node-chip .nc-bar .track i{display:block;height:100%;border-radius:2px;background:var(--accent)}
.node-chip .nc-bar .track i.warn{background:var(--warn)}
.node-chip .nc-bar .track i.bad{background:var(--bad)}
.node-chip .nc-model{display:inline-block;font-family:var(--mono);font-size:9px;color:var(--muted);border:1px solid var(--line);border-radius:999px;padding:1px 7px;margin:2px 3px 0 0;max-width:170px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.node-chip .nc-trust{display:flex;align-items:center;gap:0;margin-top:8px;flex-wrap:wrap}
.node-chip .nc-trust .tc-step{display:flex;align-items:center;gap:4px;font-size:8.5px;font-family:var(--mono);letter-spacing:.04em;color:var(--faint);text-transform:uppercase;padding:2px 5px;border-radius:6px;border:1px solid var(--line);background:rgba(8,12,19,.6);white-space:nowrap}
.node-chip .nc-trust .tc-step.done{color:var(--ok);border-color:rgba(52,211,153,.3)}
.node-chip .nc-trust .tc-step.cur{color:#0a0f16;background:var(--ok);border-color:var(--ok);font-weight:700}
.node-chip .nc-trust .tc-step.cur.warn{background:var(--warn);border-color:var(--warn)}
.node-chip .nc-trust .tc-arr{color:var(--faint);font-size:9px;opacity:.5;padding:0 1px}
.discovery-feed{margin-top:10px;font-family:var(--mono);font-size:10.5px;color:var(--faint);display:flex;flex-direction:column;gap:3px}
.disc-ev{display:flex;align-items:center;gap:7px;animation:discIn .35s ease}
@keyframes discIn{from{opacity:0;transform:translateY(-3px)}to{opacity:1;transform:none}}
.disc-ev .de-dot{width:6px;height:6px;border-radius:50%;flex:none}
.disc-ev .de-time{color:var(--faint);opacity:.7}
.disc-ev .de-msg b{color:var(--text);font-weight:600}
.disc-ev .de-msg .up{color:var(--ok)}
.disc-ev .de-msg .down{color:var(--bad)}
/* worker cards (Workers view) */
.worker-cards{display:flex;flex-direction:column;gap:10px}
.worker-card{background:linear-gradient(180deg,var(--panel),var(--panel-2));border:1px solid var(--line);border-radius:var(--radius);padding:12px 14px;box-shadow:var(--shadow)}
.worker-card.local{border-color:rgba(34,211,238,.45)}
.worker-card .wc-head{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-bottom:8px}
.worker-card .wc-name{font-size:13.5px;font-weight:700;color:var(--text)}
.worker-card .wc-id{font-family:var(--mono);font-size:10px;color:var(--faint)}
.worker-card .wc-meta{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:4px 14px;font-family:var(--mono);font-size:10.5px;color:var(--faint);margin-bottom:9px}
.worker-card .wc-meta b{color:var(--muted);font-weight:600}
.worker-card .wc-res{display:flex;flex-direction:column;gap:5px;margin-bottom:9px}
.worker-card .wc-res .nc-bar{display:flex;align-items:center;gap:7px;font-size:10px;color:var(--faint);font-family:var(--mono)}
.worker-card .wc-res .track{flex:1;height:5px;background:rgba(130,150,180,.12);border-radius:2px;overflow:hidden}
.worker-card .wc-res .track i{display:block;height:100%;border-radius:2px;background:var(--accent)}
.worker-card .wc-res .track i.warn{background:var(--warn)}
.worker-card .wc-res .track i.bad{background:var(--bad)}
.worker-card .wc-models{display:flex;gap:3px;flex-wrap:wrap;margin-bottom:9px}
.worker-card .wc-trust{margin-top:8px}
.worker-card .wc-actions{margin-top:10px;display:flex;gap:8px;align-items:center}
.worker-card .wc-actions button{font-size:10.5px;padding:4px 11px;border-radius:7px;border:1px solid var(--line);background:rgba(13,18,28,.7);color:var(--accent);cursor:pointer;transition:border-color .2s,color .2s}
.worker-card .wc-actions button:hover{border-color:var(--accent)}
.worker-card .wc-actions button.danger{color:var(--bad)}
.worker-card .wc-actions button.danger:hover{border-color:var(--bad)}
.worker-card .wc-actions .badge{margin-left:auto}
/* secondary metrics */
.secondary{margin-top:14px}
.secondary .card{padding:13px 15px}
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
.topo-wrap canvas{display:block;width:100%;height:560px}
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
@media(max-width:900px){.two-col{grid-template-columns:1fr}.layout{grid-template-columns:64px 1fr}.rail{padding:14px 8px}.brand-name,.brand-sub,.rail-label,.nav-item span:not(.ic),.rail-live span:last-child{display:none}.rail-live{justify-content:center}.fabric-stage{height:360px}.ds-row{flex-direction:column}.ds-step{border-left:1px solid var(--line);border-radius:0}.ds-step:first-child{border-radius:10px 10px 0 0}.ds-step:last-child{border-radius:0 0 10px 10px}}
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

    <!-- ================= OVERVIEW (the living fabric — primary) ================= -->
    <section class="view active" id="view-overview">
      <div class="now-strip">
        <span class="now-label">NOW</span>
        <span id="now-state">connecting to the fabric…</span>
        <span class="planner-chip" id="planner-chip"><span class="pd" id="planner-dot"></span>planner · <b id="planner-state">idle</b></span>
      </div>

      <!-- primary: the living fabric stage -->
      <div class="stage-card">
        <canvas id="fabric-stage" class="fabric-stage"></canvas>
        <div class="stage-foot">
          <div class="pipeline" id="pipeline">
            <div class="pipe" data-stage="user"><span class="pi">◉</span><span>USER</span></div><span class="pipe-arrow">→</span>
            <div class="pipe" data-stage="request"><span class="pi">✉</span><span>REQUEST</span></div><span class="pipe-arrow">→</span>
            <div class="pipe" data-stage="planner"><span class="pi">✦</span><span>PLANNER</span></div><span class="pipe-arrow">→</span>
            <div class="pipe" data-stage="reservation"><span class="pi">⊞</span><span>RESERVATION</span></div><span class="pipe-arrow">→</span>
            <div class="pipe" data-stage="fabric"><span class="pi">◈</span><span>FABRIC</span></div><span class="pipe-arrow">→</span>
            <div class="pipe" data-stage="worker"><span class="pi">▤</span><span>WORKER<span class="pipe-name" id="pipe-worker-name"></span></span></div><span class="pipe-arrow">→</span>
            <div class="pipe" data-stage="engine"><span class="pi">▦</span><span>ENGINE</span></div><span class="pipe-arrow">→</span>
            <div class="pipe" data-stage="stream"><span class="pi">⇄</span><span>STREAM</span></div><span class="pipe-arrow">→</span>
            <div class="pipe" data-stage="result"><span class="pi">✓</span><span>RESULT</span></div>
          </div>
          <div class="sf" id="stage-facts">fabric: — · peers: — · rtt: —</div>
        </div>
      </div>

      <!-- M23 autonomous decision strip — safe operational facts only -->
      <div class="decision-strip">
        <div class="ds-head">Autonomous decision <span class="count" id="ds-count"></span></div>
        <div id="decision-strip" class="ds-empty">no autonomous decision yet — the planner is idle. Send a routed request to watch it plan.</div>
      </div>

      <!-- discovered nodes: identity + resources per node, from /v1/compute + /v1/network -->
      <div class="card fabric-nodes">
        <div class="ds-head">Fabric nodes <span class="count" id="fabric-nodes-count"></span></div>
        <div id="fabric-nodes" class="fabric-nodes-wrap"><span class="badge faint">discovering peers…</span></div>
        <div class="discovery-feed" id="discovery-feed"></div>
      </div>

      <!-- secondary: metrics, queue, recent, share -->
      <div class="grid cols-3 secondary">
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
            <span id="chat-served"></span>
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
            <canvas id="fabric-topo" class="fabric-topo"></canvas>
            <div class="topo-legend">
              <span><span class="dot ok"></span>ready</span>
              <span><span class="dot warn"></span>degraded/busy</span>
              <span><span class="dot bad"></span>unhealthy/offline</span>
              <span><span class="dot accent" style="background:var(--accent)"></span>local node</span>
              <span style="margin-left:auto">edge = measured RTT (M19) · ring = health · flow = live execution</span>
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
        <!-- FABRIC GRAPH / DIGITAL TWIN (Phase C): a read-only projection of
             real fabric state from /v1/fabric. Counts and lists are derived
             from actual advertisements, claims, links and decisions — never
             fabricated. -->
        <div class="card" style="margin-top:14px">
          <h2>Fabric graph · digital twin <span class="count">projection of real state</span></h2>
          <div class="grid cols-4">
            <div class="metric"><div class="label">Nodes</div><div class="value" id="fabric-g-nodes">&mdash;</div></div>
            <div class="metric"><div class="label">Models</div><div class="value" id="fabric-g-models">&mdash;</div></div>
            <div class="metric"><div class="label">Capabilities</div><div class="value" id="fabric-g-caps">&mdash;</div></div>
            <div class="metric"><div class="label">Executions</div><div class="value" id="fabric-g-execs">&mdash;</div></div>
          </div>
          <div id="fabric-graph" class="empty">fabric graph not loaded</div>
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
          <table><thead><tr><th>Req</th><th>Worker</th><th class="num">Score</th><th class="num">Stages</th><th>Cont</th><th class="num">RTT</th><th>KV</th><th>Usage</th><th>Outcome</th><th>Reasoning</th></tr></thead>
          <tbody id="execution"><tr><td colspan="10" class="empty">no executions yet</td></tr></tbody></table>
        </div>
        <!-- SESSIONS (KV locality): real coordinator-tracked KV/session
             residency from /v1/sessions — which worker holds each
             conversation's KV prefix (and why continuations are steered
             there). Empty ledger and UNKNOWN headroom render honestly. -->
        <div class="card" style="margin-top:14px">
          <h2>Sessions (KV locality) <span class="count" id="sessions-count"></span></h2>
          <table><thead><tr><th>Session</th><th>Worker</th><th>Model</th><th class="num">KV tokens used</th><th>KV headroom</th><th>Continue</th></tr></thead>
          <tbody id="sessions"><tr><td colspan="6" class="empty">no active sessions</td></tr></tbody></table>
        </div>
      </section>

      <!-- WORKERS -->
      <section class="view" id="view-workers">
        <div class="card">
          <h2>Workers (compute registry) <span class="count" id="workers-count"></span></h2>
          <div id="workers" class="worker-cards"><div class="empty">no workers yet (compute not attached)</div></div>
        </div>
        <!-- RESOURCE PRESSURE (Part 17/22): honest aggregate of measured
             load across the fabric — the local node's real SystemSnapshot
             plus each worker's advertised availability. Every value is
             MEASURED (system probe / heartbeat); nothing is invented. -->
        <div class="card" style="margin-top:14px">
          <h2>Resource pressure <span class="count">MEASURED</span></h2>
          <div class="grid cols-3">
            <div class="card sub">
              <h3 style="margin:0 0 6px">This node</h3>
              <div id="pressure-local" class="empty">collecting…</div>
            </div>
            <div class="card sub">
              <h3 style="margin:0 0 6px">Fabric aggregate</h3>
              <div id="pressure-fabric" class="empty">no workers yet</div>
            </div>
            <div class="card sub">
              <h3 style="margin:0 0 6px">Busiest worker</h3>
              <div id="pressure-busiest" class="empty">no workers yet</div>
            </div>
          </div>
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
          <h2>CAN I RUN THIS? <span class="count">fabric-wide capability fit</span></h2>
          <div style="margin-top:6px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
            <input id="cir-model" class="mono" placeholder="model (e.g. qwen2.5-7b-instruct-q4_k_m.gguf)" style="min-width:240px;padding:4px 6px;font-size:12px">
            <input id="cir-cap" class="mono" placeholder="capability (e.g. ocr)" style="width:140px;padding:4px 6px;font-size:12px">
            <select id="cir-ev" class="mono" style="font-size:11px;padding:4px 4px">
              <option value="any">evidence: any</option>
              <option value="verified">evidence: verified</option>
            </select>
            <button class="btn small" onclick="canIRun()">check</button>
            <span id="cir-note" class="mono" style="font-size:10.5px;color:var(--faint)"></span>
          </div>
          <div id="cir-result" style="margin-top:10px;font-size:12px;color:var(--muted)"></div>
        </div>
        <!-- DECISION (Phase 3): "What should I run?" — the ONE coherent fabric
             decision from the real /v1/decision projection. Progressive
             disclosure: decision banner + why first, then per-capability model
             options, then historical. Everything is real backend state; empty
             capabilities/options/history render honestly. -->
        <div class="card" id="decision-card" style="margin-top:14px">
          <h2>Decision <span class="count">what should I run? · fabric-wide</span></h2>
          <div style="margin-top:6px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
            <input id="dec-intent" class="mono" placeholder="I need OCR and summarization" style="min-width:260px;padding:4px 6px;font-size:12px">
            <select id="dec-ev" class="mono" style="font-size:11px;padding:4px 4px">
              <option value="any">evidence: any</option>
              <option value="verified">evidence: verified</option>
            </select>
            <button class="btn small" onclick="decideNow()">decide</button>
          </div>
          <div id="dec-result" style="margin-top:10px;font-size:12px;color:var(--muted)"></div>
          <div style="margin-top:10px;border-top:1px dashed var(--line);padding-top:8px">
            <div style="display:flex;gap:6px;align-items:center;flex-wrap:wrap">
              <label style="font-size:11px;color:var(--muted)">execute</label>
              <input id="dec-model" class="mono" placeholder="model (optional, file.gguf)" style="min-width:120px;padding:4px 6px;font-size:12px">
              <input id="dec-session" class="mono" placeholder="session_id (optional — continue an earlier run)" style="min-width:200px;padding:4px 6px;font-size:12px">
              <input id="dec-max" class="mono" type="number" min="1" step="1" value="256" title="max_tokens" style="width:72px;padding:4px 6px;font-size:12px">
              <label style="font-size:11px;color:var(--muted)"><input id="dec-stream" type="checkbox" checked> stream</label>
              <button class="btn small warn" onclick="executeDecision()">Execute (confirm)</button>
              <button class="btn small" onclick="previewDecision()">Preview (dry-run)</button>
            </div>
            <div style="margin-top:6px">
              <textarea id="dec-prompt" class="mono" placeholder="prompt to run on the decided fabric (required)" rows="2" style="width:100%;padding:6px;font-size:12px;resize:vertical;font-family:monospace"></textarea>
            </div>
          </div>
          <div id="dec-exec" style="margin-top:10px;font-size:12px;color:var(--muted)"></div>
          <div id="dec-preview" style="margin-top:10px;font-size:12px;color:var(--muted)"></div>
        </div>
        <!-- Variant comparison: which on-disk variant of a model fits THIS
             fabric best, side-by-side, from the real /v1/can_run variants
             projection. Never invents variants/sizes/fits/node identities. -->
        <div class="card" style="margin-top:14px">
          <h2>Variant comparison <span class="count">which on-disk variant fits THIS fabric best</span></h2>
          <div style="margin-top:6px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
            <input id="vc-model" class="mono" placeholder="model file (e.g. qwen2.5-7b-instruct-q4_k_m.gguf)" style="min-width:240px;padding:4px 6px;font-size:12px">
            <select id="vc-cap" class="mono" style="font-size:11px;padding:4px 4px"></select>
            <button class="btn small" onclick="variantCompare()">compare variants</button>
          </div>
          <div id="variant-compare" style="margin-top:10px;font-size:12px;color:var(--muted)"></div>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Capability overview <span class="count">local, on-disk</span></h2>
          <div id="cap-overview" style="margin-top:6px;font-size:12px;color:var(--muted)"><span class="muted">loading…</span></div>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Served models <span class="count" id="models-count"></span></h2>
          <table><thead><tr><th>Model</th><th>Engine</th><th class="num">Context</th><th class="num">RAM</th><th class="num">VRAM</th><th>Active</th></tr></thead>
          <tbody id="models"><tr><td colspan="6" class="empty">no served models advertised</td></tr></tbody></table>
          <div style="margin-top:10px;font-size:12px;color:var(--muted)" id="models-status"></div>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Local registry</h2>
          <table><thead><tr><th>Model</th><th class="num">Size</th><th></th></tr></thead>
          <tbody id="registry-models"><tr><td colspan="3" class="empty">no indexed models</td></tr></tbody></table>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>On disk across the fabric <span class="count" id="disk-models-count"></span></h2>
          <table><thead><tr><th>Model</th><th>Worker</th><th class="num">Size</th><th>State</th></tr></thead>
          <tbody id="disk-models"><tr><td colspan="4" class="empty">no on-disk models reported by workers</td></tr></tbody></table>
          <div style="margin-top:10px;font-size:12px;color:var(--muted)" id="disk-models-status"></div>
        </div>
        <!-- MODEL HUB (Part 16/22): search HuggingFace and pull a model
             straight into this node's registry, then serve it. The search
             and pull calls are master-gated admin endpoints; the hub never
             runs unattended. -->
        <div class="card" style="margin-top:14px">
          <h2>Model Hub</h2>
          <div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">
            <input id="hub-q" type="text" placeholder="search GGUF models on HuggingFace, e.g. Qwen" style="flex:1;min-width:220px;padding:7px 10px;border:1px solid var(--border);border-radius:6px;background:var(--bg);color:var(--fg)">
            <button class="btn" id="hub-search-btn" onclick="hubSearch()">Search</button>
          </div>
           <div id="hub-status" style="margin-top:8px;font-size:12px;color:var(--muted)">search the Hub to discover models you can pull on this node</div>
           <div style="display:flex;gap:8px;align-items:center;margin-top:10px;flex-wrap:wrap">
             <button class="btn small" id="hub-compare-btn" onclick="hubCompareSelected()" disabled>Compare Selected (0)</button>
             <button class="btn small faint" onclick="hubClearCompare()">Clear Selection</button>
             <span style="font-size:12px;color:var(--muted)" id="hub-compare-status">select 1 or more models to compare side-by-side</span>
           </div>
           <table style="margin-top:8px"><thead><tr><th style="width:30px">Compare</th><th>Model</th><th>Category</th><th class="num">Downloads</th><th></th></tr></thead>
           <tbody id="hub-results"><tr><td colspan="5" class="empty">nothing searched yet</td></tr></tbody></table>

           <!-- Model comparison panel (Model Comparison feature) -->
           <div id="hub-compare-panel" style="display:none;margin-top:14px;border:1px solid var(--border);border-radius:8px;padding:12px">
             <div style="display:flex;justify-content:space-between;align-items:center;flex-wrap:wrap;gap:8px">
               <h3 style="margin:0;font-size:15px">Model Comparison (<span id="compare-panel-count">0</span> models)</h3>
               <button class="btn small" onclick="hubCloseCompare()">Close Comparison</button>
             </div>
             <div style="margin-top:10px"><b style="font-size:12px">Capability fit</b>
               <div style="margin-top:6px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
                 <span class="mono" style="font-size:11px;color:var(--muted)">can these models do</span>
                 <select id="compare-fit-cap" class="mono" style="font-size:11px;padding:2px 4px"></select>
                 <button class="btn small" onclick="hubCompareFit()">check</button>
                 <span id="compare-fit-note" class="mono" style="font-size:10.5px;color:var(--faint)"></span>
               </div>
               <div id="compare-fit" style="margin-top:6px;font-size:12px;color:var(--muted)"></div>
             </div>
             <div id="hub-compare-content" style="overflow-x:auto;margin-top:10px"></div>
           </div>
          <!-- Model card (Issue #26 §7–§8, §22): real Hub metadata + honest
               capability taxonomy with provenance + every GGUF variant with
               size/SHA-256 + live fabric compatibility. Hidden until a model
               is opened; nothing here is fabricated. -->
          <div id="hub-detail" style="display:none;margin-top:12px;border:1px solid var(--border);border-radius:8px;padding:12px">
            <div style="display:flex;justify-content:space-between;align-items:flex-start;gap:8px;flex-wrap:wrap">
              <div>
                <div style="font-weight:600" id="md-title"></div>
                <div class="mono" style="font-size:11px;color:var(--muted)" id="md-meta"></div>
              </div>
              <button class="btn small" onclick="hubCloseDetail()">Close</button>
            </div>
            <div style="margin-top:8px;font-size:12px;color:var(--muted)" id="md-desc"></div>
            <div style="margin-top:10px"><b style="font-size:12px">Capabilities</b>
              <div id="md-caps" style="display:flex;flex-wrap:wrap;gap:6px;margin-top:6px"></div>
            </div>
            <div style="margin-top:10px"><b style="font-size:12px">Tasks</b>
              <div id="md-tasks" style="margin-top:6px;font-size:12px;color:var(--muted)"></div>
            </div>
            <div style="margin-top:10px"><b style="font-size:12px">Capability fit</b>
              <div style="margin-top:6px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
                <span class="mono" style="font-size:11px;color:var(--muted)">can this model do</span>
                <select id="md-fit-cap" class="mono" style="font-size:11px;padding:2px 4px"></select>
                <button class="btn small" onclick="hubCheckFit()">check</button>
                <span id="md-fit-note" class="mono" style="font-size:10.5px;color:var(--faint)"></span>
              </div>
              <div id="md-fit" style="margin-top:6px;font-size:12px;color:var(--muted)"></div>
            </div>
            <div style="margin-top:10px"><b style="font-size:12px">Can I run this? (fabric)</b>
              <div style="margin-top:6px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
                <button class="btn small" onclick="hubCanIRunLocal()">CAN I RUN THIS? (fabric)</button>
                <span class="mono" style="font-size:11px;color:var(--muted)">fabric-wide fit for the selected capability, from local claims (no Hub round-trip)</span>
              </div>
              <div id="md-cir" style="margin-top:6px;font-size:12px;color:var(--muted)"></div>
            </div>
            <div style="margin-top:10px"><b style="font-size:12px">CAN I RUN THIS? — on-disk variants (fabric)</b>
              <div style="margin-top:6px;display:flex;gap:6px;align-items:center;flex-wrap:wrap">
                <span class="mono" style="font-size:11px;color:var(--muted)">on-disk file</span>
                <input id="md-vf-file" type="text" class="mono" placeholder="e.g. qwen2.5-7b-instruct-q4_k_m.gguf" style="font-size:11px;padding:2px 4px;width:280px">
                <select id="md-vf-cap" class="mono" style="font-size:11px;padding:2px 4px"></select>
                <button class="btn small" onclick="loadVariantFit()">Load variant fit</button>
              </div>
              <div class="mono" style="font-size:10.5px;color:var(--faint);margin-top:4px">per-variant fabric fit from the real on-disk GGUF files this fabric shares (matches by file name)</div>
              <div id="md-variant-fit" style="margin-top:6px;font-size:12px;color:var(--muted)"></div>
            </div>
            <div style="margin-top:10px"><b style="font-size:12px">Variants</b>
              <table style="margin-top:6px"><thead><tr><th>File</th><th class="num">Size</th><th>SHA-256</th><th></th></tr></thead>
              <tbody id="md-variants"><tr><td colspan="4" class="empty">no variants reported</td></tr></tbody></table>
            </div>
            <div style="margin-top:10px"><b style="font-size:12px">Fabric compatibility <span id="md-fabric-note" style="color:var(--muted);font-weight:400"></span></b>
              <div id="md-fabric" style="margin-top:6px;font-size:12px;color:var(--muted)"></div>
            </div>
          </div>
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
          <h2>Historical execution <span class="count">deterministic, from measured history</span></h2>
          <div id="hist-stats" style="margin-top:6px;font-size:12px;color:var(--muted)"><span class="muted">loading…</span></div>
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
          <h2>Resources <span class="count">real state · /v1/resources</span></h2>
          <div style="font-size:12px;color:var(--muted);margin-bottom:8px">Node RAM/VRAM/CPU/DISK below; per-worker fabric rows from live advertisements. Provenance is explicit — UNKNOWN is never a fabricated zero, and RAM/VRAM are separate.</div>
          <div class="mono" id="res-node" style="font-size:11.5px;margin-bottom:10px">collecting&hellip;</div>
          <div id="res-fabric" class="empty">no fabric rows (compute not attached)</div>
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
            <div class="grid" style="grid-template-columns:1fr 100px 110px 120px 90px auto;gap:8px;align-items:end">
              <div><div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">Name</div><input id="tok-name" placeholder="alice"></div>
              <div><div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">Tier</div><select id="tok-tier"><option value="1">Guest</option><option value="2">Contributor</option><option value="3">Core</option></select></div>
              <div><div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">Role</div><select id="tok-role"><option value="client">Client</option><option value="operator">Operator</option></select></div>
              <div><div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">Expiry</div><input id="tok-expiry" type="number" min="0" placeholder="hours (0 = never)" value="0"></div>
              <button id="tok-create" class="primary">Create</button>
            </div>
            <div id="tok-result" style="margin-top:10px;font-size:12px"></div>
            <table style="margin-top:12px"><thead><tr><th>Name</th><th>Tier</th><th>Role</th><th class="num">Req</th><th class="num">Tokens</th><th>Status</th><th></th></tr></thead>
            <tbody id="tok-list"><tr><td colspan="7" class="empty">loading tokens&hellip;</td></tr></tbody></table>
          </div>
          <div class="card">
            <h2>Security events <span class="count">audit log</span></h2>
            <div id="audit-list" class="empty">loading&hellip;</div>
          </div>
        </div>
        <div class="card" style="margin-top:14px">
          <h2>Developer access <span class="count">OpenAI-compatible /v1</span></h2>
          <div class="grid cols-2">
            <div>
              <div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">API endpoint</div>
              <code id="dev-endpoint" style="display:block;word-break:break-all"></code>
              <button class="ghost" style="margin-top:6px" onclick="copyDev('dev-endpoint')">Copy endpoint</button>
            </div>
            <div>
              <div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em">API key (shown once at creation)</div>
              <div id="dev-key" class="empty" style="min-height:20px">create a token above — the plaintext is shown once</div>
              <button class="ghost" style="margin-top:6px" onclick="copyDev('dev-key')">Copy key</button>
            </div>
          </div>
          <div style="margin-top:10px;font-size:12px;color:var(--muted)">
            Base URL: <code id="dev-base-url" style="word-break:break-all"></code> · model ids = file names (see Models). Point Open WebUI, OpenClaw or any OpenAI SDK at this endpoint with a created token as the API key.
          </div>
          <div style="margin-top:14px">
            <div class="label" style="font-size:10.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em;margin-bottom:6px">Config generator <span class="count">copy-paste client code</span></div>
            <div style="display:flex;gap:6px;flex-wrap:wrap">
              <button class="ghost cfg-tab active" data-cfg="curl">curl</button>
              <button class="ghost cfg-tab" data-cfg="python">Python SDK</button>
              <button class="ghost cfg-tab" data-cfg="js">JavaScript</button>
              <button class="ghost cfg-tab" data-cfg="openclaw">OpenClaw</button>
              <button class="ghost cfg-tab" data-cfg="webui">Open WebUI</button>
              <button class="ghost" onclick="copyDev('cfg-out')">Copy</button>
            </div>
            <pre id="cfg-out" class="mono" style="margin-top:8px;padding:10px;background:rgba(0,0,0,.25);border-radius:8px;overflow:auto;font-size:11.5px;white-space:pre-wrap;word-break:break-all"></pre>
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
///
/// The living-fabric canvas engine draws real state: the local node at the
/// center, every advertised worker as a living entity, measured P2P links as
/// beziers and execution flow as particles. It never fabricates activity —
/// particles only flow along links that are genuinely busy, and the pipeline /
/// decision strip light up only from real queue, execution and decision data.
pub const JS_TEMPLATE: &str = r##"
// ---- helpers ---------------------------------------------------------------
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const $ = id => document.getElementById(id);
const fmtUptime = s => { const h = Math.floor(s/3600), m = Math.floor((s%3600)/60); return h>0 ? h+'h '+m+'m' : (m>0 ? m+'m '+(s%60)+'s' : s+'s'); };
const short = (s, n=14) => (s && s.length > n) ? s.slice(0, n)+'…' : (s || '—');
const tstr = ts => ts ? new Date(ts*1000).toLocaleTimeString() : '—';
const fmtMB = mb => mb ? (mb/1024).toFixed(1)+' GiB' : '—';
// Compact stable node indicator (`dca-…`). The advertisement carries the
// real node_id; workers that predate the field fall back to deriving it the
// same way (`dca-` + 6 chars after the libp2p ed25519 base58 prefix).
const nodeIdOf = p => (p && p.length) ? ('dca-' + (p.indexOf('12D3KooW') === 0 ? p.slice(8, 14) : p.slice(0, 6))) : '—';
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
  setStageVisible(view === 'overview' || view === 'fabric');
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
// `__auto__` = fabric-wide best-model picker; `remote:<node>:<file>` = an
// explicit remote worker's model (sendChat turns it into worker_hint).
const currentModel = () => {
  const v = chatModel.value || '';
  if (v === '__auto__') return 'auto';
  if (v.startsWith('remote:')) { const i = v.indexOf(':', 7); return v.slice(i + 1); }
  return v || activeModel;
};
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
  let buffer = '', text = '', tokens = null, streamError = null;
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
        if (ev.error && ev.error.message) { streamError = ev.error.message; continue; }
        const delta = ev.choices && ev.choices[0] && ev.choices[0].delta && ev.choices[0].delta.content;
        if (delta) { text += delta; bodyEl.textContent = text; }
        if (ev.usage) tokens = ev.usage.completion_tokens;
      }
      chatbox.scrollTop = chatbox.scrollHeight;
    }
  } catch (e) { streamError = 'stream interrupted: ' + e; }
  finally { reader.releaseLock(); }
  return { text, tokens, streamError };
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
    const sel = chatModel.value || '';
    let workerHint = '';
    if (sel.startsWith('remote:')) { const i = sel.indexOf(':', 7); workerHint = sel.slice(7, i); }
    const body = JSON.stringify({ model: currentModel(), messages: hist, stream, ...(workerHint ? { worker_hint: workerHint } : {}) });
    const r = await fetch('/v1/chat/completions', { method: 'POST', headers, body, signal: controller.signal });
    const servedEl = $('chat-served');
    if (servedEl) {
      const origin = r.headers.get('x-decentra-origin') || '';
      const worker = r.headers.get('x-decentra-worker') || '';
      const node = r.headers.get('x-decentra-node') || '';
      if (origin === 'remote') {
        servedEl.textContent = 'served by ' + (node || worker || 'remote worker') + ' · remote';
        servedEl.className = 'badge remote';
      } else if (origin === 'local') {
        servedEl.textContent = 'served locally' + (node ? ' · ' + node : '');
        servedEl.className = 'badge local';
      } else {
        servedEl.textContent = '';
      }
    }
    let answer = '', tokens = null;
    if (stream && r.ok && r.body) { const out = await readSse(r); answer = out.text; tokens = out.tokens; if (out.streamError) { addMsg('node', '(stream error: ' + out.streamError + ')'); } }
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
    else {
      const msg = (e && e.name === 'TypeError' && /input stream/i.test(String(e))) || /interrupted/i.test(String(e || ''))
        ? 'stream interrupted — the worker closed the connection mid-answer'
        : ('request failed: ' + e);
      addMsg('node', msg); chatStatus.textContent = 'failed';
    }
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

// =============================================================================
// LIVING FABRIC ENGINE — Canvas 2D stage, real data only
// =============================================================================
const STATE_COLORS = { Ready:'#34d399', Busy:'#22d3ee', Degraded:'#fbbf24', Unhealthy:'#f87171', Offline:'#5a6b80' };
const stageData = { s:null, c:null, n:null, x:null, mode:'idle', lastTokens:0, lastStreamTs:0, addresses:{}, connected:new Set(), localPeer:'local', localAddr:'' };

function initStage(canvasId){
  const cv = document.getElementById(canvasId);
  if (!cv || cv.__fabric) return cv && cv.__fabric;
  const ctx = cv.getContext('2d');
  const f = {
    cv, ctx, W: 0, H: 0, dpr: Math.min(window.devicePixelRatio || 1, 2),
    visible: false, raf: null, t: 0,
    nodes: [], links: [], parts: [], pulses: [],
  };
  const resize = () => {
    f.W = cv.clientWidth || 900; f.H = cv.clientHeight || 520;
    cv.width = Math.round(f.W * f.dpr); cv.height = Math.round(f.H * f.dpr);
    ctx.setTransform(f.dpr, 0, 0, f.dpr, 0, 0);
  };
  resize();
  if (typeof ResizeObserver !== 'undefined') new ResizeObserver(resize).observe(cv);
  cv.__fabric = f;
  return f;
}

const stageOverview = initStage('fabric-stage');
const stageTopo = initStage('fabric-topo');

function setStageVisible(on){
  [stageOverview, stageTopo].forEach(f => {
    if (!f) return;
    f.visible = on;
    if (on && !f.raf) f.raf = requestAnimationFrame(() => stageLoop(f));
  });
}

function stageLoop(f){
  if (!f.visible) { f.raf = null; return; }
  f.t += 0.016;
  drawStage(f);
  f.raf = requestAnimationFrame(() => stageLoop(f));
}

// Rebuild node/link geometry from real state on every refresh.
function buildStageGeometry(f){
  const d = stageData;
  const workers = (d.c && d.c.workers) || [];
  const links = (d.n && d.n.links) || [];
  const rtt = {}; links.forEach(l => rtt[l.peer] = l.rtt_ms);
  const dec = (d.x && d.x.decisions && d.x.decisions[0]) || null;
  const activePeer = (dec && dec.selected_worker) || null;
  const connected = d.connected || new Set();
  const addrByPeer = d.addresses || {};

  f.W = f.W || f.cv.clientWidth || 900; f.H = f.H || f.cv.clientHeight || 520;
  const W = f.W, H = f.H;
  const cx = W * 0.5, cy = H * 0.56;
  const R = Math.min(W, H) * 0.30;

  // nodes: local + workers on an orbit
  f.nodes = [];
  const nW = Math.max(workers.length, 1);
  workers.forEach((w, i) => {
    const ang = -Math.PI/2 + (i / nW) * Math.PI * 2;
    const maxCtx = (w.served_models || []).reduce((m, mo) => Math.max(m, mo.context_tokens || 0), 0);
    f.nodes.push({
      id: w.peer_id, name: w.node_name || short(w.peer_id, 10),
      kind: 'worker', x: cx + R * Math.cos(ang), y: cy + R * Math.sin(ang),
      status: w.status || 'Offline', col: STATE_COLORS[w.status] || '#5a6b80',
      load: (w.load_percent || 0) / 100, in_flight: w.in_flight || 0,
      trusted: !!w.trusted, active: activePeer === w.peer_id,
      label: (w.node_name || short(w.peer_id, 14)),
      shortId: w.node_id || nodeIdOf(w.peer_id),
      addr: addrByPeer[w.peer_id] || '', connected: connected.has(w.peer_id),
      acceptsRemote: !!w.accepts_remote_inference, engine: w.engine || '',
      cpu: w.cpu_cores || 0, ramTotal: w.ram_mb || 0, ramFree: w.available_ram_mb || 0,
      gpu: w.gpu_name || null, vram: w.gpu_vram_mb || null, vramFree: w.available_vram_mb || null,
      ctx: maxCtx, queue: w.queue_depth || 0, tps: w.tokens_per_second || 0,
      lat: w.current_latency_ms || 0, lastSeen: w.last_seen_secs || 0,
      models: w.served_models || [],
    });
  });
  f.nodes.unshift({
    id: 'local', name: (d.s && d.s.node && d.s.node.name) || (d.c && d.c.local_peer) || 'this node',
    kind: 'local', x: cx, y: cy, status: 'Ready', col: '#22d3ee',
    load: 0, in_flight: 0, trusted: true, active: false,
    label: 'this node',
    shortId: (d.s && d.s.node && d.s.node.node_id) || nodeIdOf(d.localPeer || (d.c && d.c.local_peer)),
    addr: d.localAddr || '', connected: true,
    acceptsRemote: true, engine: (d.s && d.s.node && d.s.node.engine) || '',
    cpu: (d.s && d.s.system && d.s.system.cpu_threads) || 0,
    ramTotal: (d.s && d.s.system && d.s.system.ram_total_gib) ? (d.s.system.ram_total_gib * 1024) : 0,
    ramFree: (d.s && d.s.system && d.s.system.ram_available_gib) ? (d.s.system.ram_available_gib * 1024) : 0,
    gpu: (d.s && d.s.system && d.s.system.gpu && d.s.system.gpu.name) || null,
    vram: (d.s && d.s.system && d.s.system.gpu) ? (d.s.system.gpu.total_vram_mib || null) : null,
    vramFree: (d.s && d.s.system && d.s.system.gpu) ? (d.s.system.gpu.free_vram_mib || null) : null,
    ctx: (d.s && d.s.node && d.s.node.served_models || []).reduce((m, mo) => Math.max(m, mo.context_tokens || 0), 0),
    queue: (d.s && d.s.queue && d.s.queue.waiting || []).length,
    models: (d.s && d.s.node && d.s.node.served_models) || [],
  });
  // links: local -> worker, real RTT
  f.links = f.nodes.filter(n => n.kind === 'worker').map(n => {
    const r = rtt[n.id];
    return { a: f.nodes[0], b: n, rtt_ms: r, active: n.active || n.in_flight > 0 };
  });
}

function drawStage(f){
  const { ctx } = f;
  const W = f.W, H = f.H;
  ctx.clearRect(0, 0, W, H);
  if (W < 40 || H < 40) return;

  const d = stageData;
  const dec = (d.x && d.x.decisions && d.x.decisions[0]) || null;
  const recovering = d.mode === 'recovering';

  // ---- ambient dot grid (atmospheric, very subtle) ----
  ctx.fillStyle = 'rgba(130,150,180,.05)';
  for (let x = 26; x < W; x += 52) {
    for (let y = 26; y < H; y += 52) {
      ctx.beginPath(); ctx.arc(x, y, 0.9, 0, Math.PI * 2); ctx.fill();
    }
  }

  // ---- links (beziers, color = measured RTT, glow when live) ----
  f.links.forEach(lk => {
    const { a, b } = lk;
    const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2 - 26;
    const rtt = lk.rtt_ms;
    let col = 'rgba(140,160,185,.22)';
    if (rtt !== undefined) col = rtt < 5 ? 'rgba(52,211,153,.5)' : rtt < 25 ? 'rgba(34,211,238,.45)' : rtt < 100 ? 'rgba(251,191,36,.5)' : 'rgba(248,113,113,.55)';
    if (lk.active) col = 'rgba(34,211,238,.85)';
    ctx.strokeStyle = col;
    ctx.lineWidth = lk.active ? 2.2 : 1.1;
    ctx.setLineDash(lk.active ? [] : [3, 6]);
    ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.quadraticCurveTo(mx, my, b.x, b.y); ctx.stroke();
    ctx.setLineDash([]);
    // RTT label on measured links
    if (rtt !== undefined) {
      ctx.fillStyle = 'rgba(143,160,179,.7)';
      ctx.font = '10px ui-monospace, Menlo, monospace';
      ctx.textAlign = 'center';
      ctx.fillText(rtt + 'ms', mx, my - 6);
    }
  });

  // ---- nodes ----
  f.nodes.forEach(n => {
    const pulse = 0.5 + 0.5 * Math.sin(f.t * (n.active ? 4.5 : 1.1) + (n.x % 7));
    const alpha = 0.75 + 0.25 * pulse;
    if (n.kind === 'local') {
      // soft halo + core
      ctx.beginPath(); ctx.arc(n.x, n.y, 46, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(34,211,238,.07)'; ctx.fill();
      ctx.beginPath(); ctx.arc(n.x, n.y, 30, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(34,211,238,.4)'; ctx.lineWidth = 1; ctx.setLineDash([3, 5]); ctx.stroke(); ctx.setLineDash([]);
      ctx.beginPath(); ctx.arc(n.x, n.y, 16, 0, Math.PI * 2);
      ctx.fillStyle = '#22d3ee'; ctx.shadowColor = 'rgba(34,211,238,.6)'; ctx.shadowBlur = 18; ctx.fill(); ctx.shadowBlur = 0;
      ctx.fillStyle = 'rgba(143,160,179,.85)'; ctx.font = '600 11px -apple-system, Segoe UI, Roboto, sans-serif'; ctx.textAlign = 'center';
      ctx.fillText('you · this node', n.x, n.y + 38);
      ctx.fillStyle = 'rgba(34,211,238,.9)'; ctx.font = '10px ui-monospace, Menlo, monospace';
      ctx.fillText(short(n.name, 18), n.x, n.y - 34);
      ctx.fillStyle = 'rgba(34,211,238,.5)'; ctx.font = '9px ui-monospace, Menlo, monospace';
      ctx.fillText(n.shortId || nodeIdOf(d.localPeer), n.x, n.y - 22);
      // local node identity: real LAN address, if known
      if (n.addr) {
        ctx.fillStyle = 'rgba(34,211,238,.55)'; ctx.font = '9.5px ui-monospace, Menlo, monospace';
        ctx.fillText('local · ' + short(n.addr, 22), n.x, n.y + 52);
      }
    } else {
      const col = recovering && n.active ? '#fbbf24' : n.col;
      const sel = n.active;
      // halo
      ctx.beginPath(); ctx.arc(n.x, n.y, 30, 0, Math.PI * 2);
      ctx.fillStyle = sel ? 'rgba(34,211,238,.10)' : 'rgba(0,0,0,.30)'; ctx.fill();
      // outer ring + load arc
      ctx.beginPath(); ctx.arc(n.x, n.y, 24, 0, Math.PI * 2);
      ctx.strokeStyle = col; ctx.globalAlpha = sel ? 1 : .35; ctx.lineWidth = sel ? 2.5 : 1.6; ctx.stroke();
      ctx.globalAlpha = 1;
      if (n.load > 0.02) {
        const circ = 2 * Math.PI * 24;
        ctx.beginPath(); ctx.arc(n.x, n.y, 24, -Math.PI/2, -Math.PI/2 + circ * Math.min(n.load, 1));
        ctx.strokeStyle = col; ctx.lineWidth = 3; ctx.stroke();
      }
      // resource ring: RAM free/total as a thin arc (real advertised values)
      if (n.ramTotal > 0) {
        const ratio = Math.max(0, Math.min(1, n.ramFree / n.ramTotal));
        const circ = 2 * Math.PI * 20;
        ctx.beginPath(); ctx.arc(n.x, n.y, 20, -Math.PI/2, -Math.PI/2 + circ * ratio);
        ctx.strokeStyle = ratio < .15 ? 'rgba(248,113,113,.85)' : 'rgba(52,211,153,.7)';
        ctx.lineWidth = 2; ctx.stroke();
      }
      // core
      ctx.beginPath(); ctx.arc(n.x, n.y, 11, 0, Math.PI * 2);
      ctx.fillStyle = col;
      ctx.globalAlpha = alpha * (n.status === 'Offline' ? .3 : .92);
      ctx.shadowColor = col; ctx.shadowBlur = sel ? 20 : 10; ctx.fill(); ctx.shadowBlur = 0;
      ctx.globalAlpha = 1;
      // labels
      ctx.fillStyle = n.status === 'Offline' ? '#5a6b80' : '#e8eef6';
      ctx.font = '600 11.5px -apple-system, Segoe UI, Roboto, sans-serif'; ctx.textAlign = 'center';
      ctx.fillText(n.label, n.x, n.y + 40);
      ctx.fillStyle = 'rgba(143,160,179,.6)'; ctx.font = '8.5px ui-monospace, Menlo, monospace';
      ctx.fillText(n.shortId || nodeIdOf(n.id), n.x, n.y + 51);
      ctx.fillStyle = sel ? '#22d3ee' : (n.trusted ? 'rgba(52,211,153,.85)' : 'rgba(251,191,36,.8)');
      ctx.font = '9.5px ui-monospace, Menlo, monospace';
      ctx.fillText((sel ? 'SELECTED · ' : '') + (n.status || '') + ' · ' + Math.round(n.load * 100) + '%', n.x, n.y - 30);
      // connection + trust + remote-sharing identity (real state)
      ctx.fillStyle = n.connected ? 'rgba(52,211,153,.9)' : 'rgba(90,107,128,.8)';
      ctx.fillText(n.connected ? '● connected' : '○ not connected', n.x, n.y - 18);
      ctx.fillStyle = 'rgba(143,160,179,.7)';
      ctx.fillText((n.acceptsRemote ? 'REMOTE-OK' : 'local-only') + (n.engine ? ' · ' + short(n.engine, 12) : ''), n.x, n.y + 54);
      // real LAN address when the p2p node knows one
      if (n.addr) {
        ctx.fillStyle = 'rgba(143,160,179,.45)'; ctx.font = '8.5px ui-monospace, Menlo, monospace';
        ctx.fillText(short(n.addr, 24), n.x, n.y + 64);
      }
    }
  });

  // ---- particles: only along genuinely live links ----
  const flowRate = d.mode === 'active' ? 0.9 : d.mode === 'recovering' ? 0.25 : 0.06;
  if (Math.random() < flowRate && f.links.length) {
    const live = f.links.filter(l => l.active);
    const pool = (d.mode === 'idle' ? f.links : (live.length ? live : f.links));
    const lk = pool[Math.floor(Math.random() * pool.length)];
    if (lk) {
      const fromWorker = Math.random() < 0.5;
      f.parts.push({
        x: fromWorker ? lk.b.x : lk.a.x, y: fromWorker ? lk.b.y : lk.a.y,
        tx: fromWorker ? lk.a.x : lk.b.x, ty: fromWorker ? lk.a.y : lk.b.y,
        t: 0, sp: 0.006 + Math.random() * 0.004,
        col: d.mode === 'recovering' ? '#fbbf24' : '#22d3ee',
      });
    }
  }
  f.parts = f.parts.filter(p => p.t < 1);
  f.parts.forEach(p => {
    p.t += p.sp;
    const x = p.x + (p.tx - p.x) * p.t, y = p.y + (p.ty - p.y) * p.t;
    const a = 1 - p.t;
    ctx.beginPath(); ctx.arc(x, y, 2, 0, Math.PI * 2);
    ctx.fillStyle = p.col; ctx.globalAlpha = a * .85;
    ctx.shadowColor = p.col; ctx.shadowBlur = 8; ctx.fill(); ctx.shadowBlur = 0;
    ctx.globalAlpha = 1;
  });

  // ---- pulses (decision/complete/recovery rings) ----
  f.pulses = f.pulses.filter(p => p.t < 1);
  f.pulses.forEach(p => {
    p.t += 0.012;
    ctx.beginPath(); ctx.arc(p.x, p.y, 12 + p.t * 44, 0, Math.PI * 2);
    ctx.strokeStyle = p.col; ctx.globalAlpha = (1 - p.t) * .5; ctx.lineWidth = 1.5; ctx.stroke();
    ctx.globalAlpha = 1;
  });
}

// ---- fabric state derivation (real data only) ------------------------------
function recoveryEvents(s){
  return (s && s.recent_events || []).filter(e => /restart|recover|evict|offline|reconnect|respawn|replan/i.test(e.event || ''));
}
function deriveMode(s, c, x){
  const dec = (x && x.decisions && x.decisions[0]) || null;
  const serving = (s && s.queue && s.queue.serving);
  const recent = (s && s.recent_requests || [])[0];
  const now = Date.now() / 1000;
  const streaming = recent && (now - recent.timestamp) < 12;
  if (recoveryEvents(s).length > 0) return 'recovering';
  if (serving || streaming || (dec && dec.outcome === 'in_flight')) return 'active';
  return 'idle';
}

// ---- pipeline + decision strip + now-state (HTML, derived from real data) --
function setPipe(stage, cls){
  document.querySelectorAll('.pipe[data-stage="' + stage + '"]').forEach(el => {
    el.classList.remove('on', 'done', 'fail');
    if (cls) el.classList.add(cls);
  });
}
function renderPipeline(s, c, n, x){
  const dec = (x && x.decisions && x.decisions[0]) || null;
  const serving = (s && s.queue && s.queue.serving);
  const recent = (s && s.recent_requests || [])[0];
  const now = Date.now() / 1000;
  const streaming = recent && (now - recent.timestamp) < 12;
  const mode = stageData.mode;
  const recovering = mode === 'recovering';
  const done = dec && (dec.outcome === 'succeeded' || dec.outcome === 'completed');
  const failed = dec && (dec.outcome === 'failed');

  setPipe('user', (serving || streaming) ? 'on' : (done ? 'done' : ''));
  setPipe('request', (serving || streaming) ? 'on' : (done ? 'done' : ''));
  setPipe('planner', dec ? (done ? 'done' : (failed ? 'fail' : (recovering ? 'fail' : 'on'))) : '');
  setPipe('reservation', dec && dec.reservation_id ? (done ? 'done' : 'on') : (recovering ? 'fail' : ''));
  setPipe('fabric', ((c && c.workers || []).length || (n && n.links || []).length) ? 'on' : '');
  setPipe('worker', dec && dec.selected_worker ? (done ? 'done' : (failed ? 'fail' : 'on')) : '');
  setPipe('engine', s && s.model_loaded ? 'on' : '');
  setPipe('stream', streaming ? 'on' : '');
  setPipe('result', done ? 'done' : (failed ? 'fail' : (streaming ? 'on' : '')));

  // worker pipe identity: which real node is executing (local vs remote)
  const pwn = $('pipe-worker-name');
  if (dec && dec.selected_worker) {
    const w = (c && c.workers || []).find(x => x.peer_id === dec.selected_worker);
    const remote = dec.selected_worker !== stageData.localPeer;
    pwn.textContent = (w && w.node_name ? w.node_name : short(dec.selected_worker, 12)) + (remote ? ' · remote' : ' · local');
  } else {
    pwn.textContent = '';
  }

  // planner chip: visible identity + state
  const pd = $('planner-dot'), ps = $('planner-state');
  if (recovering) { pd.className = 'pd fail'; ps.textContent = 'recovering · replanning'; }
  else if (dec) {
    if (done) { pd.className = 'pd on'; ps.textContent = 'idle (last: ' + (dec.selected_worker ? short(dec.selected_worker, 10) : 'no worker') + ')'; }
    else if (failed) { pd.className = 'pd fail'; ps.textContent = 'failed · reacting'; }
    else { pd.className = 'pd busy'; ps.textContent = (dec.workload_class || '').replace(/_/g, ' ') + ' · routing'; }
  } else { pd.className = 'pd'; ps.textContent = 'idle'; }

  // now-state: the one-line answer to "what is DecentraAI doing right now?"
  const ns = $('now-state');
  if (recovering) {
    const ev = recoveryEvents(s)[0];
    ns.textContent = 'recovering — ' + (ev ? ev.event.replace(/_/g, ' ') : 'replanning after a failure') + ' · the fabric is rerouting';
  } else if (serving) {
    ns.textContent = 'executing request from ' + esc(serving.who) + ' on ' + esc((dec && dec.selected_worker ? short(dec.selected_worker, 12) : 'this node')) + ' · ' + serving.endpoint.replace('/v1/', '');
  } else if (streaming) {
    ns.textContent = 'streaming a reply · ' + recent.tokens_per_second.toFixed(1) + ' tok/s · ' + recent.completion_tokens + ' tokens generated';
  } else if (dec && dec.outcome === 'in_flight') {
    ns.textContent = 'planning ' + (dec.workload_class || 'request').replace(/_/g, ' ') + ' · ' + (dec.candidates || []).length + ' candidates';
  } else if (done) {
    ns.textContent = 'fabric calm · last request completed on ' + short(dec.selected_worker || 'this node', 12);
  } else {
    ns.textContent = 'fabric calm · ' + ((c && c.workers || []).length) + ' worker(s) · ' + ((n && n.connected || []).length) + ' peer(s) connected';
  }

  // stage facts line
  const rtts = (n && n.links || []).map(l => l.rtt_ms).filter(v => v !== undefined);
  const rttStr = rtts.length ? (Math.min(...rtts) + '–' + Math.max(...rtts) + ' ms') : 'not measured';
  $('stage-facts').innerHTML = 'fabric: <b>' + ((c && c.workers || []).length) + '</b> workers · peers: <b>' + ((n && n.connected || []).length) + '</b> · rtt: <b>' + rttStr + '</b>' + (s ? ' · tokens: <b>' + s.tokens_generated + '</b>' : '');
}

function renderDecisionStrip(x){
  const dec = (x && x.decisions && x.decisions[0]) || null;
  const el = $('decision-strip');
  $('ds-count').textContent = (x && x.decisions || []).length + ' tracked';
  if (!dec) {
    el.className = 'ds-empty';
    el.innerHTML = 'no autonomous decision yet — the planner is idle. Send a routed request to watch it plan.';
    return;
  }
  const cands = (dec.candidates || []);
  const kv = dec.kv_affinity || (cands.some(c => c.kv_prefix_resident) ? 'prefix resident' : 'cold');
  const eng = dec.engine_capability || (cands[0] && cands[0].engine) || 'llama_server';
  const done = dec.outcome === 'succeeded' || dec.outcome === 'completed';
  const failed = dec.outcome === 'failed';
  const live = done || failed ? 'done' : 'on';
  const steps = [
    { k: 'Classifying', v: (dec.workload_class || '—').replace(/_/g, ' '), cls: live },
    { k: 'Candidates', v: cands.length + ' eligible', cls: live },
    { k: 'Network cost', v: dec.network_cost_ms != null ? dec.network_cost_ms + ' ms' : '—', cls: live },
    { k: 'KV affinity', v: kv, cls: live },
    { k: 'Engine', v: eng, cls: live },
    { k: 'Selected worker', v: dec.selected_worker ? short(dec.selected_worker, 12) : 'none', cls: failed ? 'fail' : live },
    { k: 'Execution', v: failed ? 'failed · rerouting' : (done ? 'completed' : (dec.expected_mode || 'in flight')), cls: failed ? 'fail' : live },
  ];
  el.className = '';
  el.innerHTML = '<div class="ds-row">' + steps.map((st, i) =>
    '<div class="ds-step ' + st.cls + '"><span class="k">' + st.k + '</span><span class="v">' + esc(st.v) + '</span></div>' +
    (i < steps.length - 1 ? '<span class="ds-arrow">→</span>' : '')
  ).join('') + '</div>';
}

function renderFabric(s, c, n, x){
  stageData.s = s; stageData.c = c; stageData.n = n; stageData.x = x;
  stageData.mode = deriveMode(s, c, x);
  // identity enrichment for every renderer: real addresses + connectivity
  stageData.localPeer = (c && c.local_peer) || (n && n.local_peer) || 'local';
  stageData.localAddr = (n && n.local_addresses && n.local_addresses[0]) || '';
  stageData.addresses = {};
  (n && n.addresses || []).forEach(a => { if (a && a.peer) stageData.addresses[a.peer] = a.addr; });
  stageData.connected = new Set((n && n.connected) || []);
  [stageOverview, stageTopo].forEach(f => { if (f) buildStageGeometry(f); });
  renderPipeline(s, c, n, x);
  renderDecisionStrip(x);
  renderFabricNodes(s, c, n);
  updateDiscovery(c, n);
  // pulses: real events get a ring on the canvas
  const recent = (s && s.recent_requests || [])[0];
  const now = Date.now() / 1000;
  if (recent && (now - recent.timestamp) < 6) addPulse(stageOverview, stageOverview.nodes[0] || { x: 0, y: 0 }, '#34d399');
  if (stageData.mode === 'recovering') {
    const dec = (x && x.decisions && x.decisions[0]) || null;
    const w = dec && dec.selected_worker ? stageOverview.nodes.find(n => n.id === dec.selected_worker) : null;
    addPulse(stageOverview, w || stageOverview.nodes[0] || { x: 0, y: 0 }, '#fbbf24');
  }
  // topology metrics (shared with the advanced fabric view)
  const workers = (c && c.workers) || [];
  const links = (n && n.links) || [];
  $('local-peer').textContent = short((c && c.local_peer) || (n && n.local_peer) || 'local', 24);
  $('fabric-workers').textContent = workers.length;
  $('fabric-connected').textContent = ((n && n.connected) || []).length;
  $('fabric-links').textContent = links.length;
  $('fabric-sessions').textContent = (c && c.sessions) || 0;
  $('topo-count').textContent = workers.length + ' workers · ' + links.length + ' links';
  const last = (x && x.decisions || [])[0];
  if (last) { $('fabric-last').textContent = short(last.selected_worker || 'no worker', 22); $('fabric-last-sub').textContent = (last.workload_class || '') + ' · ' + (last.expected_mode || '') + ' · ' + (last.network_cost_ms || 0) + 'ms'; }
  else { $('fabric-last').textContent = '—'; $('fabric-last-sub').textContent = ''; }
}

// ---- fabric nodes strip: identity + resources per real node -----------------
function barSeg(label, pct, ok){
  const cls = pct > 80 ? 'bad' : pct > 60 ? 'warn' : '';
  return '<div class="nc-bar"><span style="flex:none;width:44px">'+label+'</span><span class="track"><i class="'+(ok ? '' : cls)+'" style="width:'+Math.max(2, Math.min(100, pct))+'%"></i></span><span style="flex:none;width:34px;text-align:right">'+pct+'%</span></div>';
}
function trustChain(steps, curIdx){
  return steps.map((st, i) => {
    const cls = i < curIdx ? 'done' : (i === curIdx ? 'cur' + (st.warn ? ' warn' : '') : '');
    return '<span class="tc-step ' + cls + '">' + st.k + '</span>' + (i < steps.length - 1 ? '<span class="tc-arr">→</span>' : '');
  }).join('');
}
function renderFabricNodes(s, c, n){
  const workers = (c && c.workers) || [];
  const chips = [];
  const localPeer = stageData.localPeer;
  // local node first — the perspective anchor
  const sys = (s && s.system) || {};
  const ramT = sys.ram_total_gib ? sys.ram_total_gib * 1024 : 0;
  const ramF = sys.ram_available_gib ? sys.ram_available_gib * 1024 : 0;
  const localModels = (s && s.node && s.node.served_models) || [];
  const localState = (s && s.queue && s.queue.serving) ? 'Busy' : 'Ready';
  chips.push(nodeChip({
    isLocal: true, name: (s && s.node && s.node.name) || 'this node', peer: localPeer,
    id: (s && s.node && s.node.node_id) || nodeIdOf(localPeer),
    addr: stageData.localAddr, connected: true, status: localState, col: '#22d3ee',
    acceptsRemote: true, engine: (s && s.node && s.node.engine) || 'llama_server',
    cpu: sys.cpu_threads || 0, ramTotal: ramT, ramFree: ramF,
    gpu: (sys.gpu && sys.gpu.name) || null, vram: (sys.gpu && sys.gpu.total_vram_mib) || null,
    vramFree: (sys.gpu && sys.gpu.free_vram_mib) || null, models: localModels, trusted: true,
  }, true, null));
  // remote workers (skip a worker row equal to the local peer — never duplicate)
  workers.forEach(w => {
    if (w.peer_id === localPeer) return;
    chips.push(nodeChip({
      isLocal: false, name: w.node_name || short(w.peer_id, 12), peer: w.peer_id,
      id: w.node_id || nodeIdOf(w.peer_id),
      addr: stageData.addresses[w.peer_id] || '', connected: stageData.connected.has(w.peer_id),
      status: w.status || 'Offline', col: STATE_COLORS[w.status] || '#5a6b80',
      acceptsRemote: !!w.accepts_remote_inference, engine: w.engine || '',
      cpu: w.cpu_cores || 0, ramTotal: w.ram_mb || 0, ramFree: w.available_ram_mb || 0,
      gpu: w.gpu_name || null, vram: w.gpu_vram_mb || null, vramFree: w.available_vram_mb || null,
      models: w.served_models || [], trusted: !!w.trusted,
    }, false, w.load_percent || 0));
  });
  $('fabric-nodes').innerHTML = chips.join('') || '<span class="badge faint">no peers discovered yet</span>';
  $('fabric-nodes-count').textContent = chips.length + ' node(s)';
}
function nodeChip(nd, isLocal, loadPct){
  const ramPct = nd.ramTotal > 0 ? Math.round(((nd.ramTotal - nd.ramFree) / nd.ramTotal) * 100) : 0;
  const vramPct = (nd.vram && nd.vram > 0) ? Math.round((((nd.vram - (nd.vramFree || nd.vram)) / nd.vram)) * 100) : 0;
  const cpuLbl = nd.cpu ? nd.cpu + ' core(s)' : '—';
  const ramLbl = nd.ramTotal > 0 ? fmtMB(nd.ramFree) + ' / ' + fmtMB(nd.ramTotal) : '—';
  const gpuLbl = nd.gpu ? (nd.gpu + (nd.vram ? ' · ' + fmtMB(nd.vram) : '')) : 'CPU-only';
  const steps = [
    { k: 'DISCOVERED', warn: false },
    { k: 'UNTRUSTED', warn: true },
    { k: 'APPROVED', warn: false },
    { k: 'CONNECTED', warn: false },
    { k: 'WORKER READY', warn: false },
  ];
  const curIdx = nd.isLocal ? steps.length - 1 : (nd.trusted ? (nd.connected ? steps.length - 1 : 3) : 1);
  const models = (nd.models || []).slice(0, 3).map(m =>
    '<span class="nc-model" title="' + esc(m.file_name || '') + (m.context_tokens ? ' · ctx ' + m.context_tokens : '') + '">' + esc(m.file_name || 'model') + (m.context_tokens ? ' · ' + m.context_tokens + ' ctx' : '') + '</span>'
  ).join('');
  const loadBar = loadPct == null ? '' : barSeg('load', loadPct, true);
  return '<div class="node-chip ' + (isLocal ? 'local' : '') + '">' +
    '<div class="nc-head"><span class="dot" style="background:' + nd.col + '"></span>' + esc(nd.name) +
    '<span class="nc-tag ' + (isLocal ? 'local-tag' : 'remote-tag') + '">' + (isLocal ? 'local' : 'remote') + '</span></div>' +
    '<div class="nc-meta"><span><b>peer</b> ' + short(nd.peer, 12) + '</span>' +
    '<span><b>id</b> ' + (nd.id || nodeIdOf(nd.peer)) + '</span>' +
    '<span><b>addr</b> ' + esc(nd.addr || '—') + '</span>' +
    '<span><b>engine</b> ' + esc(nd.engine || '—') + '</span>' +
    '<span><b>state</b> ' + esc(nd.status || '—') + (nd.acceptsRemote ? ' · accepts remote' : ' · local-only') + '</span></div>' +
    '<div class="nc-bars">' +
      loadBar +
      barSeg('ram', ramPct, true) +
      barSeg('vram', vramPct, false) +
    '</div>' +
    '<div><span class="mono" style="font-size:9px;color:var(--faint)">cpu ' + cpuLbl + ' · ' + ramLbl + ' · gpu ' + esc(gpuLbl) + '</span></div>' +
    (models ? '<div class="nc-models" style="margin-top:6px">' + models + '</div>' : '') +
    '<div class="nc-trust">' + trustChain(steps, curIdx) + '</div>' +
  '</div>';
}

// ---- discovery feed: real appearance/offline/reconnect events ---------------
const discoveryState = { seen: new Set(), status: {} };
let discoveryFeed = [];
function renderDiscoveryFeed(){
  const el = $('discovery-feed');
  if (!discoveryFeed.length) { el.innerHTML = ''; return; }
  el.innerHTML = discoveryFeed.map(ev => {
    const t = new Date(ev.ts).toLocaleTimeString();
    let dot, msg;
    if (ev.kind === 'appear') {
      dot = 'background:var(--ok)';
      msg = '<b class="up">discovered</b> ' + esc(ev.name) + ' <code>' + short(ev.peer, 10) + '</code> · ' + esc(ev.status || '') + (ev.acceptsRemote ? ' · accepts remote work' : ' · local-only');
    } else if (ev.kind === 'reconnect') {
      dot = 'background:var(--accent)';
      msg = '<b>reconnected</b> ' + esc(ev.name) + ' <code>' + short(ev.peer, 10) + '</code> · worker is ready again';
    } else {
      dot = 'background:var(--bad)';
      msg = '<b class="down">offline</b> ' + short(ev.peer, 10) + ' — no heartbeat';
    }
    return '<div class="disc-ev"><span class="de-dot" style="' + dot + '"></span><span class="de-time">' + t + '</span><span class="de-msg">' + msg + '</span></div>';
  }).join('');
}
function updateDiscovery(c, n){
  const workers = (c && c.workers) || [];
  const nowKeys = new Set(workers.map(w => w.peer_id));
  const statusNow = {}; workers.forEach(w => statusNow[w.peer_id] = w.status);
  const events = [];
  workers.forEach(w => {
    if (!discoveryState.seen.has(w.peer_id)) {
      events.push({ kind: 'appear', ts: Date.now(), peer: w.peer_id, name: w.node_name || w.peer_id, status: w.status, acceptsRemote: !!w.accepts_remote_inference });
    } else if (discoveryState.status[w.peer_id] === 'Offline' && (w.status === 'Ready' || w.status === 'Busy')) {
      events.push({ kind: 'reconnect', ts: Date.now(), peer: w.peer_id, name: w.node_name || w.peer_id });
    }
  });
  discoveryState.seen.forEach(id => {
    if (!nowKeys.has(id)) events.push({ kind: 'gone', ts: Date.now(), peer: id });
  });
  events.slice(0, 3).forEach(ev => {
    discoveryFeed.unshift(ev);
    // real events pulse the stage canvas at that node
    if (ev.kind === 'appear') {
      const nd = stageOverview && stageOverview.nodes.find(x => x.id === ev.peer);
      if (nd) addPulse(stageOverview, nd, '#34d399');
    } else if (ev.kind === 'reconnect') {
      const nd = stageOverview && stageOverview.nodes.find(x => x.id === ev.peer);
      if (nd) addPulse(stageOverview, nd, '#22d3ee');
    }
  });
  discoveryFeed = discoveryFeed.slice(0, 7);
  discoveryState.seen = nowKeys;
  discoveryState.status = statusNow;
  renderDiscoveryFeed();
}

function addPulse(f, node, col){
  if (!f || !node || node.x === undefined) return;
  f.pulses.push({ x: node.x, y: node.y, t: 0, col });
}

// ---- renderers -------------------------------------------------------------
function workerCard(w, localPeer){
  const isLocal = w.peer_id === localPeer;
  const connected = stageData.connected.has(w.peer_id);
  const status = w.status || '';
  const col = STATE_COLORS[status] || '#5a6b80';
  const badge = status === 'Ready' ? '<span class="badge ok">ready</span>' : status === 'Busy' ? '<span class="badge accent">busy</span>' : status === 'Offline' ? '<span class="badge bad">offline</span>' : status ? '<span class="badge warn">'+esc(status)+'</span>' : '<span class="badge faint">—</span>';
  const action = isLocal ? '' : (w.trusted
    ? (isAdmin ? '<button data-p="'+w.peer_id+'" onclick="revokeWorker(event)" class="danger">Revoke</button>' : '<span class="badge ok">trusted</span>')
    : (isAdmin ? '<button data-p="'+w.peer_id+'" onclick="trustWorker(event)">Approve</button>' : '<button disabled>Approve</button>'));
  const ramPct = w.ram_mb > 0 ? Math.round(((w.ram_mb - (w.available_ram_mb || 0)) / w.ram_mb) * 100) : 0;
  const vramPct = (w.gpu_vram_mb && w.gpu_vram_mb > 0) ? Math.round((((w.gpu_vram_mb - (w.available_vram_mb || w.gpu_vram_mb)) / w.gpu_vram_mb)) * 100) : 0;
  const models = (w.served_models || []).slice(0, 4).map(m =>
    '<span class="nc-model" title="'+esc(m.file_name||'')+(m.context_tokens?' · ctx '+m.context_tokens:'')+'">'+esc(m.file_name||'model')+(m.context_tokens?' · '+m.context_tokens+' ctx':'')+'</span>'
  ).join('');
  const steps = [
    { k: 'DISCOVERED' }, { k: 'UNTRUSTED', warn: true }, { k: 'APPROVED' },
    { k: 'CONNECTED' }, { k: 'WORKER READY' },
  ];
  const curIdx = isLocal ? steps.length - 1 : (w.trusted ? (connected ? steps.length - 1 : 3) : 1);
  return '<div class="worker-card '+(isLocal?'local':'')+'" id="node-card-'+esc(w.peer_id)+'">'+
    '<div class="wc-head">'+
      '<span class="dot" style="background:'+col+';width:9px;height:9px;border-radius:50%"></span>'+
      '<span class="wc-name">'+esc(w.node_name || short(w.peer_id, 14))+'</span>'+
      (isLocal ? '<span class="nc-tag local-tag">local</span>' : '<span class="nc-tag remote-tag">remote</span>')+
      badge+
      (w.accepts_remote_inference ? '<span class="nc-tag" title="this node accepts inference routed from remote peers">remote-ok</span>' : '<span class="nc-tag" title="this node serves local requests only">local-only</span>')+
      (connected ? '<span class="badge ok">p2p connected</span>' : '<span class="badge faint">not connected</span>')+
    '</div>'+
    '<div class="wc-meta">'+
      '<span><b>peer</b> <code title="'+esc(w.peer_id)+'">'+short(w.peer_id, 14)+'</code></span>'+
      '<span><b>id</b> '+(w.node_id || nodeIdOf(w.peer_id))+'</span>'+
      '<span><b>addr</b> '+esc((stageData.addresses[w.peer_id] || (isLocal ? stageData.localAddr : '')) || '—')+'</span>'+
      '<span><b>engine</b> '+esc(w.engine||'—')+'</span>'+
      '<span><b>last seen</b> '+(w.last_seen_secs != null ? fmtUptime(w.last_seen_secs)+' ago' : '—')+'</span>'+
      '<span><b>queue</b> '+w.queue_depth+'</span><span><b>in-flight</b> '+w.in_flight+'</span>'+
      '<span><b>tok/s</b> '+w.tokens_per_second+'</span><span><b>latency</b> '+w.current_latency_ms+'ms</span>'+
    '</div>'+
    '<div class="wc-res">'+
      '<div class="nc-bar"><span style="flex:none;width:52px">cpu</span><span class="track"><i class="'+(w.load_percent>80?'bad':w.load_percent>60?'warn':'')+'" style="width:'+Math.min(w.load_percent||0,100)+'%"></i></span><span style="flex:none;width:96px;text-align:right">'+w.load_percent+'% · '+(w.cpu_cores||'—')+' cores</span></div>'+
      '<div class="nc-bar"><span style="flex:none;width:52px">ram</span><span class="track"><i class="'+(ramPct>80?'bad':ramPct>60?'warn':'')+'" style="width:'+Math.max(2,Math.min(100,ramPct))+'%"></i></span><span style="flex:none;width:96px;text-align:right">'+fmtMB(w.available_ram_mb)+' / '+fmtMB(w.ram_mb)+'</span></div>'+
      '<div class="nc-bar"><span style="flex:none;width:52px">vram</span><span class="track"><i class="'+(vramPct>80?'bad':vramPct>60?'warn':'')+'" style="width:'+Math.max(2,Math.min(100,vramPct))+'%"></i></span><span style="flex:none;width:96px;text-align:right">'+(w.gpu_name ? esc(w.gpu_name)+' · '+fmtMB(w.gpu_vram_mb) : 'CPU-only')+'</span></div>'+
    '</div>'+
    (models ? '<div class="wc-models">'+models+'</div>' : '')+
    '<div class="wc-trust">'+trustChain(steps, curIdx)+'</div>'+
    (action ? '<div class="wc-actions">'+action+'</div>' : '')+
  '</div>';
}
function renderWorkers(c){
  const workers = (c && c.workers) || [];
  const localPeer = (c && c.local_peer) || 'local';
  $('workers').innerHTML = workers.map(w => workerCard(w, localPeer)).join('') || '<div class="empty">no workers yet (compute not attached)</div>';
  $('workers-count').textContent = workers.length + ' advertised';
  $('diag-workers').innerHTML = workers.length + ' worker(s)';
  $('diag-sessions').innerHTML = (c && c.sessions) + ' KV session(s)';
  $('set-trust').textContent = workers.filter(w => w.trusted).length + ' trusted of ' + workers.length;
  // contributions
  const crel = (c && c.contributions || []).map(r =>
    '<tr><td>'+esc(r.node_name || short(r.peer_id))+'</td><td class="num">'+r.cpu_cores+'</td><td class="num">'+fmtMB(r.ram_mb)+'</td><td class="num">'+fmtUptime(r.online_seconds)+'</td>'+
    '<td class="num">'+r.verified_requests+'</td><td class="num">'+r.failed_requests+'</td><td class="num">'+r.score.toFixed(2)+'</td>'+
    '<td><span class="badge '+(r.suggested_tier===3?'ok':r.suggested_tier===2?'warn':'faint')+'">T'+r.suggested_tier+'</span></td><td class="num">'+r.reward_tokens+'</td></tr>'
  ).join('');
  $('contributions').innerHTML = crel || '<tr><td colspan="9" class="empty">no contribution ledger yet</td></tr>';
}
// Resource pressure (Part 17/22): honest aggregate of MEASURED load.
// Local values come from the live SystemSnapshot in /status; worker values
// come from each advertisement's ComputeAvailability. Bars are raw numbers,
// never smoothed or invented.
function renderPressure(s, c){
  const bar = (label, pct, extra) =>
    '<div class="nc-bar"><span style="flex:none;width:58px">'+label+'</span><span class="track"><i class="'+(pct>80?'bad':pct>60?'warn':'')+'" style="width:'+Math.max(2,Math.min(100,pct))+'%"></i></span><span style="flex:none;text-align:right">'+extra+'</span></div>';
  const sys = (s && s.system) || {};
  const ramPct = sys.ram_total_gib > 0 ? Math.round(((sys.ram_total_gib - (sys.ram_available_gib||0)) / sys.ram_total_gib) * 100) : 0;
  const local = bar('cpu', Math.round(sys.cpu_usage_percent||0), (sys.cpu_usage_percent||0).toFixed(1)+'% · '+sys.cpu_threads+' cores') +
    bar('ram', ramPct, (sys.ram_available_gib||0).toFixed(1)+' / '+(sys.ram_total_gib||0).toFixed(1)+' GiB free') +
    bar('swap', Math.min(100, Math.round((sys.used_swap_gib||0)*10)), (sys.used_swap_gib||0).toFixed(2)+' GiB used') +
    bar('disk', Math.min(100, Math.max(0, 100 - Math.round((sys.disk_free_gib||0)*2))), (sys.disk_free_gib||0).toFixed(1)+' GiB free');
  $('pressure-local').innerHTML = local;
  const workers = (c && c.workers || []).filter(w => w.status !== 'Offline');
  if (!workers.length) { $('pressure-fabric').innerHTML = '<div class="empty">no live workers to aggregate</div>'; $('pressure-busiest').innerHTML = '<div class="empty">no live workers to aggregate</div>'; return; }
  const avg = xs => xs.length ? Math.round(xs.reduce((a,b)=>a+b,0)/xs.length) : 0;
  const avgCpu = avg(workers.map(w => w.load_percent || 0));
  const avgRam = avg(workers.map(w => w.ram_mb > 0 ? Math.round(((w.ram_mb - (w.available_ram_mb||0)) / w.ram_mb) * 100) : 0));
  $('pressure-fabric').innerHTML = bar('cpu', avgCpu, avgCpu+'% avg') + bar('ram', avgRam, avgRam+'% avg') +
    '<div class="mono" style="font-size:11.5px;color:var(--muted);margin-top:6px">'+workers.length+' live worker(s) · queue '+
    workers.reduce((a,w)=>a+(w.queue_depth||0),0)+' · in-flight '+workers.reduce((a,w)=>a+(w.in_flight||0),0)+'</div>';
  const busy = workers.slice().sort((a,b) => (b.load_percent||0) - (a.load_percent||0))[0];
  const bRam = busy.ram_mb > 0 ? Math.round(((busy.ram_mb - (busy.available_ram_mb||0)) / busy.ram_mb) * 100) : 0;
  $('pressure-busiest').innerHTML = '<div class="mono" style="margin-bottom:6px"><b>'+esc(busy.node_name || short(busy.peer_id, 14))+'</b> · '+esc(busy.status||'')+'</div>' +
    bar('cpu', busy.load_percent||0, (busy.load_percent||0)+'%') + bar('ram', bRam, bRam+'% used');
}
function renderNetwork(n){
  // identity enrichment is self-contained here too (renderNetwork may run
  // before renderFabric on the first refresh cycle)
  stageData.addresses = stageData.addresses || {};
  stageData.connected = stageData.connected || new Set();
  (n && n.addresses || []).forEach(a => { if (a && a.peer) stageData.addresses[a.peer] = a.addr; });
  ((n && n.connected) || []).forEach(p => stageData.connected.add(p));
  if (n && n.local_peer) stageData.localPeer = n.local_peer;
  if (n && n.local_addresses && n.local_addresses[0]) stageData.localAddr = n.local_addresses[0];
  const links = (n && n.links || []).map(l =>
    '<tr><td><code>'+short(l.peer)+'</code></td><td class="num">'+l.rtt_ms+' ms</td><td class="num">'+(l.bandwidth_mbps || '—')+'</td><td class="num">'+(l.transfer_ms_per_mib || '—')+'</td><td><span class="badge '+(l.locality==='Lan'?'ok':l.locality==='Remote'?'warn':'accent')+'">'+esc(l.locality||'')+'</span></td></tr>'
  ).join('');
  $('network').innerHTML = links || '<tr><td colspan="5" class="empty">no measured links yet</td></tr>';
  const conn = (n && n.connected || []);
  $('connected').innerHTML = conn.length
    ? conn.map(p => {
        const addr = stageData.addresses[p] || '';
        return '<div style="display:inline-block;margin:2px 6px 2px 0"><code>'+esc(p)+'</code>'+(addr ? '<span class="mono" style="display:block;font-size:9px;color:var(--faint)">'+esc(addr)+'</span>' : '')+'</div>';
      }).join(' ')
    : 'no connected peers';
  $('diag-p2p').innerHTML = conn.length + ' connected, ' + (n && n.links || []).length + ' measured link(s)' + ((n && n.addresses || []).length ? ' · ' + (n.addresses || []).length + ' address(es) known' : '');
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
  const rows = ex.map(e => {
    // Part 9/17 resource attribution: real measured usage when the worker
    // reported it, else a dash — never invented.
    const usage = (e.tokens_used !== undefined && e.tokens_used !== null)
      ? e.tokens_used + ' tok · ' + e.processing_time_ms + 'ms' + (e.attempt ? ' · attempt ' + e.attempt : '')
      : '—';
    return '<tr><td><code>'+short(e.request_id, 8)+'</code></td><td><code>'+short(e.selected_worker, 10)+'</code></td><td class="num">'+e.score.toFixed(2)+'</td><td class="num">'+e.stages+'</td>'+
    '<td>'+(e.is_continuation ? '<span class="badge accent">cont</span>' : '<span class="badge faint">cold</span>')+'</td><td class="num">'+e.network_rtt_ms+'ms</td>'+
    '<td class="mono">'+esc(e.kv_headroom || '—')+'</td><td class="mono">'+esc(usage)+'</td><td>'+outcomeBadge(e.outcome)+'</td><td class="mono" style="font-size:11px">'+esc(e.reasoning || '')+'</td></tr>';
  }).join('');
  $('execution').innerHTML = rows || '<tr><td colspan="10" class="empty">no executions yet</td></tr>';
  $('exec-count').textContent = (x && x.executions || []).length;
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
      (d.capability_requirement ? (function(cr){
        var ok = cr.satisfied;
        var badge = ok ? '<span class="badge ok">cap ✓</span>' : '<span class="badge warn">cap ✗</span>';
        return '<div class="mono" style="font-size:11.5px;margin-bottom:8px">'+badge+' required <b>'+esc(cr.capability||'')+'</b> · evidence '+esc(cr.evidence||'')+'</div>';
      })(d.capability_requirement) : '')+
      (d.reasoning ? '<div class="mono" style="font-size:11.5px;color:var(--muted);margin-bottom:8px">'+esc(d.reasoning)+'</div>' : '')+
      (d.recovery ? (function(r){
        var rc = r.recoveries || 0;
        var b = rc > 0 ? '<span class="badge warn">self-healed ×'+rc+'</span>' : '<span class="badge ok">no recovery</span>';
        var phases = (r.phases_seen||[]).map(p=>'<span class="mono" style="font-size:10px;color:var(--faint)">'+esc(p)+'</span>').join(' ');
        return '<div class="mono" style="font-size:11.5px;margin-bottom:8px">'+b+' · '+esc(r.summary||'')+'<div style="margin-top:2px">'+phases+'</div></div>';
      })(d.recovery) : '')+
      '<div>'+cands+'</div>'+
      '<ul class="trace" style="margin-top:10px">'+trace+'</ul>'+
    '</div>';
  }).join('');
  $('decisions').innerHTML = cards;
}
// ---- Sessions (KV locality) — real coordinator-tracked KV/session residency
// from /v1/sessions. Only measured backend state is shown: an empty ledger
// renders "no active sessions", a null kv_headroom renders UNKNOWN/faint.
// Nothing here is fabricated — no invented sessions, workers, models or counts.
async function renderSessions(){
  const box = $('sessions');
  if (!box) return;
  const { ok, status, j } = await apiFetch('/v1/sessions', { headers });
  const count = $('sessions-count');
  if (!ok) {
    if (count) count.textContent = '';
    box.innerHTML = '<tr><td colspan="6"><span class="badge warn">sessions unavailable (' + status + ')</span> ' +
      esc((j && j.error && j.error.message) || 'unknown error') + '</td></tr>';
    return;
  }
  const active = (j && j.sessions_active) || 0;
  if (count) count.textContent = active;
  const sessions = (j && j.sessions) || [];
  if (active === 0 || sessions.length === 0) {
    box.innerHTML = '<tr><td colspan="6" class="empty">no active sessions</td></tr>';
    return;
  }
  const rows = sessions.map(s => {
    const cap = (s.capacity || 0) > 0 ? s.capacity : 0;
    const used = (s.tokens_used !== undefined && s.tokens_used !== null) ? s.tokens_used : null;
    const hk = (s.kv_headroom !== undefined && s.kv_headroom !== null) ? s.kv_headroom : null;
    const usage = cap > 0 ? (used !== null ? used + ' / ' + cap : '&mdash; / ' + cap) : '&mdash;';
    const head = hk === null || cap === 0
      ? '<span class="badge faint">UNKNOWN</span>'
      : hk < cap * 0.8
        ? '<span class="badge ok">headroom ' + hk + '</span>'
        : '<span class="badge warn">near capacity · ' + hk + '</span>';
    return '<tr><td><code>' + short(s.session_id, 12) + '</code></td>' +
      '<td><code>' + short(s.worker, 12) + '</code></td>' +
      '<td class="mono" style="font-size:11px">' + esc(short(s.model_hash, 12)) + '</td>' +
      '<td class="num">' + esc(usage) + '</td>' +
      '<td>' + head + '</td>' +
      '<td><button class="btn small" onclick="continueSession(\'' + jsq(s.session_id) + '\')">continue</button></td></tr>';
  }).join('');
  box.innerHTML = rows;
}
// CONTINUE (KV locality): pre-populate the Decision card's execute inputs with
// a real coordinator-tracked session_id so the operator can start a
// continuation on the KV-prefix worker via the existing /v1/execute path.
// Real state only: nothing runs here — it only loads the session id (and a
// default intent/prompt when those are empty); execution happens solely
// through the confirmed Execute button.
function continueSession(sid){
  const s = $('dec-session');
  if (s) s.value = sid || '';
  const intent = $('dec-intent');
  if (intent && !(intent.value || '').trim()) intent.value = 'continue the conversation';
  const prompt = $('dec-prompt');
  if (prompt && !(prompt.value || '').trim()) prompt.value = 'continue this conversation…';
  show('models');
  toast('session loaded: ' + short(sid, 12) + ' — now Execute to continue with KV locality');
  if (prompt) prompt.focus();
}
// CAN I RUN THIS? — fabric-wide capability fit via the real /v1/can_run
// projection (same pure engine as the MCP get_worker_capability tool).
async function canIRun(){
  const model = ($('cir-model').value || '').trim();
  const cap = ($('cir-cap').value || '').trim();
  const ev = ($('cir-ev')||{}).value || 'any';
  if (!model || !cap) { toast('enter a model and a capability', true); return; }
  $('cir-note').textContent = 'checking…';
  const { ok, status, j } = await apiFetch('/v1/can_run?model='+encodeURIComponent(model)+'&capability='+encodeURIComponent(cap)+'&evidence='+encodeURIComponent(ev), { headers });
  $('cir-note').textContent = '';
  if (!ok) { $('cir-result').innerHTML = '<span class="badge warn">check failed (' + status + ')</span> ' + esc((j && j.error && j.error.message) || 'unknown'); return; }
  renderCanIRun(j, 'cir-result', model, cap);
}
// Shared renderer for the real /v1/can_run payload `j`. Renders the honest
// verdict (badge + counts + chosen worker + reasons + per-worker blockers)
// into whatever container id is passed. Used by both canIRun() and the
// Model card's fabric fit button (hubCanIRunLocal).
function renderCanIRun(j, containerId, model, cap){
  const fit = (j && j.fit) || {};
  const badge = fit.verdict === 'CAN_RUN' ? '<span class="badge ok">CAN RUN</span>'
    : fit.verdict === 'CANNOT_RUN' ? '<span class="badge bad">CANNOT RUN</span>'
    : '<span class="badge warn">UNKNOWN</span>';
  const counts = fit.counts || {};
  const reasons = (fit.reasons || []).map(r => '<div class="mono" style="font-size:11px;margin-top:2px">• '+esc(r)+'</div>').join('');
  const chosen = fit.chosen_worker ? '<div style="margin-top:6px">chosen worker: <code>'+esc(short(fit.chosen_worker, 16))+'</code></div>' : '';
  let perWorker = '';
  (j.workers || []).forEach(w => {
    const wv = w.verdict === 'CAN_RUN' ? '<span class="badge ok">CAN_RUN</span>' : w.verdict === 'CANNOT_RUN' ? '<span class="badge bad">CANNOT_RUN</span>' : '<span class="badge warn">UNKNOWN</span>';
    const id = (w.worker || {});
    const checks = (w.checks || []).filter(c => !c.pass).map(c =>
      '<li style="font-size:11px"><span class="warn">✗</span> '+esc(c.check)+' — '+esc(c.state)+'</li>').join('');
    perWorker += '<div style="margin-top:8px;border-top:1px dashed var(--border);padding-top:6px">'+
      '<code>'+esc(short(id.node_id||id.peer_id||'', 14))+'</code> · '+esc(id.node_name||'')+' · '+wv+' · '+
      'model '+esc(w.model_availability||'')+' · '+(w.trusted?'<span class="badge ok">trusted</span>':'<span class="badge warn">untrusted</span>')+' · engine '+esc(w.engine||'')+
      (checks?'<ul style="margin:4px 0 0 14px;padding:0">'+checks+'</ul>':'')+'</div>';
  });
  const con = $(containerId);
  if (!con) return;
  con.innerHTML =
    '<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">'+badge+' <span class="mono">'+esc(cap)+' · '+esc(model)+'</span> '+
    '<span class="mono" style="color:var(--faint)">'+counts.can_run+' can / '+counts.cannot_run+' cannot / '+counts.unknown+' unknown</span></div>'+
    reasons + chosen +
    (j.workers && j.workers.length ? '<div style="margin-top:8px"><b style="font-size:11px">per worker</b>'+perWorker+'</div>' : '')+
    (!j.workers || !j.workers.length ? '<div class="mono" style="margin-top:6px">no workers on the fabric</div>' : '');
}

// DECISION (Phase 3): "What should I run?" — the ONE coherent fabric decision
// from the real /v1/decision projection. Progressive disclosure: decision
// banner + why first, then per-capability model options, then historical.
// Only real backend state is shown; empty capabilities/options/history render
// as honest empty/UNKNOWN (never fabricated verdicts, workers or metrics).
async function decideNow(){
  const intent = ($('dec-intent').value || '').trim();
  const ev = ($('dec-ev')||{}).value || 'any';
  if (!intent) { toast('enter an intent', true); return; }
  $('dec-result').innerHTML = '<span class="badge faint">deciding…</span>';
  const { ok, status, j } = await apiFetch('/v1/decision?intent='+encodeURIComponent(intent)+'&evidence='+encodeURIComponent(ev), { headers });
  const con = $('dec-result');
  if (!con) return;
  if (!ok) {
    con.innerHTML = '<span class="badge warn">decision failed (' + status + ')</span> ' + esc((j && j.error && j.error.message) || 'unknown');
    return;
  }
  // ---- progressive disclosure part 1: the decision banner + why ----
  let html = '';
  if (j.decision) {
    html += '<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">'+
      '<span class="badge ok">DECISION: '+esc(j.decision.model||'')+' on '+esc(short(j.decision.worker||'', 16))+
      ' (cap '+esc(j.decision.capability||'')+')</span></div>';
  } else {
    html += '<span class="badge warn">no runnable decision on the fabric</span>';
  }
  const why = (j.why || []).map(r => '<li style="font-size:11px;margin-top:2px">'+esc(r)+'</li>').join('');
  if (why) { html += '<ul style="margin:8px 0 0 14px;padding:0;list-style:none">'+why+'</ul>'; }

  // ---- part 2: capabilities -> model options (progressive disclosure) ----
  const caps = (j.capabilities || []);
  if (caps.length) {
    let capBlock = '';
    caps.forEach(c => {
      capBlock += '<div style="margin-top:10px;border-top:1px dashed var(--line);padding-top:8px"><b style="font-size:11px">'+esc(c.label || c.capability || '')+'</b> '+
        '<span class="mono" style="font-size:10px;color:var(--faint)">'+esc(c.capability||'')+' · evidence '+esc(c.evidence||'')+'</span>';
      const opts = (c.model_options || []);
      if (!opts.length) {
        capBlock += '<div class="mono" style="margin-top:4px;font-size:11px;color:var(--faint)">no model options reported for this capability</div>';
      } else {
        opts.forEach(o => {
          const v = o.verdict === 'CAN_RUN' ? '<span class="badge ok">CAN_RUN</span>'
            : o.verdict === 'CANNOT_RUN' ? '<span class="badge bad">CANNOT_RUN</span>'
            : '<span class="badge warn">UNKNOWN</span>';
          const workers = (o.can_run_workers || []);
          const wLine = workers.length
            ? workers.map(w =>
                '<span class="badge '+(w.trusted?'ok':'faint')+'">'+esc(short(w.node_id||w.peer_id||'', 12))+' · '+
                esc(w.node_name||'')+' · '+(w.trusted?'trusted':'untrusted')+' · '+esc(w.engine||'')+'</span>').join(' ')
            : '<span class="badge faint">no runnable worker</span>';
          capBlock += '<div style="margin-top:6px;padding-left:4px"><span class="mono" style="font-size:11px">'+esc(o.model||'')+' · '+
            esc(o.quantization || '—')+'</span> '+v+'<div style="margin-top:3px;display:flex;gap:4px;flex-wrap:wrap">'+wLine+'</div></div>';
        });
      }
      capBlock += '</div>';
    });
    html += capBlock;
  } else {
    html += '<div class="mono" style="margin-top:8px;color:var(--faint)">no capabilities matched the intent</div>';
  }

  // ---- part 3: historical (collapsible, honest) ----
  const hist = (j.historical || {});
  let histLine = '';
  if (hist.records > 0) {
    const out = hist.outcomes || {};
    const m = hist.measured || {};
    histLine = 'records '+esc(String(hist.records))+
      ' · succeeded '+esc(String(out.succeeded ?? 0))+' / failed '+esc(String(out.failed ?? 0))+
      ((m.avg_tokens_per_sec != null) ? ' · avg '+esc(String(m.avg_tokens_per_sec))+' tok/s' : '')+
      ((m.avg_latency_ms != null) ? ' · avg '+esc(String(m.avg_latency_ms))+' ms' : '');
  } else {
    histLine = 'insufficient history (UNKNOWN)';
  }
  html += '<div style="margin-top:12px;border-top:1px dashed var(--line);padding-top:8px"><b style="font-size:11px">historical</b>'+
    '<div class="mono" style="font-size:11px;color:var(--muted);margin-top:3px">'+histLine+'</div></div>';

  con.innerHTML = html;
}

// EXECUTE (T3): run the decided intent on the fabric with explicit UI
// confirmation and streamed output. MUTATING: gated on the master token
// (headers) and `confirm: true`. Only real backend state/streamed output is
// rendered — never fabricated output, workers or token counts. Confirmation is
// a genuine confirm() dialog, matching the backend's confirm:true requirement.
async function executeDecision(){
  const intent = ($('dec-intent').value || '').trim();
  const prompt = ($('dec-prompt').value || '').trim();
  const ev = ($('dec-ev')||{}).value || 'any';
  const maxRaw = $('dec-max').value || '256';
  const maxTokens = parseInt(maxRaw, 10);
  const stream = !!($('dec-stream')||{}).checked;
  const model = ($('dec-model').value || '').trim();
  const sessionId = ($('dec-session').value || '').trim();
  if (!intent) { toast('enter an intent to execute', true); return; }
  if (!prompt) { toast('enter a prompt to execute', true); return; }
  const mt = (Number.isFinite(maxTokens) && maxTokens > 0) ? maxTokens : 256;
  if (!confirm('Run this on the fabric? This reserves a worker and runs real inference.')) return;
  const body = { intent, prompt, max_tokens: mt, stream, evidence: ev, confirm: true };
  if (model) body.model = model;
  if (sessionId) body.session_id = sessionId;
  const con = $('dec-exec');
  if (!con) return;
  con.innerHTML = '<span class="badge ok">EXECUTING…</span> <span class="mono" style="color:var(--faint)">'+esc(intent)+'</span>';
  const authHeaders = Object.assign({}, headers, { 'Content-Type': 'application/json' });

  if (stream) {
    // ---- streaming path: read the SSE body incrementally ----
    let resp;
    try {
      resp = await fetch('/v1/execute', { method: 'POST', headers: authHeaders, body: JSON.stringify(body) });
    } catch (e) {
      con.innerHTML = '<span class="badge bad">execute failed</span> <span class="mono">'+esc(e && e.message ? e.message : 'network error')+'</span>';
      return;
    }
    const ct = (resp.headers.get('content-type') || '');
    if (!resp.ok || ct.indexOf('text/event-stream') !== 0) {
      // non-SSE (JSON) error payload — parse and render error + replan honestly
      let j = {};
      try { j = await resp.json(); } catch (e) { j = { error: { message: 'HTTP ' + resp.status } }; }
      renderExecError(con, j, resp.ok ? 200 : resp.status);
      return;
    }
    let outHtml = '<span class="badge ok">streaming…</span><div class="mono" style="margin-top:6px;white-space:pre-wrap">';
    let done = false;
    try {
      const reader = resp.body.getReader(), dec = new TextDecoder();
      let buffer = '';
      for (;;) {
        const { done: d, value } = await reader.read();
        if (d) break;
        buffer += dec.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';
        for (const raw of lines) {
          const line = raw.trim();
          if (!line.startsWith('data:')) continue;
          const payload = line.slice(5).trim();
          if (payload === '[DONE]') { done = true; break; }
          let ev; try { ev = JSON.parse(payload); } catch (e) { continue; }
          if (ev.error && ev.error.message) {
            outHtml += '</div><div style="margin-top:6px"><span class="badge bad">stream error</span> '+esc(ev.error.message)+'</div>';
            con.innerHTML = outHtml; return;
          }
          const delta = ev.choices && ev.choices[0] && ev.choices[0].delta && ev.choices[0].delta.content;
          if (delta) outHtml += esc(delta);
          if (ev.usage && ev.usage.completion_tokens != null) outHtml += '</div><div style="margin-top:6px" class="mono">'+esc(String(ev.usage.completion_tokens))+' tokens</div>';
        }
        con.innerHTML = outHtml;
      }
    } catch (e) {
      outHtml += '</div><div style="margin-top:6px"><span class="badge bad">stream interrupted</span> '+esc(e && e.message ? e.message : String(e))+'</div>';
      con.innerHTML = outHtml; return;
    }
    outHtml += done ? '</div><div style="margin-top:6px"><span class="badge faint">[DONE]</span></div>' : '</div><div style="margin-top:6px"><span class="badge warn">stream closed without [DONE]</span></div>';
    con.innerHTML = outHtml;
    return;
  }

  // ---- non-streaming path: JSON response via apiFetch ----
  const { ok, status, j } = await apiFetch('/v1/execute', { method: 'POST', headers: authHeaders, body: JSON.stringify(body) });
  if (ok && j && j.executed) {
    const ex = j.executed;
    con.innerHTML =
      '<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">'+
        '<span class="badge ok">EXECUTED</span>'+
        '<span class="mono">'+esc(ex.model||'')+'</span>'+
        (ex.worker ? '<span class="badge faint">'+esc(short(ex.worker, 16))+'</span>' : '')+
        '<span class="mono" style="color:var(--faint)">'+esc(String(ex.tokens_used ?? '—'))+' tokens</span></div>'+
      '<div class="mono" style="margin-top:8px;white-space:pre-wrap;color:var(--fg)">'+esc(ex.output||'')+'</div>';
  } else {
    renderExecError(con, j, status);
  }
}

// PREVIEW (T2): dry-run of /v1/execute. Read-only — never reserves a worker,
// never runs inference. Still sends `confirm: true` + `dry_run: true` to pass
// the backend's mutation-path gate, but no real confirm() dialog is needed
// (a preview is safe). Renders exactly what the backend reported as
// `would_execute`; never fabricates worker/plan/estimates.
async function previewDecision(){
  const intent = ($('dec-intent').value || '').trim();
  const prompt = ($('dec-prompt').value || '').trim();
  const ev = ($('dec-ev')||{}).value || 'any';
  const maxRaw = $('dec-max').value || '256';
  const maxTokens = parseInt(maxRaw, 10);
  const model = ($('dec-model').value || '').trim();
  const sessionId = ($('dec-session').value || '').trim();
  if (!intent) { toast('enter an intent to preview', true); return; }
  if (!prompt) { toast('enter a prompt to preview', true); return; }
  const mt = (Number.isFinite(maxTokens) && maxTokens > 0) ? maxTokens : 256;
  const body = { intent, prompt, max_tokens: mt, evidence: ev, dry_run: true, confirm: true };
  if (model) body.model = model;
  if (sessionId) body.session_id = sessionId;
  const pv = $('dec-preview');
  if (!pv) return;
  pv.innerHTML = '<span class="badge ok">previewing…</span> <span class="mono" style="color:var(--faint)">'+esc(intent)+'</span>';
  const authHeaders = Object.assign({}, headers, { 'Content-Type': 'application/json' });
  const { ok, status, j } = await apiFetch('/v1/execute', { method: 'POST', headers: authHeaders, body: JSON.stringify(body) });
  if (!ok) {
    // Honest error render: the real backend message, plus the explicit
    // "no eligible worker on the fabric" case. Never invent a plan.
    const err = (j && j.error && j.error.message) || (j && j.error && j.error.type) || ('HTTP ' + status);
    pv.innerHTML = '<span class="badge bad">preview failed ('+status+')</span> <span class="mono">'+esc(String(err))+'</span>'+
      '<div style="margin-top:6px" class="mono"><span class="badge faint">no request sent · no worker reserved</span></div>';
    return;
  }
  if (!(j && j.dry_run)) {
    pv.innerHTML = '<span class="badge bad">unexpected response</span> <span class="mono">dry-run flag missing — '+esc(String(j && j.error ? (j.error.message || 'error') : 'no would_execute'))+'</span>';
    return;
  }
  const w = (j.would_execute) || {};
  pv.innerHTML =
    '<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">'+
      '<span class="badge warn">DRY-RUN</span>'+
      '<span class="badge faint">preview only</span>'+
      '<span class="mono">'+esc(w.model||'—')+'</span>'+
      (w.worker ? '<span class="badge faint">'+esc(short(w.worker, 16))+'</span>' : '')+
      (w.estimated_ms != null ? '<span class="mono" style="color:var(--faint)">~'+esc(String(w.estimated_ms))+' ms</span>' : '')+
      (w.plan_id ? '<span class="mono" style="color:var(--faint)">plan '+esc(short(w.plan_id, 12))+'</span>' : '')+
    '</div>'+
    (w.model_hash ? '<div class="mono" style="color:var(--faint)">model hash '+esc(short(w.model_hash, 16))+'</div>' : '')+
    (w.stages && w.stages.length
      ? '<div class="mono" style="margin-top:4px;color:var(--fg)">stages: '+esc(String(w.stages.join(' → ')))+'</div>'
      : '')+
    '<div style="margin-top:6px" class="mono"><span class="badge faint">dry-run · no request sent · no reservation held</span></div>'+
    ((j.note) ? '<div style="margin-top:4px" class="mono">'+esc(String(j.note))+'</div>' : '');
}

// Shared honest error renderer for /v1/execute failures: the real error
// message plus, when present, the replan advisory (never fabricated).
function renderExecError(con, j, status){
  if (!con) return;
  const err = (j && j.error && j.error.message) || (j && j.error && j.error.type) || ('HTTP ' + status);
  const msg = String(err || '');
  const missingConfirm = /confirm/i.test(msg);
  const replan = (j && j.replan) || {};
  const replanHtml = (replan.advisory)
    ? '<div style="margin-top:6px" class="mono"><span class="badge warn">replan: '+esc(String(replan.advisory))+'</span>'+
      (replan.retryable != null ? ' · retryable '+esc(String(replan.retryable)) : '')+
      ((replan.eligible_alternatives && replan.eligible_alternatives.length) ? ' · '+esc(String(replan.eligible_alternatives.length))+' eligible' : '')+
      (replan.note ? ' · '+esc(String(replan.note)) : '')+'</div>'
    : '';
  con.innerHTML = '<span class="badge bad">execute failed (' + status + ')</span> <span class="mono">'+esc(msg)+'</span>'+
    (missingConfirm ? '<div style="margin-top:6px" class="mono"><span class="badge warn">mutation refused: confirm not accepted</span></div>' : '')+
    replanHtml;
}

function renderModels(s, c){  const served = (s && s.node && s.node.served_models || []);
  const rows = served.map(m =>
    '<tr><td>'+esc(m.name||'')+'</td><td>'+esc((s && s.node && s.node.engine)||'')+'</td><td class="num">'+(m.context_tokens||'—')+'</td><td class="num">'+fmtMB(m.est_ram_mb)+'</td><td class="num">'+(m.est_vram_mb?fmtMB(m.est_vram_mb):'—')+'</td><td>'+(s.model===m.name?'<span class="badge ok">loaded</span>':'<span class="badge faint">-</span>')+'</td></tr>'
  ).join('');
  $('models').innerHTML = rows || '<tr><td colspan="6" class="empty">no served models advertised</td></tr>';
  $('models-count').textContent = served.length;
  $('models-status').innerHTML = 'active model: '+esc(s.model||'')+(s.model_loaded?' · <span class="badge ok">loaded</span>':' · <span class="badge faint">unloaded</span>');
  const reg = (s && s.available_models || []);
  $('registry-models').innerHTML = reg.map(m =>
    '<tr><td>'+esc(m.name)+'</td><td class="num">'+(m.size_bytes/1073741824).toFixed(2)+' GiB</td><td>'+
    '<button class="btn small warn" onclick="removeModel(\''+jsq(m.name)+'\')">Delete</button></td></tr>'
  ).join('') || '<tr><td colspan="3" class="empty">no indexed models</td></tr>';
  // On-disk models reported by every fabric worker (Part 3/17): the honest
  // "could serve" set, distinct from what is currently loaded.
  const workers = (c && c.workers || []);
  const disk = [];
  workers.forEach(w => {
    (w.available_models || []).forEach(m => {
      disk.push({
        file: m.file_name || '', size: m.size_mb || 0,
        node: w.node_name || w.node_id || short(w.peer_id, 12),
        serving: (w.served_models || []).some(sm => sm.file_name === m.file_name),
      });
    });
  });
  disk.sort((a, b) => a.file.localeCompare(b.file));
  $('disk-models').innerHTML = disk.map(d =>
    '<tr><td>'+esc(d.file)+'</td><td>'+esc(d.node)+'</td><td class="num">'+fmtMB(d.size)+'</td><td>'+
    (d.serving ? '<span class="badge ok">serving</span>' : '<span class="badge faint">on disk</span>')+'</td></tr>'
  ).join('') || '<tr><td colspan="4" class="empty">no on-disk models reported by workers</td></tr>';
  $('disk-models-count').textContent = disk.length;
  $('disk-models-status').innerHTML = disk.length
    ? 'workers report models they can swap in on request — ask for any of these by file name'
    : 'workers only report models they currently serve';
}
// Model Hub (Part 16/22): search + pull against the master-gated admin
// endpoints. `hubResults` state is local; a pull is long-running so the
// button shows a spinner and the row is locked until it resolves.
let hubPulling = {};
let hubSelectedModels = {};
function hubToggleCompare(id, chk){
  if (chk.checked) { hubSelectedModels[id] = true; } else { delete hubSelectedModels[id]; }
  updateCompareUI();
}
function updateCompareUI(){
  const keys = Object.keys(hubSelectedModels);
  const count = keys.length;
  const btn = $('hub-compare-btn');
  if (btn) {
    btn.disabled = count === 0;
    btn.textContent = 'Compare Selected (' + count + ')';
  }
  const status = $('hub-compare-status');
  if (status) {
    status.textContent = count > 0 ? count + ' model(s) selected for side-by-side comparison' : 'select 1 or more models to compare side-by-side';
  }
}
function hubClearCompare(){
  hubSelectedModels = {};
  document.querySelectorAll('.hub-compare-chk').forEach(el => el.checked = false);
  updateCompareUI();
  hubCloseCompare();
}
function hubCloseCompare(){
  const panel = $('hub-compare-panel');
  if (panel) { panel.style.display = 'none'; }
}
async function apiFetch(url, options={}){
  try {
    const r = await fetch(url, options);
    const text = await r.text();
    let j = {};
    try { j = JSON.parse(text); } catch (e) { j = { error: { message: text || r.statusText || 'HTTP ' + r.status } }; }
    return { ok: r.ok, status: r.status, j };
  } catch (e) {
    return { ok: false, status: 0, j: { error: { message: e.message || 'network error' } } };
  }
}
function hubCompareSelected(){
  const keys = Object.keys(hubSelectedModels);
  if (keys.length === 0) { toast('select at least one model to compare', true); return; }
  const query = keys.map(k => encodeURIComponent(k)).join(',');
  hubCompareRepos = keys.slice();
  $('hub-status').innerHTML = 'loading comparison for '+keys.length+' model(s)…';
  apiFetch('/api/admin/hub/compare?repos=' + query, { headers }).then(({ ok, status, j }) => {
    if (!ok) { $('hub-status').innerHTML = '<span class="badge warn">comparison failed ('+status+')</span> ' + esc((j.error && j.error.message) || 'unknown error'); return; }
    const models = j.models || [];
    renderComparisonTable(models);
    // Capability fit: populate the dropdown from the KNOWN taxonomy (all
    // snake_case capability labels), not from invented state.
    var fitSel = $('compare-fit-cap');
    if (fitSel && fitSel.options.length === 0) {
      var known = ['ocr','vision','coding','summarization','translation','embeddings','tool_calling','structured_output','reasoning','speech_to_text','text_to_speech','image_generation','classification','multimodal'];
      known.forEach(function(k){ var o = document.createElement('option'); o.value = k; o.textContent = k; fitSel.appendChild(o); });
    }
    $('compare-fit').innerHTML = '';
    $('compare-fit-note').textContent = '';
    $('hub-compare-panel').style.display = 'block';
    $('compare-panel-count').textContent = models.length;
    $('hub-status').innerHTML = 'comparison loaded successfully';
  });
}
var hubCompareRepos = [];
function renderComparisonTable(models){
  if (!models.length) { $('hub-compare-content').innerHTML = '<div class="empty">no models to compare</div>'; return; }
  let html = '<table style="width:100%;border-collapse:collapse;font-size:12px"><thead><tr style="border-bottom:1px solid var(--border)"><th>Attribute</th>';
  models.forEach(m => {
    html += '<th style="text-align:left;padding:6px;min-width:200px"><b>'+esc(m.id || m.error)+'</b></th>';
  });
  html += '</tr></thead><tbody>';

  html += '<tr style="border-bottom:1px solid var(--border)"><td style="font-weight:600;padding:6px">Pipeline &amp; Tags</td>';
  models.forEach(m => {
    const md = m.metadata || {};
    const tags = (md.tags || []).slice(0, 5).join(', ');
    html += '<td style="padding:6px"><span class="badge">'+esc(md.pipeline_tag || '—')+'</span><div class="sub" style="font-size:11px;margin-top:2px">'+esc(tags || 'no tags')+'</div></td>';
  });
  html += '</tr>';

  html += '<tr style="border-bottom:1px solid var(--border)"><td style="font-weight:600;padding:6px">Specs / License</td>';
  models.forEach(m => {
    const md = m.metadata || {};
    html += '<td style="padding:6px">License: <b>'+esc(md.license || 'Unknown')+'</b><br>Params: <b>'+esc(md.params || '—')+'</b><br>Context: <b>'+(md.context_length || '—')+'</b><br>Downloads: '+nfmt(md.downloads || 0)+' · Likes: '+(md.likes || 0)+'</td>';
  });
  html += '</tr>';

  html += '<tr style="border-bottom:1px solid var(--border)"><td style="font-weight:600;padding:6px">Capabilities</td>';
  models.forEach(m => {
    const claims = (m.capabilities && m.capabilities.claims) || [];
    const capsStr = claims.length ? claims.map(c => '<span class="badge" title="provenance: '+esc(c.provenance)+'">'+esc(c.label)+' ('+esc(c.provenance)+')</span>').join(' ') : '<span class="muted">none</span>';
    html += '<td style="padding:6px">'+capsStr+'</td>';
  });
  html += '</tr>';

  html += '<tr style="border-bottom:1px solid var(--border)"><td style="font-weight:600;padding:6px">GGUF Variants &amp; Fit</td>';
  models.forEach(m => {
    const variants = m.variants || [];
    const vStr = variants.length ? variants.map(v => 
      '<div style="margin-bottom:6px;border-bottom:1px dashed var(--border);padding-bottom:4px">'+
      '<b class="mono">'+esc(v.file)+'</b><br>'+
      'Size: '+fmtMB((v.size_bytes||0)/1048576)+' · Est. RAM: ~'+fmtMB(v.est_ram_mb)+'<br>'+
      '<span class="badge '+(v.fit_classification==='BEST FIT'||v.fit_classification==='GOOD FIT'?'ok':'warn')+'">'+esc(v.fit_classification)+'</span>'+
      (v.local_fit ? ' <span class="badge ok">local fit</span>' : ' <span class="badge warn">exceeds local RAM</span>')+
      '</div>'
    ).join('') : '<span class="muted">no variants</span>';
    html += '<td style="padding:6px">'+vStr+'</td>';
  });
  html += '</tr>';

  html += '<tr style="border-bottom:1px solid var(--border)"><td style="font-weight:600;padding:6px">Fit Trade-offs &amp; Why</td>';
  models.forEach(m => {
    const variants = m.variants || [];
    let reasonsHtml = '';
    variants.forEach(v => {
      const reasons = v.fit_reasons || [];
      reasonsHtml += '<div style="margin-bottom:6px"><b class="mono" style="font-size:10px">'+esc(v.file)+'</b><ul style="margin:2px 0 0 14px;padding:0;font-size:11px">';
      reasons.forEach(r => {
        reasonsHtml += '<li style="color:'+(r.pass?'var(--ok, #22c55e)':'var(--warn, #f59e0b)')+'">'+(r.pass?'✓':'✗')+' <b>'+esc(r.check)+'</b>: '+esc(r.reason)+' <span style="font-size:9px;color:var(--muted)">['+esc(r.provenance)+']</span></li>';
      });
      reasonsHtml += '</ul></div>';
    });
    html += '<td style="padding:6px">'+(reasonsHtml || '<span class="muted">—</span>')+'</td>';
  });
  html += '</tr>';

  html += '<tr style="border-bottom:1px solid var(--border)"><td style="font-weight:600;padding:6px">Fabric Nodes</td>';
  models.forEach(m => {
    const fabric = m.fabric || [];
    const fStr = fabric.length ? fabric.map(f =>
      '<div style="margin-top:2px">'+esc(f.node_name || f.node_id)+' ('+esc(f.status)+') '+
      (f.served ? '<span class="badge ok">served</span>' : '')+
      (f.available && !f.served ? '<span class="badge">on disk</span>' : '')+
      (!f.trusted ? ' <span class="badge warn">untrusted</span>' : '')+'</div>'
    ).join('') : '<span class="muted">not present on fabric nodes</span>';
    html += '<td style="padding:6px">'+fStr+'</td>';
  });
  html += '</tr>';

  html += '</tbody></table>';
  $('hub-compare-content').innerHTML = html;
}
// Capability fit check for the comparison view: re-fetch with `requires=<cap>`
// and render the honest provenance-aware verdict (VERIFIED vs INFERRED vs
// MISSING) for EACH compared model. Mirrors the model-card hubCheckFit honesty
// rules: an INFERRED claim never satisfies a VERIFIED requirement.
async function hubCompareFit(){
  var cap = ($('compare-fit-cap') || {}).value || '';
  if (!cap) return;
  if (!hubCompareRepos.length) { toast('compare models first', true); return; }
  var query = hubCompareRepos.map(k => encodeURIComponent(k)).join(',');
  $('compare-fit-note').textContent = 'checking '+esc(cap)+'…';
  const { ok, j } = await apiFetch('/api/admin/hub/compare?repos=' + query + '&requires=' + encodeURIComponent(cap), { headers });
  $('compare-fit-note').textContent = '';
  if (!ok) { $('compare-fit').innerHTML = '<span class="badge warn">check failed</span>'; return; }
  var models = j.models || [];
  if (!models.length) { $('compare-fit').innerHTML = '<span class="muted">no models to compare</span>'; return; }
  var html = '';
  models.forEach(function(m){
    var fit = (m.capabilities && m.capabilities.fit) || null;
    if (!fit) { html += '<div style="margin-bottom:8px"><b>'+esc(m.id || m.error)+'</b>: <span class="muted">no verdict available</span></div>'; return; }
    var badge = fit.satisfied ? '<span class="badge ok">satisfied</span>' : '<span class="badge warn">not satisfied</span>';
    var checks = (fit.checks || []).map(function(c){
      var sat = (c.status && c.status.satisfied) ? true : false;
      var color = sat ? 'var(--ok,#22c55e)' : 'var(--warn,#f59e0b)';
      var mark = sat ? '✓' : '✗';
      return '<div style="margin-top:3px"><span style="color:'+color+'">'+mark+'</span> <b class="mono">'+esc(c.capability)+'</b>: '+esc(c.reason||'')+'</div>';
    }).join('');
    html += '<div style="margin-bottom:10px;border-bottom:1px dashed var(--border);padding-bottom:6px">'+
      '<b>'+esc(m.id || m.error)+'</b>: '+badge+' — requires <b class="mono">'+esc(fit.label||fit.capability)+'</b> (VERIFIED evidence)'+checks+'</div>';
  });
  $('compare-fit').innerHTML = html;
}
async function hubSearch(){
  const q = ($('hub-q').value || '').trim();
  if (!q) { toast('type a model name to search', true); return; }
  $('hub-status').innerHTML = 'searching HuggingFace for “'+esc(q)+'”…';
  const { ok, status, j } = await apiFetch('/api/admin/hub/search?query='+encodeURIComponent(q)+'&limit=8', { headers });
  if (!ok) {
    $('hub-status').innerHTML = '<span class="badge warn">search failed (' + status + ')</span> ' + esc((j.error && j.error.message) || 'unknown error');
    return;
  }
  const models = j.models || [];
  const rows = models.map(m =>
    '<tr><td><input type="checkbox" class="hub-compare-chk" value="'+esc(m.id)+'" '+(hubSelectedModels[m.id]?'checked':'')+
    ' onchange="hubToggleCompare(\''+jsq(m.id)+'\', this)"></td>'+
    '<td>'+esc(m.id)+'</td><td>'+esc(m.pipeline_tag || '—')+'</td><td class="num">'+nfmt(m.downloads || 0)+'</td><td style="white-space:nowrap">'+
    '<button class="btn small" onclick="hubOpenDetail(\''+jsq(m.id)+'\')">Details</button> '+
    '<button class="btn small" id="hub-pull-'+safeId(m.id)+'" onclick="hubPull(\''+jsq(m.id)+'\')">Pull</button></td></tr>'
  ).join('');
  $('hub-results').innerHTML = rows || '<tr><td colspan="5" class="empty">no GGUF models found for “'+esc(q)+'”</td></tr>';
  $('hub-status').innerHTML = 'search returned '+models.length+' GGUF model(s)';
}
async function hubPull(id){
  if (hubPulling[id]) return;
  hubPulling[id] = true;
  const btn = $('hub-pull-'+safeId(id));
  if (btn) { btn.disabled = true; btn.textContent = 'pulling…'; }
  $('hub-status').innerHTML = 'downloading '+esc(id)+' — large models take a while; the node keeps serving';
  const { ok, status, j } = await apiFetch('/api/admin/hub/pull', {
    method: 'POST', headers: Object.assign({}, headers, { 'Content-Type': 'application/json' }),
    body: JSON.stringify({ reference: 'hf:' + id }),
  });
  if (!ok) {
    $('hub-status').innerHTML = '<span class="badge warn">pull failed (' + status + ')</span> ' + esc((j.error && j.error.message) || 'unknown error');
  } else {
    $('hub-status').innerHTML = '<span class="badge ok">pulled</span> '+esc(j.reference)+' — '+fmtMB(j.bytes / 1048576)+' · sha256 <code>'+esc(short(j.sha256, 16))+'</code> — refresh the page to see it in the registry';
    toast('model pulled: ' + short(j.reference, 24));
    refresh();
  }
  delete hubPulling[id];
  if (btn) { btn.disabled = false; btn.textContent = 'Pull'; }
}
function safeId(s){ return String(s).replace(/[^a-zA-Z0-9_-]/g, '_'); }
// Escape a value for a single-quoted JS string literal inside a double-quoted
// HTML attribute (e.g. onclick="fn('…')"). Must neutralize BOTH contexts a
// hostile model/repo name can target:
//   - the JS string terminator `'` (and the JS escape backslash), and
//   - the HTML double-quoted attribute terminator `"` plus the HTML specials
//     &<> .
// Order matters: `&` is encoded first so the entities produced by later steps
// are never double-encoded.
function jsq(s){ return String(s ?? '')
  .replace(/&/g, '&amp;')
  .replace(/"/g, '&quot;')
  .replace(/</g, '&lt;')
  .replace(/>/g, '&gt;')
  .replace(/\\/g, '\\\\')
  .replace(/'/g, "\\'"); }
function nfmt(n){ return n >= 1000000 ? (n/1000000).toFixed(1)+'M' : n >= 1000 ? (n/1000).toFixed(1)+'k' : String(n); }
// Model card (Issue #26 §7–§8, §22, §31): fetch real Hub metadata + honest
// capability taxonomy + variants, and the live fabric view of who can run it.
// The fabric list comes from the node's own compute registry — never mocked.
async function hubOpenDetail(id){
  $('hub-status').innerHTML = 'loading model card for '+esc(id)+'…';
  const { ok, status, j } = await apiFetch('/api/admin/hub/model/'+encodeURIComponent(id), { headers });
  if (!ok) {
    $('hub-status').innerHTML = '<span class="badge warn">model card failed (' + status + ')</span> ' + esc((j.error && j.error.message) || 'unknown error');
    return;
  }
  const md = j.metadata || {};
  $('md-title').textContent = j.id || '';
  $('md-meta').textContent = [md.pipeline_tag, md.license, md.params ? md.params+' params' : '', md.context_length ? 'ctx '+md.context_length : '', md.downloads ? nfmt(md.downloads)+' downloads' : '', md.likes ? md.likes+' likes' : ''].filter(Boolean).join(' · ');
  $('md-desc').textContent = md.description || 'No description on the Hub.';
  const claims = j.capabilities && j.capabilities.claims || [];
  $('md-caps').innerHTML = claims.length ? claims.map(c =>
    '<span class="badge" title="provenance: '+esc(c.provenance)+'">'+esc(c.label)+'</span>'
  ).join('') : '<span class="muted">no capabilities classified</span>';
  const tasks = j.capabilities && j.capabilities.tasks || [];
  $('md-tasks').innerHTML = tasks.length ? tasks.map(t => esc(t.task)).join(' · ') : '—';
  const variants = j.variants || [];
  $('md-variants').innerHTML = variants.length ? variants.map(v =>
    '<tr><td class="mono">'+esc(v.file)+'<div class="sub" style="font-size:10px">'+(v.local_fit?'<span class="badge ok" style="font-size:9px">Can run locally (~'+fmtMB(v.est_ram_mb)+')</span>':'<span class="badge warn" style="font-size:9px">Needs ~'+fmtMB(v.est_ram_mb)+' RAM/VRAM</span>')+(v.fabric_fit_nodes && v.fabric_fit_nodes.length ? ' · '+v.fabric_fit_nodes.length+' fabric node(s) fit' : '')+'</div></td><td class="num">'+fmtMB((v.size_bytes||0)/1048576)+'</td><td class="mono">'+esc(short(v.sha256 || '', 12))+'</td><td>'+
    '<button class="btn small" id="hub-pull-'+safeId(j.id+':'+v.file)+'" onclick="hubPullVariant(\''+jsq(j.id)+'\',\''+jsq(v.file)+'\')">Pull</button></td></tr>'
  ).join('') : '<tr><td colspan="4" class="empty">no variants reported</td></tr>';
  const fabric = j.fabric || [];
  $('md-fabric-note').textContent = fabric.length ? ' — '+fabric.length+' node(s) have this model' : ' — no node has this model yet';
  $('md-fabric').innerHTML = fabric.length ? fabric.map(f =>
    '<div style="margin-top:4px">'+esc(f.node_name || f.node_id)+' · <code class="mono">'+esc(f.node_id)+'</code> · '+esc(f.status)+' · '+
    (f.served ? '<span class="badge ok">served</span>' : '')+(f.available && !f.served ? '<span class="badge">on disk</span>' : '')+
    (f.trusted ? '' : ' <span class="badge warn">untrusted</span>')+'</div>'
  ).join('') : 'pull it here to make it available fabric-wide';
  // Capability fit: populate the dropdown from the KNOWN taxonomy (all
  // snake_case capability labels), not from invented state.
  var fitSel = $('md-fit-cap');
  if (fitSel && fitSel.options.length === 0) {
    var known = ['ocr','vision','coding','summarization','translation','embeddings','tool_calling','structured_output','reasoning','speech_to_text','text_to_speech','image_generation','classification','multimodal'];
    known.forEach(function(k){ var o = document.createElement('option'); o.value = k; o.textContent = k; fitSel.appendChild(o); });
  }
  // On-disk variant fit selector: same KNOWN taxonomy (never invented), and
  // default the file name to the repo id (the backend matches by file-name
  // suffix, so the operator can correct it to the real on-disk file).
  var vfCap = $('md-vf-cap');
  if (vfCap && vfCap.options.length === 0) {
    var knownVf = ['ocr','vision','coding','summarization','translation','embeddings','tool_calling','structured_output','reasoning','speech_to_text','text_to_speech','image_generation','classification','multimodal'];
    knownVf.forEach(function(k){ var o = document.createElement('option'); o.value = k; o.textContent = k; vfCap.appendChild(o); });
  }
  if (vfCap) vfCap.value = (fitSel && fitSel.value) || '';
  var vfFile = $('md-vf-file');
  if (vfFile) vfFile.value = j.id || '';
  $('md-variant-fit').innerHTML = '';
  $('md-fit').innerHTML = '';
  $('md-fit-note').textContent = '';
  hubFitCache = j.id || '';
  $('hub-detail').style.display = 'block';
  $('hub-status').innerHTML = 'model card loaded';
}
var hubFitCache = '';
// Capability fit check: ask the model card "can this model do X?" and render
// the honest provenance-aware verdict (VERIFIED vs INFERRED vs MISSING).
async function hubCheckFit(){
  var cap = ($('md-fit-cap') || {}).value || '';
  if (!cap) return;
  var id = hubFitCache;
  $('md-fit-note').textContent = 'checking '+esc(cap)+'…';
  const { ok, j } = await apiFetch('/api/admin/hub/model/'+encodeURIComponent(id)+'?requires='+encodeURIComponent(cap), { headers });
  $('md-fit-note').textContent = '';
  if (!ok) { $('md-fit').innerHTML = '<span class="badge warn">check failed</span>'; return; }
  var fit = (j.capabilities && j.capabilities.fit) || null;
  if (!fit) { $('md-fit').innerHTML = '<span class="muted">no verdict available</span>'; return; }
  var badge = fit.satisfied ? '<span class="badge ok">satisfied</span>' : '<span class="badge warn">not satisfied</span>';
  var checks = (fit.checks || []).map(function(c){
    var color = (c.status && c.status.satisfied) ? 'var(--ok,#22c55e)' : 'var(--warn,#f59e0b)';
    var mark = (c.status && c.status.satisfied) ? '✓' : '✗';
    return '<div style="margin-top:3px"><span style="color:'+color+'">'+mark+'</span> <b class="mono">'+esc(c.capability)+'</b>: '+esc(c.reason||'')+'</div>';
  }).join('');
  $('md-fit').innerHTML = '<div>'+badge+' — requires <b class="mono">'+esc(fit.label||fit.capability)+'</b> (VERIFIED evidence)</div>'+checks;
}
// Fabric fit from LOCAL persisted claims (no Hub round-trip): asks the real
// /v1/can_run endpoint with the selected capability against the model's repo
// id. The backend matches by file-name suffix, so a repo id may not resolve
// to a specific variant — we render whatever honest result it returns,
// including "no workers on the fabric".
async function hubCanIRunLocal(){
  var cap = ($('md-fit-cap') || {}).value || '';
  var model = hubFitCache || '';
  if (!cap || !model) { toast('select a capability for the model card', true); return; }
  $('md-cir').innerHTML = '<span class="mono" style="font-size:11px;color:var(--faint)">checking '+esc(cap)+'…</span>';
  const { ok, status, j } = await apiFetch('/v1/can_run?model='+encodeURIComponent(model)+'&capability='+encodeURIComponent(cap)+'&evidence=any', { headers });
  if (!ok) { $('md-cir').innerHTML = '<span class="badge warn">check failed (' + status + ')</span> ' + esc((j && j.error && j.error.message) || 'unknown'); return; }
  renderCanIRun(j, 'md-cir', model, cap);
}
// CAN I RUN THIS? — on-disk variants (fabric). Fetches the real /v1/can_run
// projection for a typed on-disk file name and renders each real on-disk GGUF
// variant with its per-variant fabric verdict. Never fabricates: an empty
// variants array renders "no on-disk variants on this fabric", and backend
// errors render as-is.
async function loadVariantFit(){
  var file = (($('md-vf-file') || {}).value || '').trim();
  var cap = ($('md-vf-cap') || {}).value || '';
  var con = $('md-variant-fit');
  if (!file) { toast('enter an on-disk file name to check', true); return; }
  if (!con) return;
  con.innerHTML = '<span class="mono" style="font-size:11px;color:var(--faint)">checking '+esc(file)+'…</span>';
  const { ok, status, j } = await apiFetch('/v1/can_run?model='+encodeURIComponent(file)+'&capability='+encodeURIComponent(cap)+'&evidence=any', { headers });
  if (!ok) { con.innerHTML = '<span class="badge warn">check failed (' + status + ')</span> ' + esc((j && j.error && j.error.message) || 'unknown'); return; }
  var variants = (j && j.variants) || [];
  if (!variants.length) { con.innerHTML = '<span class="mono" style="font-size:11px;color:var(--muted)">no on-disk variants on this fabric</span>'; return; }
  con.innerHTML = variants.map(function(v){
    var q = v.quantization || '—';
    var size = v.size_bytes ? fmtMB(v.size_bytes / 1048576) : '—';
    var fit = (v.fit || {});
    var badge = fit.verdict === 'CAN_RUN' ? '<span class="badge ok">CAN_RUN</span>'
      : fit.verdict === 'CANNOT_RUN' ? '<span class="badge bad">CANNOT_RUN</span>'
      : '<span class="badge warn">UNKNOWN</span>';
    var counts = fit.counts || {};
    var reasons = (fit.reasons || []).map(function(r){ return '<div class="mono" style="font-size:11px;margin-top:2px">• '+esc(r)+'</div>'; }).join('');
    var chosen = fit.chosen_worker ? '<div style="margin-top:4px">chosen worker: <code>'+esc(short(fit.chosen_worker, 16))+'</code></div>' : '';
    var perWorker = '';
    (v.workers || []).forEach(function(w){
      var wv = w.verdict === 'CAN_RUN' ? '<span class="badge ok">CAN_RUN</span>' : w.verdict === 'CANNOT_RUN' ? '<span class="badge bad">CANNOT_RUN</span>' : '<span class="badge warn">UNKNOWN</span>';
      var id = (w.worker || {});
      var checks = (w.checks || []).filter(function(c){ return !c.pass; }).map(function(c){
        return '<li style="font-size:11px"><span class="warn">✗</span> '+esc(c.check)+' — '+esc(c.state)+'</li>';
      }).join('');
      perWorker += '<div style="margin-top:6px;border-top:1px dashed var(--border);padding-top:4px">'+
        '<code>'+esc(short(id.node_id||id.peer_id||'', 14))+'</code> · '+esc(id.node_name||'')+' · '+wv+' · '+
        'model '+esc(w.model_availability||'')+' · '+(w.trusted?'<span class="badge ok">trusted</span>':'<span class="badge warn">untrusted</span>')+' · engine '+esc(w.engine||'')+
        (checks?'<ul style="margin:4px 0 0 14px;padding:0">'+checks+'</ul>':'')+'</div>';
    });
    return '<div style="margin-top:8px;border-top:1px solid var(--border);padding-top:6px">'+
      '<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">'+
        '<span class="mono" style="font-size:11.5px">'+esc(v.file||'')+'</span> '+badge+
        ' <span class="mono" style="font-size:11px;color:var(--muted)">'+esc(q)+' · '+size+'</span>'+
        ' <span class="mono" style="font-size:11px;color:var(--faint)">'+counts.can_run+' can / '+counts.cannot_run+' cannot / '+counts.unknown+' unknown</span>'+
      '</div>'+
      reasons + chosen +
      (v.workers && v.workers.length ? '<div style="margin-top:6px"><b style="font-size:11px">per worker</b>'+perWorker+'</div>' : '')+
      (!v.workers || !v.workers.length ? '<div class="mono" style="font-size:11px;color:var(--faint)">no workers on the fabric</div>' : '')+
    '</div>';
  }).join('');
}
// Variant comparison (Models view): answers "which on-disk variant should I
// deploy on THIS fabric?" by listing every real on-disk variant of a model
// with its per-variant fabric verdict, best-fit first. Everything rendered
// comes from the real /v1/can_run projection — never fabricated.
// Deterministic sort: verdict group (CAN_RUN, CANNOT_RUN, UNKNOWN), stable
// within a group by file name.
function sortVariantFits(variants){
  var rank = { 'CAN_RUN': 0, 'CANNOT_RUN': 1, 'UNKNOWN': 2 };
  return (variants || []).slice().sort(function(a, b){
    var va = (a && a.fit && a.fit.verdict) || 'UNKNOWN';
    var vb = (b && b.fit && b.fit.verdict) || 'UNKNOWN';
    var ra = rank[va] !== undefined ? rank[va] : 2;
    var rb = rank[vb] !== undefined ? rank[vb] : 2;
    if (ra !== rb) return ra - rb;
    return String(a && a.file || '').localeCompare(String(b && b.file || ''));
  });
}
async function variantCompare(){
  var modelInput = $('vc-model');
  if (modelInput && !modelInput.value) {
    var cir = $('cir-model');
    if (cir && cir.value) modelInput.value = cir.value.trim();
  }
  var capSel = $('vc-cap');
  if (capSel && capSel.options.length === 0) {
    var known = ['ocr','vision','coding','summarization','translation','embeddings','tool_calling','structured_output','reasoning','speech_to_text','text_to_speech','image_generation','classification','multimodal'];
    known.forEach(function(k){ var o = document.createElement('option'); o.value = k; o.textContent = k; capSel.appendChild(o); });
  }
  var model = (modelInput ? modelInput.value : '').trim();
  var cap = (capSel || {}).value || '';
  var con = $('variant-compare');
  if (!model) { toast('enter a model file to compare', true); return; }
  if (!con) return;
  con.innerHTML = '<span class="mono" style="font-size:11px;color:var(--faint)">checking variants for '+esc(model)+'…</span>';
  const { ok, status, j } = await apiFetch('/v1/can_run?model='+encodeURIComponent(model)+'&capability='+encodeURIComponent(cap)+'&evidence=any', { headers });
  if (!ok) { con.innerHTML = '<span class="badge warn">check failed (' + status + ')</span> ' + esc((j && j.error && j.error.message) || 'unknown'); return; }
  var variants = sortVariantFits(j && j.variants);
  if (!variants.length) { con.innerHTML = '<span class="mono" style="font-size:11px;color:var(--muted)">no on-disk variants on this fabric for '+esc(model)+'</span>'; return; }
  con.innerHTML = variants.map(function(v){
    var q = v.quantization || '—';
    var size = v.size_bytes ? fmtMB(v.size_bytes / 1048576) : '—';
    var fit = (v.fit || {});
    var badge = fit.verdict === 'CAN_RUN' ? '<span class="badge ok">CAN_RUN</span>'
      : fit.verdict === 'CANNOT_RUN' ? '<span class="badge bad">CANNOT_RUN</span>'
      : '<span class="badge warn">UNKNOWN</span>';
    var counts = fit.counts || {};
    var chosen = fit.chosen_worker ? ' · chosen worker: <code>'+esc(short(fit.chosen_worker, 16))+'</code>' : '';
    return '<div style="margin-top:8px;border-top:1px solid var(--border);padding-top:6px">'+
      '<div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap">'+
        '<span class="mono" style="font-size:11.5px">'+esc(v.file||'')+'</span> '+badge+
        ' <span class="mono" style="font-size:11px;color:var(--muted)">'+esc(q)+' · '+size+'</span>'+
        ' <span class="mono" style="font-size:11px;color:var(--faint)">'+counts.can_run+' can / '+counts.cannot_run+' cannot / '+counts.unknown+' unknown</span>'+
        chosen+
      '</div>'+
    '</div>';
  }).join('');
}
async function hubPullVariant(id, file){
  if (hubPulling[id+':'+file]) return;
  hubPulling[id+':'+file] = true;
  const btn = $('hub-pull-'+safeId(id+':'+file));
  if (btn) { btn.disabled = true; btn.textContent = 'pulling…'; }
  $('hub-status').innerHTML = 'downloading '+esc(id)+' variant '+esc(file)+' — the node keeps serving';
  const { ok, status, j } = await apiFetch('/api/admin/hub/pull', {
    method: 'POST', headers: Object.assign({}, headers, { 'Content-Type': 'application/json' }),
    body: JSON.stringify({ reference: 'hf:' + id, file: file }),
  });
  if (!ok) {
    $('hub-status').innerHTML = '<span class="badge warn">pull failed (' + status + ')</span> ' + esc((j.error && j.error.message) || 'unknown error');
  } else {
    $('hub-status').innerHTML = '<span class="badge ok">pulled</span> '+esc(j.file)+' — '+fmtMB(j.bytes / 1048576)+' · sha256 <code>'+esc(short(j.sha256, 16))+'</code> — refresh the page to see it in the registry';
    toast('variant pulled: ' + short(j.file, 24));
    refresh();
  }
  delete hubPulling[id+':'+file];
  if (btn) { btn.disabled = false; btn.textContent = 'Pull'; }
}
function hubCloseDetail(){ $('hub-detail').style.display = 'none'; }
function removeModel(path){
  if (!confirm('Are you sure you want to delete model ' + path + ' from disk?')) return;
  fetch('/api/admin/models/remove', {
    method: 'POST', headers: Object.assign({}, headers, { 'Content-Type': 'application/json' }),
    body: JSON.stringify({ path: path }),
  }).then(r => r.json().then(j => ({ ok: r.ok, j })))
    .then(({ ok, j }) => {
      if (!ok) { toast('Delete failed: ' + ((j.error && j.error.message) || 'unknown'), true); }
      else {
        toast('Model deleted: ' + path);
        refresh();
      }
    })
    .catch(e => { toast('Delete error: ' + e, true); });
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
  loadHistStats();
}
// Historical execution statistics (Phase N): deterministic aggregates from real
// measured execution history via /v1/stats. Operator/admin-gated.
async function loadHistStats(){
  const con = $('hist-stats');
  if (!con) return;
  try {
    const { ok, j } = await apiFetch('/v1/stats', { headers });
    if (!ok) { con.innerHTML = '<span class="badge warn">stats unavailable ('+(j.error&&j.error.message||'')+')</span>'; return; }
    if (!j.records) { con.innerHTML = '<span class="muted">no executed requests yet</span>'; return; }
    const o = j.outcomes || {};
    const m = j.measured || {};
    const row = (label, v) => '<div class="metric"><div class="label">'+esc(label)+'</div><div class="value">'+v+'</div></div>';
    let html = '<div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(180px,1fr));gap:8px">'+
      row('Records', j.records)+
      row('Succeeded', o.succeeded||0)+
      row('Failed', o.failed||0)+
      row('Retries', j.retries||0)+
      row('Avg tok/s', m.avg_tokens_per_sec ? m.avg_tokens_per_sec.toFixed(1) : '—')+
      row('Avg latency', m.avg_latency_ms ? m.avg_latency_ms.toFixed(0)+'ms' : '—')+
      '</div>';
    const pm = (j.per_model||[]).map(p =>
      '<span class="badge" title="'+esc(p.succeeded)+' ok / '+esc(p.failed)+' fail">'+esc(p.model)+'</span> '+esc(p.total)).join(' · ');
    if (pm) html += '<div style="margin-top:8px"><b style="font-size:11px">per model</b> '+pm+'</div>';
    con.innerHTML = html;
  } catch (e) { con.innerHTML = '<span class="badge warn">stats failed</span>'; }
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
// Resource view (Phase B): real node state + real fabric worker rows, each
// tagged with provenance. UNKNOWN is never rendered as a fabricated zero.
function renderResources(r){
  if (!r) return;
  const node = r.node || {};
  const ram = node.ram || {};
  const cpu = node.cpu || {};
  const disk = node.disk || {};
  let nodeHtml =
    '<div><span class="badge ok">cpu</span> '+cpu.logical_cpus+' cores · '+(cpu.usage_percent||0).toFixed(1)+'% ['+esc(cpu.provenance||'')+']</div>'+
    '<div><span class="badge ok">ram</span> '+fmtMB(ram.available_mb)+' free / '+fmtMB(ram.total_mb)+' total · headroom '+fmtMB(ram.headroom_mb)+' ['+esc(ram.provenance||'')+']</div>';
  const vram = node.vram || {};
  if (vram.present) {
    nodeHtml += '<div><span class="badge ok">vram</span> '+esc(vram.name||'gpu')+' · '+fmtMB(vram.free_mb)+' free / '+fmtMB(vram.total_mb)+' ['+esc(vram.provenance||'')+']</div>';
  } else {
    nodeHtml += '<div><span class="badge warn">vram</span> no GPU surfaced ['+esc(vram.provenance||'UNKNOWN')+']</div>';
  }
  nodeHtml += '<div><span class="badge ok">disk</span> '+fmtMB(disk.free_mb)+' free ['+esc(disk.provenance||'')+']</div>';
  $('res-node').innerHTML = nodeHtml;

  const rows = (r.fabric||[]).map(w => {
    const prow = (v, unit) => v ? '<span class="badge faint" title="provenance">'+esc(v)+'</span>' : unit;
    const ramH = w.ram ? 'RAM '+fmtMB(w.ram.available_mb)+' free / '+fmtMB(w.ram.total_mb)+' · res '+fmtMB(w.ram.reserved_mb)+' '+prow(w.ram.provenance,'') : '';
    const vramH = w.vram && w.vram.present
      ? ' · VRAM '+fmtMB(w.vram.available_mb)+' / '+fmtMB(w.vram.total_mb)+' '+prow(w.vram.provenance,'')
      : ' · <span class="badge warn">CPU-only</span>';
    return '<div class="mono" style="font-size:11px;padding:4px 0;border-bottom:1px solid var(--border,#223)">'+
      '<b>'+esc(w.node_name || short(w.peer_id,14))+'</b>'+
      (w.trusted ? ' <span class="badge ok">trusted</span>' : ' <span class="badge warn">untrusted</span>')+
      ' · '+esc(w.engine||'—')+
      '<div style="color:var(--muted)">cpu '+((w.cpu&&w.cpu.load_percent)||0)+'% · '+ramH+vramH+
      ' · queue '+(w.queue?w.queue.depth:'—')+' · latency '+(w.latency?w.latency.ms+'ms @ '+(w.latency.tokens_per_second||0)+' t/s':'—')+'</div></div>';
  }).join('');
  $('res-fabric').innerHTML = rows || '<div class="empty">no fabric workers advertised</div>';
}
// Fabric graph / digital twin (Phase C): read-only projection of real state.
// Counts and lists are derived from actual advertisements, persisted claims,
// measured links and recorded decisions — never fabricated.
function renderFabricGraph(f){
  if (!f) return;
  const nodes = f.nodes || [];
  const models = f.models || [];
  const caps = f.capabilities || [];
  const execs = f.executions || [];
  $('fabric-g-nodes').textContent = nodes.length;
  $('fabric-g-models').textContent = models.length;
  $('fabric-g-caps').textContent = caps.length;
  $('fabric-g-execs').textContent = execs.length;
  const nodeHtml = nodes.map(n =>
    '<div class="mono" style="font-size:11px;padding:3px 0">'+
      '<b>'+esc(n.node_name || short(n.peer_id,14))+'</b>'+
      (n.trusted ? ' <span class="badge ok">trusted</span>' : ' <span class="badge warn">untrusted</span>')+
      ' · <span class="badge faint">'+esc(n.engine||'—')+'</span>'+
      '<span style="color:var(--muted)"> · '+esc(n.node_id||'')+'</span></div>'
  ).join('');
  const capHtml = caps.slice(0, 8).map(c =>
    '<div class="mono" style="font-size:11px;padding:3px 0"><span class="badge accent">'+esc(c.capability)+'</span> '+c.models.length+' model(s) · '+c.nodes.length+' node(s)</div>'
  ).join('');
  $('fabric-graph').innerHTML =
    '<div class="grid cols-2">'+
      '<div><h3 style="margin:8px 0 4px">Nodes</h3>'+(nodeHtml || '<div class="empty">no nodes</div>')+'</div>'+
      '<div><h3 style="margin:8px 0 4px">Capabilities</h3>'+(capHtml || '<div class="empty">no capabilities (UNKNOWN)</div>')+'</div>'+
    '</div>';
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
  // tokens (with expiry + live usage from the API)
  fetch('/api/admin/token/list', { headers }).then(r => r.json()).then(d => {
    const toks = (d && d.tokens || []);
    const rows = toks.map(t => {
      let status = t.revoked ? '<span class="badge bad">revoked</span>'
        : t.expired ? '<span class="badge bad">expired</span>'
        : '<span class="badge ok">active</span>';
      if (!t.revoked && t.expires_at) status += ' <span class="badge faint">exp ' + new Date(t.expires_at*1000).toLocaleDateString() + '</span>';
      return '<tr><td>'+esc(t.name)+'</td><td><span class="badge '+(t.tier===3?'ok':t.tier===2?'warn':'faint')+'">T'+t.tier+'</span></td><td>'+esc(t.role||'client')+'</td>'+
      '<td class="num">'+(t.requests||0)+'</td><td class="num">'+(t.tokens_generated||0)+'</td><td>'+status+'</td>'+
      '<td>'+(isAdmin && !t.revoked ? '<button class="danger" data-n="'+t.name+'" onclick="revokeToken(event)">Revoke</button>' : '')+'</td></tr>';
    }).join('');
    $('tok-list').innerHTML = rows || '<tr><td colspan="7" class="empty">no tokens issued</td></tr>';
  }).catch(() => { $('tok-list').innerHTML = '<tr><td colspan="7" class="empty">master token required</td></tr>'; });
  // Developer access: real endpoint this page is served from.
  const ep = (location.protocol + '//' + location.host);
  $('dev-endpoint').textContent = ep + '/v1';
  $('dev-base-url').textContent = ep;
  buildCfg();
}
window.copyDev = id => {
  const el = $(id);
  const txt = el && (el.textContent || el.innerText || '').trim();
  if (!txt || txt === 'create a token above — the plaintext is shown once') { toast('nothing to copy yet — create a token first', true); return; }
  navigator.clipboard.writeText(txt).then(() => toast('copied: ' + short(txt, 24)));
};
// ---- config generator (Part 13/22): copy-paste client snippets that point
// at this node's real OpenAI-compatible /v1 endpoint. Pure frontend string
// building from live state — no backend call, no secret stored anywhere.
const cfgTemplates = {
  curl: (ep, key) => `curl ${ep}/v1/chat/completions \\
  -H "Authorization: Bearer ${key}" \\
  -H "Content-Type: application/json" \\
  -d '{"model":"<model-file-name.gguf>","messages":[{"role":"user","content":"Hello DecentraAI"}]}'`,
  python: (ep, key) => `from openai import OpenAI

client = OpenAI(base_url="${ep}/v1", api_key="${key}")
r = client.chat.completions.create(
    model="<model-file-name.gguf>",
    messages=[{"role": "user", "content": "Hello DecentraAI"}],
)
print(r.choices[0].message.content)`,
  js: (ep, key) => `const r = await fetch("${ep}/v1/chat/completions", {
  method: "POST",
  headers: { "Authorization": "Bearer ${key}", "Content-Type": "application/json" },
  body: JSON.stringify({
    model: "<model-file-name.gguf>",
    messages: [{ role: "user", content: "Hello DecentraAI" }],
    stream: true,
  }),
});
const reader = r.body.getReader(); // SSE chunks as they arrive`,
  openclaw: (ep, key) => `# OpenClaw — OpenAI-compatible provider pointing at this node
providers:
  decentraai:
    type: openai
    base_url: ${ep}/v1
    api_key: ${key}
    models:
      - "<model-file-name.gguf>"`,
  webui: (ep, key) => `# Open WebUI — "Admin → Settings → Connections → OpenAI API"
# URL:  ${ep}/v1
# Key:  ${key}   (the token you created above)
# Model: <model-file-name.gguf>`,
};
let cfgTab = 'curl';
window.selectCfg = tab => {
  cfgTab = tab;
  document.querySelectorAll('.cfg-tab').forEach(b => b.classList.toggle('active', b.dataset.cfg === tab));
  buildCfg();
};
function buildCfg(){
  const ep = location.protocol + '//' + location.host;
  const key = token || '<your-created-token>';
  const tpl = cfgTemplates[cfgTab];
  $('cfg-out').textContent = tpl ? tpl(ep, key) : '';
}
document.querySelectorAll('.cfg-tab').forEach(b => b.addEventListener('click', () => selectCfg(b.dataset.cfg)));
window.revokeToken = async e => {
  const name = e.target.dataset.n;
  const r = await fetch('/api/admin/token/revoke', { method:'POST', headers, body: JSON.stringify({ name }) });
  if (r.ok) toast('token revoked'); else { const d = await r.json().catch(()=>({})); toast((d.error&&d.error.message)||'revoke failed', true); }
  renderSecurity();
};
$('tok-create').addEventListener('click', async () => {
  const name = $('tok-name').value.trim(), tier = +$('tok-tier').value, role = $('tok-role').value;
  const hours = Math.max(0, +($('tok-expiry').value || 0));
  if (!name) { toast('token name required', true); return; }
  const body = { name, tier, role };
  if (hours > 0) body.expires_at = Math.floor(Date.now()/1000) + hours*3600;
  const r = await fetch('/api/admin/token/create', { method:'POST', headers, body: JSON.stringify(body) });
  const d = await r.json().catch(()=>({}));
  if (r.ok) {
    $('tok-result').innerHTML = '<div class="badge ok" style="margin-bottom:6px">created — copy now, shown once:</div><code style="display:block;word-break:break-all">'+esc(d.token)+'</code>';
    $('dev-key').innerHTML = '<code style="word-break:break-all">'+esc(d.token)+'</code>';
    $('tok-name').value = ''; toast('token created');
  } else $('tok-result').innerHTML = '<span class="badge bad">' + esc((d.error&&d.error.message)||'create failed') + '</span>';
  renderSecurity();
});

// ---- main refresh (every 3s, real data only) -------------------------------
// Capability overview (Digital Twin): the distinct capabilities known across
// on-disk models, with verified/inferred model counts, from /v1/capabilities.
async function loadCapOverview(){
  const con = $('cap-overview');
  if (!con) return;
  try {
    const j = await (await fetch('/v1/capabilities', { headers })).json();
    const caps = j.capabilities || [];
    if (!caps.length) { con.innerHTML = '<span class="muted">no capability claims yet — pull models from the Hub to record them</span>'; return; }
    con.innerHTML = caps.map(c =>
      '<span class="badge" title="'+esc(c.verified_models)+' verified · '+esc(c.inferred_models)+' inferred model(s)">'+esc(c.capability)+'</span> '+
      '<span class="mono" style="font-size:10px;color:var(--faint)">'+esc(c.verified_models)+'✓ / '+esc(c.inferred_models)+'~</span>'
    ).join(' ');
  } catch (e) { con.innerHTML = '<span class="badge warn">capability overview failed</span>'; }
}

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
    populateChatModels(s, c);
    renderModels(s, c);
    renderDiag(s, null, null);
    renderSettings(s);
    renderObservability(s, null);
    renderRecovery(s, null, null);
  }
  try { const p = await (await fetch('/v1/peers', { headers })).json(); renderPeers(p); } catch (e) {}
  try { c = await (await fetch('/v1/compute', { headers })).json(); renderWorkers(c); renderPressure(s, c); renderObservability(s, c); renderRecovery(s, c, null); renderModels(s, c); } catch (e) {}
  try { n = await (await fetch('/v1/network', { headers })).json(); renderNetwork(n); } catch (e) {}
  try { x = await (await fetch('/v1/execution', { headers })).json(); renderExecutions(x); renderDecisions(x); } catch (e) {}
  try { await renderSessions(); } catch (e) {}
  try { const rr = await (await fetch('/v1/resources', { headers })).json(); renderResources(rr); } catch (e) {}
  try { const fg = await (await fetch('/v1/fabric', { headers })).json(); renderFabricGraph(fg); } catch (e) {}
  renderFabric(s, c, n, x);
  if (s) renderDiag(s, c, n);
  if (s) renderRecovery(s, c, x);
  loadCapOverview();
}
setStageVisible(true);
// Build the chat model selector once, from real data only:
//   Auto (best available)  — fabric-wide best picker (local wins ties)
//   Local models           — this node's /status available_models
//   Remote workers         — every model advertised by other workers, even
//                            when a local copy exists, labelled with its node
const populateChatModels = (s, c) => {
  if (!c || !c.workers || chatModel.querySelector('optgroup[label="Remote workers"]')) return;
  chatModel.innerHTML = '';
  const auto = document.createElement('option');
  auto.value = '__auto__';
  auto.textContent = 'Auto (best available)';
  chatModel.appendChild(auto);
  const names = new Set([activeModel]);
  ((s && s.available_models) || []).forEach(m => { if (m && m.name) names.add(m.name); });
  if (names.size) {
    const og = document.createElement('optgroup'); og.label = 'Local models';
    names.forEach(name => { const opt = document.createElement('option'); opt.value = name; opt.textContent = name; og.appendChild(opt); });
    chatModel.appendChild(og);
  }
  const remote = [];
  (c.workers || []).forEach(w => {
    if (w && w.peer_id === c.local_peer) return;
    (w.served_models || []).forEach(m => {
      if (m && m.file_name) remote.push({ file: m.file_name, node: w.node_id || w.node_name || w.peer_id });
    });
  });
  if (remote.length) {
    const og = document.createElement('optgroup'); og.label = 'Remote workers';
    remote.forEach(r => {
      const opt = document.createElement('option');
      opt.value = 'remote:' + r.node + ':' + r.file;
      opt.textContent = r.file + '  (remote · ' + r.node + ')';
      og.appendChild(opt);
    });
    chatModel.appendChild(og);
  }
  chatModel.value = '__auto__';
};
refresh(); setInterval(refresh, 3000);
"##;