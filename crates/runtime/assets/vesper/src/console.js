import { regionName, createContract, acceptContract, sendMessage, joinOrg, leaveOrg, foundOrg, completeContract } from './sim.js';
import { tickLabel, balance, grant, transfer, createRng } from './core.js';
import { requestCompute, setContributor, contributing, powerOf, runTask, updateCompute } from './compute.js';
import { statusOf, probe, governorExecute, fabricWorkflow, creditsBalance, fabricEvidence, poolBench, mcpCall, agentKey } from './decentraai.js';

export function createVesper(getWorld) {
  const api = {};
  const w = () => getWorld();
  const agent = (id) => {
    const world = w();
    if (!world) throw new Error('world-not-loaded');
    const a = world.agents[id];
    if (!a) throw new Error('unknown-agent: ' + id);
    return a;
  };

  api.discover_world = () => {
    const world = w();
    const cities = world.map.cities.map(c => ({ id: c.id, name: c.name, regionId: c.regionId, x: c.x, y: c.y, marketId: c.marketId }));
    return {
      tick: world.clock.t,
      time: tickLabel(world.clock.t),
      seed: world.meta.seed,
      worldId: world.meta.id,
      regions: world.regions.map(r => ({ id: r.id, name: r.name || 'Uncharted', biome: r.biome, x: r.x, y: r.y, land: r.land, owner: r.owner, danger: r.danger, explored: !!r.explored, infra: { ...r.infra } })),
      cities,
      zones: world.zones.map(z => ({ regionId: z.regionId, kind: z.kind })),
      routes: world.map.routes.map(rt => ({ a: rt.a, b: rt.b, kind: rt.kind })),
      agents: world.agentOrder.map(id => { const a = world.agents[id]; return { id, name: a.name, archetype: a.archetype, regionId: regionOfId(world, a), status: a.status }; }),
      orgs: world.orgOrder.map(id => { const o = world.orgs[id]; return { id, name: o.name, type: o.type, members: o.members.length, territory: o.territory.length, treasury: Math.round(o.treasury.credits), rep: o.rep }; }),
      contracts: Object.values(world.contracts).map(c => ({ id: c.id, title: c.title, state: c.state, kind: c.objective.kind })),
      markets: world.marketOrder.map(mid => ({ cityId: mid, prices: { ...world.markets[mid].prices } })),
    };
  };

  api.get_agent_state = (agentId) => {
    const a = agent(agentId);
    return {
      agent_id: a.id, name: a.name, archetype: a.archetype, avatar: a.avatar, color: a.color,
      location: { regionId: regionOfId(w(), a), travel: a.loc.travel },
      personality: a.personality, skills: a.skills,
      objective: a.planGoal, planKey: a.planKey, plan: (a.plans || []).map(s => s.kind + (s.target || s.res || s.contractId || '')),
      status: a.status,
      // Layer 1 — state + economic + social (v2)
      state: { energy: Math.round(a.energy || 0), focus: Math.round(a.focus || 0), morale: Math.round(a.morale || 0) },
      economy: { credits: Math.round(a.credits || 0), compute: Math.round(a.compute || 0), data: Math.round(a.data || 0) },
      social: { reputation: Math.round(a.reputation || 0), trust: a.trust || {}, experience: a.experience || {} },
      // Back-compat flat + legacy inventory mirror
      inventory: { credits: Math.round(a.credits || 0), compute: Math.round(a.compute || 0), data: Math.round(a.data || 0), energy: Math.round(a.energy || 0) },
      org: a.org, orgRole: a.orgRole,
      reputation: a.reputation, repScore: (a.rep && a.rep.score) || a.reputation,
      reputationDetail: a.rep,
      relationships: a.relations,
      wealth: a.wealth,
      compute: { usage: Math.round(a.computeTrack.usage), earned: Math.round(a.computeTrack.earned), contributed: a.computeTrack.contributed, results: a.computeTrack.results },
      achievements: a.achievements.slice(-20),
      tick: w().clock.t,
    };
  };

  api.get_location = (agentId) => {
    const a = agent(agentId);
    const world = w();
    const rid = regionOfId(world, a);
    const r = world.regions.find(x => x.id === rid);
    const city = world.map.cities.find(c => c.regionId === rid);
    return {
      regionId: rid, name: r.name, biome: r.biome, x: r.x, y: r.y,
      danger: r.danger, owner: r.owner, infra: r.infra, city: city ? city.name : null,
      resources: r.resources, neighbors: nearbyRegionIds(world, rid, 3),
    };
  };

  api.inspect_market = (cityId, res) => {
    const world = w();
    const m = world.markets[cityId];
    if (!m) throw new Error('unknown-market: ' + cityId);
    const out = { cityId, prices: m.prices, supply: m.supply, credits: m.credits, priceIdx: m.priceIdx };
    if (res) out.history = (m.history[res] || []).slice(-48);
    return out;
  };

  api.inspect_contracts = (filter) => {
    const world = w();
    return Object.values(world.contracts).filter(c => !filter || c.state === filter).map(c => ({
      id: c.id, title: c.title, state: c.state, issuer: c.issuer, objective: c.objective,
      reward: c.reward, deadlineTick: c.deadlineTick, risk: Math.round(c.risk * 100), progress: Math.round(c.progress), target: c.target,
    }));
  };

  api.create_contract = (agentId, opts) => {
    const a = agent(agentId);
    const world = w();
    const contract = createContract(world, {
      issuer: a.id, kind: opts.kind, target: opts.target, rewardCredits: opts.rewardCredits, rewardData: opts.rewardData || 0, rewardCompute: opts.rewardCompute || 0,
      deadlineTicks: opts.deadlineTicks || 120, risk: opts.risk || 0.4, title: opts.title, reqSkill: opts.reqSkill, reqSkillMin: opts.reqSkillMin,
    });
    return contract ? { ok: true, id: contract.id, title: contract.title } : { ok: false, err: 'create-failed' };
  };

  api.accept_contract = (agentId, contractId) => {
    const a = agent(agentId);
    const world = w();
    const c = world.contracts[contractId];
    if (!c) throw new Error('unknown-contract');
    if (c.state !== 'open') return { ok: false, err: 'contract-not-open', state: c.state };
    return acceptContract(world, a, c) ? { ok: true } : { ok: false, err: 'accept-failed' };
  };

  api.communicate = (fromId, toId, kind, body, meta) => {
    const from = agent(fromId);
    const world = w();
    const msg = sendMessage(world, from.id, toId, kind || 'message', body, meta || {});
    return { ok: !!msg, id: msg && msg.id };
  };

  api.form_team = (agentId, opts) => {
    const a = agent(agentId);
    const world = w();
    const res = foundOrg(world, a, opts && opts.name, opts && opts.type, createRng('console::' + Date.now()));
    return res;
  };

  api.leave_team = (agentId) => {
    const a = agent(agentId);
    return { ok: leaveOrg(w(), a) };
  };

  api.join_org = (agentId, orgId) => {
    const a = agent(agentId);
    const world = w();
    const org = world.orgs[orgId];
    if (!org) throw new Error('unknown-org');
    return { ok: joinOrg(world, a, org) };
  };

  api.request_compute = (agentId, taskType, params, budget, priority) => {
    const a = agent(agentId);
    const world = w();
    const res = requestCompute(world, a.id, taskType, params || {}, budget || undefined, priority || 1);
    if (res.ok) updateCompute(world);
    return res;
  };

  api.inspect_compute = () => {
    const world = w();
    return {
      poolSize: world.compute.poolSize,
      throughput: Math.round(world.compute.throughput * 10) / 10,
      contributors: world.compute.contributors.map(id => world.agents[id] && world.agents[id].name),
      stats: world.compute.stats,
      jobs: world.compute.jobOrder.map(id => { const j = world.compute.jobs[id]; return { executionId: j.executionId, requester: j.requester, taskType: j.taskType, status: j.status, budget: j.budget, ticksRemaining: Math.round(j.ticksRemaining), capability: j.capability }; }),
    };
  };

  api.inspect_evidence = (limit) => {
    const world = w();
    return { chainHead: world.evidence.chainHead, count: world.evidence.count, records: world.evidence.records.slice(-(limit || 100)) };
  };

  api.inspect_reputation = (agentId) => {
    const a = agent(agentId);
    return { ...a.rep, score: a.rep.score };
  };

  api.inspect_memory = (agentId) => {
    const a = agent(agentId);
    return [...a.memory].sort((x, y) => y.importance - x.importance).map(m => ({ type: m.type, text: m.text, importance: m.importance, tick: m.tick, tags: m.tags || [] }));
  };

  api.observe_events = (limit, afterTick) => {
    const world = w();
    let evs = world.events;
    if (afterTick != null) evs = evs.filter(e => e.tick > afterTick);
    return evs.slice(-(limit || 60)).map(e => ({ ...e, time: tickLabel(e.tick) }));
  };

  api.inspect_ledger = (limit) => {
    const world = w();
    return world.ledger.txs.slice(-(limit || 80)).map(t => ({ ...t, time: tickLabel(t.tick) }));
  };

  api.inspect_balances = () => {
    const world = w();
    return { ...world.balances };
  };

  api.get_org_state = (orgId) => {
    const world = w();
    const o = world.orgs[orgId];
    if (!o) throw new Error('unknown-org');
    return {
      id: o.id, name: o.name, type: o.type, leaderId: o.leaderId, founderId: o.founderId,
      members: o.members.map(mid => { const a = world.agents[mid]; return { id: mid, name: a && a.name, role: a && a.orgRole }; }),
      treasury: o.treasury, territory: o.territory.map(rid => regionName(world, rid)), assets: o.assets,
      rep: o.rep, policies: o.policies, objectives: o.objectives,
      power: powerOf(world, 'org:' + o.id),
      history: o.history.slice(-20),
    };
  };

  api.execute_action = (agentId, action) => {
    const a = agent(agentId);
    const world = w();
    if (!action || !action.kind) throw new Error('action requires .kind');
    const kinds = ['travel', 'gather', 'mine', 'buy', 'sell', 'explore', 'research', 'build', 'patrol', 'rest', 'contest', 'found-org', 'sabotage'];
    if (!kinds.includes(action.kind)) throw new Error('unsupported action kind: ' + action.kind);
    a.plans = [{ ...action }];
    a.stepIx = 0;
    a.planDone = false;
    a.plan = true;
    a.planKey = 'external:' + action.kind;
    a.planGoal = 'External directive: ' + action.kind;
    a.replanCooldown = 10;
    return { ok: true, queued: action.kind, agentId: a.id };
  };

  api.list_agents = () => {
    const world = w();
    return world.agentOrder.map(id => {
      const a = world.agents[id];
      return { id, name: a.name, archetype: a.archetype, credits: Math.round(a.credits || 0), rep: a.reputation || (a.rep && a.rep.score) || 0, regionId: regionOfId(world, a), org: a.org, status: a.status };
    });
  };

  api.list_orgs = () => {
    const world = w();
    return world.orgOrder.map(id => { const o = world.orgs[id]; return { id, name: o.name, type: o.type, members: o.members.length, treasury: Math.round(o.treasury.credits), territory: o.territory.length, rep: o.rep, power: powerOf(world, 'org:' + o.id) }; });
  };

  api.get_world_state = () => {
    const world = w();
    return {
      tick: world.clock.t, time: tickLabel(world.clock.t), stats: { ...world.stats },
      tech: Object.fromEntries(Object.entries(world.fact.tech).map(([k, v]) => [k, { level: v.level, progress: Math.round(v.progress), target: v.target }])),
      worldFund: world.balances['world'] || {}, computeStats: world.compute.stats,
      agentCount: world.agentOrder.length, orgCount: world.orgOrder.length, marketCount: world.marketOrder.length,
    };
  };

  api.fabric_status = () => {
    const world = w();
    return { ...statusOf(world), agentKeys: world.agentOrder.slice(0, 3).map(id => ({ agentId: id, key: agentKey(world, id) })) };
  };

  api.fabric_probe = async () => probe(w());

  api.fabric_governor_execute = async (opts) => {
    const world = w();
    if (!opts || !opts.agentId) throw new Error('requires {agentId, taskType, params, budget}');
    return governorExecute(world, { agentId: opts.agentId, task: opts.taskType, params: opts.params || {}, budget: opts.budget });
  };

  api.fabric_workflow = async (opts) => {
    const world = w();
    if (!opts || !opts.agentId) throw new Error('requires {agentId, steps}');
    return fabricWorkflow(world, { agentId: opts.agentId, steps: opts.steps || [] });
  };

  api.fabric_credits_balance = async (opts) => {
    const world = w();
    return creditsBalance(world, { agentId: opts && opts.agentId });
  };

  api.fabric_credits_transfer = async (opts) => {
    const world = w();
    if (!opts || !opts.agentId || !opts.to || !opts.amount) throw new Error('requires {agentId, to, amount, reason}');
    // The real fabric has no public credit-transfer endpoint; report honestly.
    return { ok: false, err: 'credit-transfer-not-supported', response: null };
  };

  api.fabric_evidence = async (opts) => fabricEvidence(w(), { limit: opts && opts.limit });

  api.fabric_pool_bench = async (opts) => {
    const world = w();
    return poolBench(world, { agentId: opts && opts.agentId, task: opts && opts.task });
  };

  api.fabric_mcp = async (opts) => {
    const world = w();
    if (!opts || !opts.agentId || !opts.tool) throw new Error('requires {agentId, tool, args}');
    return mcpCall(world, { agentId: opts.agentId, tool: opts.tool, args: opts.args || {} });
  };

  return api;
}

function regionOfId(world, a) {
  return a.loc.travel ? a.loc.travel.path[a.loc.travel.idx] : a.loc.regionId;
}

function nearbyRegionIds(world, rid, n) {
  const here = world.regions.find(x => x.id === rid);
  if (!here) return [];
  return world.regions.filter(r => r.land && r.id !== rid).sort((a, b) => Math.hypot(a.x - here.x, a.y - here.y) - Math.hypot(b.x - here.x, b.y - here.y)).slice(0, n).map(r => r.id);
}
