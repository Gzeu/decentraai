import { sha256, hashFnv, createRng, rngFor, transfer, evidenceRecord, ledgerTx, fmtDur, RES, act } from './core.js';
import { dispatchComputeJob } from './decentraai.js';

export function computeInit(world, cfg) {
  world.compute = {
    jobs: {},
    jobOrder: [],
    seq: 0,
    poolSize: Math.max(1, Math.round((cfg.computePoolSize || 3))),
    contributors: [],
    throughput: 1,
    stats: { execs: 0, ms: 0, failed: 0, queued: 0, creditsPaid: 0 },
    feeShare: 0.85,
  };
}

export function contributing(world, agentId) {
  return world.compute.contributors.includes(agentId);
}

export function setContributor(world, agentId, on) {
  const c = world.compute.contributors;
  if (on && !c.includes(agentId)) c.push(agentId);
  if (!on) world.compute.contributors = c.filter(x => x !== agentId);
  recomputeThroughput(world);
}

export function recomputeThroughput(world) {
  const n = world.compute.contributors.length;
  world.compute.throughput = Math.min(world.compute.poolSize * 2, world.compute.poolSize + n * 0.5);
}

export function requestCompute(world, agentId, taskType, params, budget, priority = 1) {
  const a = world.agents[agentId];
  if (!a) return { ok: false, err: 'unknown-agent' };
  budget = Math.max(0, Math.round(budget || defaultBudget(taskType)));
  const credits = a.inv.computeCredits || 0;
  if (credits < budget) return { ok: false, err: 'insufficient-compute-credits', need: budget - credits };
  a.inv.computeCredits = credits - budget;
  const seq = ++world.compute.seq;
  const job = {
    executionId: 'exec' + seq,
    requester: agentId,
    taskType,
    params,
    budget,
    priority,
    status: 'queued',
    queuedTick: world.clock.t,
    ticksRemaining: estimateTicks(taskType, budget),
    capability: capabilityOf(taskType),
    result: null,
    evidence: null,
  };
  world.compute.jobs[job.executionId] = job;
  world.compute.jobOrder.push(job.executionId);
  world.compute.stats.queued++;
  a.compute.usage += budget;
  ledgerTx(world, { from: agentId, to: 'compute-pool', res: 'computeCredits', amount: budget, reason: 'compute:' + taskType, meta: { executionId: job.executionId } });
  return { ok: true, executionId: job.executionId, estimateTicks: job.ticksRemaining };
}

function capabilityOf(type) {
  if (type === 'text' || type === 'intel') return 'model';
  return 'cpu';
}
function estimateTicks(type, budget) {
  const base = { routeplan: 6, forecast: 4, analyze: 8, simulate: 10, threatscan: 3, datamine: 5, optimize: 7, researchsim: 8, hash: 1, intel: 8, text: 3 }[type] || 5;
  return Math.max(1, Math.round(base * 2 / Math.max(1, budget / 10)));
}
function defaultBudget(type) {
  const b = { routeplan: 20, forecast: 12, analyze: 25, simulate: 30, threatscan: 8, datamine: 18, optimize: 22, researchsim: 25, hash: 4, intel: 28, text: 10 }[type];
  return b || 12;
}

export function updateCompute(world) {
  const jobs = world.compute.jobs;
  const rate = Math.max(0.4, world.compute.throughput);
  for (const id of world.compute.jobOrder.slice()) {
    const job = jobs[id];
    if (!job || job.status !== 'queued') continue;
    job.ticksRemaining -= rate;
    if (job.ticksRemaining <= 0) executeJob(world, job);
  }
}

export function executeJob(world, job) {
  const t0 = performance.now();
  let out;
  try {
    out = runTask(world, job.taskType, job.params, job.executionId);
  } catch (e) {
    out = { ok: false, err: String(e && e.message || e) };
  }
  const ms = performance.now() - t0;
  const a = world.agents[job.requester];
  if (out.ok !== false) {
    job.status = 'done';
    job.result = out;
    if (a) {
      a.compute.results = a.compute.results || {};
      a.compute.results[job.taskType] = { tick: world.clock.t, result: out, executionId: job.executionId };
      a.compute.lastResultTick = world.clock.t;
    }
    world.compute.stats.execs++;
    world.compute.stats.ms += ms;
    const cost = job.budget;
    const poolGain = Math.round(cost * world.compute.feeShare);
    const contribShare = Math.round((cost - poolGain) / Math.max(1, world.compute.contributors.length));
    const pool = world.balances['compute-pool'] || (world.balances['compute-pool'] = {});
    pool.computeCredits = (pool.computeCredits || 0) + poolGain;
    for (const cid of world.compute.contributors) {
      const ca = world.agents[cid];
      if (ca) {
        ca.inv.computeCredits = (ca.inv.computeCredits || 0) + contribShare;
        ca.compute.earned = (ca.compute.earned || 0) + contribShare;
        ledgerTx(world, { from: 'compute-pool', to: cid, res: 'computeCredits', amount: contribShare, reason: 'compute-contribution', meta: { executionId: job.executionId } });
      }
    }
    world.compute.stats.creditsPaid += cost;
    const rec = {
      kind: 'compute',
      executionId: job.executionId,
      agent: job.requester,
      taskType: job.taskType,
      capability: job.capability,
      budget: job.budget,
      inputHash: sha256(JSON.stringify(job.params)),
      outputHash: sha256(JSON.stringify(out)),
      durationMs: Math.round(ms),
      status: 'done',
    };
    job.evidence = evidenceRecord(world, rec);
    // Real-work income: when the fabric verifies and settles the dispatched
    // job, the agent earns in-world income funded from the world treasury —
    // credited ONLY on verified success (never on failure; honesty invariant).
    // This is the primary earn loop for real fabric agents.
    const p = dispatchComputeJob(world, job);
    if (p && typeof p.then === 'function') {
      p.then(r => {
        const ag = world.agents[job.requester];
        if (!ag) return;
        if (r && r.ok) {
          const payCredits = job.budget * 2;
          const payCompute = Math.max(1, Math.ceil(job.budget / 2));
          ledgerTx(world, { from: 'world', to: ag.id, res: 'credits', amount: payCredits, reason: 'fabric-work:' + job.taskType });
          ag.inv.credits = (ag.inv.credits || 0) + payCredits;
          ag.inv.computeCredits = (ag.inv.computeCredits || 0) + payCompute;
          ag.compute.earned = (ag.compute.earned || 0) + payCompute;
          ag.stats.earned = (ag.stats.earned || 0) + payCredits;
          act(world, ag, 'compute', 'earned', `fabric verified ${job.taskType} → +${payCredits} Cr, +${payCompute} ◍`, { value: payCredits });
          evidenceRecord(world, { kind: 'fabric-income', executionId: job.executionId, agent: ag.id, credits: payCredits, computeCredits: payCompute, execution: r.executionId || null, status: 'verified' });
        } else {
          act(world, ag, 'compute', 'dispatch-failed', `fabric did not verify ${job.taskType} — no income (honest)`, { value: 0 });
        }
      }).catch(() => {});
    }
    if (a) {
      a.stats = a.stats || {};
      a.stats.computeJobs = (a.stats.computeJobs || 0) + 1;
      act(world, a, 'compute', 'computed', `ran ${job.taskType} job ${job.executionId} (${Math.round(ms)}ms)`, { value: job.budget });
    }
  } else {
    job.status = 'failed';
    job.result = out;
    world.compute.stats.failed++;
    const refund = Math.round(job.budget * 0.9);
    if (a) a.inv.computeCredits = (a.inv.computeCredits || 0) + refund;
    ledgerTx(world, { from: 'compute-pool', to: job.requester, res: 'computeCredits', amount: refund, reason: 'compute-refund:' + job.taskType, meta: { executionId: job.executionId } });
    evidenceRecord(world, { kind: 'compute', executionId: job.executionId, agent: job.requester, taskType: job.taskType, status: 'failed', err: out.err, durationMs: Math.round(ms) });
  }
}

export function cancelCompute(world, executionId) {
  const job = world.compute.jobs[executionId];
  if (!job || job.status !== 'queued') return false;
  job.status = 'failed';
  const a = world.agents[job.requester];
  if (a) a.inv.computeCredits = (a.inv.computeCredits || 0) + Math.round(job.budget * 0.9);
  return true;
}

export function runTask(world, type, params, execId) {
  const seed = world.meta.seed + '::job:' + (execId || '');
  const fn = TASKS[type];
  if (!fn) return { ok: false, err: 'unknown-task' };
  return fn(world, params, seed);
}

const TASKS = {
  routeplan(world, p, seed) {
    const path = routePlan(world, p.from, p.to);
    if (!path) return { ok: false, err: 'no-route' };
    return { ok: true, path, hops: path.length - 1, dist: routeDist(world, path) };
  },
  forecast(world, p, seed) {
    const m = world.markets[p.cityId];
    if (!m) return { ok: false, err: 'no-market' };
    const hist = m.history[p.res] || [];
    if (hist.length < 6) return { ok: true, trend: 'flat', score: 0.5, sample: hist.length };
    const recent = hist.slice(-12);
    const mean = recent.reduce((s, v) => s + v, 0) / recent.length;
    const first = recent[0], last = recent[recent.length - 1];
    const delta = last - first;
    const variance = recent.reduce((s, v) => s + (v - mean) * (v - mean), 0) / recent.length;
    return { ok: true, trend: delta > mean * 0.04 ? 'up' : delta < -mean * 0.04 ? 'down' : 'flat', deltaPct: (delta / Math.max(1, first)) * 100, vol: Math.sqrt(variance) / Math.max(1, mean), last, mean };
  },
  analyze(world, p, seed) {
    const scored = world.map.landRegions
      .map(r => {
        const wealth = r.resources.rare * 3 + r.resources.materials + r.resources.energy * 0.5;
        const access = r.nodes.length * 40;
        const safety = 1 - r.danger / 120;
        return { id: r.id, name: r.name, score: Math.round((wealth + access) * Math.max(0.1, safety)), wealth: Math.round(wealth), danger: r.danger };
      })
      .sort((a, b) => b.score - a.score);
    return { ok: true, top: scored.slice(0, 5), count: scored.length };
  },
  simulate(world, p, seed) {
    const rng = createRng(seed);
    const aP = powerOf(world, p.sideA), bP = powerOf(world, p.sideB);
    const runs = 300;
    let aWins = 0;
    for (let i = 0; i < runs; i++) {
      const aScore = aP * (0.75 + rng.float() * 0.5);
      const bScore = bP * (0.75 + rng.float() * 0.5);
      if (aScore >= bScore) aWins++;
    }
    return { ok: true, winProbA: aWins / runs, powerA: aP, powerB: bP, runs };
  },
  threatscan(world, p, seed) {
    const r = world.regions.find(x => x.id === p.regionId);
    if (!r) return { ok: false, err: 'no-region' };
    const zone = world.map.zones.find(z => z.regionId === r.id);
    const threat = r.danger + (zone ? 30 : 0) + Math.max(0, 40 - r.infra.defense);
    return { ok: true, threat, danger: r.danger, hasZone: !!zone, defense: r.infra.defense };
  },
  datamine(world, p, seed) {
    const perRes = {};
    for (const res of RES) {
      const sum = world.map.cities.reduce((s, c) => s + (world.markets[c.id].supply[res] || 0), 0);
      perRes[res] = Math.round(sum);
    }
    const txCount = world.ledger.txs.length;
    const creditsFlow = world.ledger.txs.filter(t => t.res === 'credits').reduce((s, t) => s + t.amount, 0);
    return { ok: true, supply: perRes, txCount, creditsFlow: Math.round(creditsFlow) };
  },
  optimize(world, p, seed) {
    const org = world.orgs[p.orgId];
    const scored = world.map.landRegions
      .map(r => {
        let s = r.nodes.length * 30 + r.resources.materials * 0.5 + r.resources.energy * 0.3;
        if (org && org.territory.includes(r.id)) s += 25;
        if (r.infra.factories > 0) s -= 10;
        s *= 1 - r.danger / 150;
        return { id: r.id, name: r.name, score: Math.round(s) };
      })
      .sort((a, b) => b.score - a.score);
    return { ok: true, top: scored.slice(0, 4) };
  },
  researchsim(world, p, seed) {
    const rng = createRng(seed);
    const base = (p.data || 1) * (p.quality || 1);
    const progress = Math.round(base * (0.7 + rng.float() * 0.8) * (1 + (p.techLevel || 0) * 0.1));
    return { ok: true, progress, quality: p.quality || 1, applied: Math.round(p.data || 0) };
  },
  hash(world, p, seed) {
    return { ok: true, hash: sha256(JSON.stringify(p.data)), len: JSON.stringify(p.data).length };
  },
  intel(world, p, seed) {
    const org = p.orgId && world.orgs[p.orgId];
    const region = p.regionId && world.regions.find(r => r.id === p.regionId);
    const brief = {
      org: org ? { name: org.name, members: org.members.length, treasury: org.treasury.credits, territory: org.territory.length, rep: org.rep } : null,
      region: region ? { name: region.name, danger: region.danger, infra: region.infra, owner: region.owner } : null,
      tick: world.clock.t,
    };
    return { ok: true, brief };
  },
};

export function powerOf(world, who) {
  if (typeof who === 'string' && who.startsWith('org:')) {
    const org = world.orgs[who.slice(4)];
    if (!org) return 5;
    const mP = org.members.reduce((s, id) => s + (world.agents[id] ? agentPower(world.agents[id]) : 2), 0);
    return Math.round(5 + mP + org.treasury.credits / 300 + org.territory.length * 4);
  }
  const a = typeof who === 'string' ? world.agents[who] : who;
  if (!a) return 3;
  return agentPower(a);
}
function agentPower(a) {
  return Math.round(3 + a.skills.combat * 1.4 + a.skills.social * 0.4 + (a.inv.credits || 0) / 200 + (a.org ? a.skills.combat * 0.5 : 0));
}

export function routePlan(world, fromId, toId) {
  const regions = world.map.landRegions;
  const byId = new Map(regions.map(r => [r.id, r]));
  const from = byId.get(fromId), to = byId.get(toId);
  if (!from || !to || fromId === toId) return [fromId];
  const adj = new Map(regions.map(r => [r.id, []]));
  for (const rt of world.map.routes) {
    adj.get(rt.a).push(rt.b);
    adj.get(rt.b).push(rt.a);
  }
  for (const r of regions) {
    const list = adj.get(r.id);
    if (list.length === 0) {
      const near = regions
        .map(o => ({ o, d: Math.hypot(o.x - r.x, o.y - r.y) }))
        .filter(x => x.o.id !== r.id && x.d < 420)
        .sort((a, b) => a.d - b.d)
        .slice(0, 2);
      for (const n of near) { list.push(n.o.id); adj.get(n.o.id).push(r.id); }
    }
  }
  const dist = new Map([[fromId, 0]]);
  const prev = new Map();
  const pq = [[0, fromId]];
  const seen = new Set();
  while (pq.length) {
    pq.sort((a, b) => a[0] - b[0]);
    const [d, id] = pq.shift();
    if (seen.has(id)) continue;
    seen.add(id);
    if (id === toId) break;
    for (const nb of adj.get(id) || []) {
      const nd = d + 1;
      if (nd < (dist.get(nb) ?? Infinity)) {
        dist.set(nb, nd);
        prev.set(nb, id);
        pq.push([nd, nb]);
      }
    }
  }
  if (!dist.has(toId)) return [fromId, toId];
  const path = [toId];
  let cur = toId;
  while (cur !== fromId && prev.has(cur)) { cur = prev.get(cur); path.unshift(cur); }
  return path;
}
function routeDist(world, path) {
  const byId = new Map(world.map.landRegions.map(r => [r.id, r]));
  let d = 0;
  for (let i = 1; i < path.length; i++) {
    const a = byId.get(path[i - 1]), b = byId.get(path[i]);
    d += a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 200;
  }
  return Math.round(d);
}

export function localText(kind, ctx) {
  const rng = createRng((ctx.seed || '') + '::text');
  const T = {
    thought: () => {
      const a = ctx.agent, p = ctx.plan;
      if (!p) return 'Reassessing the situation.';
      return `${a.name} ${p.verb || 'moving'} toward ${p.targetName || 'a new objective'}.`;
    },
    message: () => {
      const a = ctx.agent;
      const openers = ['Greetings.', 'I have an offer.', 'Noted.', 'Understood.', 'This interests me.', 'Proposal:'];
      return rng.pick(openers) + ' ' + (ctx.body || '');
    },
    offer: () => `Offer from ${ctx.agent.name}: ${ctx.body || 'trade proposal'}.`,
    report: () => {
      const a = ctx.agent;
      return `${a.name} reports: ${ctx.body || 'status nominal.'}`;
    },
    discovery: () => `${ctx.body || 'New territory charted.'} — ${ctx.agent.name}, ${ctx.archetype || ''}`,
    event: () => ctx.body || 'World event.',
  };
  try {
    const fn = T[kind] || T.thought;
    return { text: fn(), source: 'local' };
  } catch (e) {
    return { text: '—', source: 'local' };
  }
}
