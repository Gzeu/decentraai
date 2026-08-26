import { createWorld, advanceTicks } from './sim.js';
import { createVesper } from './console.js';
import { createUI } from './ui.js';
import { pointInPoly, rebuildPolys } from './core.js';
import { configure as configureFabric, probe as probeFabric, fetchRealAgents, fetchEconomy } from './decentraai.js';

// ---- Self-contained config (no external pjs/plugin dependency) ----
// World + DecentraAI fabric bridge. Set `fabric.baseUrl` to the live fabric
// host (same-origin is fine: '' = same host, '/path' = relative). When
// reachable, agent compute jobs dispatch real workload to the fabric.
const VESPER_CONFIG = {
  seed: 'vesper-alpha-7',
  regionCount: 24,
  initialAgents: 26,
  seedOrganizations: 4,
  tickHours: 1,
  baseSpeed: 1,
  narrativeEngine: 'local', // 'local' | 'ai' (ai requires a model provider)
  narrativeBudgetPerMin: 3,
  computePoolSize: 3,
  maxCatchupDays: 7,
  fabric: {
    // Same-origin by default: '' resolves to the current host (no CORS).
    // e.g. ''  → the fabric this page is served from
    baseUrl: '',
    adminDcaKey: '',
    enabled: true,
  },
};

const wc = VESPER_CONFIG;
const dca = VESPER_CONFIG.fabric || {};

const cfg = {
  seed: wc.seed || 'vesper-alpha-7',
  regionCount: wc.regionCount || 24,
  initialAgents: wc.initialAgents || 26,
  seedOrganizations: wc.seedOrganizations || 4,
  tickHours: wc.tickHours || 1,
  baseSpeed: wc.baseSpeed || 1,
  narrativeEngine: wc.narrativeEngine || 'local',
  narrativeBudgetPerMin: wc.narrativeBudgetPerMin || 3,
  computePoolSize: wc.computePoolSize || 3,
  maxCatchupDays: wc.maxCatchupDays || 7,
  decentraai: {
    baseUrl: (dca.baseUrl || '').replace(/^https?:\/\//i, ''),
    adminDcaKey: dca.adminDcaKey || '',
    enabled: !(dca.enabled === false),
  },
};

// ---- IndexedDB persistence (native; replaces the external kv plugin) ----
const DB_NAME = 'vesper';
const DB_STORE = 'world';
let _db = null;
function openDb() {
  return new Promise((resolve, reject) => {
    if (_db) return resolve(_db);
    if (!window.indexedDB) return reject(new Error('indexedDB unavailable'));
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(DB_STORE)) db.createObjectStore(DB_STORE);
    };
    req.onsuccess = () => { _db = req.result; resolve(_db); };
    req.onerror = () => reject(req.error);
  });
}
async function dbGet() {
  try {
    const db = await openDb();
    return await new Promise((resolve, reject) => {
      const tx = db.transaction(DB_STORE, 'readonly');
      const r = tx.objectStore(DB_STORE).get('world');
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
  } catch (e) { return undefined; }
}
async function dbSet(value) {
  try {
    const db = await openDb();
    await new Promise((resolve, reject) => {
      const tx = db.transaction(DB_STORE, 'readwrite');
      tx.objectStore(DB_STORE).put(value, 'world');
      tx.oncomplete = resolve;
      tx.onerror = () => reject(tx.error);
    });
  } catch (e) { console.warn('vesper: save failed', e); }
}

let world = null;
let realAgents = [];
let speed = cfg.baseSpeed || 1;
let ui = null;
let saveQueued = false;
let curRoute = 'world';

function getWorld() { return world; }

function freshWorld(seedStr) {
  const w = createWorld(seedStr || cfg.seed, Object.assign({}, cfg, { realAgents: realAgents }));
  w.meta.lastTickReal = Date.now();
  world = w;
  return w;
}

function saveWorld() {
  if (!world) return Promise.resolve();
  world.meta.lastTickReal = Date.now();
  return dbSet(world);
}

function queueSave() {
  if (saveQueued) return;
  saveQueued = true;
  setTimeout(() => { saveQueued = false; saveWorld(); }, 2000);
}

function catchUpOffline() {
  if (!world) return;
  const last = world.meta.lastTickReal || Date.now();
  const elapsedH = (Date.now() - last) / 3600000;
  const ticks = Math.min(Math.max(0, Math.floor(elapsedH)), (cfg.maxCatchupDays || 7) * 24);
  if (ticks <= 0) return;
  const t0 = world.clock.t;
  try {
    advanceTicks(world, ticks);
    console.log('vesper: caught up ' + ticks + ' ticks (' + (world.clock.t - t0) + ' advanced)');
  } catch (e) {
    console.error('vesper: catchup failed', e);
  }
}

function runCatchup() {
  const ticks = 24;
  advanceTicks(world, ticks);
  speed = 1;
  queueSave();
  if (ui) { ui.toast('Advanced 1 day — ' + ticks + ' ticks simulated'); ui.refresh(); }
}

function tickLoop() {
  if (!world || !ui) return;
  if (speed === 0) return;
  if (speed === -1) { runCatchup(); return; }
  try {
    advanceTicks(world, speed);
  } catch (e) {
    console.error('vesper: tick failed', e);
    speed = 0;
    if (ui) ui.toast('Simulation error — paused', 'err');
  }
  queueSave();
}

async function boot() {
  console.log('vesper: boot start');
  configureFabric(cfg.decentraai);
  realAgents = await fetchRealAgents();
  console.log('vesper: fetched realAgents =', realAgents.length);
  if (realAgents.length) {
    console.log('vesper: importing ' + realAgents.length + ' real fabric agents');
  }

  const saved = await dbGet();
  if (saved && saved.meta && saved.meta.id) { world = saved; }

  if (!world) freshWorld(cfg.seed);
  for (const id of world.agentOrder) {
    const a = world.agents[id];
    if (a && a.org && !world.orgs[a.org]) { a.org = null; a.orgRole = null; }
  }
  const seenOrgs = new Set();
  const fixedOrder = [];
  for (const id of world.orgOrder || []) {
    if (world.orgs[id] && !seenOrgs.has(id)) { seenOrgs.add(id); fixedOrder.push(id); }
  }
  for (const id of Object.keys(world.orgs)) {
    if (!seenOrgs.has(id)) { seenOrgs.add(id); fixedOrder.push(id); }
  }
  world.orgOrder = fixedOrder;
  for (const c of (world.map && world.map.cities) || []) {
    const cr = (world.regions || []).find(x => x.id === c.regionId);
    if (cr) { if (c.x == null) c.x = cr.x; if (c.y == null) c.y = cr.y; }
  }
  const geoBroken = !world.map || !world.regions || world.regions.some(r => r.land && (!r.poly || r.poly.length < 3 || !pointInPoly(r.x, r.y, r.poly)));
  if (geoBroken) {
    rebuildPolys(world);
    console.log('vesper: repaired region geometry (' + world.regions.length + ' cells rebuilt)');
  }
  if (!world.activity) world.activity = [];
  if (world.map && !world.map.trail) world.map.trail = {};
  let capped = 0;
  for (const r of (world.regions || [])) {
    if ((r.danger || 0) > 100) { r.danger = 100; capped++; }
  }
  if (capped) console.log('vesper: capped ' + capped + ' region danger values to 100');
  for (const id of world.agentOrder) {
    const a = world.agents[id];
    if (a && !a.stats) a.stats = { earned: 0, spent: 0, taxesPaid: 0, contracts: 0, discoveries: 0, research: 0, breakthroughs: 0, built: 0, produced: 0, tradedVol: 0, computeJobs: 0 };
  }
  if (!world.fabric) world.fabric = { log: [], calls: 0, ok: 0, fail: 0, sinceTick: world.clock.t, status: null };
  world.meta.narrativeEngine = world.meta.narrativeEngine || cfg.narrativeEngine;
  catchUpOffline();
  await saveWorld();

  probeFabric(world).then(() => { if (ui) ui.refresh(); });

  window.Vesper = createVesper(getWorld);

  ui = createUI({
    getWorld,
    getRoute: () => curRoute,
    setRoute: (name) => { curRoute = name; },
    getSpeed: () => speed,
    onSpeed: (v) => {
      speed = v;
      if (v === -1) runCatchup();
    },
    getVesper: () => window.Vesper,
    advance: (n) => {
      advanceTicks(world, Math.max(1, Math.floor(n) || 1));
      queueSave();
      if (ui) ui.refresh();
    },
    saveNow: () => { saveWorld().then(() => { if (ui) ui.toast('World saved'); }); },
    probeFabric: () => probeFabric(world).then(() => { if (ui) ui.refresh(); }),
    resetWorld: (seed) => {
      freshWorld(seed || ('vesper-' + Math.random().toString(36).slice(2, 10)));
      queueSave();
      if (ui) { ui.markDirty(); ui.nav('world'); ui.refresh(); ui.toast('World reset — new civilization born'); }
    },
  });

  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') saveWorld();
  });
  window.addEventListener('beforeunload', () => { saveWorld(); });

  setInterval(tickLoop, 150);
  setInterval(() => { if (ui) ui.refresh(); }, 900);
  // Real-economy mirror refresh (slow — the ledger changes on real work only).
  const refreshEconomy = () => { fetchEconomy().then(e => { if (world) world.fabric = world.fabric || { log: [], calls: 0, ok: 0, fail: 0, sinceTick: world.clock.t, status: null }; if (world) world.fabric.realEconomy = e; }).catch(() => {}); };
  refreshEconomy();
  setInterval(refreshEconomy, 30000);
}

boot();

// Keep the boot error visible instead of dying silently (unhandled rejection
// otherwise vanishes in a module context).
window.addEventListener('unhandledrejection', (e) => {
  console.error('vesper: unhandled rejection', e && e.reason);
});