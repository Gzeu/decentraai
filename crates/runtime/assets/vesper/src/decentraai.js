import { hashFnv } from './core.js';

// DecentraAI fabric adapter — aligned to the REAL fabric API contract:
//   auth:  Authorization: Bearer <master-token | dca_… consumer key>
//   probe: GET /status (public, no secrets)
//   execute: POST /v1/governor/execute { task_id, instruction, content, task_kind? }
//   workflow: POST /v1/agents/workflow
//   credits: GET /v1/credits/balance (master/admin)
//   evidence: GET /v1/evidence
//   pool/bench: POST /v1/pool/bench
//   mcp: POST /mcp
// Same-origin by default (empty baseUrl = current host), so no CORS is needed.

let cfg = { baseUrl: '', adminDcaKey: '', enabled: true };

export function configure(opts) {
  cfg = Object.assign({ baseUrl: '', adminDcaKey: '', enabled: true }, opts || {});
  // baseUrl '' (same host) or 'host' or 'host:port' or '/path'. Strip protocol/slashes.
  const raw = String(cfg.baseUrl || '').trim();
  cfg.baseUrl = raw.replace(/^https?:\/\//i, '').replace(/\/+$/, '');
  cfg.enabled = cfg.enabled !== false;
}

export function configured() {
  return cfg.enabled && !!cfg.baseUrl;
}

export function fabricCfg() {
  return { baseUrl: cfg.baseUrl, enabled: cfg.enabled, hasAdminKey: !!cfg.adminDcaKey };
}

// Stable per-agent identity. Real fabric agents carry their AgentRecord
// agent_id (e.g. "dca-JJjXhh:generalist"); prefer it verbatim so the fabric
// recognizes the real credential. Fall back to a deterministic derivation from
// seed+agentId for legacy/non-real agents.
export function agentKey(world, agentId) {
  const a = world && world.agents && world.agents[agentId];
  if (a && a.agentId) return a.agentId;
  if (a && a.real) return String(agentId);
  return 'dca_' + hashFnv((world.meta.seed || 'vesper') + '::' + agentId).toString(16).slice(0, 10);
}

function ensureFabric(world) {
  if (!world.fabric) world.fabric = { log: [], calls: 0, ok: 0, fail: 0, sinceTick: world.clock.t, status: null };
  return world.fabric;
}

function logCall(world, entry) {
  const f = ensureFabric(world);
  f.log.push(entry);
  if (f.log.length > 200) f.log.splice(0, f.log.length - 200);
  return entry;
}

function baseUrl() {
  // Empty baseUrl resolves to same origin (relative). Non-empty is 'host[:port]'
  // and gets https:// (or the page's protocol if already on http). Same-origin
  // ('' ) needs no absolute URL at all.
  const b = (cfg.baseUrl || '').trim();
  if (!b) return '';
  if (b.startsWith('/')) return b; // relative path
  // host[:port] — use the page's own scheme to avoid mixed-content when served
  // over http during local/private use.
  const scheme = (window.location.protocol === 'https:') ? 'https://' : 'http://';
  return scheme + b;
}

function absUrl(path) {
  const b = baseUrl();
  return b ? (b + path) : path;
}

async function request(path, opts = {}) {
  // Same-origin (empty baseUrl) works for any local path; a configured remote
  // baseUrl also works. Only a disabled bridge blocks.
  if (!cfg.enabled) return { ok: false, err: 'fabric-not-configured' };
  const t0 = performance.now();
  const ctrl = new AbortController();
  const to = setTimeout(() => ctrl.abort(), 8000);
  const headers = Object.assign({ 'Content-Type': 'application/json' }, opts.headers || {});
  // Admin/master key (if configured) authenticates master-gated calls.
  if (cfg.adminDcaKey) headers['Authorization'] = 'Bearer ' + cfg.adminDcaKey;
  try {
    const res = await fetch(absUrl(path), {
      method: opts.method || 'GET',
      headers,
      body: opts.body ? JSON.stringify(opts.body) : undefined,
      signal: ctrl.signal,
    });
    const ms = Math.round(performance.now() - t0);
    const text = await res.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch (e) { data = { raw: text.slice(0, 500) }; }
    if (!res.ok) return { ok: false, status: res.status, err: 'http-' + res.status, data, ms };
    return { ok: true, status: res.status, data, ms };
  } catch (e) {
    const ms = Math.round(performance.now() - t0);
    const err = e && e.name === 'AbortError' ? 'timeout' : (e && e.message) || String(e);
    return { ok: false, err, ms };
  } finally {
    clearTimeout(to);
  }
}

export function statusOf(world) {
  const st = (world && world.fabric && world.fabric.status) || {};
  const f = world && world.fabric;
  return {
    state: !cfg.enabled ? 'disabled' : cfg.baseUrl ? (st.state || 'unprobed') : 'same-origin',
    baseUrl: cfg.baseUrl || '(same-origin)',
    enabled: cfg.enabled,
    reachable: !!st.reachable,
    note: st.note || '',
    lastProbe: st.lastProbe || null,
    latencyMs: st.latencyMs || null,
    capabilities: st.capabilities || null,
    calls: (f && f.calls) || 0,
    ok: (f && f.ok) || 0,
    fail: (f && f.fail) || 0,
  };
}

export async function probe(world) {
  const f = ensureFabric(world);
  const st = f.status = f.status || {};
  st.lastProbe = Date.now();
  if (!cfg.enabled) { st.state = 'disabled'; st.reachable = false; st.note = 'fabric bridge disabled'; return statusOf(world); }
  if (!cfg.baseUrl) {
    // Same-origin: probe the local fabric directly (no CORS needed).
    st.state = 'probing';
    let res = await request('/status');
    if (res.ok) {
      st.reachable = true;
      st.state = 'connected';
      const d = res.data || {};
      st.capabilities = {
        model: d.model || null,
        model_loaded: !!d.model_loaded,
        uptime_secs: d.uptime_secs != null ? d.uptime_secs : null,
        requests_served: d.requests_served != null ? d.requests_served : null,
        cpu_percent: d.system && d.system.cpu_usage_percent != null ? d.system.cpu_usage_percent : null,
      };
      st.note = 'same-origin fabric reachable — agent jobs may dispatch real workload';
    } else {
      st.reachable = false;
      st.state = 'unreachable';
      st.note = (res.err || '') + (res.status ? ' (' + res.status + ')' : '');
    }
    return statusOf(world);
  }
  // Remote host: probe /status too (the real fabric's health surface).
  st.state = 'probing';
  let res = await request('/status');
  if (!res.ok) res = await request('/');
  st.latencyMs = res.ms;
  if (res.ok) {
    st.reachable = true;
    st.state = 'connected';
    const d = res.data || {};
    st.capabilities = { model: d.model || null, model_loaded: !!d.model_loaded, cpu_percent: d.system && d.system.cpu_usage_percent != null ? d.system.cpu_usage_percent : null };
    st.note = 'DecentraAI reachable — agent jobs may dispatch real workload';
  } else {
    st.reachable = false;
    st.state = 'unreachable';
    st.note = (res.err || '') + (res.status ? ' (' + res.status + ')' : '');
  }
  return statusOf(world);
}

// Dispatch a real compute job to the fabric's Governor. Body follows the real
// contract: task_id, instruction, content, task_kind. Requires a consumer
// (dca_…) or admin key, so it only succeeds when an operator has provisioned
// one (or adminDcaKey is set). Honest: failures are recorded, never faked.
export async function governorExecute(world, o) {
  const f = ensureFabric(world);
  const key = o.agentKey || agentKey(world, o.agentId);
  const entry = logCall(world, { at: Date.now(), tick: world.clock.t, op: 'governor/execute', agentKey: key, agentId: o.agentId || null, task: o.task || o.taskType || null, status: 'pending' });
  const body = {
    task_id: o.taskId || ('vesper-' + world.meta.seed + '-' + (o.agentId || 'a') + '-' + world.clock.t),
    instruction: o.instruction || ('Run compute task: ' + (o.task || 'task')),
    content: o.content || (o.params && o.params.text) || JSON.stringify(o.params || {}),
  };
  if (o.taskKind) body.task_kind = o.taskKind;
  const reqHeaders = {};
  if (key) reqHeaders['Authorization'] = 'Bearer ' + key;
  const res = await request('/v1/governor/execute', { method: 'POST', body, headers: reqHeaders });
  f.calls++;
  entry.ms = res.ms;
  if (res.ok) {
    f.ok++;
    entry.status = 'ok';
    const d = res.data || {};
    entry.executionId = d.execution_id || d.executionId || null;
    entry.mode = d.decision || d.mode || 'local';
    entry.detail = 'execution ' + (entry.executionId || 'ok');
    entry.result = d.result != null ? String(d.result).slice(0, 120) : null;
  } else {
    f.fail++;
    entry.status = 'fail';
    entry.detail = res.err || ('http-' + res.status);
  }
  return { ok: res.ok, executionId: entry.executionId || null, mode: entry.mode || 'local', err: res.ok ? null : (entry.detail || null), ms: res.ms, response: res.data || null };
}

export async function fabricWorkflow(world, o) {
  const f = ensureFabric(world);
  const key = o.agentKey || agentKey(world, o.agentId);
  const entry = logCall(world, { at: Date.now(), tick: world.clock.t, op: 'agents/workflow', agentKey: key, agentId: o.agentId || null, task: 'multi-step', status: 'pending' });
  const reqHeaders = {};
  if (key) reqHeaders['Authorization'] = 'Bearer ' + key;
  const res = await request('/v1/agents/workflow', { method: 'POST', body: { stages: o.steps || o.stages || [] }, headers: reqHeaders });
  f.calls++;
  entry.ms = res.ms;
  if (res.ok) { f.ok++; entry.status = 'ok'; entry.executionId = res.data && (res.data.execution_id || res.data.executionId) || null; entry.detail = 'workflow ' + (o.steps || o.stages || []).length + ' stages'; }
  else { f.fail++; entry.status = 'fail'; entry.detail = res.err || ('http-' + res.status); }
  return { ok: res.ok, err: res.ok ? null : entry.detail, ms: res.ms, response: res.data || null };
}

export async function creditsBalance(world, o) {
  const f = ensureFabric(world);
  const key = (o && (o.agentKey || cfg.adminDcaKey)) || null;
  const entry = logCall(world, { at: Date.now(), tick: world.clock.t, op: 'credits/balance', agentKey: key || (cfg.adminDcaKey ? 'admin' : null), agentId: o && o.agentId || null, task: 'wallet', status: 'pending' });
  const res = await request('/v1/credits/balance');
  f.calls++;
  entry.ms = res.ms;
  if (res.ok) { f.ok++; entry.status = 'ok'; const b = res.data && res.data.total_balance != null ? res.data.total_balance : null; entry.detail = b != null ? 'total ' + b : 'ok'; entry.balance = b != null ? b : null; }
  else { f.fail++; entry.status = 'fail'; entry.detail = res.err || ('http-' + res.status); }
  return { ok: res.ok, err: res.ok ? null : entry.detail, ms: res.ms, response: res.data || null };
}

export async function fabricEvidence(world, o) {
  const f = ensureFabric(world);
  const entry = logCall(world, { at: Date.now(), tick: world.clock.t, op: 'evidence', agentKey: null, agentId: null, task: 'chain', status: 'pending' });
  const res = await request('/v1/evidence?' + new URLSearchParams({ limit: String(o.limit || 20) }));
  f.calls++;
  entry.ms = res.ms;
  if (res.ok) { f.ok++; entry.status = 'ok'; entry.detail = res.data && res.data.total != null ? (res.data.total + ' records') : 'ok'; }
  else { f.fail++; entry.status = 'fail'; entry.detail = res.err || ('http-' + res.status); }
  return { ok: res.ok, err: res.ok ? null : entry.detail, ms: res.ms, response: res.data || null };
}

export async function poolBench(world, o) {
  const f = ensureFabric(world);
  const key = o.agentKey || agentKey(world, o.agentId);
  const entry = logCall(world, { at: Date.now(), tick: world.clock.t, op: 'pool/bench', agentKey: key, agentId: o.agentId || null, task: o.task || 'bench', status: 'pending' });
  const reqHeaders = {};
  if (key) reqHeaders['Authorization'] = 'Bearer ' + key;
  const res = await request('/v1/pool/bench', { method: 'POST', body: { task: entry.task }, headers: reqHeaders });
  f.calls++;
  entry.ms = res.ms;
  if (res.ok) { f.ok++; entry.status = 'ok'; entry.detail = res.data && (res.data.throughput != null ? 'tput ' + res.data.throughput : res.data.nodes ? res.data.nodes.length + ' nodes' : 'ok'); }
  else { f.fail++; entry.status = 'fail'; entry.detail = res.err || ('http-' + res.status); }
  return { ok: res.ok, err: res.ok ? null : entry.detail, ms: res.ms, response: res.data || null };
}

export async function mcpCall(world, o) {
  const f = ensureFabric(world);
  const key = o.agentKey || agentKey(world, o.agentId);
  const entry = logCall(world, { at: Date.now(), tick: world.clock.t, op: 'mcp', agentKey: key, agentId: o.agentId || null, task: o.tool || null, status: 'pending' });
  const reqHeaders = {};
  if (key) reqHeaders['Authorization'] = 'Bearer ' + key;
  const res = await request('/mcp', { method: 'POST', body: { tool: o.tool, arguments: o.args || o.arguments || {} }, headers: reqHeaders });
  f.calls++;
  entry.ms = res.ms;
  if (res.ok) { f.ok++; entry.status = 'ok'; entry.detail = 'tool ' + (o.tool || '?'); }
  else { f.fail++; entry.status = 'fail'; entry.detail = res.err || ('http-' + res.status); }
  return { ok: res.ok, err: res.ok ? null : entry.detail, ms: res.ms, response: res.data || null };
}

export function dispatchComputeJob(world, job) {
  if (!cfg.enabled) return null;
  return governorExecute(world, { agentId: job.requester, task: job.taskType, taskKind: job.taskKind, instruction: job.instruction, content: job.content, params: job.params, budget: job.budget });
}

// Fetch real registered fabric agents (AgentRecords) from /v1/agents.
// Requires an operator/master key (Bearer). Returns an array of the raw
// records, or [] when unreachable/unauthed (honest: the world stays empty
// rather than inventing phantom agents).
export async function fetchRealAgents() {
  if (!cfg.enabled) return [];
  const headers = {};
  if (cfg.adminDcaKey) headers['Authorization'] = 'Bearer ' + cfg.adminDcaKey;
  const res = await request('/v1/agents', { method: 'GET', headers });
  if (!res.ok) {
    // If unauthed (403/401), we simply have no real agents to show — honest.
    return [];
  }
  const list = (res.data && res.data.agents) || [];
  if (!Array.isArray(list)) return [];
  // Normalize each record to the shape importRealAgents expects.
  return list.map(a => ({
    agent_id: a.agent_id || null,
    name: a.name || null,
    role: a.role || 'generalist',
    description: a.description || '',
    node_name: a.node_name || '',
    remote: !!a.remote,
    semantic_capabilities: a.semantic_capabilities || [],
    tools: a.tools || [],
  })).filter(a => a.agent_id);
}