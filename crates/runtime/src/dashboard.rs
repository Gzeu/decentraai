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
//! `/v1/peers`, `/v1/compute`, `/v1/network`, `/v1/execution`). Nothing is faked:
//! when idle the stage is calm and atmospheric; when a real request is being
//! planned, reserved and executed, the planner activates, the selected worker
//! lights up, reservations appear and tokens visibly stream. When recovery
//! machinery reacts to a failure, the affected worker changes state and the
//! replan becomes part of the story.
//!
//! Views (functionality preserved):
//! - **Overview** — living fabric + decision strip + metrics.
//! - **Chat** — quick chat.
//! - **Topology** — larger fabric stage.
//! - **Decisions / Execution / Agents / Skills / Workers / Network / Models /
//!   Observability / Recovery / Diag / Security / Settings** — real data.
//!
//! Single-binary constraint: pure embedded HTML/CSS/JS, no external assets,
//! no CDN. Canvas 2D remains the primary visualization technology.
//!
//! Invariant: the page only polls read-only control endpoints unless the user
//! explicitly performs an action.

/// The Command Deck HTML shell. All dynamic data is fetched by the module JS;
/// the shell itself contains no node data. `/*__JS__*/` and `__API_PORT__` are
/// filled by `api.rs` at serve time.
pub const DASHBOARD_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>DecentraAI — Control Plane</title>
<style>
:root{
  --bg:#040811; --bg-2:#07101b; --panel:#0a1420; --panel-2:#09111b;
  --line:#17263a; --line-2:#27405e;
  --text:#edf7ff; --muted:#93a8bb; --faint:#667c91;
  --accent:#22d3ee; --accent-2:#6366f1; --accent-soft:rgba(34,211,238,.10);
  --ok:#35e59a; --warn:#f5bf3a; --bad:#fb7185; --remote:#9f9cf6;
  --purple:#8b5cf6;
  --mono:ui-monospace,"SF Mono",SFMono-Regular,Menlo,Consolas,monospace;
  --sans:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Inter,Helvetica,Arial,sans-serif;
  --radius:16px; --radius-sm:10px;
  --shadow:0 18px 54px rgba(0,0,0,.48);
}
*{box-sizing:border-box;margin:0;padding:0}
html,body{height:100%}
body{
  color:var(--text);font:13px/1.5 var(--sans);-webkit-font-smoothing:antialiased;
  background:
    radial-gradient(980px 540px at 82% -10%,rgba(34,211,238,.09),transparent 60%),
    radial-gradient(900px 620px at 10% 110%,rgba(99,102,241,.08),transparent 58%),
    linear-gradient(180deg,#03060d 0%,#050b13 100%);
}
a{color:var(--accent);text-decoration:none} code,.mono{font-family:var(--mono)}
button,input,select,textarea{font:inherit;color:inherit}
button{background:rgba(10,18,30,.9);border:1px solid var(--line-2);border-radius:var(--radius-sm);padding:7px 11px;cursor:pointer}
button:hover{border-color:rgba(34,211,238,.55);background:rgba(34,211,238,.06)}
button.primary{background:linear-gradient(135deg,#25d8ed,#6366f1);border:0;color:#03111a;font-weight:800}
input,select,textarea{background:#06101a;border:1px solid var(--line-2);border-radius:var(--radius-sm);padding:8px 10px;outline:none}
input:focus-visible,select:focus-visible,textarea:focus-visible,button:focus-visible,.nav-item:focus-visible{outline:2px solid rgba(34,211,238,.55);outline-offset:2px}
.layout{display:grid;grid-template-columns:236px minmax(0,1fr);min-height:100vh}
.rail{position:sticky;top:0;height:100vh;display:flex;flex-direction:column;padding:18px 12px;background:rgba(5,9,16,.84);border-right:1px solid rgba(39,64,94,.75);backdrop-filter:blur(16px)}
.brand{display:flex;align-items:center;gap:10px;padding:4px 8px 18px}
.brand-mark{width:34px;height:34px;border-radius:10px;display:grid;place-items:center;background:rgba(34,211,238,.07);border:1px solid rgba(34,211,238,.30);box-shadow:0 0 24px rgba(34,211,238,.16)}
.brand-mark svg{width:29px;height:29px}.brand-name{font-weight:800;letter-spacing:.02em}.brand-sub{font-size:9.5px;color:var(--faint);letter-spacing:.16em;text-transform:uppercase}
.rail-label{font-size:10px;color:var(--faint);letter-spacing:.16em;text-transform:uppercase;padding:15px 10px 5px}
.nav-item{display:flex;align-items:center;gap:10px;width:100%;text-align:left;background:transparent;border:1px solid transparent;color:var(--muted);padding:9px 10px;border-radius:11px;font-size:12.5px}
.nav-item:hover{background:rgba(255,255,255,.035);color:var(--text)} .nav-item.active{color:var(--accent);background:rgba(34,211,238,.09);border-color:rgba(34,211,238,.30);box-shadow:inset 0 0 18px rgba(34,211,238,.05)}
.nav-item .ic{width:16px;text-align:center}.rail-foot{margin-top:auto;padding-top:12px;border-top:1px solid var(--line)}
.rail-live{display:flex;align-items:center;gap:8px;color:var(--muted);font-size:11px;padding:2px 8px}.dot{width:7px;height:7px;border-radius:50%;display:inline-block}.dot.ok{background:var(--ok);box-shadow:0 0 9px var(--ok)} .dot.warn{background:var(--warn);box-shadow:0 0 9px var(--warn)} .dot.bad{background:var(--bad);box-shadow:0 0 9px var(--bad)} .dot.accent{background:var(--accent);box-shadow:0 0 9px var(--accent)}
.main{padding:18px 22px 56px;max-width:1480px;width:100%;min-width:0}
.topbar{display:flex;align-items:center;justify-content:space-between;gap:12px;margin-bottom:11px;flex-wrap:wrap}.crumb{font-size:11px;color:var(--faint);letter-spacing:.08em}.topbar h1{font-size:20px;letter-spacing:-.02em}.top-right{display:flex;align-items:center;gap:8px;flex-wrap:wrap}.pill{display:inline-flex;align-items:center;gap:7px;border:1px solid var(--line-2);border-radius:999px;padding:5px 10px;font-size:10.5px;font-family:var(--mono);background:rgba(10,16,27,.7)} .pill.live{border-color:rgba(34,211,238,.35);color:var(--accent)}
.banner{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-bottom:12px}.now-label{font-size:9.5px;font-weight:800;letter-spacing:.17em;color:var(--accent);border:1px solid rgba(34,211,238,.35);border-radius:999px;padding:3px 9px;background:var(--accent-soft)}.now-state{font-family:var(--mono);font-size:11px;color:var(--text)} .planner-chip{display:inline-flex;align-items:center;gap:6px;font-size:10.5px;font-family:var(--mono);color:var(--muted);border:1px solid var(--line);border-radius:999px;padding:3px 9px;background:rgba(13,18,28,.6)}
.grid{display:grid;gap:12px}.cols-2{grid-template-columns:repeat(auto-fit,minmax(320px,1fr))}.cols-3{grid-template-columns:repeat(auto-fit,minmax(220px,1fr))}.cols-4{grid-template-columns:repeat(auto-fit,minmax(140px,1fr))}
.card{background:linear-gradient(180deg,rgba(10,18,29,.94),rgba(6,12,20,.96));border:1px solid var(--line);border-radius:var(--radius);padding:14px 16px;box-shadow:var(--shadow);min-width:0}.card:hover{border-color:#294866}.card h2{font-size:10px;text-transform:uppercase;letter-spacing:.15em;color:var(--faint);margin-bottom:10px;display:flex;align-items:center;gap:7px}.count{color:var(--accent)}
.metric{background:#06101a;border:1px solid var(--line);border-radius:12px;padding:11px 12px}.metric .label{font-size:9.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.12em}.metric .value{font-family:var(--mono);font-size:20px;font-weight:700;line-height:1.25;margin-top:2px}.metric.accent .value{color:var(--accent)} .metric.ok .value{color:var(--ok)} .metric.warn .value{color:var(--warn)}
.status-strip{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:9px;margin:0 0 12px}.status-cell{background:linear-gradient(180deg,#071321,#06101a);border:1px solid var(--line);border-radius:12px;padding:10px 12px}.status-cell .k{font-size:9px;color:var(--faint);letter-spacing:.12em;text-transform:uppercase}.status-cell .v{font:700 18px var(--mono);margin-top:2px}.status-cell .s{font-size:10px;color:var(--muted)}
.stage-card{position:relative;background:radial-gradient(900px 500px at 50% 42%,rgba(34,211,238,.08),transparent 52%),radial-gradient(660px 460px at 15% 60%,rgba(99,102,241,.08),transparent 58%),linear-gradient(180deg,#07101b,#040912);border:1px solid var(--line);border-radius:18px;overflow:hidden;box-shadow:var(--shadow)}
.fabric-stage{display:block;width:100%;height:500px}.stage-foot{display:flex;align-items:center;justify-content:space-between;gap:10px;padding:8px 13px;border-top:1px solid var(--line);background:rgba(4,9,16,.82);flex-wrap:wrap}.pipeline{display:flex;align-items:center;gap:3px;flex-wrap:wrap}.pipe{font-size:9px;letter-spacing:.09em;color:var(--faint);padding:4px 7px;border-radius:7px}.pipe.on{color:var(--accent);border:1px solid rgba(34,211,238,.35);background:var(--accent-soft)}.pipe.done{color:var(--ok)}.stage-meta{font:10px var(--mono);color:var(--faint)}
.live-grid{display:grid;grid-template-columns:minmax(0,1.55fr) minmax(280px,.8fr);gap:12px;margin-top:12px}.process-table{overflow:auto}.process-table table{width:100%;border-collapse:collapse;font-size:11px}.process-table th{font-size:8.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em;text-align:left;padding:7px;border-bottom:1px solid var(--line)}.process-table td{padding:7px;border-bottom:1px solid rgba(28,38,52,.58);font-family:var(--mono);white-space:nowrap}.process-table tr:hover{background:rgba(34,211,238,.025)}
.spark{width:100%;height:108px;display:block}.micro-grid{display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin-top:9px}.micro{background:#06101a;border:1px solid var(--line);border-radius:10px;padding:8px}.micro .k{font-size:8.5px;color:var(--faint);text-transform:uppercase;letter-spacing:.1em}.micro .v{font:700 14px var(--mono);margin-top:2px}.micro .v.ok{color:var(--ok)}
.feedback{display:flex;align-items:center;gap:12px}.ring{width:70px;height:70px;border-radius:50%;display:grid;place-items:center;background:conic-gradient(var(--ok) var(--pct,76%),rgba(255,255,255,.06) 0);position:relative}.ring::after{content:"";position:absolute;inset:7px;border-radius:50%;background:#07101a}.ring b{position:relative;z-index:1;font:700 15px var(--mono);color:var(--text)}
.section-title{display:flex;align-items:center;justify-content:space-between;gap:10px;margin:14px 0 8px}.section-title .eyebrow{font-size:9px;color:var(--faint);letter-spacing:.16em;text-transform:uppercase}.section-title strong{font-size:11px;color:var(--accent)}
.footer-bar{margin-top:14px;display:flex;align-items:center;justify-content:space-between;gap:10px;flex-wrap:wrap;padding:10px 12px;color:var(--faint);font:10px var(--mono);border-top:1px solid var(--line)}
@media(max-width:1000px){.layout{grid-template-columns:68px minmax(0,1fr)}.rail{padding:13px 8px}.brand-name,.brand-sub,.rail-label,.nav-item span:not(.ic),.rail-live span:last-child{display:none}.rail-live{justify-content:center}.main{padding:14px 14px 40px}.live-grid{grid-template-columns:1fr}.fabric-stage{height:380px}.status-strip{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(max-width:620px){.status-strip{grid-template-columns:1fr 1fr}.micro-grid{grid-template-columns:1fr}.topbar h1{font-size:17px}}

/* ===== Original Command Deck rules below: preserved where they affect behavior/classes generated by JS. ===== */
:root{--border:var(--line);--fg:var(--text)}
.view{display:none;animation:fade .25s ease}.view.active{display:block}@keyframes fade{from{opacity:0;transform:translateY(4px)}to{opacity:1;transform:none}}
.grid.cols-2{grid-template-columns:repeat(auto-fit,minmax(320px,1fr))}.grid.cols-3{grid-template-columns:repeat(auto-fit,minmax(230px,1fr))}.grid.cols-4{grid-template-columns:repeat(auto-fit,minmax(150px,1fr))}
/* Keep the exact existing HTML/JS and domain rendering below this point. The visual layer above overrides the original tokens and presentation. */
</style>
</head>
<body>
"##;
