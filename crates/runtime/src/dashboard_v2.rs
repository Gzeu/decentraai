//! Visual-refresh dashboard shell.
//!
//! This is deliberately independent from `dashboard.rs`: v1 remains a stable
//! fallback while operators evaluate v2 at `/ui2`. Dynamic values are fetched
//! from the node's public status views, never from the llama-server backend.

pub const DASHBOARD_V2_HTML: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>DecentraAI · Node</title><style>
:root{color-scheme:light dark;--bg:#f5f7fb;--panel:#fff;--soft:#edf1f8;--ink:#172033;--muted:#647089;--line:#dfe5ef;--accent:#635bff;--good:#078c63;--warn:#b86b00;--danger:#c23b4a;--shadow:0 12px 35px #1d29421a}
@media(prefers-color-scheme:dark){:root{--bg:#111522;--panel:#1a2030;--soft:#232b3e;--ink:#edf1fa;--muted:#aab5ca;--line:#303a50;--accent:#9a94ff;--good:#4bd6a8;--warn:#ffb657;--danger:#ff8491;--shadow:0 12px 35px #0005}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.45 ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}.shell{display:grid;grid-template-columns:250px minmax(0,1fr);min-height:100vh}.rail{padding:22px 14px;border-right:1px solid var(--line);background:color-mix(in srgb,var(--panel) 86%,transparent);position:sticky;top:0;height:100vh}.brand{font-weight:800;letter-spacing:-.04em;font-size:21px;padding:0 10px 24px}.brand i{display:inline-block;width:10px;height:10px;background:var(--good);border-radius:9px;margin-right:8px}.nav{display:grid;gap:3px}.nav button,.quiet{border:0;background:transparent;color:var(--muted);text-align:left;border-radius:8px;padding:9px 10px;font:inherit;cursor:pointer}.nav button:hover,.nav button.active,.quiet:hover{background:var(--soft);color:var(--ink)}.nav small{margin:18px 10px 5px;color:var(--muted);font-size:10px;letter-spacing:.1em;text-transform:uppercase}.rail-bottom{position:absolute;bottom:18px;left:14px;right:14px}.main{max-width:1440px;width:100%;margin:auto;padding:28px}.top{display:flex;align-items:center;justify-content:space-between;gap:15px;margin-bottom:24px}.top h1{font-size:25px;letter-spacing:-.04em;margin:0}.sub{color:var(--muted);margin-top:3px}.actions{display:flex;gap:8px;align-items:center}.button,input,textarea,select{font:inherit;border:1px solid var(--line);border-radius:8px;background:var(--panel);color:var(--ink)}.button{padding:8px 12px;cursor:pointer}.button.primary{background:var(--accent);color:#fff;border-color:var(--accent)}.view{display:none}.view.active{display:block}.grid{display:grid;gap:14px}.metrics{grid-template-columns:repeat(4,minmax(0,1fr))}.card{background:var(--panel);border:1px solid var(--line);border-radius:13px;padding:17px;box-shadow:var(--shadow)}.card h2,.card h3{font-size:13px;margin:0 0 12px}.metric .value{font-size:25px;font-weight:750;letter-spacing:-.04em}.metric .label,.hint{color:var(--muted);font-size:12px}.split{grid-template-columns:1.15fr .85fr;margin-top:14px}.stack{display:grid;gap:14px}.status{display:inline-flex;align-items:center;gap:6px;border-radius:99px;padding:4px 8px;background:var(--soft);font-size:12px}.dot{width:7px;height:7px;border-radius:9px;background:var(--muted)}.dot.good{background:var(--good)}.dot.warn{background:var(--warn)}.list{display:grid;gap:8px}.row{display:flex;justify-content:space-between;gap:12px;padding:8px 0;border-bottom:1px solid var(--line)}.row:last-child{border-bottom:0}.row span:last-child{color:var(--muted);text-align:right;overflow-wrap:anywhere}pre{margin:0;white-space:pre-wrap;word-break:break-word;font:12px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;color:var(--muted)}.chat{min-height:300px;max-height:54vh;overflow:auto;background:var(--soft);border-radius:10px;padding:12px}.msg{padding:10px 12px;border-radius:9px;max-width:85%;margin:7px 0;white-space:pre-wrap}.msg.user{background:var(--accent);color:#fff;margin-left:auto}.msg.assistant{background:var(--panel);border:1px solid var(--line)}textarea{width:100%;min-height:85px;padding:10px;resize:vertical}.chatbar{display:grid;grid-template-columns:1fr auto;gap:9px;margin-top:10px}.advanced[hidden]{display:none}.token{width:170px;padding:8px}.empty{color:var(--muted);padding:12px 0}@media(max-width:800px){.shell{display:block}.rail{position:static;height:auto;border-right:0}.nav{grid-template-columns:repeat(3,1fr)}.nav small,.rail-bottom{display:none}.main{padding:17px}.metrics,.split{grid-template-columns:1fr 1fr}.top{align-items:flex-start;flex-direction:column}}@media(max-width:510px){.metrics,.split,.chatbar{grid-template-columns:1fr}.actions{flex-wrap:wrap}}
</style></head><body><div class="shell"><aside class="rail"><div class="brand"><i></i>DecentraAI</div><nav class="nav" id="nav"><button class="active" data-view="overview">Overview</button><button data-view="chat">Chat</button><small>Advanced fabric</small><div id="advanced-nav"><button data-view="workers">Workers</button><button data-view="network">Network</button><button data-view="execution">Execution</button><button data-view="models">Models</button><button data-view="settings">Settings</button><button data-view="diagnostics">Diagnostics</button></div></nav><div class="rail-bottom"><button class="quiet" id="advanced-toggle">Show advanced</button></div></aside><main class="main"><header class="top"><div><h1 id="title">Node overview</h1><div class="sub" id="node-line">Connecting to local node…</div></div><div class="actions"><input class="token" id="token" type="password" autocomplete="off" placeholder="API token (optional)"><button class="button" id="refresh">Refresh</button></div></header>
<section class="view active" id="view-overview"><div class="grid metrics"><article class="card metric"><div class="label">Model</div><div class="value" id="model-loaded">—</div><div class="hint" id="model-name">Checking status…</div></article><article class="card metric"><div class="label">Backend</div><div class="value" id="backend">—</div><div class="hint" id="idle">Idle time unknown</div></article><article class="card metric"><div class="label">RAM pressure</div><div class="value" id="ram">—</div><div class="hint" id="cpu">CPU unavailable</div></article><article class="card metric"><div class="label">GPU pressure</div><div class="value" id="gpu">—</div><div class="hint" id="queue">Queue unknown</div></article></div><div class="grid split"><article class="card"><h2>Recent inference</h2><div id="recent" class="list"><div class="empty">No observations yet.</div></div></article><article class="card"><h2>Node health</h2><div id="health" class="list"></div></article><article class="card"><h2>Queue</h2><div id="queue-detail" class="list"></div></article><article class="card"><h2>Share verified models</h2><div id="share" class="hint"></div></article></div></section>
<section class="view" id="view-chat"><article class="card"><h2>Chat with this node</h2><p class="hint">Your token stays in this browser. Replies stream directly from the node API.</p><div class="chat" id="chat"><div class="empty">Start a conversation.</div></div><div class="chatbar"><textarea id="prompt" placeholder="Ask the currently served model…"></textarea><div class="stack"><select id="chat-model"><option value="">Current model</option></select><label class="hint"><input id="stream" type="checkbox" checked> Stream response</label><button class="button primary" id="send">Send</button></div></div><div class="hint" id="chat-status"></div></article></section>
<div id="advanced" class="advanced" hidden><section class="view" id="view-workers"><article class="card"><h2>Workers</h2><pre id="workers">Open this view to load live worker data.</pre></article></section><section class="view" id="view-network"><article class="card"><h2>Network</h2><pre id="network">Open this view to load live network data.</pre></article></section><section class="view" id="view-execution"><article class="card"><h2>Execution</h2><pre id="execution">Open this view to load real planner decisions.</pre></article></section><section class="view" id="view-models"><article class="card"><h2>Models</h2><div id="models" class="list"></div></article></section><section class="view" id="view-settings"><article class="card"><h2>Settings</h2><pre id="settings">Open this view to load the node's exposed settings.</pre></article></section><section class="view" id="view-diagnostics"><article class="card"><h2>Diagnostics</h2><pre id="diagnostics">Open this view to load current diagnostics.</pre></article></section></div>
</main></div><script>/*__JS__*/</script></body></html>"##;

pub const JS_V2_TEMPLATE: &str = r##"
const $ = id => document.getElementById(id);
const esc = v => String(v ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const fmt = n => Number.isFinite(n) ? n.toFixed(1) : '—';
const auth = () => $('token').value.trim() ? {Authorization:'Bearer '+$('token').value.trim()} : {};
const tokenKey = 'decentraai.dashboard-v2.token';
try { $('token').value = localStorage.getItem(tokenKey) || ''; } catch (_) {}
$('token').addEventListener('change', () => { try { localStorage.setItem(tokenKey, $('token').value.trim()); } catch (_) {} });

const title = {overview:'Node overview',chat:'Chat',workers:'Workers',network:'Network',execution:'Execution',models:'Models',settings:'Settings',diagnostics:'Diagnostics'};
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
function setAdvanced(on) { advanced.hidden = !on; $('advanced-nav').hidden = !on; $('advanced-toggle').textContent = on ? 'Hide advanced' : 'Show advanced'; if (!on && !['overview','chat'].includes(currentView)) show('overview'); }
setAdvanced((localStorage.getItem('decentraai.dashboard-v2.advanced') || '0') === '1');
$('advanced-toggle').addEventListener('click', () => { const on = advanced.hidden; try { localStorage.setItem('decentraai.dashboard-v2.advanced', on ? '1' : '0'); } catch (_) {} setAdvanced(on); });

function valueRows(values) { return Object.entries(values).map(([k,v]) => '<div class="row"><b>'+esc(k)+'</b><span>'+esc(v)+'</span></div>').join(''); }
function renderStatus(s) {
  lastStatus = s;
  const loaded = !!s.model_loaded, sys = s.system || {}, gpu = sys.gpu;
  $('model-loaded').textContent = loaded ? 'Loaded' : 'Unloaded';
  $('model-name').textContent = s.model || 'No model selected';
  $('backend').textContent = s.backend || 'Unavailable';
  $('idle').textContent = 'Idle '+(s.idle_for_secs ?? 0)+' seconds';
  $('ram').textContent = sys.ram_available_gib !== undefined ? fmt(sys.ram_available_gib)+' GiB free' : 'Unavailable';
  $('cpu').textContent = sys.cpu_threads ? sys.cpu_threads+' CPU threads' : 'CPU unavailable';
  $('gpu').textContent = gpu ? fmt(gpu.utilization_percent)+'%' : 'None';
  $('queue').textContent = (s.queue?.waiting || []).length+' waiting';
  $('node-line').textContent = (s.node?.name || s.p2p_peer_id || 'Local node')+' · '+(loaded ? 'ready' : 'engine not loaded');
  $('health').innerHTML = valueRows({Model:loaded ? 'Ready' : 'Not loaded',Uptime:(s.uptime_secs ?? 0)+' seconds',Requests:s.requests_served ?? 0,Tokens:s.tokens_generated ?? 0,Success:s.success_rate_percent !== undefined ? fmt(s.success_rate_percent)+'%' : '—'});
  $('queue-detail').innerHTML = valueRows({Serving:s.queue?.serving?.who || 'Nobody',Waiting:(s.queue?.waiting || []).length,Timeout:(s.queue?.timeout_secs ?? '—')+' seconds'});
  $('recent').innerHTML = (s.recent_requests || []).map(r => '<div class="row"><b>'+esc((r.endpoint || '').replace('/v1/',''))+'</b><span>'+esc(r.completion_tokens)+' tokens · '+esc(r.duration_ms)+' ms</span></div>').join('') || '<div class="empty">No inference calls yet.</div>';
  $('models').innerHTML = (s.available_models || []).map(m => '<div class="row"><b>'+esc(m.name)+'</b><span>'+((m.size_bytes || 0)/1073741824).toFixed(2)+' GiB</span></div>').join('') || '<div class="empty">No indexed models.</div>';
  const select = $('chat-model'), chosen = select.value;
  select.innerHTML = '<option value="">Current model</option>'+(s.available_models || []).map(m => '<option value="'+esc(m.name)+'">'+esc(m.name)+'</option>').join('');
  select.value = chosen;
}
function renderPeers(peers) { const healthy = (peers || []).filter(p => !p.banned).length; const health = $('health'); health.insertAdjacentHTML('beforeend', '<div class="row"><b>Peers</b><span>'+healthy+' healthy / '+(peers || []).length+' known</span></div>'); }
async function refresh() { try { renderStatus(await (await fetch('/status')).json()); } catch (_) { $('node-line').textContent = 'Status unavailable — is the node running?'; } try { renderPeers(await (await fetch('/v1/peers',{headers:auth()})).json()); } catch (_) {} }
$('refresh').addEventListener('click', refresh);
refresh();
// These are the only recurring requests: dashboard observation must never
// invoke the inference proxy or reset the managed engine's idle clock.
setInterval(refresh, 5000);

const advancedEndpoints = {workers:'/v1/compute',network:'/v1/network',execution:'/v1/execution',settings:'/v1/resources',diagnostics:'/v1/fabric'};
const loadedAdvanced = new Set();
async function loadAdvanced(view) { if (loadedAdvanced.has(view) || !advancedEndpoints[view]) return; const target = $(view); try { const r = await fetch(advancedEndpoints[view], {headers:auth()}); const j = await r.json(); target.textContent = JSON.stringify(j,null,2); loadedAdvanced.add(view); } catch (_) { target.textContent = 'This view needs a valid operator token or the node is unavailable.'; } }

const chat = $('chat'); let history = [];
function addMessage(role, text) { if (chat.querySelector('.empty')) chat.innerHTML = ''; const el = document.createElement('div'); el.className = 'msg '+role; el.textContent = text; chat.appendChild(el); chat.scrollTop = chat.scrollHeight; return el; }
async function readSse(response, node) { const reader = response.body.getReader(), decoder = new TextDecoder(); let buffer = '', output = ''; for (;;) { const {done,value} = await reader.read(); if (done) break; buffer += decoder.decode(value,{stream:true}); const lines = buffer.split('\n'); buffer = lines.pop() || ''; for (const line of lines) { if (!line.startsWith('data:')) continue; const data = line.slice(5).trim(); if (data === '[DONE]') continue; try { const event = JSON.parse(data), delta = event.choices?.[0]?.delta?.content; if (delta) { output += delta; node.textContent = output; chat.scrollTop = chat.scrollHeight; } } catch (_) {} } } return output; }
$('send').addEventListener('click', async () => { const prompt = $('prompt').value.trim(); if (!prompt) return; $('prompt').value = ''; addMessage('user',prompt); history.push({role:'user',content:prompt}); const streaming = $('stream').checked, model = $('chat-model').value || lastStatus?.model || 'auto'; $('chat-status').textContent = 'Generating…'; try { const response = await fetch('/v1/chat/completions',{method:'POST',headers:{'Content-Type':'application/json',...auth()},body:JSON.stringify({model,messages:history,stream:streaming})}); let answer = ''; if (streaming && response.ok && response.body) { answer = await readSse(response,addMessage('assistant','')); } else { const body = await response.json(); answer = body.choices?.[0]?.message?.content || body.error?.message || 'No response'; addMessage('assistant',answer); } history.push({role:'assistant',content:answer}); history = history.slice(-24); $('chat-status').textContent = response.ok ? 'Done' : 'Request failed'; } catch (error) { addMessage('assistant','Request failed: '+error); $('chat-status').textContent = 'Request failed'; } });
$('share').innerHTML = "__SHARE__";
"##;
