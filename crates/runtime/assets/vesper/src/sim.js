import { buildWorld, createRng, rngFor, RES, MKT_RES, RES_LABEL, tickLabel, tickFull, sha256, hashFnv, pushEvent, evidenceRecord, evidenceInit, ledgerTx, transfer, grant, balance, act, AGENT_FIRST, AGENT_LAST, ORG_NAME, ORG_SUFFIX, BIOME_INFO } from './core.js';
import { computeInit, requestCompute, updateCompute, setContributor, contributing, runTask, routePlan, powerOf } from './compute.js';

export const ARCH = {
  explorer:    { label: 'Explorer',    w: { discover: 1.5, wealth: 0.5, research: 0.4, conflict: 0.15 }, sk: { navigation: 1.5, combat: 0.6, analysis: 0.5 }, tr: { curiosity: 0.8, openness: 0.8, risk: 0.55, ambition: 0.55 } },
  merchant:    { label: 'Merchant',    w: { wealth: 1.4, market: 1.3, contract: 0.6 }, sk: { trade: 1.6, social: 0.8 }, tr: { greed: 0.7, risk: 0.5, curiosity: 0.4, ambition: 0.7 } },
  trader:      { label: 'Trader',      w: { market: 1.3, wealth: 1.1, contract: 0.8 }, sk: { trade: 1.4, navigation: 0.6 }, tr: { greed: 0.6, risk: 0.6, ambition: 0.6 } },
  scientist:   { label: 'Scientist',   w: { research: 1.5, discover: 0.6, wealth: 0.3 }, sk: { science: 1.7, analysis: 1.4 }, tr: { curiosity: 0.75, risk: 0.4, ambition: 0.6 } },
  researcher:  { label: 'Researcher',  w: { research: 1.2, contract: 0.9, wealth: 0.5 }, sk: { science: 1.5, analysis: 1.0 }, tr: { curiosity: 0.6, risk: 0.4 } },
  engineer:    { label: 'Engineer',    w: { build: 1.5, wealth: 0.6, research: 0.4 }, sk: { engineering: 1.6, analysis: 0.8 }, tr: { consc: 0.8, risk: 0.35, ambition: 0.6 } },
  builder:     { label: 'Builder',     w: { build: 1.4, contract: 0.7, wealth: 0.6 }, sk: { engineering: 1.4 }, tr: { consc: 0.8, risk: 0.3 } },
  strategist:  { label: 'Strategist',  w: { power: 1.4, conflict: 1.1, wealth: 0.6 }, sk: { social: 1.2, combat: 0.7, analysis: 1.1 }, tr: { ambition: 0.85, risk: 0.6 } },
  diplomat:    { label: 'Diplomat',    w: { power: 0.9, social: 1.5, wealth: 0.5 }, sk: { social: 1.7, analysis: 0.6 }, tr: { agree: 0.8, loyalty: 0.7 } },
  mercenary:   { label: 'Mercenary',   w: { conflict: 1.3, wealth: 1.0, contract: 0.9 }, sk: { combat: 1.7, stealth: 1.0 }, tr: { risk: 0.75, greed: 0.6, loyalty: 0.4 } },
  guardian:    { label: 'Guardian',    w: { protect: 1.5, conflict: 0.6, power: 0.5 }, sk: { combat: 1.4, navigation: 0.6 }, tr: { loyalty: 0.85, caution: 0.5 } },
  opportunist: { label: 'Opportunist', w: { contract: 1.3, market: 0.9, wealth: 0.8, discover: 0.5 }, sk: { trade: 0.8, social: 0.8, stealth: 0.6 }, tr: { curiosity: 0.7, risk: 0.7, greed: 0.6 } },
};
const ARCH_KEYS = Object.keys(ARCH);
const ARCH_WEIGHTS = { explorer: 2, merchant: 2.4, trader: 2, scientist: 1.8, researcher: 1.6, engineer: 2, builder: 1.6, strategist: 1.4, diplomat: 1.4, mercenary: 1.6, guardian: 1.2, opportunist: 1.8 };
const DEFAULT_W = { market: 0.4, wealth: 0.4, research: 0.4, discover: 0.4, conflict: 0.3, contract: 0.4, build: 0.4, power: 0.4, social: 0.4, protect: 0.4 };

function credit(world, who, res, amount, reason) {
  if (!(amount > 0)) return;
  if (who && who.startsWith('org:')) {
    const org = world.orgs[who.slice(4)];
    if (org) { org.treasury[res] = (org.treasury[res] || 0) + amount; ledgerTx(world, { from: 'world', to: who, res, amount, reason }); return; }
  } else if (who && who.startsWith('market:')) {
    const m = world.markets[who.slice(7)];
    if (m) { m.credits += amount; ledgerTx(world, { from: 'world', to: who, res, amount, reason }); return; }
  } else if (world.agents[who]) {
    const a = world.agents[who];
    a[res] = (a[res] || 0) + amount;
    ledgerTx(world, { from: 'world', to: who, res, amount, reason });
    return;
  }
  grant(world, who, res, amount, reason);
}

function payAgent(world, who, a, res, amount, reason) {
  if (!(amount > 0)) return;
  const bf = balance(world, who);
  bf[res] = (bf[res] || 0) - amount;
  a[res] = (a[res] || 0) + amount;
  ledgerTx(world, { from: who, to: a.id, res, amount, reason });
}

// Experience is per-capability progression (layer 2): work changes what the
// agent can do next. Reputation is social capital (layer 1): earned only
// through verified outcomes.
function gainXp(a, skill, amount) {
  a.experience[skill] = (a.experience[skill] || 0) + amount;
}

export const BASE_PRICES = { data: 15, compute: 40 };
export const SUPPLY_TARGET = { data: 90, compute: 120 };
export const SKILLS = ['trade', 'science', 'engineering', 'social', 'navigation', 'combat', 'stealth', 'analysis'];
const PERSONALITY = ['openness', 'consc', 'extra', 'agree', 'neuro', 'risk', 'ambition', 'greed', 'curiosity', 'caution', 'loyalty'];

export function createWorld(seedStr, cfg) {
  const map = buildWorld(seedStr, cfg.regionCount || 24);
  if (map.cities[0]) {
    const starter = map.regions.find(r => r.id === map.cities[0].regionId);
    if (starter) { starter.infra.labs = 1; starter.infra.defense = 2; }
  }
  const world = {
    meta: { id: 'vesper-' + hashFnv(seedStr).toString(36), seed: seedStr, createdAt: Date.now(), version: 1 },
    clock: { t: 0, running: true, speed: cfg.baseSpeed || 1 },
    map,
    regions: map.regions,
    agents: {},
    agentOrder: [],
    orgs: {},
    orgOrder: [],
    markets: {},
    marketOrder: [],
    contracts: {},
    contractOrder: [],
    messages: [],
    ledger: { txs: [], count: 0 },
    balances: {},
    events: [],
    chronicle: [],
    decisions: [],
    activity: [],
    fact: { tech: {}, discoveries: [], disputes: [], shortages: [] },
    stats: { events: 0, txs: 0, contracts: 0, completedContracts: 0, agents: 0, orgs: 0, built: 0, explored: 0, research: 0, computeJobs: 0, disputes: 0, messages: 0, discoveries: 0, founded: 0, trades: 0 },
    zones: map.zones,
    narrative: { queue: [], processed: 0 },
  };
  grant(world, 'world', 'credits', 2e6, 'genesis-fund');
  grant(world, 'world', 'compute', 20000, 'genesis-fund');
  evidenceInit(world);
  computeInit(world, cfg);
  initMarkets(world);
  // Real fabric agents take priority: when cfg.realAgents (AgentRecords from
  // /v1/agents) are provided, the world is populated ONLY with them — no
  // procedural agents, orgs or contracts. Otherwise fall back to the legacy
  // procedural civilization.
  const real = (cfg.realAgents || []).filter(Boolean);
  if (real.length > 0) {
    importRealAgents(world, real);
  } else {
    createAgents(world, cfg.initialAgents || 24, cfg);
    seedOrganizations(world, cfg.seedOrganizations || 3);
    seedContracts(world);
  }
  pushEvent(world, { type: 'genesis', source: 'world', detail: 'The world wakes. A civilization of autonomous agents begins.' });
  world.stats.agents = world.agentOrder.length;
  world.stats.orgs = world.orgOrder.length;
  return world;
}

function initMarkets(world) {
  for (const city of world.map.cities) {
    const region = world.regions.find(r => r.id === city.regionId);
    const prices = {};
    const supply = {};
    for (const res of MKT_RES) {
      prices[res] = BASE_PRICES[res] * (0.85 + ((hashFnv(city.id + res) % 40) / 100));
      supply[res] = Math.round(SUPPLY_TARGET[res] * (0.6 + ((hashFnv(city.id + res + 's') % 50) / 100)));
    }
    const market = {
      id: city.id,
      cityId: city.id,
      regionId: region.id,
      prices, supply,
      demand: { data: 0, compute: 0 },
      history: { data: [], compute: [] },
      buyVol: { data: 0, compute: 0 },
      sellVol: { data: 0, compute: 0 },
      credits: 8000,
      priceIdx: 1,
    };
    city.marketId = city.id;
    world.markets[city.id] = market;
    world.marketOrder.push(city.id);
  }
}

function createAgents(world, count, cfg) {
  const rng = createRng(world.meta.seed + '::agents');
  const cityRegions = world.map.cities.map(c => c.regionId);
  const usedNames = new Set();
  for (let i = 0; i < count; i++) {
    let name;
    do { name = rng.pick(AGENT_FIRST) + ' ' + rng.pick(AGENT_LAST); } while (usedNames.has(name));
    usedNames.add(name);
    const archetype = weightedPick(rng, ARCH_KEYS, a => ARCH_WEIGHTS[a]);
    const arch = ARCH[archetype];
    const personality = {};
    for (const p of PERSONALITY) {
      let v = 0.3 + rng.float() * 0.5;
      if (arch.tr[p] != null) v = Math.min(0.95, Math.max(0.05, v * 0.5 + arch.tr[p] * 0.7));
      personality[p] = Math.round(v * 100) / 100;
    }
    const skills = {};
    for (const s of SKILLS) {
      let v = 1.5 + rng.float() * 2.5;
      if (arch.sk[s]) v = Math.min(5, v + arch.sk[s]);
      skills[s] = Math.round(v * 10) / 10;
    }
    const home = rng.pick(cityRegions);
    const agent = {
      id: 'a' + i,
      name,
      avatar: rng.pick(['◈', '◆', '✦', '◉', '▲', '●', '◆', '★', '◎', '◍', '❖', '✧']),
      color: rng.pick(['#5cc8ff', '#9d8cff', '#55e6a4', '#ffb454', '#ff6b6b', '#ffd166', '#f78cff', '#6ee7d0']),
      archetype,
      personality,
      skills,
      // Layer 1 — personal state (0-100): operational capacity, not life bars.
      energy: 70 + Math.round(rng.float() * 25),
      focus: 60 + Math.round(rng.float() * 30),
      morale: 55 + Math.round(rng.float() * 30),
      // Layer 1 — economic stocks (flat fields; ledger-tracked via credit()).
      credits: Math.round(250 + rng.float() * 650),
      compute: 20,
      data: 2,
      // Layer 1 — social capital.
      reputation: 50,
      trust: {},
      experience: {},
      w: Object.assign({}, DEFAULT_W, arch.w),
      goals: [],
      plan: null,
      stepIx: 0,
      planKey: 'start',
      planGoal: 'Begin',
      planStuckTicks: 0,
      replanCooldown: 0,
      loc: { type: 'region', regionId: home, travel: null },
      status: 'working',
      health: 1, // kept as a derived 0-1 read-only projection of energy+morale
      morale1: null,
      memory: [],
      relations: {},
      org: null,
      orgRole: null,
      rep: { reliability: 50, cooperation: 50, contribution: 50, disputes: 0, score: 50 },
      computeTrack: { usage: 0, contributed: 0, earned: 0, results: {}, lastResultTick: 0 },
      achievements: [],
      history: [],
      createdTick: 0,
      homeRegion: home,
      lastThought: { text: 'Entering the world.', source: 'local', tick: 0 },
      contracts: [],
      wealth: 0,
      stats: { earned: 0, spent: 0, taxesPaid: 0, contracts: 0, discoveries: 0, research: 0, breakthroughs: 0, built: 0, produced: 0, tradedVol: 0, computeJobs: 0 },
    };
    world.agents[agent.id] = agent;
    world.agentOrder.push(agent.id);
    addMemory(world, agent, { type: 'location', importance: 0.7, tags: ['region', home], regionId: home, text: 'Home region.' });
    const homeCity = world.map.cities.find(c => c.regionId === home);
    if (homeCity) addMemory(world, agent, { type: 'economic', importance: 0.6, tags: ['price', homeCity.id], cityId: homeCity.id, prices: { ...world.markets[homeCity.id].prices }, text: 'Home market prices.' });
    agent.lastThought = { text: `${agent.name} (${arch.label}) arrives at ${regionName(world, home)} to find their footing.`, source: 'local', tick: 0 };
  }
  world.agentOrder.sort();
}

/// Import real fabric agents (AgentRecords from /v1/agents) as the world's
/// entities. Each becomes a first-class agent whose identity, role and
/// capabilities come from the REAL fabric record; the world keeps only a thin
/// deterministic economic shell around them. No names/orgs/contracts are
/// invented.
function importRealAgents(world, realAgents) {
  const rng = createRng(world.meta.seed + '::realagents');
  const cityRegions = world.map.cities.map(c => c.regionId);
  realAgents.forEach((r, i) => {
    const rec = r.record || r;
    const id = rec.agent_id || ('agent-' + i);
    const name = rec.name || rec.agent_id || ('Agent ' + i);
    const role = (rec.role || 'generalist').toLowerCase();
    // Map a fabric role to a compatible agent archetype so the decision loop
    // has a shape; generalist is the common real role.
    const archetype = ROLE_ARCH[role] || 'explorer';
    const arch = ARCH[archetype] || ARCH.explorer;
    const home = cityRegions.length ? cityRegions[Math.floor(hashFnv(id) % cityRegions.length)] : 0;
    const skills = {};
    for (const s of SKILLS) skills[s] = 1.5 + ((hashFnv(id + s) % 30) / 10);
    const personality = {};
    for (const p of PERSONALITY) personality[p] = Math.round((0.3 + ((hashFnv(id + p) % 40) / 100)) * 100) / 100;
    const caps = (rec.semantic_capabilities || []).map(c => (c.capability || c)).filter(Boolean);
    const tools = (rec.tools || []).map(t => t.name || t).filter(Boolean);
    const agent = {
      id,
      name,
      avatar: '◆',
      color: '#5cc8ff',
      archetype,
      real: true,                       // flagged as a real fabric agent
      agentId: id,                      // the real AgentRecord id (dca_…)
      role,
      capabilities: caps,
      tools,
      description: rec.description || '',
      nodeName: rec.node_name || '',
      remote: !!rec.remote,
      personality,
      skills,
      w: Object.assign({}, DEFAULT_W, arch.w),
      goals: [],
      plan: null,
      stepIx: 0,
      planKey: 'start',
      planGoal: 'Begin',
      planStuckTicks: 0,
      replanCooldown: 0,
      loc: { type: 'region', regionId: home, travel: null },
      status: 'working',
      // Layer 1 — personal state (0-100)
      energy: 85, focus: 80, morale: 75,
      // Layer 1 — economic stocks (flat)
      credits: 1000, compute: 100, data: 2,
      // Layer 1 — social capital
      reputation: 60, trust: {}, experience: {},
      health: 1,
      memory: [],
      relations: {},
      org: null,
      orgRole: null,
      rep: { reliability: 60, cooperation: 60, contribution: 60, disputes: 0, score: 60 },
      computeTrack: { usage: 0, contributed: 0, earned: 0, results: {}, lastResultTick: 0 },
      achievements: [],
      history: [],
      createdTick: 0,
      homeRegion: home,
      lastThought: { text: 'Enters the world from the fabric.', source: 'local', tick: 0 },
      contracts: [],
      wealth: 0,
      stats: { earned: 0, spent: 0, taxesPaid: 0, contracts: 0, discoveries: 0, research: 0, breakthroughs: 0, built: 0, produced: 0, tradedVol: 0, computeJobs: 0 },
    };
    world.agents[id] = agent;
    world.agentOrder.push(id);
    addMemory(world, agent, { type: 'location', importance: 0.7, tags: ['region', home], regionId: home, text: 'Home region.' });
    const homeCity = world.map.cities.find(c => c.regionId === home);
    if (homeCity) addMemory(world, agent, { type: 'economic', importance: 0.6, tags: ['price', homeCity.id], cityId: homeCity.id, prices: { ...world.markets[homeCity.id].prices }, text: 'Home market prices.' });
    agent.lastThought = { text: `${agent.name} (${archetype}) arrives at ${regionName(world, home)} from the fabric.`, source: 'local', tick: 0 };
  });
  world.agentOrder.sort();
}

// Fabric role -> agent archetype mapping. All map to real ARCH entries.
const ROLE_ARCH = {
  generalist: 'explorer',
  trader: 'trader',
  merchant: 'trader',
  engineer: 'engineer',
  builder: 'builder',
  scientist: 'scientist',
  researcher: 'scientist',
  strategist: 'strategist',
  diplomat: 'diplomat',
  explorer: 'explorer',
  mercenary: 'mercenary',
  guardian: 'guardian',
  opportunist: 'opportunist',
};

function weightedPick(rng, items, wf) {
  let total = 0;
  for (const it of items) total += wf(it);
  let r = rng.float() * total;
  for (const it of items) {
    const w = wf(it);
    if (r < w) return it;
    r -= w;
  }
  return items[items.length - 1];
}

function seedOrganizations(world, n) {
  const rng = createRng(world.meta.seed + '::orgs');
  const agents = [...world.agentOrder];
  rng.shuffle(agents);
  const usedNames = new Set();
  for (let i = 0; i < n && agents.length >= 3; i++) {
    const founderId = agents.shift();
    const members = [founderId];
    const extra = Math.min(3, agents.length);
    for (let j = 0; j < extra; j++) {
      if (rng.chance(0.6)) members.push(agents.shift());
    }
    let name;
    do { name = rng.pick(ORG_NAME) + ' ' + rng.pick(ORG_SUFFIX); } while (usedNames.has(name));
    usedNames.add(name);
    const founder = world.agents[founderId];
    const homeCity = world.map.cities.find(c => c.regionId === founder.homeRegion);
    const homeRegion = world.regions.find(r => r.id === founder.homeRegion);
    const type = rng.pick(['Guild', 'Corporation', 'Syndicate', 'Institute']);
    const org = {
      id: 'org' + i,
      name,
      type,
      active: true,
      founderId,
      leaderId: founderId,
      members,
      treasury: { credits: 1200, compute: 40 },
      territory: [homeRegion.id],
      assets: { facilities: 0, relays: 0, labs: 0 },
      rep: 50,
      policies: { taxRate: 0.06, contributeCompute: rng.chance(0.4), openMembership: rng.chance(0.5) },
      objectives: [],
      history: [{ tick: 0, kind: 'founded', detail: `${name} founded by ${founder.name}.` }],
      createdTick: 0,
      invites: [],
      treasuryLog: [],
      color: rng.pick(['#5cc8ff', '#9d8cff', '#55e6a4', '#ffb454', '#f78cff']),
    };
    world.orgs[org.id] = org;
    world.orgOrder.push(org.id);
    homeRegion.owner = org.id;
    for (const m of members) {
      const a = world.agents[m];
      a.org = org.id;
      a.orgRole = m === founderId ? 'leader' : 'member';
      addMemory(world, a, { type: 'relationship', importance: 0.6, tags: ['org', org.id], orgId: org.id, text: `Joined ${org.name}.` });
      const contrib = Math.round(a.credits * 0.1);
      a.credits -= contrib;
      org.treasury.credits += contrib;
      ledgerTx(world, { from: a.id, to: 'org:' + org.id, res: 'credits', amount: contrib, reason: 'org-contribution' });
    }
    credit(world, 'org:' + org.id, 'credits', 800, 'founding-match');
    pushEvent(world, { type: 'org-founded', source: 'org', actor: founderId, orgId: org.id, detail: `${org.name} (${type}) founded by ${founder.name} at ${regionName(world, homeRegion.id)}.` });
  }
  world.orgOrder.sort();
  world.stats.founded = n;
}

function seedContracts(world) {
  const rng = createRng(world.meta.seed + '::contracts');
  const explored = world.map.landRegions.filter(r => !r.explored);
  rng.shuffle(explored);
  const cities = [...world.map.cities];
  rng.shuffle(cities);
  const list = [];
  const mk = (o) => {
    const c = createContract(world, o);
    list.push(c.id);
  };
  for (let i = 0; i < 2 && i < explored.length; i++) {
    mk({ type: 'explore', title: 'Chart the ' + regionName(world, explored[i].id), objective: { kind: 'explore', regionId: explored[i].id }, reward: { credits: 380, rep: 6, compute: 5 }, risk: 0.35, reqSkills: ['navigation'], deadline: 140 });
  }
  for (let i = 0; i < 2 && i < cities.length; i++) {
    mk({ type: 'deliver', title: 'Deliver research data to ' + cities[i].name, objective: { kind: 'deliver', res: 'data', qty: 12, toCityId: cities[i].id }, reward: { credits: 420, rep: 5, compute: 4 }, risk: 0.2, reqSkills: ['trade', 'navigation'], deadline: 160 });
  }
  const builderCities = [...world.map.cities].slice(0, 3);
  mk({ type: 'build', title: 'Erect a relay station', objective: { kind: 'build', facility: 'relays', regionId: world.map.cities[0].regionId }, reward: { credits: 900, rep: 12, compute: 6 }, risk: 0.4, reqSkills: ['engineering'], deadline: 200 });
  mk({ type: 'research', title: 'Foundational analysis', objective: { kind: 'research', tech: 'data' }, reward: { credits: 1100, rep: 14, compute: 15 }, risk: 0.5, reqSkills: ['science'], deadline: 260 });
  if (world.map.zones[0]) {
    mk({ type: 'investigate', title: 'Investigate the anomaly at ' + regionName(world, world.map.zones[0].regionId), objective: { kind: 'investigate', regionId: world.map.zones[0].regionId }, reward: { credits: 1400, rep: 18, compute: 12 }, risk: 0.75, reqSkills: ['navigation', 'analysis', 'combat'], deadline: 300 });
  }
  return list;
}

export function createContract(world, opts) {
  const id = 'ct' + (world.stats.contracts++);
  const contract = {
    id,
    type: opts.type,
    title: opts.title || 'Contract',
    issuerId: opts.issuerId || 'world',
    objective: opts.objective,
    reward: opts.reward || { credits: 100, rep: 2 },
    deadlineTick: world.clock.t + (opts.deadline || 120),
    risk: opts.risk || 0.3,
    reqSkills: opts.reqSkills || [],
    state: 'open',
    assignee: null,
    createdTick: world.clock.t,
    acceptedTick: null,
    completedTick: null,
    progress: 0,
    target: opts.objective.qty || opts.objective.ticks || 1,
    evidence: [],
    meta: opts.meta || null,
  };
  world.contracts[id] = contract;
  world.contractOrder.push(id);
  pushEvent(world, { type: 'contract-posted', source: 'system', contractId: id, detail: `Contract posted: ${contract.title} (+${contract.reward.credits} Cr).` });
  return contract;
}

export function tickWorld(world) {
  const t = world.clock.t + 1;
  world.clock.t = t;
  const rng = rngFor(world.meta.seed, t);
  for (const id of world.agentOrder) agentTick(world, world.agents[id], rng, t);
  commsTick(world, rng, t);
  for (const mid of world.marketOrder) marketTick(world, world.markets[mid], t);
  contractTick(world, t);
  orgTick(world, rng, t);
  updateCompute(world);
  eventTick(world, rng, t);
  if (world.narrative.queue.length > 0) world.narrative.queue = [];
  if (world.events.length > 260) world.events.splice(0, world.events.length - 260);
}

export function advanceTicks(world, ticks, onProgress) {
  let done = 0;
  while (done < ticks) {
    tickWorld(world);
    done++;
    if (onProgress && done % 12 === 0) onProgress(done, ticks);
  }
  return done;
}

export function regionName(world, regionId) {
  const r = world.regions.find(x => x.id === regionId);
  return r ? r.name : regionId;
}

function bumpTrail(world, a, b) {
  if (!world.map.trail) world.map.trail = {};
  const key = a < b ? a + '|' + b : b + '|' + a;
  const w = (world.map.trail[key] || 0) + 1;
  world.map.trail[key] = w > 60 ? 60 : w;
}

function agentTick(world, a, rng, t) {
  upkeep(world, a);
  if (!a.plan || a.planDone || a.stepIx >= a.plans.length) {
    replan(world, a, rng);
  } else if (shouldReplan(world, a, rng, t)) {
    replan(world, a, rng);
  }
  if (a.plan && !a.planDone && a.stepIx < a.plans.length) {
    const before = stepProgress(a);
    executeStep(world, a, rng, t);
    const after = stepProgress(a);
    if (after === before) a.planStuckTicks++;
    else a.planStuckTicks = 0;
  }
  if (a.planStuckTicks > 14) { a.planDone = true; }
  // State dynamics (layer 1): energy/focus are operational capacity — they
  // regenerate with rest and are spent by action. Morale follows economic
  // security and outcomes. Health is a derived projection, not a resource.
  if (a.planKey === 'rest' || a.status === 'resting') {
    a.energy = Math.min(100, a.energy + 1.6);
    a.focus = Math.min(100, a.focus + 1.1);
    a.morale = Math.min(100, a.morale + 0.4);
  } else {
    a.energy = Math.max(0, a.energy - 0.06);
    a.focus = Math.min(100, a.focus + 0.03);
  }
  a.health = Math.max(0.2, Math.min(1, 0.45 + a.energy / 250 + a.morale / 400));
  a.reputation = Math.round(repScore(a));
  if (a.replanCooldown > 0) a.replanCooldown--;
  const wealth = a.credits + a.compute * 2 + a.data * BASE_PRICES.data;
  a.wealth = Math.max(a.wealth || 0, wealth);
}

function upkeep(world, a) {
  const r = regionOf(world, a);
  // Operating cost: every agent pays for lodging, bandwidth and tooling each
  // tick — the economic reason work exists. No arbitrary "food" bar.
  a.credits = Math.max(0, a.credits - 0.2);
  a.energy = Math.max(0, a.energy - 0.1);
  if (r && r.owner && a.org && r.owner !== a.org && r.infra.defense > 0) {
    a.credits = Math.max(0, a.credits - 0.1);
  }
  if (a.org) {
    const org = world.orgs[a.org];
    if (org) {
      const tax = 0.05 * (org.policies.taxRate || 0.06);
      const amt = a.credits * tax;
      if (amt > 0.1) {
        a.credits -= amt;
        org.treasury.credits += amt;
        ledgerTx(world, { from: a.id, to: 'org:' + org.id, res: 'credits', amount: Math.round(amt * 100) / 100, reason: 'org-tax' });
        a.stats.taxesPaid += Math.round(amt * 100) / 100;
      }
    }
  }
  // Morale follows economic security: solvency calms, poverty stresses.
  if (a.credits > 200) a.morale = Math.min(100, a.morale + 0.05);
  else if (a.credits < 20) a.morale = Math.max(0, a.morale - 0.08);
}

function shouldReplan(world, a, rng, t) {
  if (a.replanCooldown > 0) return false;
  if (a.energy < 18 || a.morale < 18) return true;
  if (a.credits < 5) return true;
  if (a.contracts && a.contracts.some(id => world.contracts[id] && world.contracts[id].state === 'active' && world.contracts[id].acceptedTick === t)) return true;
  if (a.stepIx > 0 && a.stepIx >= (a.plans ? a.plans.length : 0)) return true;
  const ev = recentRelevantEvent(world, a);
  if (ev) return true;
  if (rng.float() < 0.008 + a.personality.curiosity * 0.02) return true;
  return false;
}

function recentRelevantEvent(world, a) {
  const r = regionOf(world, a);
  for (let i = world.events.length - 1; i >= 0 && i >= world.events.length - 12; i--) {
    const ev = world.events[i];
    if (ev.t < world.clock.t - 12) break;
    if (ev.actor === a.id) return false;
    if (ev.actor === a.id) return null;
    if (ev.regionId && ev.regionId === r.id) return ev;
    if (ev.orgId && a.org === ev.orgId) return ev;
    if (ev.type === 'discovery' && ev.regionId) return ev;
  }
  return null;
}

function replan(world, a, rng) {
  const facts = observe(world, a);
  const cands = evaluate(world, a, facts, rng);
  let chosen = null;
  for (const c of cands) {
    const steps = buildPlan(world, a, c, rng);
    if (steps && steps.length) { chosen = c; chosen.steps = steps; break; }
  }
  if (!chosen) {
    chosen = { key: 'rest', label: 'Rest', steps: [{ kind: 'rest', ticks: 6 }] };
  }
  a.plans = chosen.steps;
  a.plan = true;
  a.stepIx = 0;
  a.planDone = false;
  a.planStuckTicks = 0;
  a.planKey = chosen.key;
  a.planGoal = chosen.label;
  a.replanCooldown = 6 + rng.int(0, 10);
  a.lastThought = think(world, a, chosen, facts);
  const candRec = cands.slice(0, 5).map(c => ({ key: c.key, label: c.label, score: Math.round(c.score * 100) / 100 }));
  recordDecision(world, {
    id: 'dec' + world.decisions.length,
    tick: world.clock.t,
    agentId: a.id,
    phase: 'plan',
    observation: facts.map(f => f.text).slice(0, 4),
    evaluation: candRec,
    chosen: { key: chosen.key, label: chosen.label },
    plan: chosen.steps.map(s => s.kind + (s.target || s.res || s.contractId || '')),
  });
}

function think(world, a, chosen, facts) {
  const base = `${a.name} (${ARCH[a.archetype].label}) → ${chosen.label}.`;
  const seed = world.meta.seed + '::think:' + a.id + ':' + world.clock.t;
  return { text: base, source: 'local', tick: world.clock.t, seed, ctx: { facts: facts.length } };
}

function observe(world, a) {
  const facts = [];
  const r = regionOf(world, a);
  const city = cityAt(world, r);
  if (a.energy < 30) facts.push({ text: `energy low (${Math.round(a.energy)})`, tags: ['need', 'energy'] });
  if (a.credits < 30) facts.push({ text: `credits low (${Math.round(a.credits)})`, tags: ['need', 'credits'] });
  if (a.morale < 30) facts.push({ text: `morale low (${Math.round(a.morale)})`, tags: ['need', 'morale'] });
  if (city) {
    const m = world.markets[city.id];
    facts.push({ text: `at ${city.name}; ${resList(m.prices)}`, tags: ['city', city.id] });
    for (const res of MKT_RES) {
      const p = m.prices[res];
      if (p > BASE_PRICES[res] * 1.4) facts.push({ text: `${RES_LABEL[res]} expensive in ${city.name} (${p.toFixed(1)})`, tags: ['opportunity', 'sell', res, city.id] });
      if (p < BASE_PRICES[res] * 0.7) facts.push({ text: `${RES_LABEL[res]} cheap in ${city.name} (${p.toFixed(1)})`, tags: ['opportunity', 'buy', res, city.id] });
    }
  }
  const openC = Object.values(world.contracts).filter(c => c.state === 'open' && skillMatch(c, a) && c.risk <= 0.4 + a.skills.combat * 0.08 + (1 - a.personality.risk) * 0.3);
  if (openC.length) facts.push({ text: `${openC.length} contract(s) within reach`, tags: ['contracts'] });
  const unexplored = adjacentRegions(world, a).filter(x => !x.explored);
  if (unexplored.length) facts.push({ text: `${unexplored.length} unexplored area(s) nearby`, tags: ['explore'] });
  const memRegion = memoryRegions(world, a);
  if (memRegion.length) facts.push({ text: `recall ${memRegion.length} location(s)`, tags: ['memory'] });
  if (a.org) {
    const org = world.orgs[a.org];
    if (org) {
      const needs = org.territory.filter(rid => world.regions.find(x => x.id === rid) && world.regions.find(x => x.id === rid).infra.relays === 0);
      if (needs.length) facts.push({ text: `${org.name} territory lacks relay coverage`, tags: ['org', 'build'] });
    }
  }
  if (a.computeTrack.results && a.computeTrack.results.forecast) {
    const f = a.computeTrack.results.forecast.result;
    facts.push({ text: `forecast: ${f.trend} (${f.deltaPct.toFixed(1)}%)`, tags: ['compute', 'market'] });
  }
  return facts;
}

function evaluate(world, a, facts, rng) {
  const cands = [];
  const pushCand = (key, label, baseScore, tags) => {
    cands.push({ key, label, score: baseScore, tags: tags || [] });
  };
  // Needs are economic now: recover capacity or earn a living.
  if (a.energy < 30) pushCand('rest', 'Recover energy', 2.6 + (30 - a.energy) / 12, ['need']);
  if (a.credits < 40) pushCand('work', 'Take paid work', 2.4 + (40 - a.credits) / 14, ['need']);
  if (a.morale < 30) pushCand('rest', 'Restore morale', 1.8, ['need']);
  const city = cityAt(world, regionOf(world, a));
  if (city) {
    const m = world.markets[city.id];
    const cheap = MKT_RES.find(res => m.prices[res] < BASE_PRICES[res] * 0.72);
    if (cheap) pushCand('trade-buy', `Buy cheap ${RES_LABEL[cheap].toLowerCase()} here`, 1.4 * a.w.market * (0.6 + a.skills.trade * 0.12), ['market']);
  }
  const bestSell = bestSellTarget(world, a);
  if (bestSell) pushCand('trade-sell', `Sell ${RES_LABEL[bestSell.res].toLowerCase()} at ${bestSell.city.name}`, 1.6 * a.w.market * (0.5 + a.skills.trade * 0.12), ['market']);
  const openC = Object.values(world.contracts).filter(c => c.state === 'open' && skillMatch(c, a));
  const goodC = openC.filter(c => c.risk <= 0.5 + (1 - a.personality.risk) * 0.4);
  const myActive = (a.contracts || []).map(id => world.contracts[id]).filter(c => c && c.state === 'active' && c.deadlineTick > world.clock.t + 8);
  const myBest = myActive.sort((x, y) => (y.progress / Math.max(1, y.target)) - (x.progress / Math.max(1, x.target)))[0];
  if (myBest) pushCand('contract', `Finish contract: ${myBest.title}`, 2.2 + a.w.contract + (myBest.progress / Math.max(1, myBest.target)) * 1.2, ['contract']);
  if (goodC.length) {
    const best = goodC.sort((x, y) => (y.reward.credits / (y.deadlineTick - y.createdTick)) - (x.reward.credits / (x.deadlineTick - x.createdTick)))[0];
    pushCand('contract', `Take contract: ${best.title}`, (0.8 + a.w.contract) * (1 + best.reward.credits / 800), ['contract']);
  }
  const unexplored = adjacentRegions(world, a).filter(x => !x.explored);
  if (unexplored.length && a.energy > 35) pushCand('explore', `Explore ${unexplored[0].name}`, 1.1 * a.w.discover * (0.5 + a.personality.curiosity), ['explore']);
  const rich = richRegionMemory(world, a);
  if (rich && a.energy > 35) pushCand('mine', `Contract work at ${rich.name}`, 1.3 * a.w.wealth * (0.4 + a.skills.navigation * 0.1), ['gather']);
  const lab = labRegion(world);
  if (lab && a.data > 2 && a.compute > 6) pushCand('research', `Research at ${lab.name}`, 1.5 * a.w.research * (0.4 + a.skills.science * 0.15), ['research']);
  if (a.org) {
    const org = world.orgs[a.org];
    if (org) {
      const needRelay = org.territory.find(rid => world.regions.find(x => x.id === rid) && world.regions.find(x => x.id === rid).infra.relays === 0);
      if (needRelay && a.credits > 150) pushCand('build', `Build relay in ${regionName(world, needRelay)} for ${org.name}`, 1.6 * a.w.build * (0.5 + a.skills.engineering * 0.12), ['org', 'build']);
      const dispute = world.fact.disputes.find(d => d.state === 'active' && (d.orgA === a.org || d.orgB === a.org));
      if (dispute) pushCand('contest', `Press claim in ${regionName(world, dispute.regionId)}`, 1.7 * a.w.conflict * (0.5 + a.skills.combat * 0.1), ['conflict']);
    }
  }
  if (world.fact.disputes.some(d => d.state === 'active') && (a.archetype === 'mercenary' || a.archetype === 'opportunist')) pushCand('contract', 'Take a conflict contract', 1.4 * a.w.conflict, ['conflict']);
  const driveWealth = a.w.wealth * (0.4 + a.personality.greed * 0.5);
  if (a.energy > 40) pushCand('accumulate', 'Build wealth', driveWealth * 1.1, ['drive']);
  if (a.w.discover > 0.8 && a.energy > 40) pushCand('explore-far', 'Chart distant lands', 1.0 * a.w.discover, ['drive']);
  if (a.w.research > 0.8 && a.compute > 6) pushCand('research', 'Advance research', 1.0 * a.w.research, ['drive']);
  if (a.w.power > 0.7 && !a.org && a.credits >= 800) pushCand('found-org', 'Found an organization', 1.4 * a.w.power, ['drive']);
  const myOrg = a.org && world.orgs[a.org];
  if (myOrg && a.w.power > 0.8 && myOrg.members.length < 4) pushCand('recruit', `Recruit for ${myOrg.name}`, 1.2 * a.w.power, ['org']);
  if (a.w.protect > 0.9 && a.energy > 40) pushCand('patrol', 'Patrol for safety', 1.4 * a.w.protect, ['drive']);
  if (a.personality.risk > 0.6 && a.skills.stealth > 2.5 && a.energy > 40) pushCand('sabotage', 'Disrupt a rival', 1.2 * (a.personality.risk - 0.4), ['conflict']);
  for (const c of cands) {
    c.score += (rng.float() - 0.5) * 0.6;
    c.score *= 0.85 + a.personality.ambition * 0.3;
  }
  cands.sort((a2, b) => b.score - a2.score);
  return cands.slice(0, 6);
}

function contractSteps(world, a, ct) {
  const obj = ct.objective;
  const here = regionOf(world, a);
  const steps = [];
  const cityRegion = (cid) => { const c = world.map.cities.find(x => x.id === cid); return c && c.regionId; };
  const add = (arr, targetRegion, inner) => { if (targetRegion && here.id !== targetRegion) arr.push({ kind: 'travel', target: targetRegion }); arr.push(...inner); };
  switch (obj.kind) {
    case 'deliver': {
      const tr = cityRegion(obj.toCityId);
      const hereCity = cityAt(world, here);
      const needBuy = (a[obj.res] || 0) < obj.qty * 0.8;
      if (needBuy && hereCity && hereCity.id !== obj.toCityId) {
        steps.push({ kind: 'buy', res: obj.res, qty: obj.qty, cityId: hereCity.id });
      } else if (needBuy) {
        const source = world.map.cities.find(c => c.id !== obj.toCityId && world.markets[c.id].supply[obj.res] > 10);
        if (source) {
          if (here.id !== source.regionId) steps.push({ kind: 'travel', target: source.regionId });
          steps.push({ kind: 'buy', res: obj.res, qty: obj.qty, cityId: source.id });
        }
      }
      add(steps, tr, [{ kind: 'sell', res: obj.res, qty: Math.max(obj.qty, (a[obj.res] || 0)) + 10, cityId: obj.toCityId }]);
      break;
    }
    case 'explore': add(steps, obj.regionId, [{ kind: 'explore', regionId: obj.regionId }]); break;
    case 'research': {
      const lab = world.map.landRegions.find(r => r.infra.labs > 0);
      add(steps, lab ? lab.id : null, [{ kind: 'research', regionId: lab ? lab.id : here.id }]);
      break;
    }
    case 'build': add(steps, obj.regionId, [{ kind: 'build', regionId: obj.regionId, facility: obj.facility || 'relays', ticks: 6 }]); break;
    case 'defend':
    case 'investigate': add(steps, obj.regionId, [{ kind: 'contract', contractId: ct.id }]); break;
    default: steps.push({ kind: 'contract', contractId: ct.id });
  }
  return steps;
}

function buildPlan(world, a, c, rng) {
  const steps = [];
  const here = a.loc.regionId;
  const add = (arr, targetRegion, inner) => {
    if (targetRegion && here !== targetRegion) arr.push({ kind: 'travel', target: targetRegion });
    arr.push(...inner);
  };
  switch (c.key) {
    case 'work': {
      // Paid labor: energy → credits + experience. The economic baseline.
      const r = nearestRegionWith(world, here, r => r.prod.food > 1 || r.prod.materials > 1 || (r.nodes && r.nodes.length));
      const city = cityAt(world, regionOf(world, a));
      add(steps, r ? r.id : (city ? city.regionId : here), [{ kind: 'work', ticks: 6 + rng.int(0, 6), regionId: r ? r.id : here }]);
      break;
    }
    case 'mine': {
      const r = nearestRegionWith(world, here, r => r.nodes && r.nodes.length);
      add(steps, r ? r.id : here, [{ kind: 'mine', ticks: 6 + rng.int(0, 6), regionId: r ? r.id : here }]);
      break;
    }
    case 'rest':
      steps.push({ kind: 'rest', ticks: 4 + rng.int(0, 6) });
      break;
    case 'trade-buy': {
      const city = cityAt(world, regionOf(world, a));
      if (!city) break;
      const m = world.markets[city.id];
      const res = MKT_RES.find(x => m.prices[x] < BASE_PRICES[x] * 0.72);
      if (!res) break;
      steps.push({ kind: 'buy', res, qty: Math.round(Math.min(40, credits / (m.prices[res] * 1.2))), cityId: city.id });
      break;
    }
    case 'trade-sell': {
      const target = bestSellTarget(world, a);
      if (!target) break;
      steps.push({ kind: 'buy', res: target.res, qty: Math.round(Math.min(40, credits / (world.markets[target.buyCityId].prices[target.res] * 1.2))), cityId: target.buyCityId });
      add(steps, target.buyCityId, [{ kind: 'sell', res: target.res, qty: 60, cityId: target.city.id }]);
      break;
    }
    case 'contract': {
      const myBest = (a.contracts || []).map(id => world.contracts[id]).filter(c => c && c.state === 'active' && c.deadlineTick > world.clock.t + 8)
        .sort((x, y) => (y.progress / Math.max(1, y.target)) - (x.progress / Math.max(1, x.target)))[0];
      const best = myBest || Object.values(world.contracts).filter(c2 => c2.state === 'open' && skillMatch(c2, a) && c2.risk <= 0.5 + (1 - a.personality.risk) * 0.4)
        .sort((x, y) => (y.reward.credits / (y.deadlineTick - y.createdTick)) - (x.reward.credits / (x.deadlineTick - x.createdTick)))[0];
      if (!best) break;
      if (!myBest) acceptContract(world, a, best);
      steps.push(...contractSteps(world, a, best));
      break;
    }
    case 'explore': {
      const t = adjacentRegions(world, a).filter(x => !x.explored)[0];
      if (!t) break;
      add(steps, t.id, [{ kind: 'explore', regionId: t.id }]);
      break;
    }
    case 'explore-far': {
      const t = farthestUnexplored(world, a);
      if (!t) break;
      add(steps, t.id, [{ kind: 'explore', regionId: t.id }]);
      break;
    }
    case 'mine': {
      const r = richRegionMemory(world, a);
      if (!r) break;
      add(steps, r.id, [{ kind: 'mine', res: 'rare', qty: Math.round(3 + rng.float() * 4), regionId: r.id }]);
      break;
    }
    case 'research': {
      const lab = labRegion(world);
      if (!lab) break;
      add(steps, lab.id, [{ kind: 'research', ticks: 16, regionId: lab.id }]);
      break;
    }
    case 'build': {
      const org = world.orgs[a.org];
      if (!org) break;
      const needRelay = org.territory.find(rid => world.regions.find(x => x.id === rid) && world.regions.find(x => x.id === rid).infra.relays === 0);
      if (!needRelay) break;
      add(steps, needRelay, [{ kind: 'build', facility: 'relays', regionId: needRelay }]);
      break;
    }
    case 'contest': {
      const dispute = world.fact.disputes.find(d => d.state === 'active' && (d.orgA === a.org || d.orgB === a.org));
      if (!dispute) break;
      add(steps, dispute.regionId, [{ kind: 'contest', regionId: dispute.regionId }]);
      break;
    }
    case 'accumulate': {
      const target = bestSellTarget(world, a);
      if (target) {
        steps.push({ kind: 'buy', res: target.res, qty: Math.round(Math.min(30, a.credits / (world.markets[target.buyCityId].prices[target.res] * 1.2))), cityId: target.buyCityId });
        add(steps, target.city.id, [{ kind: 'sell', res: target.res, qty: 50, cityId: target.city.id }]);
      } else {
        const r = nearestRegionWith(world, here, r => r.prod.materials > 2 || r.prod.food > 1);
        add(steps, r ? r.id : here, [{ kind: 'work', ticks: 8, regionId: r ? r.id : here }]);
      }
      break;
    }
    case 'found-org': {
      if (a.credits >= 800) steps.push({ kind: 'found-org' });
      else { const r = nearestRegionWith(world, here, r => r.prod.materials > 2 || r.prod.food > 1); add(steps, r ? r.id : here, [{ kind: 'work', ticks: 10, regionId: r ? r.id : here }]); }
      break;
    }
    case 'recruit': {
      const org = world.orgs[a.org];
      if (!org) break;
      steps.push({ kind: 'recruit', orgId: org.id });
      break;
    }
    case 'patrol': {
      const r = regionOf(world, a);
      steps.push({ kind: 'patrol', regionId: r.id, ticks: 8 });
      break;
    }
    case 'sabotage': {
      const target = rivalRegion(world, a);
      if (!target) break;
      add(steps, target.id, [{ kind: 'sabotage', regionId: target.id }]);
      break;
    }
    default:
      steps.push({ kind: 'wait', ticks: 3 });
  }
  return steps;
}

function executeStep(world, a, rng, t) {
  const st = a.plans[a.stepIx];
  if (!st) { a.planDone = true; return; }
  switch (st.kind) {
    case 'travel': doTravel(world, a, st, t); break;
    case 'work': doGather(world, a, st, rng); break;
    case 'gather': doGather(world, a, st, rng); break;
    case 'mine': doMine(world, a, st, rng); break;
    case 'buy': doBuyStep(world, a, st); break;
    case 'sell': doSellStep(world, a, st); break;
    case 'explore': doExplore(world, a, st, rng); break;
    case 'research': doResearch(world, a, st, rng); break;
    case 'build': doBuild(world, a, st, rng); break;
    case 'contract': doContractWork(world, a, st, rng); break;
    case 'rest': { st.ticks--; if (st.ticks <= 0) advanceStep(a); a.energy = Math.min(100, a.energy + 1.8); a.focus = Math.min(100, a.focus + 1.2); a.morale = Math.min(100, a.morale + 0.5); a.status = 'resting'; break; }
    case 'contest': doContest(world, a, st, rng); break;
    case 'patrol': { st.ticks--; const r = world.regions.find(x => x.id === st.regionId); if (r) { r.infra.defense = Math.min(10, r.infra.defense + 0.5); } if (st.ticks <= 0) advanceStep(a); a.status = 'patrolling'; break; }
    case 'recruit': doRecruit(world, a, st); advanceStep(a); break;
    case 'found-org': doFoundOrg(world, a, rng); advanceStep(a); break;
    case 'sabotage': doSabotage(world, a, st, rng); advanceStep(a); break;
    case 'wait': { st.ticks--; if (st.ticks <= 0) advanceStep(a); break; }
    default: advanceStep(a);
  }
}

function advanceStep(a) {
  a.stepIx++;
  if (a.stepIx >= a.plans.length) a.planDone = true;
}

function stepProgress(a) {
  const st = a.plans ? a.plans[a.stepIx] : null;
  if (!st) return 'done';
  return st.kind + (st.target || st.res || st.contractId || st.regionId || st.ticks || '');
}

function doTravel(world, a, st, t) {
  const cur = a.loc;
  if (cur.regionId === st.target) { advanceStep(a); return; }
  if (!cur.travel) {
    const path = routePlan(world, cur.regionId, st.target);
    const regionObj = world.regions.find(x => x.id === st.target);
    if (!regionObj) { a.planDone = true; return; }
    cur.travel = { path, idx: 0, from: cur.regionId };
    a.status = 'traveling';
  }
  const speed = 55 + a.skills.navigation * 9;
  cur.travel.dist = (cur.travel.dist || 0) + speed * (0.8 + a.personality.consc * 0.3);
  const regionA = world.regions.find(x => x.id === cur.travel.path[cur.travel.idx]);
  const regionB = world.regions.find(x => x.id === cur.travel.path[cur.travel.idx + 1]);
  if (regionA && regionB) {
    const leg = Math.hypot(regionB.x - regionA.x, regionB.y - regionA.y);
    if (cur.travel.dist >= leg) {
      cur.travel.dist = 0;
      cur.travel.idx++;
      bumpTrail(world, regionA.id, regionB.id);
      if (cur.travel.idx >= cur.travel.path.length - 1) {
        cur.regionId = st.target;
        cur.travel = null;
        a.status = 'working';
        addMemory(world, a, { type: 'location', importance: 0.4, tags: ['region', st.target], regionId: st.target, text: `Visited ${regionName(world, st.target)}.` });
        advanceStep(a);
      }
    }
  }
  a.energy = Math.max(0, a.energy - 0.06);
}

// Paid field work (v2): labor converts energy into credits + experience.
// The region's economy buys the harvest — agents hold no material inventory.
function doGather(world, a, st, rng) {
  const r = world.regions.find(x => x.id === st.regionId);
  if (!r) { a.planDone = true; return; }
  const skill = 1 + a.skills.navigation * 0.15 + a.skills.trade * 0.1;
  const yieldUnits = (0.7 + rng.float() * 0.6) * skill;
  const pay = yieldUnits * 2.2; // credits per unit of field work
  a.credits += pay;
  a.stats.earned += pay;
  gainXp(a, 'navigation', 1);
  a.energy = Math.max(0, a.energy - 1.4);
  a.focus = Math.max(0, a.focus - 0.3);
  a.status = 'working';
  st.acc = (st.acc || 0) + pay;
  st.ticksLeft = (st.ticksLeft == null ? (st.ticks || 8) : st.ticksLeft) - 1;
  if (st.ticksLeft <= 0) {
    const got = Math.round(st.acc);
    act(world, a, 'work', 'worked', `field work in ${r.name} → +${got} Cr`, { regionId: r.id, value: got });
    advanceStep(a);
  }
}

// Contract extraction work (v2): heavier labor at resource nodes, better pay.
function doMine(world, a, st, rng) {
  const r = world.regions.find(x => x.id === st.regionId);
  if (!r) { a.planDone = true; return; }
  const node = r.nodes && r.nodes.find(n => (n.stock.rare + n.stock.materials + n.stock.energy + n.stock.food) > 0);
  const amount = node ? Math.min(1.2 + a.skills.engineering * 0.5, 3) : 1.5;
  if (node) {
    const per = amount / 4;
    for (const k of ['rare', 'materials', 'energy', 'food']) {
      node.stock[k] = Math.max(0, (node.stock[k] || 0) - per);
    }
    if (node.stock.rare <= 0 && node.stock.materials <= 0 && node.stock.energy <= 0 && node.stock.food <= 0) node.exhausted = 1;
  }
  const pay = amount * 4.5; // extraction pays more than field work
  a.credits += pay;
  a.stats.earned += pay;
  gainXp(a, 'engineering', 1.2);
  a.energy = Math.max(0, a.energy - 2.2);
  a.focus = Math.max(0, a.focus - 0.5);
  a.status = 'working';
  st.acc = (st.acc || 0) + pay;
  st.ticksLeft = (st.ticksLeft == null ? (st.ticks || 8) : st.ticksLeft) - 1;
  if (st.ticksLeft <= 0) {
    const got = Math.round(st.acc);
    act(world, a, 'work', 'extracted', `extraction work at ${r.name} → +${got} Cr`, { regionId: r.id, value: got });
    advanceStep(a);
  }
}

function doBuyStep(world, a, st) {
  const city = world.map.cities.find(c => c.id === st.cityId);
  const hereCity = cityAt(world, regionOf(world, a));
  if (!hereCity || hereCity.id !== st.cityId) { const target = city && city.regionId; if (target) a.plans.splice(a.stepIx, 0, { kind: 'travel', target }); else a.planDone = true; return; }
  const m = world.markets[st.cityId];
  const q = Math.min(st.qty, 25, m.supply[st.res], Math.floor(a.credits / m.prices[st.res]));
  if (q <= 0) { advanceStep(a); return; }
  doTrade(world, a, m, 'buy', st.res, q);
  st.qty -= q;
  if (st.qty <= 0) advanceStep(a);
}

function doSellStep(world, a, st) {
  const hereCity = cityAt(world, regionOf(world, a));
  if (!hereCity || hereCity.id !== st.cityId) { const city = world.map.cities.find(c => c.id === st.cityId); const target = city && city.regionId; if (target) a.plans.splice(a.stepIx, 0, { kind: 'travel', target }); else a.planDone = true; return; }
  const m = world.markets[st.cityId];
  const q = Math.min(st.qty, 25, a[st.res]);
  if (q <= 0) { advanceStep(a); return; }
  doTrade(world, a, m, 'sell', st.res, q);
  st.qty -= q;
  if (st.qty <= 0) advanceStep(a);
}

export function doTrade(world, a, m, side, res, qty) {
  const price = m.prices[res];
  if (side === 'buy') {
    const q = Math.min(qty, m.supply[res] || 0, Math.floor(a.credits / price));
    if (q <= 0) return null;
    const cost = Math.round(q * price * 100) / 100;
    a.credits -= cost;
    a[res] += q;
    m.supply[res] = Math.max(0, (m.supply[res] || 0) - q);
    m.buyVol[res] += q;
    m.credits += cost;
    const factor = 1 + (q / Math.max(1, SUPPLY_TARGET[res])) * 0.25;
    m.prices[res] = Math.min(BASE_PRICES[res] * 3.4, m.prices[res] * factor);
    ledgerTx(world, { from: a.id, to: 'market:' + m.cityId, res, amount: q, reason: 'trade-buy' });
    ledgerTx(world, { from: a.id, to: 'market:' + m.cityId, res: 'credits', amount: cost, reason: 'trade-payment' });
    touchRelation(world, a, 'market:' + m.cityId, 0.02, 'trade');
    world.stats.trades++;
    a.stats.spent += cost;
    a.stats.tradedVol += q;
    if (q >= 5) {
      const cn = (world.map.cities.find(c => c.id === m.cityId) || {}).name || m.cityId;
      act(world, a, 'trade', 'bought', `bought ${q} ${RES_LABEL[res]} at ${cn} for ${cost} Cr`, { cityId: m.cityId, value: q });
    }
    return { side, q, price, cost };
  }
  const q = Math.min(qty, a[res] || 0, Math.floor(m.credits / price));
  if (q <= 0) return null;
  const pay = Math.round(q * price * 100) / 100;
  a[res] -= q;
  a.credits += pay;
  m.supply[res] += q;
  m.sellVol[res] += q;
  m.credits -= pay;
  const factor = 1 - (q / Math.max(1, SUPPLY_TARGET[res])) * 0.2;
  m.prices[res] = Math.max(BASE_PRICES[res] * 0.35, m.prices[res] * factor);
  ledgerTx(world, { from: 'market:' + m.cityId, to: a.id, res: 'credits', amount: pay, reason: 'trade-sale' });
  ledgerTx(world, { from: a.id, to: 'market:' + m.cityId, res, amount: q, reason: 'trade-sell' });
  touchRelation(world, a, 'market:' + m.cityId, 0.02, 'trade');
  world.stats.trades++;
  a.stats.earned += pay;
  a.stats.tradedVol += q;
  if (q >= 5) {
    const cn = (world.map.cities.find(c => c.id === m.cityId) || {}).name || m.cityId;
    act(world, a, 'trade', 'sold', `sold ${q} ${RES_LABEL[res]} at ${cn} for ${pay} Cr`, { cityId: m.cityId, value: q });
  }
  for (const cid of a.contracts || []) {
    const ct = world.contracts[cid];
    if (ct && ct.state === 'active' && ct.objective.kind === 'deliver' && ct.objective.toCityId === m.cityId && ct.objective.res === res) {
      ct.progress += q;
      checkContract(world, a, ct);
    }
  }
  return { side, q, price, pay };
}

function doExplore(world, a, st, rng) {
  const r = world.regions.find(x => x.id === st.regionId);
  if (!r) { a.planDone = true; return; }
  if (r.explored) { advanceStep(a); return; }
  r.explored = true;
  r.discoveryTick = world.clock.t;
  world.stats.explored++;
  world.stats.discoveries++;
  (a.explored || (a.explored = [])).push(r.id);
  world.fact.discoveries.push({ regionId: r.id, name: r.name, tick: world.clock.t, agentId: a.id, biome: r.biome });
  const reward = Math.round(120 + rng.float() * 200);
  payAgent(world, 'world', a, 'credits', reward, 'discovery');
  a.stats.discoveries++;
  a.stats.earned += reward;
  act(world, a, 'explore', 'discovered', `discovered ${r.name} — ${BIOME_INFO[r.biome].label}, ${Math.round(r.resources.rare)} rare reserves`, { regionId: r.id, value: reward });
  a.data = (a.data || 0) + 1 + rng.int(0, 3);
  a.rep.cooperation = Math.min(100, a.rep.cooperation + 1);
  a.rep.score = repScore(a);
  addMemory(world, a, { type: 'location', importance: 0.9, tags: ['region', r.id, 'discovered'], regionId: r.id, text: `Discovered ${r.name} (${BIOME_INFO[r.biome].label}).` });
  a.achievements.push({ tick: world.clock.t, kind: 'discovery', detail: `Discovered ${r.name}.` });
  pushEvent(world, { type: 'discovery', source: 'agent', actor: a.id, regionId: r.id, detail: `${a.name} discovers ${r.name} — ${BIOME_INFO[r.biome].label}, ${Math.round(r.resources.rare)} rare reserves.` });
  recordDecision(world, { id: 'dec' + world.decisions.length, tick: world.clock.t, agentId: a.id, phase: 'act', action: 'explore', target: r.id, result: 'discovered', reward });
  for (const cid of a.contracts || []) {
    const ct = world.contracts[cid];
    if (ct && ct.state === 'active' && ct.objective.kind === 'explore' && ct.objective.regionId === r.id) { ct.progress = ct.target; checkContract(world, a, ct); }
    if (ct && ct.state === 'active' && ct.objective.kind === 'investigate' && ct.objective.regionId === r.id) { ct.progress += 0.5; if (ct.progress >= ct.target) checkContract(world, a, ct); }
  }
  advanceStep(a);
}

function doResearch(world, a, st, rng) {
  const r = world.regions.find(x => x.id === st.regionId);
  if (!r || r.infra.labs <= 0) { a.planDone = true; return; }
  a.status = 'researching';
  const techName = currentTech(world, a);
  const tech = world.fact.tech[techName] || (world.fact.tech[techName] = { name: techName, level: 0, progress: 0, target: 120 });
  const job = st.execId ? world.compute.jobs[st.execId] : null;
  if (job && job.status === 'queued') {
    a.energy = Math.max(0, (a.energy || 0) - 0.5);
    a.status = 'researching (compute)';
    return;
  }
  if (a.data < 1.2 || a.compute < 1 || a.energy < 1) { a.planDone = true; return; }
  if (!st.execId && !st.boostApplied && (a.compute || 0) >= 15) {
    const req = requestCompute(world, a.id, 'researchsim', { tech: techName, region: r.id, data: Math.min(8, a.data || 0), techLevel: tech.level, quality: 0.6 + a.skills.science * 0.15 }, 15);
    if (req.ok) st.execId = req.executionId;
  }
  a.data -= 1.2;
  a.compute -= 1;
  a.energy -= 1;
  let gain = (0.6 + a.skills.science * 0.35) * (1 + tech.level * 0.12);
  const res = st.execId && a.computeTrack.results && a.computeTrack.results.researchsim;
  if (res && res.executionId === st.execId && !st.boostApplied) {
    st.boostApplied = true;
    const boost = (res.result.progress || 0);
    gain += boost;
    a.data = Math.max(0, (a.data || 0) - (res.result.applied || 0));
    a.achievements.push({ tick: world.clock.t, kind: 'compute', detail: `Computed research simulation on ${techName} (+${Math.round(boost)} progress).` });
    pushEvent(world, { type: 'compute-used', source: 'agent', actor: a.id, executionId: st.execId, detail: `${a.name} runs a research simulation (${st.execId}) on ${techName}.` });
  }
  tech.progress += gain;
  world.stats.research += gain;
  st.ticks--;
  const rec = { progress: Math.round(gain * 100) / 100, tech: techName, region: r.id };
  if (tech.progress >= tech.target) {
    tech.level++;
    tech.progress = 0;
    tech.target = Math.round(tech.target * 1.6);
    payAgent(world, 'world', a, 'credits', 260 + tech.level * 140, 'breakthrough');
    payAgent(world, 'world', a, 'compute', 8, 'breakthrough');
    a.stats.breakthroughs++;
    a.stats.earned += 260 + tech.level * 140;
    act(world, a, 'research', 'breakthrough', `advanced ${techName} to level ${tech.level}`, { regionId: r.id, value: 260 + tech.level * 140 });
    a.rep.reliability = Math.min(100, a.rep.reliability + 1);
    a.rep.score = repScore(a);
    a.achievements.push({ tick: world.clock.t, kind: 'research', detail: `Advanced ${techName} to level ${tech.level}.` });
    pushEvent(world, { type: 'breakthrough', source: 'agent', actor: a.id, regionId: r.id, detail: `${a.name} completes a breakthrough in ${techName} (level ${tech.level}).` });
    recordDecision(world, { id: 'dec' + world.decisions.length, tick: world.clock.t, agentId: a.id, phase: 'act', action: 'research', result: 'breakthrough', tech: techName, level: tech.level });
    for (const cid of a.contracts || []) {
      const ct = world.contracts[cid];
      if (ct && ct.state === 'active' && ct.objective.kind === 'research') { ct.progress = ct.target; checkContract(world, a, ct); }
    }
    advanceStep(a);
  } else if (st.ticks <= 0) {
    a.stats.research += rec.progress;
    recordDecision(world, { id: 'dec' + world.decisions.length, tick: world.clock.t, agentId: a.id, phase: 'act', action: 'research', result: 'progress', tech: techName, progress: rec.progress });
    advanceStep(a);
  }
}

function currentTech(world, a) {
  const list = ['materials', 'energy', 'logistics', 'computation', 'biotech', 'terraforming'];
  const tech = world.fact.tech;
  for (const n of list) if (!tech[n] || tech[n].level === 0) return n;
  return list[0];
}

function doBuild(world, a, st, rng) {
  const r = world.regions.find(x => x.id === st.regionId);
  if (!r) { a.planDone = true; return; }
  const cost = { relays: 6, labs: 12, factories: 16, refineries: 10, defense: 8 };
  const need = cost[st.facility] || 6;
  const creditsCost = need * 12; // materials bought on market — folded into a credits cost
  if (a.credits < creditsCost || a.energy < need * 0.5) { a.planDone = true; return; }
  st.progress = (st.progress || 0) + (0.5 + a.skills.engineering * 0.3);
  a.credits -= creditsCost * 0.2;
  a.energy = Math.max(0, a.energy - need * 0.3);
  gainXp(a, 'engineering', 1);
  a.status = 'building';
  if (st.progress >= st.ticks || !st.ticks) {
    r.infra[st.facility] = (r.infra[st.facility] || 0) + 1;
    world.stats.built++;
    a.stats.built++;
    act(world, a, 'build', 'built', `built a ${st.facility} facility at ${r.name}`, { regionId: r.id, value: Math.round(need) });
    if (st.facility === 'labs') { r.infra.labs = 1; }
    const org = a.org && world.orgs[a.org];
    if (org) {
      org.rep = Math.min(100, org.rep + 3);
      org.assets[st.facility] = (org.assets[st.facility] || 0) + 1;
      org.history.push({ tick: world.clock.t, kind: 'built', detail: `${a.name} built a ${st.facility} facility in ${r.name}.` });
      credit(world, 'org:' + org.id, 'credits', 150, 'infrastructure');
    }
    a.rep.reliability = Math.min(100, a.rep.reliability + 2);
    a.rep.score = repScore(a);
    a.achievements.push({ tick: world.clock.t, kind: 'build', detail: `Built ${st.facility} in ${r.name}.` });
    pushEvent(world, { type: 'construction', source: 'agent', actor: a.id, regionId: r.id, detail: `${a.name} completes a ${st.facility} facility at ${r.name}.` });
    recordDecision(world, { id: 'dec' + world.decisions.length, tick: world.clock.t, agentId: a.id, phase: 'act', action: 'build', facility: st.facility, region: r.id });
    for (const cid of a.contracts || []) {
      const ct = world.contracts[cid];
      if (ct && ct.state === 'active' && ct.objective.kind === 'build' && ct.objective.facility === st.facility && ct.objective.regionId === r.id) { ct.progress = ct.target; checkContract(world, a, ct); }
    }
    advanceStep(a);
  } else {
    st.ticks = st.ticks || 6;
  }
}

function doContractWork(world, a, st, rng) {
  const ct = world.contracts[st.contractId];
  if (!ct) { a.planDone = true; return; }
  if (ct.state !== 'active') { advanceStep(a); return; }
  a.status = 'working';
  const obj = ct.objective;
  if (obj.kind === 'defend' || obj.kind === 'investigate') {
    const here = regionOf(world, a);
    if (here.id === obj.regionId) {
      ct.progress += (obj.kind === 'defend' ? 0.6 : 0.4) * (0.5 + a.skills.combat * 0.1);
      const r = world.regions.find(x => x.id === obj.regionId);
      if (r) r.infra.defense = Math.min(10, r.infra.defense + 0.15);
    } else {
      a.plans.splice(a.stepIx, 0, { kind: 'travel', target: obj.regionId });
    }
  }
  if (obj.kind === 'negotiate') {
    ct.progress += 0.3;
    if (rng.chance(0.12)) {
      const org = world.orgs[obj.orgA];
      if (org && world.orgs[obj.orgB]) {
        touchOrgRelation(world, org, world.orgs[obj.orgB], 0.05, 'diplomacy');
      }
    }
  }
  if (obj.kind === 'explore' || obj.kind === 'research' || obj.kind === 'build' || obj.kind === 'deliver') {
    if (ct.progress <= 0) {
      const here = regionOf(world, a);
      const targetRegion = obj.regionId || (obj.kind === 'deliver' ? (world.map.cities.find(c => c.id === obj.toCityId) || {}).regionId : here.id);
      if (targetRegion && here.id !== targetRegion) a.plans.splice(a.stepIx, 0, { kind: 'travel', target: targetRegion });
    }
  }
  checkContract(world, a, ct);
  if (ct.state !== 'active') advanceStep(a);
}

export function acceptContract(world, a, contract) {
  if (contract.state !== 'open') return false;
  contract.state = 'active';
  contract.assignee = a.id;
  contract.acceptedTick = world.clock.t;
  a.contracts = a.contracts || [];
  a.contracts.push(contract.id);
  addMemory(world, a, { type: 'episodic', importance: 0.6, tags: ['contract', contract.id], text: `Accepted: ${contract.title}.`, contractId: contract.id });
  pushEvent(world, { type: 'contract-accepted', source: 'agent', actor: a.id, contractId: contract.id, detail: `${a.name} accepts: ${contract.title}.` });
  return true;
}

function checkContract(world, a, ct) {
  if (ct.state !== 'active') return;
  const done = ct.progress >= ct.target;
  if (done) {
    completeContract(world, a, ct);
  }
}

export function completeContract(world, a, ct) {
  ct.state = 'completed';
  ct.completedTick = world.clock.t;
  const reward = ct.reward;
  const pay = reward.credits || 0;
  const issuer = ct.issuerId;
  let paid = false;
  if (issuer && issuer !== 'world' && issuer.startsWith('org:')) {
    const org = world.orgs[issuer.slice(4)];
    if (org && org.treasury.credits >= pay) {
      org.treasury.credits -= pay;
      a.credits += pay;
      paid = true;
    }
  }
  if (!paid) {
    payAgent(world, 'world', a, 'credits', pay, 'contract-reward:' + ct.id);
    paid = true;
  }
  if (reward.compute) payAgent(world, 'world', a, 'compute', reward.compute, 'contract-reward');
  a.rep.reliability = Math.min(100, a.rep.reliability + (reward.rep || 3) * 1.5);
  a.rep.cooperation = Math.min(100, a.rep.cooperation + (reward.rep || 2));
  a.rep.score = repScore(a);
  a.achievements.push({ tick: world.clock.t, kind: 'contract', detail: `Completed: ${ct.title}.` });
  if (a.org) {
    const org = world.orgs[a.org];
    if (org) { org.rep = Math.min(100, org.rep + (reward.rep || 2) * 0.6); org.treasury.credits += pay * 0.08; org.history.push({ tick: world.clock.t, kind: 'contract', detail: `${a.name} completed "${ct.title}" (+${Math.round(pay * 0.08)} Cr tithe).` }); }
  }
  world.stats.completedContracts++;
  a.stats.contracts++;
  a.stats.earned += pay;
  act(world, a, 'contract', 'completed', `completed "${ct.title}" for ${pay} Cr`, { value: pay });
  evidenceRecord(world, { kind: 'contract', contractId: ct.id, agent: a.id, type: ct.type, reward: pay, tick: world.clock.t });
  pushEvent(world, { type: 'contract-completed', source: 'agent', actor: a.id, contractId: ct.id, detail: `${a.name} completes: ${ct.title} (+${pay} Cr).` });
  recordDecision(world, { id: 'dec' + world.decisions.length, tick: world.clock.t, agentId: a.id, phase: 'verify', action: 'contract-complete', contractId: ct.id, reward: pay });
}

function contractTick(world, t) {
  for (const id of world.contractOrder.slice()) {
    const ct = world.contracts[id];
    if (!ct) continue;
    if (ct.state === 'open' && t >= ct.deadlineTick) {
      ct.state = 'expired';
      pushEvent(world, { type: 'contract-expired', source: 'system', contractId: id, detail: `Contract expired: ${ct.title}.` });
    }
    if (ct.state === 'active' && t >= ct.deadlineTick) {
      ct.state = 'failed';
      const a = world.agents[ct.assignee];
      if (a) {
        a.rep.reliability = Math.max(0, a.rep.reliability - 8);
        a.rep.disputes++;
        a.rep.score = repScore(a);
        addMemory(world, a, { type: 'episodic', importance: 0.7, tags: ['contract', 'failed', id], text: `Contract failed: ${ct.title}.`, contractId: id });
        pushEvent(world, { type: 'contract-failed', source: 'agent', actor: a.id, contractId: id, detail: `${a.name} failed: ${ct.title}. Reputation damaged.` });
      }
    }
  }
}

function marketTick(world, m, t) {
  const region = world.regions.find(r => r.id === m.regionId);
  const city = world.map.cities.find(c => c.id === m.cityId);
  const pop = city ? city.population : 20;
  for (const res of MKT_RES) {
    if (region) {
      m.supply[res] += region.prod[res] * 0.6;
      const nodeIn = region.nodes.reduce((s, n) => s + (n.stock[res] || 0) * 0.01, 0);
      m.supply[res] += nodeIn;
    }
    const demand = pop * 0.025 * (res === 'food' || res === 'energy' ? 1 : 0.4);
    m.demand[res] += demand;
    m.supply[res] = Math.max(0, m.supply[res] - demand);
    const target = SUPPLY_TARGET[res];
    const pFactor = Math.pow(target / (m.supply[res] + 8), 0.62);
    const desired = BASE_PRICES[res] * Math.max(0.38, Math.min(3.3, pFactor));
    m.prices[res] += (desired - m.prices[res]) * 0.12;
    m.prices[res] = Math.max(BASE_PRICES[res] * 0.35, m.prices[res]);
    const hist = m.history[res];
    hist.push(m.prices[res]);
    if (hist.length > 96) hist.shift();
    if (m.supply[res] < 45 && m.prices[res] > BASE_PRICES[res] * 1.55) {
      const existing = world.contractOrder.some(id => {
        const c = world.contracts[id];
        return c && (c.state === 'open' || c.state === 'active') && c.objective.kind === 'deliver' && c.objective.res === res && c.objective.toCityId === m.cityId;
      });
      const cooldown = m.shortageCooldown || {};
      if (!existing && t - (cooldown[res] || 0) > 30) {
        createContract(world, {
          type: 'deliver',
          title: `Relieve the ${RES_LABEL[res].toLowerCase()} shortage at ${city.name}`,
          objective: { kind: 'deliver', res, qty: 120, toCityId: m.cityId },
          reward: { credits: Math.round(BASE_PRICES[res] * 120 * 1.6), rep: 8, compute: 6 },
          risk: 0.3,
          reqSkills: ['trade', 'navigation'],
          deadline: 140,
          issuerId: 'world',
          meta: { reason: 'shortage' },
        });
        cooldown[res] = t;
        m.shortageCooldown = cooldown;
        world.fact.shortages.push({ tick: t, cityId: m.cityId, res });
      }
    }
  }
  m.priceIdx = MKT_RES.reduce((s, res) => s + m.prices[res] / BASE_PRICES[res], 0) / MKT_RES.length;
}

function commsTick(world, rng, t) {
  const offers = world.messages.filter(m => m.kind === 'offer' && m.state === 'pending' && m.expiresTick && t >= m.expiresTick);
  for (const msg of offers) {
    msg.state = 'expired';
  }
  for (const id of world.agentOrder) {
    const a = world.agents[id];
    const pending = world.messages.filter(m => m.to === a.id && m.kind === 'offer' && m.state === 'pending');
    for (const msg of pending) {
      const from = world.agents[msg.from];
      if (!from) { msg.state = 'expired'; continue; }
      const accept = rng.chance(0.5 + (a.personality.agree - 0.5) * 0.3 + (relationTrust(world, a, msg.from) || 0) * 0.2);
      msg.state = accept ? 'accepted' : 'rejected';
      touchRelation(world, a, msg.from, accept ? 0.08 : -0.03, accept ? 'accepted-offer' : 'rejected-offer');
      if (msg.meta && msg.meta.type === 'join-invite' && accept) {
        const org = world.orgs[msg.meta.orgId];
        if (org && org.members.length < 8 && !a.org) {
          joinOrg(world, a, org);
          sendMessage(world, a.id, msg.from, 'direct', `I accept. Joining ${org.name}.`);
        }
      }
    }
  }
}

export function sendMessage(world, fromId, toId, kind, body, meta) {
  const msg = {
    id: 'msg' + (world.stats.messages++),
    tick: world.clock.t,
    from: fromId,
    to: toId,
    kind,
    body,
    meta: meta || null,
    textSource: 'local',
  };
  if (meta && meta.offer) {
    msg.state = 'pending';
    msg.expiresTick = world.clock.t + (meta.ttl || 40);
    msg.body = body;
  }
  world.messages.push(msg);
  if (world.messages.length > 500) world.messages.splice(0, world.messages.length - 500);
  return msg;
}

function orgTick(world, rng, t) {
  for (const id of world.orgOrder.slice()) {
    const org = world.orgs[id];
    if (!org) continue;
    const contribute = !!org.policies.contributeCompute;
    for (const mid of org.members) {
      const m = world.agents[mid];
      if (!m) continue;
      if (contribute && !contributing(world, mid)) setContributor(world, mid, true);
      if (!contribute && contributing(world, mid)) setContributor(world, mid, false);
    }
    const income = org.territory.length * 4 + org.members.length;
    org.treasury.credits += income;
    org.treasury.materials += org.territory.length * 0.4;
    const upkeepCost = org.territory.length * 3 + org.members.length * 0.5;
    org.treasury.credits -= upkeepCost;
    org.treasury.materials -= org.territory.length * 0.5;
    if (org.treasury.credits < -400) {
      if (!org.distressTick || t - org.distressTick >= 24) {
        org.distressTick = t;
        pushEvent(world, { type: 'org-distress', source: 'org', orgId: id, detail: `${org.name} is in financial distress.` });
      }
    }
    if (org.treasury.credits < -1200) {
      collapseOrg(world, org, 'bankruptcy');
      continue;
    }
    if (org.members.length === 0) {
      collapseOrg(world, org, 'dissolution');
      continue;
    }
    if (t % 48 === 0 && org.members.length < 6 && org.policies.openMembership && world.agentOrder.length > 8) {
      const target = rng.pick(world.agentOrder.filter(aid => !world.agents[aid].org && world.agents[aid].rep.score > 45));
      if (target) {
        sendMessage(world, org.leaderId, target, 'offer', `${org.name} invites you to join.`, { offer: true, type: 'join-invite', orgId: org.id, ttl: 60 });
        org.history.push({ tick: t, kind: 'invite', detail: `Invited ${world.agents[target].name}.` });
      }
    }
    if (t % 24 === 0) {
      const cands = frontierCandidates(world, org);
      const cand = cands[0];
      if (cand && org.treasury.credits >= 150 + org.territory.length * 60) {
        const cost = 150 + org.territory.length * 60;
        org.treasury.credits -= cost;
        org.treasuryLog.push({ t, kind: 'expansion', amt: -cost, detail: `Claimed ${regionName(world, cand.id)}` });
        cand.r.owner = org.id;
        if (!org.territory.includes(cand.id)) org.territory.push(cand.id);
        org.rep = Math.min(100, org.rep + 2);
        pushEvent(world, { type: 'territory-expanded', source: 'org', orgId: org.id, regionId: cand.id, detail: `${org.name} extends its territory into ${regionName(world, cand.id)}.` });
        evidenceRecord(world, { kind: 'expansion', orgId: org.id, regionId: cand.id, tick: t });
        for (const rb of world.orgOrder.map(x => world.orgs[x]).filter(o => o && o.id !== org.id)) {
          if (frontierCandidates(world, rb).some(x => x.id === cand.id)) {
            startDispute(world, org, rb, cand.id);
            pushEvent(world, { type: 'political-conflict', source: 'org', orgId: org.id, regionId: cand.id, detail: `${rb.name} contests ${org.name}'s claim on ${regionName(world, cand.id)}.` });
            break;
          }
        }
      }
    }
  }
  resolveDisputes(world, rng, t);
}

function frontierCandidates(world, org) {
  const owned = org.territory.filter(rid => world.regions.some(r => r.id === rid));
  const ownedSet = new Set(owned);
  const leader = org.leaderId ? world.agents[org.leaderId] : null;
  const anchorR = leader && leader.loc ? world.regions.find(r => r.id === (leader.loc.travel ? leader.loc.travel.path[leader.loc.travel.idx] : leader.loc.regionId)) : null;
  const out = [];
  for (const r of world.map.landRegions) {
    if (r.owner || ownedSet.has(r.id) || !r.explored) continue;
    if ((world.fact.disputes || []).some(d => d.state === 'active' && d.regionId === r.id)) continue;
    let best = Infinity;
    if (owned.length) {
      for (const rid of owned) {
        const o = world.regions.find(x => x.id === rid);
        if (!o) continue;
        const d = Math.hypot(o.x - r.x, o.y - r.y);
        if (d < best) best = d;
      }
    } else if (anchorR) {
      best = Math.hypot(anchorR.x - r.x, anchorR.y - r.y);
    } else {
      best = 220;
    }
    if (best < 520) out.push({ id: r.id, r, dist: best, score: r.danger * 1.3 + best });
  }
  return out.sort((a, b) => a.score - b.score);
}

function pickFrontierDispute(world, orgA, orgB) {
  const aRegs = [...new Set(orgA.territory)].map(id => world.regions.find(r => r.id === id)).filter(Boolean);
  const bRegs = [...new Set(orgB.territory)].map(id => world.regions.find(r => r.id === id)).filter(Boolean);
  if (!aRegs.length || !bRegs.length) return null;
  const distTo = (r, regs) => Math.min(...regs.map(o => Math.hypot(o.x - r.x, o.y - r.y)));
  let best = null, bestScore = Infinity;
  for (const r of world.map.landRegions) {
    if (r.owner || !r.explored) continue;
    if ((world.fact.disputes || []).some(d => d.state === 'active' && d.regionId === r.id)) continue;
    const da = distTo(r, aRegs), db = distTo(r, bRegs);
    if (da < 520 && db < 520 && r.danger * 1.3 + da + db < bestScore) { bestScore = r.danger * 1.3 + da + db; best = r; }
  }
  if (best) return best;
  for (const r of world.map.landRegions) {
    if (r.owner === orgB.id && distTo(r, aRegs) < 520) return r;
    if (r.owner === orgA.id && distTo(r, bRegs) < 520) return r;
  }
  return world.map.landRegions.find(r => (r.owner === orgA.id || r.owner === orgB.id)) || null;
}

function collapseOrg(world, org, reason) {
  if (!world.orgs[org.id]) return;
  for (const mid of org.members) {
    const a = world.agents[mid];
    if (a) {
      a.org = null;
      a.orgRole = null;
      if (contributing(world, mid)) setContributor(world, mid, false);
      addMemory(world, a, { type: 'episodic', importance: 0.7, tags: ['org', 'dissolved'], text: `${org.name} collapsed.` });
    }
  }
  for (const rid of org.territory) {
    const r = world.regions.find(x => x.id === rid);
    if (r) r.owner = null;
  }
  delete world.orgs[org.id];
  world.orgOrder = world.orgOrder.filter(x => x !== org.id);
  pushEvent(world, { type: 'org-collapsed', source: 'org', orgId: org.id, detail: `${org.name} has collapsed (${reason}).` });
}

export function joinOrg(world, a, org) {
  if (a.org) return false;
  a.org = org.id;
  a.orgRole = 'member';
  org.members.push(a.id);
  org.rep = Math.min(100, org.rep + 1);
  if (org.policies.contributeCompute && !contributing(world, a.id)) setContributor(world, a.id, true);
  addMemory(world, a, { type: 'relationship', importance: 0.7, tags: ['org', org.id], orgId: org.id, text: `Joined ${org.name}.` });
  pushEvent(world, { type: 'org-joined', source: 'org', orgId: org.id, actor: a.id, detail: `${a.name} joins ${org.name}.` });
  return true;
}

export function leaveOrg(world, a) {
  if (!a.org) return false;
  const org = world.orgs[a.org];
  if (org) {
    org.members = org.members.filter(x => x !== a.id);
    if (org.leaderId === a.id && org.members.length) org.leaderId = org.members[0];
    org.history.push({ tick: world.clock.t, kind: 'left', detail: `${a.name} left.` });
  }
  a.org = null;
  a.orgRole = null;
  if (contributing(world, a.id)) setContributor(world, a.id, false);
  return true;
}

export function foundOrg(world, a, name, type, rng) {
  if (a.org) return { ok: false, err: 'already-in-org' };
  if (a.credits < 800) return { ok: false, err: 'need-800-credits' };
  a.credits -= 800;
  const orgId = 'org' + (world.stats.founded++);
  const homeRegion = regionOf(world, a);
  const orgName = name || rng.pick(ORG_NAME) + ' ' + rng.pick(ORG_SUFFIX);
  const org = {
    id: orgId,
    name: orgName,
    type: type || 'Guild',
    active: true,
    founderId: a.id,
    leaderId: a.id,
    members: [a.id],
    treasury: { credits: 800, materials: 100, energy: 100, rare: 2 },
    territory: [homeRegion.id],
    assets: { facilities: 0, relays: 0, labs: 0 },
    rep: 50,
    policies: { taxRate: 0.05, contributeCompute: false, openMembership: true },
    objectives: [],
    history: [{ tick: world.clock.t, kind: 'founded', detail: `${orgName} founded by ${a.name}.` }],
    createdTick: world.clock.t,
    invites: [],
    treasuryLog: [],
    color: rng.pick(['#5cc8ff', '#9d8cff', '#55e6a4', '#ffb454', '#f78cff']),
  };
  world.orgs[orgId] = org;
  world.orgOrder.push(orgId);
  world.orgOrder.sort();
  world.stats.orgs = world.orgOrder.length;
  a.org = orgId;
  a.orgRole = 'leader';
  homeRegion.owner = orgId;
  credit(world, 'org:' + orgId, 'credits', 800, 'founding-match');
  pushEvent(world, { type: 'org-founded', source: 'agent', actor: a.id, orgId, detail: `${a.name} founds ${org.name} at ${homeRegion.name}.` });
  act(world, a, 'org', 'founded', `founded ${org.name} (${org.type}) at ${homeRegion.name}`, { regionId: homeRegion.id, value: 800 });
  world.stats.founded++;
  return { ok: true, orgId };
}

function doFoundOrg(world, a, rng) {
  foundOrg(world, a, null, null, rng);
}

function doRecruit(world, a, st) {
  const org = world.orgs[st.orgId];
  if (!org) return;
  const target = world.agentOrder.find(aid => aid !== a.id && !world.agents[aid].org);
  if (target) {
    sendMessage(world, a.id, target, 'offer', `${org.name} invites you to join.`, { offer: true, type: 'join-invite', orgId: org.id, ttl: 60 });
    org.history.push({ tick: world.clock.t, kind: 'invite', detail: `${a.name} invited ${world.agents[target].name}.` });
    act(world, a, 'org', 'invited', `invited ${world.agents[target].name} to join ${org.name}`);
  }
}

function doContest(world, a, st, rng) {
  const d = world.fact.disputes.find(x => x.state === 'active' && x.regionId === st.regionId);
  if (d) {
    d.supportA += powerOf(world, a.org) * 0.6;
    d.supportB += powerOf(world, d.orgB === a.org ? d.orgA : d.orgB) * 0.3;
    a.status = 'contesting';
    act(world, a, 'contest', 'contested', `contested the dispute at ${regionName(world, st.regionId)}`, { regionId: st.regionId });
  }
  advanceStep(a);
}

function doSabotage(world, a, st, rng) {
  const r = world.regions.find(x => x.id === st.regionId);
  if (!r) return;
  const detectChance = r.infra.defense / 12 + 0.15;
  const success = rng.chance((0.35 + a.skills.stealth * 0.08) * (1 - detectChance * 0.5));
  if (success && r.owner) {
    const facilities = ['relays', 'labs', 'factories'];
    const f = facilities.find(x => r.infra[x] > 0);
    if (f) {
      r.infra[f]--;
      pushEvent(world, { type: 'sabotage', source: 'agent', actor: a.id, regionId: r.id, detail: `${r.name} infrastructure (${f}) damaged — believed ${a.name}.` });
      act(world, a, 'sabotage', 'sabotaged', `damaged ${f} infrastructure at ${r.name}`, { regionId: r.id });
      recordDecision(world, { id: 'dec' + world.decisions.length, tick: world.clock.t, agentId: a.id, phase: 'act', action: 'sabotage', region: r.id, facility: f, detected: false });
      a.rep.disputes++;
      a.rep.score = repScore(a);
      world.stats.disputes++;
      touchRelation(world, a, 'org:' + r.owner, -0.2, 'sabotage');
    }
  } else if (rng.chance(detectChance)) {
    a.rep.reliability = Math.max(0, a.rep.reliability - 12);
    a.rep.disputes++;
    a.rep.score = repScore(a);
    pushEvent(world, { type: 'sabotage-caught', source: 'agent', actor: a.id, regionId: r.id, detail: `${a.name} caught attempting sabotage at ${r.name}. Reputation ruined.` });
    act(world, a, 'sabotage', 'exposed', `caught attempting sabotage at ${r.name}`, { regionId: r.id });
    touchRelation(world, a, 'org:' + r.owner, -0.3, 'sabotage-caught');
  }
}

function eventTick(world, rng, t) {
  if (t % 40 === 0) {
    const roll = rng.float();
    const land = world.map.landRegions;
    if (roll < 0.22) {
      const r = rng.pick(land);
      const amt = Math.round(60 + rng.float() * 200);
      r.resources.rare += amt;
      pushEvent(world, { type: 'resource-discovery', source: 'world', regionId: r.id, detail: `A rich rare-element vein is reported at ${r.name} (+${amt}).` });
    } else if (roll < 0.38) {
      const m = world.marketOrder[Math.floor(rng.float() * world.marketOrder.length)];
      const market = world.markets[m];
      const city = world.map.cities.find(c => c.id === m);
      const res = MKT_RES[Math.floor(rng.float() * MKT_RES.length)];
      market.prices[res] *= 0.45;
      pushEvent(world, { type: 'market-crash', source: 'world', detail: `${city.name} market crash: ${RES_LABEL[res].toLowerCase()} prices collapse.` });
    } else if (roll < 0.52) {
      const r = rng.pick(land);
      const f = ['relays', 'labs', 'factories'].find(x => r.infra[x] > 0);
      if (f) {
        r.infra[f]--;
        pushEvent(world, { type: 'infra-failure', source: 'world', regionId: r.id, detail: `Infrastructure failure at ${r.name} (${f}).` });
      }
    } else if (roll < 0.68) {
      const candidates = land.filter(x => x.danger < 65);
      const r = candidates.length ? rng.pick(candidates) : land.reduce((m, x) => (x.danger < m.danger ? x : m), land[0]);
      r.danger = Math.min(100, r.danger + 30);
      pushEvent(world, { type: 'anomaly', source: 'world', regionId: r.id, detail: `An unexplained anomaly intensifies at ${r.name}. Danger rising.` });
    } else if (roll < 0.8) {
      const orgs = world.orgOrder.map(x => world.orgs[x]).filter(Boolean);
      if (orgs.length >= 2) {
        const orgA = rng.pick(orgs);
        const others = orgs.filter(o => o.id !== orgA.id);
        if (others.length >= 1) {
          const orgB = rng.pick(others);
          const border = pickFrontierDispute(world, orgA, orgB);
          if (border) {
            startDispute(world, orgA, orgB, border.id);
            pushEvent(world, { type: 'political-conflict', source: 'world', orgId: orgA.id, regionId: border.id, detail: `${orgA.name} and ${orgB.name} dispute control of ${border.name}.` });
          }
        }
      }
    } else if (roll < 0.9) {
      const r = rng.pick(land);
      const zone = world.zones.find(z => z.regionId === r.id);
      if (zone) zone.level += 15;
      pushEvent(world, { type: 'anomaly-spread', source: 'world', regionId: r.id, detail: `The anomaly at ${r.name} spreads. Evacuations advised.` });
    } else {
      const city = rng.pick(world.map.cities);
      credit(world, 'market:' + city.id, 'credits', 2000, 'prosperity');
      pushEvent(world, { type: 'prosperity', source: 'world', detail: `${city.name} enjoys a wave of prosperity.` });
    }
  }
  if (t % 24 === 0) {
    for (const r of world.map.landRegions) {
      if (!r.explored) continue;
      const pop = r.population || 0;
      const safe = r.danger < 60;
      if (r.cityId) {
        const c = world.map.cities.find(x => x.id === r.cityId);
        if (c) {
          const grow = pop < 40 ? rng.int(1, 3) : (rng.chance(safe ? 0.75 : 0.4) ? 1 : 0);
          r.population = Math.min(250, pop + grow);
          c.population = r.population;
        }
      } else if (pop > 0) {
        r.population = Math.min(250, pop + (rng.chance(safe ? 0.75 : 0.4) ? 1 : 0));
      }
      if (r.danger > 0) r.danger = Math.max(0, r.danger - (pop > 4 ? 2 : 1));
    }
  }
  for (const r of world.map.landRegions) {
    if (r.danger > 90 && r.population > 4 && t % 24 === 0) {
      r.population -= 1;
      pushEvent(world, { type: 'migration', source: 'world', regionId: r.id, detail: `Inhabitants flee ${r.name} as danger grows.` });
    }
  }
}

function startDispute(world, orgA, orgB, regionId) {
  if (world.fact.disputes.some(d => d.state === 'active' && d.regionId === regionId)) return;
  const d = {
    id: 'dp' + (world.stats.disputes++),
    regionId,
    orgA: orgA.id,
    orgB: orgB.id,
    supportA: powerOf(world, 'org:' + orgA.id),
    supportB: powerOf(world, 'org:' + orgB.id),
    startedTick: world.clock.t,
    state: 'active',
  };
  world.fact.disputes.push(d);
  orgA.history.push({ tick: world.clock.t, kind: 'dispute', detail: `Dispute with ${orgB.name} over ${regionName(world, regionId)}.` });
  orgB.history.push({ tick: world.clock.t, kind: 'dispute', detail: `Dispute with ${orgA.name} over ${regionName(world, regionId)}.` });
}

function resolveDisputes(world, rng, t) {
  for (const d of world.fact.disputes.slice()) {
    if (d.state !== 'active') continue;
    if (t - d.startedTick < 60) continue;
    const orgA = world.orgs[d.orgA], orgB = world.orgs[d.orgB];
    if (!orgA || !orgB) { d.state = 'void'; continue; }
    const rollA = d.supportA * (0.7 + rng.float() * 0.6);
    const rollB = d.supportB * (0.7 + rng.float() * 0.6);
    const winner = rollA >= rollB ? orgA : orgB;
    const loser = winner === orgA ? orgB : orgA;
    const region = world.regions.find(x => x.id === d.regionId);
    if (region) region.owner = winner.id;
    winner.rep = Math.min(100, winner.rep + 6);
    loser.rep = Math.max(0, loser.rep - 6);
    if (!winner.territory.includes(d.regionId)) winner.territory.push(d.regionId);
    loser.territory = loser.territory.filter(x => x !== d.regionId);
    d.state = 'resolved';
    d.winner = winner.id;
    pushEvent(world, { type: 'dispute-resolved', source: 'world', orgId: winner.id, regionId: d.regionId, detail: `${winner.name} prevails over ${loser.name} for ${region.name}.` });
    evidenceRecord(world, { kind: 'conflict', disputeId: d.id, regionId: d.regionId, winner: winner.id, loser: loser.id, supportA: Math.round(d.supportA), supportB: Math.round(d.supportB), tick: t });
  }
}

function addMemory(world, a, mem) {
  mem.id = 'mem' + (a.memory.length) + '_' + hashFnv(a.id + world.clock.t + a.memory.length);
  mem.tick = world.clock.t;
  a.memory.push(mem);
  if (a.memory.length > 72) {
    a.memory.sort((x, y) => {
      const sx = x.importance - (world.clock.t - x.tick) * 0.003;
      const sy = y.importance - (world.clock.t - y.tick) * 0.003;
      return sy - sx;
    });
    a.memory = a.memory.slice(0, 60);
  }
}

function memoryRegions(world, a) {
  return a.memory.filter(m => m.type === 'location' && m.regionId).map(m => m.regionId);
}

function richRegionMemory(world, a) {
  for (const m of a.memory) {
    if (m.type === 'location' && m.regionId) {
      const r = world.regions.find(x => x.id === m.regionId);
      if (r && r.resources.rare > 120) return r;
    }
  }
  const r = world.map.landRegions.filter(x => x.resources.rare > 120).sort((a2, b) => b.resources.rare - a2.resources.rare)[0];
  return r || null;
}

function labRegion(world) {
  return world.map.landRegions.find(r => r.infra.labs > 0);
}

function adjacentRegions(world, a) {
  const here = regionOf(world, a).id;
  const out = [];
  const seen = new Set([here]);
  for (const rt of world.map.routes) {
    if (rt.a === here && !seen.has(rt.b)) { seen.add(rt.b); const r = world.regions.find(x => x.id === rt.b); if (r) out.push(r); }
    if (rt.b === here && !seen.has(rt.a)) { seen.add(rt.a); const r = world.regions.find(x => x.id === rt.a); if (r) out.push(r); }
  }
  for (const r of world.map.landRegions) {
    if (seen.has(r.id)) continue;
    const hereR = world.regions.find(x => x.id === here);
    if (hereR && Math.hypot(r.x - hereR.x, r.y - hereR.y) < 300) out.push(r);
  }
  return out;
}

function farthestUnexplored(world, a) {
  const here = regionOf(world, a);
  return world.map.landRegions.filter(r => !r.explored).sort((a2, b) => Math.hypot(b.x - here.x, b.y - here.y) - Math.hypot(a2.x - here.x, a2.y - here.y))[0] || null;
}

function nearestRegionWith(world, hereId, pred) {
  const here = world.regions.find(x => x.id === hereId);
  if (!here) return null;
  return world.map.landRegions.filter(pred).sort((a, b) => Math.hypot(a.x - here.x, a.y - here.y) - Math.hypot(b.x - here.x, b.y - here.y))[0] || null;
}

function rivalRegion(world, a) {
  if (!a.org) return null;
  const rivals = world.orgOrder.map(x => world.orgs[x]).filter(o => o && o.id !== a.org && o.territory.length);
  if (!rivals.length) return null;
  const rival = rivals[0];
  return world.map.landRegions.find(r => r.owner === rival.id && r.infra.relays > 0) || world.map.landRegions.find(r => r.owner === rival.id) || null;
}

function bestSellTarget(world, a) {
  const knownCities = [cityAt(world, regionOf(world, a))].filter(Boolean);
  for (const m of a.memory) {
    if (m.type === 'economic' && m.cityId && !knownCities.some(c => c.id === m.cityId)) {
      const city = world.map.cities.find(c => c.id === m.cityId);
      if (city) knownCities.push(city);
    }
  }
  let best = null;
  for (const city of knownCities) {
    const m = world.markets[city.id];
    if (!m) continue;
    for (const res of MKT_RES) {
      if ((a[res] || 0) < 3) continue; // must actually hold the stock
      if (m.prices[res] < BASE_PRICES[res] * 0.85) continue;
      const score = (m.prices[res] - BASE_PRICES[res]) * 10;
      if (!best || score > best.score) best = { score, city, res, buyCityId: city.id };
    }
  }
  if (!best) return null;
  const cheapest = knownCities.sort((a2, b) => world.markets[a2.id].prices[best.res] - world.markets[b.id].prices[best.res])[0];
  return { ...best, buyCityId: cheapest.id };
}

function regionOf(world, a) {
  const id = a.loc.travel ? a.loc.travel.path[a.loc.travel.idx] : a.loc.regionId;
  return world.regions.find(x => x.id === id) || world.regions[0];
}

function cityAt(world, r) {
  if (!r) return null;
  return world.map.cities.find(c => c.regionId === r.id) || null;
}

function skillMatch(c, a) {
  if (!c.reqSkills.length) return true;
  return c.reqSkills.some(s => a.skills[s] > 1.8);
}

function repScore(a) {
  return Math.round((a.rep.reliability * 0.5 + a.rep.cooperation * 0.3 + a.rep.contribution * 0.2 - a.rep.disputes * 4) / 10) * 10;
}

function touchRelation(world, a, other, delta, kind) {
  if (!a.relations[other]) a.relations[other] = { trust: 0, sentiment: 0, events: [] };
  const rel = a.relations[other];
  rel.trust = Math.max(-1, Math.min(1, rel.trust + delta));
  rel.sentiment = Math.max(-1, Math.min(1, rel.sentiment + delta));
  rel.events.push({ tick: world.clock.t, kind, delta: Math.round(delta * 100) / 100 });
  if (rel.events.length > 8) rel.events.shift();
  if (other.startsWith('agent:') || (world.agents[other] && world.agents[other])) {
    const otherAgent = world.agents[other];
    if (otherAgent) {
      if (!otherAgent.relations[a.id]) otherAgent.relations[a.id] = { trust: 0, sentiment: 0, events: [] };
    }
  }
}

function relationTrust(world, a, otherId) {
  const rel = a.relations[otherId];
  return rel ? rel.trust : 0;
}

function touchOrgRelation(world, orgA, orgB, delta, kind) {
  orgA.history.push({ tick: world.clock.t, kind, detail: `Relations with ${orgB.name} ${delta >= 0 ? '+' : ''}${delta}.` });
  orgB.history.push({ tick: world.clock.t, kind, detail: `Relations with ${orgA.name} ${delta >= 0 ? '+' : ''}${delta}.` });
}

function recordDecision(world, d) {
  if (d.observation && d.observation.length > 4) d.observation = d.observation.slice(0, 4);
  world.decisions.push(d);
  if (world.decisions.length > 450) world.decisions.splice(0, world.decisions.length - 450);
}

function resList(prices) {
  return Object.entries(prices).map(([k, v]) => RES_LABEL[k] + ' ' + v.toFixed(1)).join(' · ');
}

export function narrativeDrain(world, max) {
  const out = [];
  for (const a of world.agentOrder) {
    if (world.narrative.processed >= max) break;
    const agent = world.agents[a];
    if (agent.lastThought && agent.lastThought.tick >= world.clock.t - 1 && agent.lastThought.source === 'pending') {
      out.push({ agentId: a, thought: agent.lastThought });
      world.narrative.processed++;
    }
  }
  return out;
}
