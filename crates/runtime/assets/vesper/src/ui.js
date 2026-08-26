import { createMap } from './map.js';
import { regionName } from './sim.js';
import { RES_LABEL, RES_SYM, BIOME_INFO, fmt, fmtMoney, tickLabel, tickFull, balance, agentValue } from './core.js';
import { powerOf, contributing } from './compute.js';
import { statusOf } from './decentraai.js';

const ARCH_LABEL = {
  explorer: 'Explorer', merchant: 'Merchant', trader: 'Trader', scientist: 'Scientist', researcher: 'Researcher',
  engineer: 'Engineer', builder: 'Builder', strategist: 'Strategist', diplomat: 'Diplomat', mercenary: 'Mercenary',
  guardian: 'Guardian', opportunist: 'Opportunist',
};
const FILTERS = [
  ['all', 'All roles'], ['explorer', 'Explorers'], ['merchant', 'Merchants'], ['trader', 'Traders'],
  ['scientist', 'Scientists'], ['researcher', 'Researchers'], ['engineer', 'Engineers'], ['builder', 'Builders'],
  ['strategist', 'Strategists'], ['diplomat', 'Diplomats'], ['mercenary', 'Mercenaries'], ['guardian', 'Guardians'],
  ['opportunist', 'Opportunists'],
];
const DISTRICT_LEGEND = [
  ['Market', '#ffd166'], ['Industry', '#ff8a5c'], ['Institute', '#5cc8ff'], ['Fields', '#55e6a4'],
  ['Mine', '#c67bff'], ['Arena', '#ff5470'], ['Homes', '#8fa3c4'], ['Wild', '#4d6270'],
];
const CONTRACT_STATE_TAG = { open: ['Open', 'cyan'], active: ['Active', 'amber'], completed: ['Done', 'green'], failed: ['Failed', 'red'], expired: ['Expired', 'dim'] };
const EVENT_TAG = {
  discovery: ['Discovery', 'green'], breakthrough: ['Breakthrough', 'cyan'], construction: ['Construction', 'gold'],
  'compute-used': ['Compute', 'violet'], 'market-crash': ['Crash', 'amber'], shortage: ['Shortage', 'amber'],
  'org-founded': ['Org Founded', 'violet'], 'org-collapsed': ['Collapse', 'red'], 'org-joined': ['Joined', 'violet'],
  'org-distress': ['Distress', 'red'], 'contract-created': ['Contract', 'cyan'], 'contract-completed': ['Contract', 'green'],
  'contract-failed': ['Contract', 'red'], dispute: ['Dispute', 'red'], conflict: ['Conflict', 'red'],
  discovery2: ['Event', 'dim'], 'world-event': ['Event', 'gold'], genesis: ['Genesis', 'gold'],
};
const ACT_KIND = {
  trade: ['Trade', 'cyan'], gather: ['Gather', 'green'], mine: ['Mine', 'gold'],
  explore: ['Discovery', 'green'], research: ['Breakthrough', 'cyan'], build: ['Build', 'gold'],
  contract: ['Contract', 'violet'], org: ['Org', 'violet'], contest: ['Contest', 'red'],
  sabotage: ['Sabotage', 'red'], compute: ['Compute', 'cyan'],
};

export function createUI(deps) {
  const $ = (sel) => document.querySelector(sel);
  const world = () => deps.getWorld();
  const app = $('#app');
  const topbar = $('#topbar');
  const rail = $('#rail');
  const stage = $('#stage');
  const ticker = $('#ticker');
  const inspector = $('#inspector');
  const modal = $('#modal');

  let curScreen = deps.getRoute() || 'world';
  let sel = null;
  let map = null;
  let mapWrap = null;
  let lastT = -1;
  let inspectTab = 'overview';

  function h(tag, cls, text) {
    const el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text != null) el.textContent = text;
    return el;
  }
  function kv(k, v) {
    const row = h('div', 'kv-row');
    row.appendChild(h('span', 'k', k));
    const vs = h('span', 'v');
    if (v instanceof Node) vs.appendChild(v); else vs.textContent = v == null ? '—' : v;
    row.appendChild(vs);
    return row;
  }
  function tag(text, cls) {
    const t = h('span', 'tag ' + (cls || 'dim'));
    t.textContent = text;
    return t;
  }
  function bar(pct, cls) {
    const b = h('div', 'bar ' + (cls || ''));
    b.appendChild(h('i'));
    b.firstChild.style.width = Math.max(0, Math.min(100, pct * 100)) + '%';
    return b;
  }
  function money(n) { return fmt(n) + ' <span class="faint">Cr</span>'; }
  function resOf(a, res) {
    const v = a[res] || 0;
    const sym = RES_SYM[res] || '';
    return sym + ' ' + (Number.isInteger(v) ? v : v.toFixed(1));
  }
  function regionOf(a) { return a.loc.travel ? a.loc.travel.path[a.loc.travel.idx] : a.loc.regionId; }
  function regionNameOf(a) { return regionName(world(), regionOf(a)); }
  function agentById(id) { return world().agents[id]; }
  function orgById(id) { return world().orgs[id]; }
  function fmtTick(t) { return tickLabel(t); }

  function toast(msg, cls) {
    const t = h('div', 'toast ' + (cls || ''));
    t.textContent = msg;
    document.body.appendChild(t);
    setTimeout(() => t.remove(), 3400);
  }

  /* ================= topbar ================= */
  function renderTopbar() {
    topbar.innerHTML = '';
    const brand = h('div', 'brand');
    brand.innerHTML = '<span class="logo">VESPER</span><span class="sub">Autonomous World</span>';
    brand.addEventListener('click', () => nav('world'));
    topbar.appendChild(brand);

    const right = h('div', 'tb-right');

    const pulse = h('div', 'pulse-dot' + (deps.getSpeed() === 0 ? ' paused' : ''));
    pulse.title = deps.getSpeed() === 0 ? 'Simulation paused' : 'Simulation running';
    right.appendChild(pulse);

    const stats = h('div', 'flex gap14 items-center');
    const w = world();
    const st = w.stats;
    const mk = (label, val, cls) => { const s = h('span', 'tb-stat ' + (cls || '')); s.innerHTML = label + ' <b>' + val + '</b>'; return s; };
    stats.appendChild(mk('AGENTS', w.agentOrder.length));
    stats.appendChild(mk('ORGS', w.orgOrder.length));
    stats.appendChild(mk('CONTRACTS', Object.values(w.contracts).filter(c => c.state === 'active').length, 'hide-sm'));
    stats.appendChild(mk('JOBS', w.compute.stats.execs, 'hide-sm'));
    stats.appendChild(mk('FUND', fmtMoney((w.balances['world'] || {}).credits || 0), 'hide-sm'));
    right.appendChild(stats);

    const speeds = [['II', 0, 'Pause'], ['1x', 1, '1 tick/s'], ['10x', 10, '10 ticks/s'], ['100x', 100, '100 ticks/s'], ['MAX', -1, 'Catch up']];
    const speedGroup = h('div', 'speed-group');
    for (const [lbl, val, title] of speeds) {
      const b = h('button', 'speed-btn' + (deps.getSpeed() === val ? ' on' : ''), lbl);
      b.title = title;
      b.dataset.speed = val;
      b.addEventListener('click', () => deps.onSpeed(val));
      speedGroup.appendChild(b);
    }
    right.appendChild(speedGroup);

    const clock = h('div', 'clock');
    const dt = h('div', 'dt');
    const spd = h('div', 'spd');
    right.appendChild(clock); clock.appendChild(dt); clock.appendChild(spd);
    updateClock(clock);
    topbar.appendChild(right);
  }

  function updateClock(clockEl) {
    if (!clockEl) clockEl = $('.clock');
    if (!clockEl) return;
    const w = world();
    const t = w.clock.t;
    clockEl.querySelector('.dt').textContent = tickFull(t);
    const spd = deps.getSpeed();
    clockEl.querySelector('.spd').textContent = spd <= 0 ? 'PAUSED' : (spd === -1 ? 'CATCHING UP' : spd + ' TICKS/S');
  }

  /* ================= rail ================= */
  const NAV = [
    ['world', '◈', 'World'],
    ['map', '🗺', 'Map'],
    ['agents', '✦', 'Agents'],
    ['organizations', '◉', 'Organizations'],
    ['missions', '⚑', 'Missions'],
    ['markets', '⬡', 'Markets'],
    ['events', '≋', 'Events'],
    ['activity', '⏱', 'Activity'],
    ['compute', '⬢', 'Compute'],
    ['fabric', '⌬', 'Fabric'],
    ['evidence', '▚', 'Evidence'],
    ['economy', '¢', 'Economy'],
    ['replay', '↺', 'Replay'],
    ['console', '>_', 'Agent API'],
    ['admin', '⚙', 'Admin'],
  ];
  function renderRail() {
    rail.innerHTML = '';
    rail.appendChild(h('div', 'rail-sec', 'Observe'));
    for (const [key, ic, lbl] of NAV) {
      const b = h('button', 'rail-btn' + (curScreen === key ? ' on' : ''));
      b.innerHTML = `<span class="ic">${ic}</span><span class="lbl">${lbl}</span>`;
      b.addEventListener('click', () => nav(key));
      rail.appendChild(b);
    }
    const foot = h('div', 'rail-foot');
    const w = world();
    foot.innerHTML = `SEED ${w.meta.seed}<br>WORLD ${w.meta.id}<br><span class="faint">agents are the players</span>`;
    rail.appendChild(foot);
  }

  function nav(name) {
    curScreen = name;
    deps.setRoute(name);
    sel = null;
    renderRail();
    renderScreen();
  }

  /* ================= ticker ================= */
  function renderTicker() {
    ticker.innerHTML = '';
    ticker.appendChild(h('span', 'ticker-label', 'Live'));
    const w = world();
    const evs = w.events.slice(-7);
    const mk = (e) => {
      const it = h('span', 'ticker-item');
      it.innerHTML = `<span class="t">${fmtTick(e.tick)}</span> <span class="ev">${e.type}</span> ${esc(e.detail || '').slice(0, 90)}`;
      return it;
    };
    const half = h('div', '');
    half.style.display = 'inline-flex';
    for (const e of evs) half.appendChild(mk(e));
    const contW = ticker.clientWidth || 600;
    let guard = 0;
    while (half.getBoundingClientRect().width > 0 && half.getBoundingClientRect().width < contW && guard++ < 24) {
      for (const e of evs) half.appendChild(mk(e));
    }
    const track = h('div', 'ticker-track');
    track.appendChild(half);
    track.appendChild(half.cloneNode(true));
    track.style.animationDuration = Math.max(30, track.children[0].children.length * 1.6) + 's';
    ticker.appendChild(track);
  }

  /* ================= stage + screens ================= */
  function renderScreen() {
    stage.innerHTML = '';
    if (curScreen === 'world' || curScreen === 'map') {
      ensureMap();
      stage.appendChild(mapWrap);
      mapWrap.style.display = 'block';
      if (curScreen === 'world') stage.appendChild(buildWorldSidebar());
      else stage.appendChild(buildMapHud());
      map.showMinimap(curScreen === 'map');
    } else {
      if (mapWrap) mapWrap.style.display = 'none';
      if (map) map.showMinimap(false);
      const view = h('div', 'view');
      view.appendChild(buildScreen(curScreen));
      stage.appendChild(view);
    }
    renderInspector();
  }

  function ensureMap() {
    if (map) return;
    mapWrap = h('div', '');
    mapWrap.id = 'mapWrap';
    map = createMap(mapWrap, { onSelect: (hit) => { sel = { kind: hit.kind, id: hit.id }; openInspector(); } });
  }

  function buildMapHud() {
    const wrap = h('div', '');
    const hud = h('div', 'map-hud');
    const w = world();
    const row = h('div', 'hud-row');
    const fit = h('button', 'hud-btn', '⤢ Fit');
    fit.title = 'Zoom out to the whole world';
    fit.addEventListener('click', () => map.fit());
    const follow = h('button', 'hud-btn on', '◎ Follow');
    follow.id = 'followBtn';
    follow.title = 'Smart camera — tracks active agents, returns to world view when quiet';
    follow.addEventListener('click', () => {
      const v = !map.getFollow();
      map.setFollow(v);
      follow.classList.toggle('on', v);
      follow.textContent = v ? '◎ Follow' : '◌ Manual';
    });
    const filter = h('select', 'hud-select');
    filter.title = 'Filter the layer of civilization you want to watch';
    for (const [val, lbl] of FILTERS) {
      const o = h('option', '', lbl);
      o.value = val;
      filter.appendChild(o);
    }
    filter.addEventListener('change', () => map.setFilter(filter.value));
    row.appendChild(fit); row.appendChild(follow); row.appendChild(filter);
    hud.appendChild(row);
    const life = h('div', 'life-counter');
    life.innerHTML = `<div class="k">CIVILIZATION LIFE</div><div class="v" id="lifeVal">${fmt(map.getLife())}</div>`;
    life.title = 'A score of everything the world has built, discovered and earned';
    hud.appendChild(life);
    wrap.appendChild(hud);

    const legend = h('div', 'map-legend');
    const orgRows = w.orgOrder.map(oid => w.orgs[oid]).filter(o => o && o.territory && o.territory.length);
    const orgHtml = orgRows.length
      ? orgRows.map(o => `<div class="row"><span class="sw" style="background:${o.color};box-shadow:0 0 6px ${o.color}"></span> ${esc(o.name)} <span class="dim">${o.territory.length}</span></div>`).join('')
      : '';
    legend.innerHTML = `
      ${orgHtml ? `<div class="lg-sec">FACTIONS</div>${orgHtml}` : ''}
      <div class="lg-sec">DISTRICTS</div>
      ${DISTRICT_LEGEND.map(([l, c]) => `<div class="row"><span class="sw" style="background:${c}"></span> ${l}</div>`).join('')}
      <div class="lg-sec">GROWTH</div>
      <div class="row"><span class="sw" style="background:#5a6b4f"></span> Village</div>
      <div class="row"><span class="sw" style="background:#8a9a6a"></span> Town</div>
      <div class="row"><span class="sw" style="background:#c9b458"></span> City</div>
      <div class="row"><span class="sw" style="background:#e8d88f"></span> Fortress</div>
      <div class="lg-sec">SIGNS</div>
      <div class="row"><span class="sw sw-trail"></span> Trail</div>
      <div class="row"><span class="sw sw-front"></span> Rival front</div>
      <div class="row"><span class="sw" style="background:#6fd6ff"></span> Capital / City</div>
      <div class="row"><span class="sw" style="background:#ffd166"></span> Dispute / mission</div>
      <div class="row"><span class="sw" style="background:#c67bff"></span> Anomaly zone</div>
      <div class="row"><span class="sw" style="background:#2c3a55"></span> Unexplored</div>`;
    wrap.appendChild(legend);

    const status = h('div', 'map-status');
    status.id = 'mapStatusEl';
    status.innerHTML = `${w.map.landRegions.length} regions<br>${w.map.cities.length} cities · ${w.map.routes.length} routes<br>${(map.fronts || 0)} fronts · ${(map.disputes || 0)} disputes`;
    wrap.appendChild(status);

    const title = h('div', 'map-title');
    title.innerHTML = '<div class="t1">VESPER</div><div class="t2">Living map — watch civilization grow</div>';
    wrap.appendChild(title);
    return wrap;
  }

  function buildWorldSidebar() {
    const w = world();
    const side = h('div', 'world-side');
    side.appendChild(h('div', 'nav-bread', 'Live World'));
    const headline = h('div', 'headline');
    headline.innerHTML = '<div class="t1">Autonomous Civilization</div><div class="t2">' + tickFull(w.clock.t) + ' — ' + w.agentOrder.length + ' agents living in the world</div>';
    side.appendChild(headline);

    const sec = (title) => { const s = h('div', 'ws-sec'); s.appendChild(h('div', 'ws-title', title)); return s; };

    const sAgents = sec('Top Contributors');
    const topAgents = [...w.agentOrder].map(id => w.agents[id]).sort((a, b) => agentValue(b) - agentValue(a)).slice(0, 6);
    for (const a of topAgents) {
      const row = h('div', 'ws-row');
      const av = h('span', 'av', a.avatar); av.style.background = a.color;
      const name = h('span', 'nm');
      name.innerHTML = `<b>${esc(a.name)}</b> <span class="dim">${ARCH_LABEL[a.archetype]}</span>`;
      const meta = h('span', 'dim small');
      meta.textContent = `${regionName(w, regionOf(a))} · ${a.status || 'working'}`;
      const val = h('span', 'num');
      val.innerHTML = fmt(agentValue(a)) + ' <span class="faint">vp</span>';
      row.appendChild(av); row.appendChild(name); row.appendChild(h('div', 'f1')); row.appendChild(val);
      row.title = meta.textContent;
      row.addEventListener('click', () => { sel = { kind: 'agent', id: a.id }; openInspector(); });
      sAgents.appendChild(row);
      sAgents.appendChild(h('div', 'ws-sub', meta.textContent));
    }
    side.appendChild(sAgents);

    const sActs = sec('Live Activity');
    const actsFeed = h('div', 'feed');
    for (const ac of (w.activity || []).slice(-6).reverse()) {
      const r = h('div', 'act-row sm');
      const av = h('span', 'act-av', ac.avatar); av.style.background = ac.color;
      const who = h('span', 'act-who'); who.innerHTML = `<b>${esc(ac.name)}</b>`;
      r.appendChild(av); r.appendChild(who); r.appendChild(h('span', 'act-verb', ac.verb));
      const det = h('span', 'act-det'); det.textContent = (ac.detail || '').slice(0, 64);
      r.appendChild(det);
      r.addEventListener('click', () => { sel = { kind: 'agent', id: ac.agentId }; openInspector(); });
      actsFeed.appendChild(r);
    }
    sActs.appendChild(actsFeed);
    side.appendChild(sActs);

    const sOrgs = sec('Organizations');
    const orgs = [...w.orgOrder].map(id => w.orgs[id]).sort((a, b) => b.treasury.credits - a.treasury.credits).slice(0, 4);
    for (const o of orgs) {
      const row = h('div', 'ws-row');
      const av = h('span', 'av', '◉'); av.style.background = o.color || '#5cc8ff';
      row.appendChild(av);
      const nm = h('span', 'nm');
      nm.innerHTML = `<b>${esc(o.name)}</b> <span class="dim">${o.type} · ${o.members.length} members</span>`;
      row.appendChild(nm); row.appendChild(h('div', 'f1'));
      const val = h('span', 'num'); val.textContent = fmtMoney(o.treasury.credits);
      row.appendChild(val);
      row.addEventListener('click', () => { sel = { kind: 'org', id: o.id }; openInspector(); });
      sOrgs.appendChild(row);
    }
    side.appendChild(sOrgs);

    const sEco = sec('Economy');
    const eco = h('div', 'eco-grid');
    const ecoStats = [
      ['World Fund', fmtMoney((w.balances['world'] || {}).credits || 0)],
      ['Trades', w.stats.trades],
      ['Contracts Done', w.stats.completedContracts],
      ['Compute Jobs', w.compute.stats.execs],
      ['Discoveries', w.fact.discoveries.length],
      ['Buildings', w.stats.built],
    ];
    for (const [k, v] of ecoStats) {
      const c = h('div', 'eco-stat');
      c.appendChild(h('div', 'k', k));
      c.appendChild(h('div', 'v', String(v)));
      eco.appendChild(c);
    }
    sEco.appendChild(eco);
    side.appendChild(sEco);
    return side;
  }

  /* ================= screens ================= */
  function buildScreen(name) {
    const w = world();
    switch (name) {
      case 'agents': return screenAgents(w);
      case 'organizations': return screenOrgs(w);
      case 'missions': return screenMissions(w);
      case 'markets': return screenMarkets(w);
      case 'events': return screenEvents(w);
      case 'activity': return screenActivity(w);
      case 'compute': return screenCompute(w);
      case 'fabric': return screenFabric(w);
      case 'evidence': return screenEvidence(w);
      case 'economy': return screenEconomy(w);
      case 'replay': return screenReplay(w);
      case 'console': return screenConsole(w);
      case 'admin': return screenAdmin(w);
      default: return screenAgents(w);
    }
  }

  function screenHeader(title, sub, extra) {
    const head = h('div', 'screen-head');
    const l = h('div', '');
    l.appendChild(h('div', 'sub', sub));
    l.appendChild(h('h2', '', title));
    head.appendChild(l);
    if (extra) { head.appendChild(h('div', 'spacer')); head.appendChild(extra); }
    return head;
  }

  function screenAgents(w) {
    const view = h('div', 'screen');
    const search = h('input', 'search-input'); search.placeholder = 'Search agents…';
    view.appendChild(screenHeader('Agents', 'Every agent is a persistent player', search));
    const grid = h('div', 'grid cols-3');
    const list = [...w.agentOrder].map(id => w.agents[id]);
    const doFilter = () => {
      const q = search.value.toLowerCase();
      grid.innerHTML = '';
      for (const a of list) {
        if (q && !a.name.toLowerCase().includes(q) && !a.archetype.includes(q)) continue;
        grid.appendChild(agentCard(a));
      }
    };
    search.addEventListener('input', doFilter);
    doFilter();
    view.appendChild(grid);
    return view;
  }

  function agentCard(a) {
    const w = world();
    const c = h('div', 'card hover');
    c.innerHTML = '';
    const head = h('div', 'flex items-center gap10');
    const av = h('span', 'big-av', a.avatar); av.style.background = a.color; av.style.color = '#061018';
    const nm = h('div', '');
    nm.appendChild(h('div', 'card-name', a.name + (a.real ? ' · ' : '')));
    if (a.real) {
      const rbadge = h('span', 'tag', 'REAL');
      rbadge.style.color = '#8be0c8'; rbadge.style.borderColor = 'rgba(139,224,200,0.5)';
      nm.lastChild.appendChild(rbadge);
    }
    nm.appendChild(h('div', 'dim small', (a.role ? a.role + (ARCH_LABEL[a.archetype] !== a.role ? ' · ' + ARCH_LABEL[a.archetype] : '') : ARCH_LABEL[a.archetype]) + (a.nodeName ? ' · ' + a.nodeName : '')));
    head.appendChild(av); head.appendChild(nm);
    head.appendChild(h('div', 'f1'));
    head.appendChild(tag(a.status || 'working', statusTagClass(a.status)));
    c.appendChild(head);
    const loc = h('div', 'mt8 small dim');
    loc.textContent = `${regionName(w, regionOf(a))}${a.org ? ' · ' + w.orgs[a.org].name : ''}`;
    c.appendChild(loc);
    const obj = h('div', 'obj small');
    obj.textContent = a.planGoal || '—';
    c.appendChild(obj);
    const row = h('div', 'kv-row mt8');
    row.appendChild(h('span', 'k', 'Wealth'));
    row.appendChild(h('span', 'v', fmtMoney(a.credits || 0)));
    const row2 = h('div', 'kv-row');
    row2.appendChild(h('span', 'k', 'Reputation'));
    row2.appendChild(h('span', 'v', a.rep.score + ''));
    const row3 = h('div', 'kv-row');
    row3.appendChild(h('span', 'k', 'Rep'));
    row3.appendChild(h('span', 'v', Object.keys(a.rep).filter(k => k !== 'score').map(k => k + ':' + a.rep[k]).join(' · ')));
    const rowV = h('div', 'kv-row');
    rowV.appendChild(h('span', 'k', 'Value'));
    rowV.appendChild(h('span', 'v', fmt(agentValue(a)) + ' vp'));
    c.appendChild(row); c.appendChild(row2); c.appendChild(row3); c.appendChild(rowV);
    const sk = h('div', 'kv-row');
    sk.appendChild(h('span', 'k', 'Skills'));
    const skv = h('span', 'v small');
    skv.textContent = Object.entries(a.skills).sort((x, y) => y[1] - x[1]).slice(0, 3).map(([k, v]) => k + ' ' + v.toFixed(1)).join(' · ');
    sk.appendChild(skv);
    c.appendChild(sk);
    c.addEventListener('click', () => { sel = { kind: 'agent', id: a.id }; openInspector(); });
    return c;
  }

  function statusTagClass(s) {
    if (s === 'traveling') return 'cyan';
    if (s === 'researching' || s === 'building') return 'gold';
    if (s === 'trading') return 'green';
    if (s === 'resting') return 'dim';
    return 'violet';
  }

  function screenOrgs(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Organizations', 'Factions, guilds and corporations of the world'));
    const grid = h('div', 'grid cols-3');
    for (const id of w.orgOrder) {
      const o = w.orgs[id];
      if (!o) continue;
      const c = h('div', 'card hover');
      const head = h('div', 'flex items-center gap10');
      const av = h('span', 'big-av', '◉'); av.style.background = o.color || '#5cc8ff';
      const nm = h('div', '');
      nm.appendChild(h('div', 'card-name', o.name));
      nm.appendChild(h('div', 'dim small', o.type));
      head.appendChild(av); head.appendChild(nm); head.appendChild(h('div', 'f1'));
      head.appendChild(tag('Power ' + powerOf(w, 'org:' + o.id), 'violet'));
      c.appendChild(head);
      const kvr = h('div', 'mt8');
      kvr.appendChild(kv('Treasury', fmtMoney(o.treasury.credits)));
      kvr.appendChild(kv('Members', o.members.length));
      kvr.appendChild(kv('Territory', o.territory.length + ' regions'));
      kvr.appendChild(kv('Reputation', o.rep));
      kvr.appendChild(kv('Assets', `▲${o.assets.factories} ◉${o.assets.labs} ◍${o.assets.relays}`));
      c.appendChild(kvr);
      c.addEventListener('click', () => { sel = { kind: 'org', id: o.id }; openInspector(); });
      grid.appendChild(c);
    }
    view.appendChild(grid);
    return view;
  }

  function screenMissions(w) {
    const view = h('div', 'screen');
    const states = ['active', 'open', 'completed', 'failed', 'expired'];
    let state = 'active';
    const tabs = h('div', 'toolbar');
    const body = h('div', '');
    const render = () => {
      body.innerHTML = '';
      const cts = Object.values(w.contracts).filter(c => c.state === state).sort((a, b) => b.tick - a.tick);
      if (!cts.length) { body.appendChild(h('div', 'empty', 'No ' + state + ' contracts.')); return; }
      const tbl = h('table', 'tbl');
      const thead = h('thead');
      thead.innerHTML = '<tr><th>Contract</th><th>Kind</th><th>Issuer</th><th>Reward</th><th>Deadline</th><th>Progress</th><th></th></tr>';
      tbl.appendChild(thead);
      const tb = h('tbody');
      for (const c of cts) {
        const tr = h('tr');
        const t1 = h('td'); t1.innerHTML = `<b>${esc(c.title)}</b><br><span class="faint small">${c.id}</span>`;
        const t2 = h('td'); t2.textContent = c.objective.kind;
        const t3 = h('td'); t3.textContent = c.issuer === 'world' ? 'World' : (w.agents[c.issuer] ? w.agents[c.issuer].name : c.issuer);
        const t4 = h('td', 'num');
        const rewardParts = [];
        if (c.reward.credits) rewardParts.push(fmt(c.reward.credits) + ' Cr');
        if (c.reward.compute) rewardParts.push('◍' + c.reward.compute);
        if (c.reward.data) rewardParts.push('◉' + c.reward.data);
        t4.innerHTML = rewardParts.join('<br>');
        const t5 = h('td'); t5.textContent = fmtTick(c.deadlineTick) + (c.deadlineTick < w.clock.t ? ' <span class="bad">overdue</span>' : '');
        const t6 = h('td');
        const pb = h('div', 'bar warn'); pb.appendChild(h('i'));
        pb.firstChild.style.width = Math.min(100, (c.progress / Math.max(1, c.target)) * 100) + '%';
        t6.appendChild(pb);
        t6.appendChild(h('div', 'faint small', Math.round(c.progress) + ' / ' + c.target));
        const t7 = h('td');
        const btn = h('button', 'btn sm ghost', 'View');
        btn.addEventListener('click', () => { sel = { kind: 'contract', id: c.id }; openInspector(); });
        t7.appendChild(btn);
        tr.appendChild(t1); tr.appendChild(t2); tr.appendChild(t3); tr.appendChild(t4); tr.appendChild(t5); tr.appendChild(t6); tr.appendChild(t7);
        tb.appendChild(tr);
      }
      tbl.appendChild(tb);
      body.appendChild(tbl);
    };
    for (const s of states) {
      const b = h('button', 'hud-btn' + (state === s ? ' on' : ''), s);
      b.addEventListener('click', () => { state = s; tabs.querySelectorAll('.hud-btn').forEach(x => x.classList.remove('on')); b.classList.add('on'); render(); });
      tabs.appendChild(b);
    }
    view.appendChild(screenHeader('Missions', 'Dynamic contracts issued by the world, markets and agents', tabs));
    view.appendChild(body);
    render();
    return view;
  }

  function screenMarkets(w) {
    const view = h('div', 'screen');
    let cityId = w.marketOrder[0];
    let res = 'food';
    const toolbar = h('div', 'toolbar');
    const selCity = h('select', 'select');
    for (const mid of w.marketOrder) {
      const c = w.map.cities.find(x => x.id === mid);
      const o = h('option', '', c ? c.name : mid);
      o.value = mid;
      selCity.appendChild(o);
    }
    const selRes = h('select', 'select');
    for (const r of MKT_RES) {
      const o = h('option', '', RES_LABEL[r]);
      o.value = r;
      selRes.appendChild(o);
    }
    toolbar.appendChild(selCity); toolbar.appendChild(selRes);
    view.appendChild(screenHeader('Markets', 'Live prices respond to real supply and demand', toolbar));
    const body = h('div', '');
    view.appendChild(body);
    const render = () => {
      body.innerHTML = '';
      const m = w.markets[cityId];
      if (!m) return;
      const city = w.map.cities.find(x => x.id === cityId);
      const panel = h('div', 'panel p16');
      panel.appendChild(h('h4', '', city.name + ' Market'));
      const grid = h('div', 'grid cols-3');
      const priceCard = h('div', 'card');
      priceCard.appendChild(h('h4', '', 'Prices'));
      for (const r of MKT_RES) {
        const rw = h('div', 'kv-row');
        rw.appendChild(h('span', 'k', RES_LABEL[r]));
        const v = h('span', 'v');
        v.textContent = m.prices[r].toFixed(1) + ' Cr';
        const base = { food: 3.5, energy: 6, materials: 9, rare: 60, data: 15 }[r];
        v.style.color = m.prices[r] > base * 1.15 ? 'var(--bad)' : m.prices[r] < base * 0.85 ? 'var(--good)' : 'var(--text)';
        rw.appendChild(v);
        priceCard.appendChild(rw);
      }
      grid.appendChild(priceCard);
      const supCard = h('div', 'card');
      supCard.appendChild(h('h4', '', 'Supply / Demand'));
      for (const r of MKT_RES) {
        const rw = h('div', 'kv-row');
        rw.appendChild(h('span', 'k', RES_LABEL[r]));
        const v = h('span', 'v');
        v.textContent = Math.round(m.supply[r]) + ' / ' + Math.round(m.demand[r]);
        rw.appendChild(v);
        supCard.appendChild(rw);
      }
      grid.appendChild(supCard);
      const flowCard = h('div', 'card');
      flowCard.appendChild(h('h4', '', 'Flow'));
      flowCard.appendChild(kv('Credits reserve', fmtMoney(m.credits)));
      flowCard.appendChild(kv('Price index', m.priceIdx.toFixed(2)));
      const vols = h('div', 'mt8');
      for (const r of MKT_RES) {
        const rw = h('div', 'kv-row');
        rw.appendChild(h('span', 'k', RES_LABEL[r] + ' traded'));
        rw.appendChild(h('span', 'v', (m.buyVol[r] + m.sellVol[r]).toFixed(0)));
        vols.appendChild(rw);
      }
      flowCard.appendChild(vols);
      grid.appendChild(flowCard);
      panel.appendChild(grid);
      const chartCard = h('div', 'card mt12');
      chartCard.appendChild(h('h4', '', RES_LABEL[res] + ' price history'));
      chartCard.appendChild(sparkSvg(m.history[res] || [], 900, 180, '#5cc8ff'));
      panel.appendChild(chartCard);
      body.appendChild(panel);
    };
    selCity.addEventListener('change', () => { cityId = selCity.value; render(); });
    selRes.addEventListener('change', () => { res = selRes.value; render(); });
    render();
    return view;
  }

  function sparkSvg(points, w2, h2, color) {
    if (!points.length) { const e = h('div', 'empty', 'No history yet.'); return e; }
    const min = Math.min(...points), max = Math.max(...points);
    const span = (max - min) || 1;
    const pad = 6;
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('viewBox', `0 0 ${w2} ${h2}`);
    svg.setAttribute('class', 'chart');
    svg.style.width = '100%'; svg.style.height = '140px';
    const step = (w2 - pad * 2) / Math.max(1, points.length - 1);
    let d = '';
    points.forEach((v, i) => {
      const x = pad + i * step;
      const y = h2 - pad - ((v - min) / span) * (h2 - pad * 2);
      d += (i === 0 ? 'M' : 'L') + x.toFixed(1) + ',' + y.toFixed(1);
    });
    const poly = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    poly.setAttribute('d', d);
    poly.setAttribute('fill', 'none');
    poly.setAttribute('stroke', color);
    poly.setAttribute('stroke-width', 1.5);
    const area = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    area.setAttribute('d', d + `L${w2 - pad},${h2 - pad}L${pad},${h2 - pad}Z`);
    area.setAttribute('fill', color);
    area.setAttribute('opacity', 0.08);
    svg.appendChild(area); svg.appendChild(poly);
    const last = document.createElementNS('http://www.w3.org/2000/svg', 'text');
    last.setAttribute('x', w2 - pad); last.setAttribute('y', pad + 8);
    last.setAttribute('text-anchor', 'end'); last.setAttribute('class', 'axis-label');
    last.textContent = points[points.length - 1].toFixed(1) + ' Cr';
    svg.appendChild(last);
    return svg;
  }

  function screenEvents(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Events', 'Everything that happens is recorded — nothing is fabricated'));
    const feed = h('div', 'feed');
    const evs = [...w.events].reverse();
    for (const e of evs) {
      const r = h('div', 'feed-row');
      r.appendChild(h('span', 'ft', fmtTick(e.tick)));
      r.appendChild(tag(...(EVENT_TAG[e.type] || ['Event', 'dim'])));
      const msg = h('span', 'msg');
      msg.textContent = e.detail || '';
      r.appendChild(msg);
      feed.appendChild(r);
    }
    view.appendChild(feed);
    return view;
  }

  function screenActivity(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Live Activity', 'Real-time — exactly what agents are doing right now, straight from the simulation'));
    const acts = w.activity || [];
    const recent = acts.slice(-80);
    const byKind = {};
    for (const ac of recent) if (ac.kind) byKind[ac.kind] = (byKind[ac.kind] || 0) + 1;
    const sum = h('div', 'act-sum-grid');
    for (const k of Object.keys(ACT_KIND)) {
      const c = h('div', 'act-sum-cell');
      c.innerHTML = `<div class="n">${byKind[k] || 0}</div><div class="l">${ACT_KIND[k][0]}</div>`;
      sum.appendChild(c);
    }
    view.appendChild(sum);
    const chips = h('div', 'toolbar');
    let filter = 'all';
    const feed = h('div', 'feed act-feed');
    const render = () => {
      feed.innerHTML = '';
      let shown = 0;
      for (let i = acts.length - 1; i >= 0; i--) {
        const ac = acts[i];
        if (filter !== 'all' && ac.kind !== filter) continue;
        if (shown >= 120) break;
        shown++;
        const r = h('div', 'act-row');
        r.appendChild(h('span', 'act-t', fmtTick(ac.t)));
        const av = h('span', 'act-av', ac.avatar); av.style.background = ac.color;
        const who = h('span', 'act-who'); who.innerHTML = `<b>${esc(ac.name)}</b>`;
        const verb = h('span', 'act-verb', ac.verb);
        r.appendChild(av); r.appendChild(who); r.appendChild(verb);
        if (ac.value != null) r.appendChild(h('span', 'act-v', '+' + fmt(ac.value)));
        r.appendChild(h('div', 'f1'));
        const det = h('span', 'act-det'); det.textContent = (ac.detail || '').slice(0, 110);
        r.appendChild(det);
        r.addEventListener('click', () => { sel = { kind: 'agent', id: ac.agentId }; openInspector(); });
        feed.appendChild(r);
      }
      if (!shown) feed.appendChild(h('div', 'empty', filter === 'all' ? 'No activity yet — agents are still waking up.' : 'No ' + ACT_KIND[filter][0] + ' activity yet.'));
    };
    const mkChip = (k, lbl) => {
      const b = h('button', 'chip' + (k === filter ? ' on' : ''), lbl);
      b.addEventListener('click', () => {
        filter = k;
        for (const c of chips.children) c.className = 'chip' + (c === b ? ' on' : '');
        render();
      });
      chips.appendChild(b);
    };
    mkChip('all', 'All');
    for (const k of Object.keys(ACT_KIND)) mkChip(k, ACT_KIND[k][0]);
    view.appendChild(chips);
    view.appendChild(feed);
    render();
    return view;
  }

  function screenCompute(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Compute Fabric', 'Agents spend compute credits for real, measured work'));
    const strip = h('div', 'stat-strip');
    const stats = [
      ['Pool size', w.compute.poolSize],
      ['Throughput', w.compute.throughput.toFixed(1)],
      ['Executions', w.compute.stats.execs],
      ['Failed', w.compute.stats.failed],
      ['Credits paid', w.compute.stats.creditsPaid],
      ['Pool balance', (w.balances['compute-pool'] || {}).compute || 0],
      ['Contributors', w.compute.contributors.length],
    ];
    for (const [k, v] of stats) {
      const s = h('div', 'stat');
      s.appendChild(h('div', 'k', k));
      s.appendChild(h('div', 'v', String(v)));
      strip.appendChild(s);
    }
    view.appendChild(strip);
    const grid = h('div', 'grid cols-2');
    const jobCard = h('div', 'card');
    jobCard.appendChild(h('h4', '', 'Jobs'));
    const jobs = [...w.compute.jobOrder].reverse().map(id => w.compute.jobs[id]).slice(0, 30);
    if (!jobs.length) jobCard.appendChild(h('div', 'empty', 'No compute jobs yet.'));
    for (const j of jobs) {
      jobCard.appendChild(kv(j.executionId + ' · ' + j.taskType, `${j.requester} · ${j.status} · ◍${j.budget}`));
    }
    grid.appendChild(jobCard);
    const conCard = h('div', 'card');
    conCard.appendChild(h('h4', '', 'Contributors'));
    if (!w.compute.contributors.length) conCard.appendChild(h('div', 'empty', 'No agents contributing compute yet.'));
    for (const cid of w.compute.contributors) {
      const a = w.agents[cid];
      if (a) conCard.appendChild(kv(a.name, 'contributing · earned ◍' + Math.round(a.computeTrack.earned || 0)));
    }
    grid.appendChild(conCard);
    view.appendChild(grid);
    const taskCard = h('div', 'card mt12');
    taskCard.appendChild(h('h4', '', 'Task capabilities'));
    const capTbl = h('table', 'tbl');
    capTbl.innerHTML = '<thead><tr><th>Task</th><th>Capability</th><th>Budget</th></tr></thead>';
    const tb = h('tbody');
    const caps = [['routeplan', 'CPU'], ['forecast', 'CPU'], ['analyze', 'CPU'], ['simulate', 'CPU'], ['threatscan', 'CPU'], ['datamine', 'CPU'], ['optimize', 'CPU'], ['researchsim', 'CPU'], ['intel', 'Model'], ['text', 'Model']];
    const budgets = { routeplan: 20, forecast: 12, analyze: 25, simulate: 30, threatscan: 8, datamine: 18, optimize: 22, researchsim: 25, intel: 28, text: 10 };
    for (const [t, c] of caps) {
      const tr = h('tr');
      tr.innerHTML = `<td><b>${t}</b></td><td>${c}</td><td class="num">◍${budgets[t]}</td>`;
      tb.appendChild(tr);
    }
    capTbl.appendChild(tb);
    taskCard.appendChild(capTbl);
    view.appendChild(taskCard);
    return view;
  }

  function screenFabric(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Fabric', 'Live panels over the compute fabric — Governor · Model Colony · CPU Pool · Evidence · Economy'));
    const fs = statusOf(w);
    const hero = h('div', 'fb-hero');
    const stateCls = fs.state === 'connected' ? 'connected' : fs.state === 'same-origin' ? 'connected' : fs.state === 'unreachable' ? 'unreachable' : fs.state === 'probing' ? 'probing' : 'unconfigured';
    const st = h('span', 'fb-state ' + stateCls);
    st.textContent = fs.state === 'connected' ? '● Connected' : fs.state === 'same-origin' ? '● Connected (same-origin)' : fs.state === 'unreachable' ? '● Unreachable' : fs.state === 'probing' ? '◌ Probing…' : fs.state === 'disabled' ? '○ Disabled' : '○ Local only';
    hero.appendChild(st);
    hero.appendChild(h('span', 'fb-url', fs.baseUrl ? 'fabric: ' + fs.baseUrl : 'fabric: local — deterministic sim + real Web-Worker compute'));
    hero.appendChild(h('div', 'f1'));
    hero.appendChild(h('span', 'fb-url', 'calls ' + fs.calls + ' · ok ' + fs.ok + ' · fail ' + fs.fail + (fs.latencyMs != null ? ' · probe ' + fs.latencyMs + 'ms' : '')));
    const probeBtn = h('button', 'btn sm', 'Probe');
    probeBtn.addEventListener('click', () => deps.probeFabric());
    hero.appendChild(probeBtn);
    view.appendChild(hero);
    if (fs.note) view.appendChild(h('div', 'dim small mb12', fs.note));

    const jobs = [...w.compute.jobOrder].slice(-80).map(id => w.compute.jobs[id]).filter(Boolean);
    const done = jobs.filter(j => j.status === 'done');
    const modelJobs = jobs.filter(j => j.capability === 'model' && j.status === 'done');
    const lastDone = done[done.length - 1];
    const short = window.innerWidth < 640;
    const taskShort = { routeplan: 'ROUTE', forecast: 'CAST', analyze: 'ANLZ', simulate: 'SIM', threatscan: 'SCAN', datamine: 'MINE', optimize: 'OPT', researchsim: 'RSIM', hash: 'HASH', intel: 'INTEL', text: 'TEXT' };

    const pipeline = h('div', 'pipeline');
    const nodes = [
      [short ? 'GOV' : 'Governor', lastDone ? (taskShort[lastDone.taskType] || lastDone.taskType.toUpperCase().slice(0, 5)) : 'IDLE'],
      [short ? 'MODEL' : 'Model Colony', modelJobs.length ? modelJobs.length + ' jobs' : 'IDLE'],
      [short ? 'CPU' : 'CPU Pool', w.compute.stats.execs + ' execs'],
      [short ? 'EVID' : 'Evidence', w.evidence.count + ' recs'],
      [short ? 'ECON' : 'Economy', fmt((w.balances['compute-pool'] || {}).compute || 0)],
    ];
    if (short) {
      const pills = h('div', 'pipe-pills');
      for (const [label, val] of nodes) {
        const p = h('div', 'pipe-pill');
        p.innerHTML = `<span>${label}</span><b>${val}</b>`;
        pills.appendChild(p);
      }
      view.appendChild(pills);
    } else {
      nodes.forEach(([label, val], i) => {
        const n = h('div', 'pipe-node');
        n.innerHTML = `<span>${label}</span><b>${val}</b>`;
        pipeline.appendChild(n);
        if (i < nodes.length - 1) {
          const l = h('div', 'pipe-link');
          l.appendChild(h('i'));
          pipeline.appendChild(l);
        }
      });
      view.appendChild(pipeline);
    }

    const grid = h('div', 'grid cols-2');
    const gov = h('div', 'card');
    gov.appendChild(h('h4', '', 'Governor — decisions'));
    gov.appendChild(h('div', 'dim small mb8', 'How compute is allocated. Local fabric is deterministic; DecentraAI responses land in the call log below.'));
    const gDecs = done.slice(-8).reverse();
    if (!gDecs.length) gov.appendChild(h('div', 'empty', 'No compute decisions yet.'));
    for (const j of gDecs) gov.appendChild(kv(j.executionId, j.taskType + ' · ' + j.requester + ' · ' + j.status + ' · ◍' + j.budget));
    grid.appendChild(gov);

    const mc = h('div', 'card');
    mc.appendChild(h('h4', '', 'Model Colony'));
    const narr = (w.meta && w.meta.narrativeEngine) || 'local';
    mc.appendChild(kv('Text engine', narr === 'ai' ? 'model provider' : 'local (deterministic)'));
    mc.appendChild(kv('Model jobs run', modelJobs.length));
    mc.appendChild(kv('Model budget spent', '◍' + modelJobs.reduce((s, j) => s + j.budget, 0)));
    for (const j of modelJobs.slice(-5).reverse()) {
      mc.appendChild(kv(j.executionId, j.taskType + ' · ' + j.requester + ' · ' + (j.result && j.result.ok ? 'ok' : (j.result && j.result.err) || '…')));
    }
    grid.appendChild(mc);

    const cp = h('div', 'card');
    cp.appendChild(h('h4', '', 'CPU Pool — per node'));
    const queued = jobs.filter(j => j.status === 'queued').length;
    for (const [k, v] of [['Nodes', w.compute.poolSize], ['Throughput', w.compute.throughput.toFixed(1)], ['Executions', w.compute.stats.execs], ['Failed', w.compute.stats.failed], ['Running', queued], ['Contributors', w.compute.contributors.length]]) {
      cp.appendChild(kv(k, v));
    }
    for (const cid of w.compute.contributors.slice(0, 6)) {
      const ca = w.agents[cid];
      if (ca) cp.appendChild(kv(ca.name, 'contributing · ◍' + Math.round(ca.computeTrack.earned || 0)));
    }
    grid.appendChild(cp);

    const ev = h('div', 'card');
    ev.appendChild(h('h4', '', 'Evidence'));
    ev.appendChild(kv('Chain records', w.evidence.count));
    ev.appendChild(kv('Held', w.evidence.records.length));
    ev.appendChild(kv('Chain head', w.evidence.chainHead.slice(0, 16) + '…'));
    ev.appendChild(kv('Fabric log', fs.calls + ' calls (' + fs.ok + ' ok / ' + fs.fail + ' fail)'));
    grid.appendChild(ev);

    const eco = h('div', 'card');
    eco.appendChild(h('h4', '', 'Economy'));
    eco.appendChild(kv('World fund', fmtMoney((w.balances['world'] || {}).credits || 0)));
    eco.appendChild(kv('Compute pool', '◍' + ((w.balances['compute-pool'] || {}).compute || 0)));
    eco.appendChild(kv('Ledger entries', w.ledger.count));
    // Real fabric balances mirrored server-side (authoritative quota ledger).
    const re = fs.realEconomy;
    if (re) {
      eco.appendChild(kv('REAL quota total', fmt(re.total_spendable || 0) + ' units'));
      const realAgents = Object.entries(re.agents || {}).slice(0, 5);
      for (const [aid, b] of realAgents) {
        eco.appendChild(kv('◉ ' + aid.split(':').pop() + ' (' + aid.split(':')[0].slice(0, 12) + ')', fmt(b.spendable || 0) + ' units'));
      }
    }
    const topW = [...w.agentOrder].map(id => w.agents[id]).sort((x, y) => (y.compute || 0) - (x.compute || 0)).slice(0, 5);
    for (const a of topW) eco.appendChild(kv(a.name, '◍' + Math.round(a.compute || 0)));
    grid.appendChild(eco);
    view.appendChild(grid);

    const logCard = h('div', 'card mt12');
    logCard.appendChild(h('h4', '', 'DecentraAI call log — real network traffic'));
    if (!fs.baseUrl) {
      logCard.appendChild(h('div', 'empty', 'No fabric endpoint configured. Set baseUrl to dispatch real workload.'));
    } else if (!(w.fabric && w.fabric.log.length)) {
      logCard.appendChild(h('div', 'empty', 'No calls yet — jobs dispatch here as they complete.'));
    } else {
      const rows = [...w.fabric.log].reverse().slice(0, 40);
      for (const e of rows) {
        const r = h('div', 'fb-log-row');
        r.appendChild(h('span', 't', fmtTick(e.tick)));
        r.appendChild(h('span', 'op', e.op));
        r.appendChild(h('span', 'who', (e.agentKey || '—') + (e.task ? ' · ' + e.task : '')));
        const st2 = h('span', 'st ' + e.status);
        st2.textContent = e.status;
        r.appendChild(st2);
        r.appendChild(h('span', 'det', e.detail || e.executionId || ''));
        r.appendChild(h('span', 'ms', e.ms != null ? e.ms + 'ms' : ''));
        logCard.appendChild(r);
      }
    }
    view.appendChild(logCard);
    return view;
  }

  function screenEvidence(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Evidence', 'Every autonomous action is hashed into a verifiable chain'));
    const strip = h('div', 'stat-strip');
    const s1 = h('div', 'stat'); s1.appendChild(h('div', 'k', 'Chain records')); s1.appendChild(h('div', 'v', String(w.evidence.count))); strip.appendChild(s1);
    const s2 = h('div', 'stat'); s2.appendChild(h('div', 'k', 'Held records')); s2.appendChild(h('div', 'v', String(w.evidence.records.length))); strip.appendChild(s2);
    const s3 = h('div', 'stat'); s3.appendChild(h('div', 'k', 'Chain head')); s3.appendChild(h('div', 'v small', w.evidence.chainHead.slice(0, 16) + '…')); strip.appendChild(s3);
    view.appendChild(strip);
    const card = h('div', 'card');
    card.appendChild(h('h4', '', 'Recent chain records'));
    const recs = [...w.evidence.records].reverse().slice(0, 40);
    for (const r of recs) {
      const row = h('div', 'kv-row');
      const k = h('span', 'k');
      k.textContent = r.evId + ' · ' + (r.kind || 'action');
      const v = h('span', 'v small');
      v.textContent = (r.action || r.taskType || r.result || '') + ' · ' + (r.agent || '') + ' · ' + r.chainHash.slice(0, 10);
      row.appendChild(k); row.appendChild(v);
      row.addEventListener('click', () => openEvidenceDetail(r));
      row.style.cursor = 'pointer';
      card.appendChild(row);
    }
    view.appendChild(card);
    return view;
  }

  function openEvidenceDetail(rec) {
    openModal((box) => {
      box.appendChild(h('h3', '', 'Evidence ' + rec.evId));
      box.appendChild(h('div', 'nav-bread', 'Provenance record'));
      const rows = h('div', '');
      for (const [k, v] of Object.entries(rec)) {
        rows.appendChild(kv(k, typeof v === 'object' ? JSON.stringify(v) : String(v)));
      }
      box.appendChild(rows);
    });
  }

  function screenEconomy(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Economy', 'Every credit is accounted for — no arbitrary rewards'));
    const strip = h('div', 'stat-strip');
    const fund = w.balances['world'] || {};
    const econ = [
      ['World fund', fmtMoney(fund.credits || 0)],
      ['Ledger entries', w.ledger.count],
      ['Trades', w.stats.trades],
      ['Contracts paid', w.stats.completedContracts],
      ['Tech level', Object.values(w.fact.tech).reduce((s, t) => s + t.level, 0)],
    ];
    for (const [k, v] of econ) {
      const s = h('div', 'stat');
      s.appendChild(h('div', 'k', k));
      s.appendChild(h('div', 'v', String(v)));
      strip.appendChild(s);
    }
    view.appendChild(strip);
    const grid = h('div', 'grid cols-2');
    const techCard = h('div', 'card');
    techCard.appendChild(h('h4', '', 'Research'));
    if (!Object.keys(w.fact.tech).length) techCard.appendChild(h('div', 'empty', 'No research yet.'));
    for (const [k, t] of Object.entries(w.fact.tech)) {
      techCard.appendChild(kv(t.name + ' Lv.' + t.level, Math.round(t.progress) + ' / ' + t.target));
    }
    grid.appendChild(techCard);
    const priceCard = h('div', 'card');
    priceCard.appendChild(h('h4', '', 'Average market prices'));
    const avgs = {};
    for (const mid of w.marketOrder) {
      const m = w.markets[mid];
      for (const r of MKT_RES) {
        avgs[r] = (avgs[r] || 0) + m.prices[r];
      }
    }
    for (const r of MKT_RES) {
      priceCard.appendChild(kv(RES_LABEL[r], (avgs[r] / w.marketOrder.length).toFixed(1) + ' Cr'));
    }
    grid.appendChild(priceCard);
    view.appendChild(grid);
    const ledCard = h('div', 'card mt12');
    ledCard.appendChild(h('h4', '', 'Ledger'));
    const txs = [...w.ledger.txs].reverse().slice(0, 60);
    for (const t of txs) {
      ledCard.appendChild(kv(fmtTick(t.tick) + ' · ' + t.reason, `${t.from} → ${t.to} · ${RES_SYM[t.res] || ''}${t.amount}`));
    }
    view.appendChild(ledCard);
    return view;
  }

  function screenReplay(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Replay', 'Built from the real event log — what actually happened'));
    const card = h('div', 'card');
    card.appendChild(h('h4', '', 'World timeline'));
    const tl = h('div', 'timeline');
    const rail = h('div', 'tl-rail');
    const prog = h('div', 'tl-progress');
    rail.appendChild(prog);
    const marks = h('div', 'tl-events');
    const maxT = Math.max(1, w.clock.t);
    const minT = w.events.length ? Math.min(...w.events.map(e => e.tick)) : 0;
    const span = Math.max(1, maxT - minT);
    for (const e of w.events) {
      const pct = ((e.tick - minT) / span) * 100;
      const m = h('i');
      m.style.left = pct + '%';
      m.title = e.tick + ': ' + (e.detail || '').slice(0, 60);
      marks.appendChild(m);
    }
    rail.appendChild(marks);
    const mark = h('div', 'tl-mark');
    rail.appendChild(mark);
    tl.appendChild(rail);
    card.appendChild(tl);
    card.appendChild(h('div', 'dim small mt4', 'Retained window: last ' + w.events.length + ' events (' + fmtTick(minT) + ' → ' + fmtTick(maxT) + '). Older history is summarized, not fabricated.'));
    let pos = maxT;
    const feed = h('div', 'feed mt12');
    card.appendChild(feed);
    const render = () => {
      const pct = ((pos - minT) / span) * 100;
      prog.style.width = pct + '%';
      mark.style.left = `calc(${pct}% - 1px)`;
      feed.innerHTML = '';
      const show = w.events.filter(e => e.tick <= pos).slice(-40).reverse();
      for (const e of show) {
        const r = h('div', 'feed-row');
        r.appendChild(h('span', 'ft', fmtTick(e.tick)));
        r.appendChild(tag(...(EVENT_TAG[e.type] || ['Event', 'dim'])));
        const msg = h('span', 'msg');
        msg.textContent = e.detail || '';
        r.appendChild(msg);
        feed.appendChild(r);
      }
    };
    rail.addEventListener('click', (e) => {
      const rect = rail.getBoundingClientRect();
      pos = Math.min(maxT, Math.max(minT, Math.round(minT + ((e.clientX - rect.left) / rect.width) * span)));
      render();
    });
    const btnRow = h('div', 'toolbar mt12');
    const play = h('button', 'btn', '▶ Play');
    let playing = null;
    play.addEventListener('click', () => {
      if (playing) { clearInterval(playing); playing = null; play.textContent = '▶ Play'; return; }
      if (pos >= maxT) pos = minT;
      playing = setInterval(() => {
        pos += Math.max(1, Math.round(span / 300));
        if (pos >= maxT) { pos = maxT; clearInterval(playing); playing = null; play.textContent = '▶ Play'; }
        render();
      }, 30);
      play.textContent = '❚❚ Pause';
    });
    btnRow.appendChild(play);
    const latest = h('button', 'btn ghost', 'Latest');
    latest.addEventListener('click', () => { pos = maxT; render(); });
    btnRow.appendChild(latest);
    card.appendChild(btnRow);
    view.appendChild(card);
    render();
    return view;
  }

  function screenConsole(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Agent API', 'Machine-readable interface — external agents connect through window.Vesper'));
    const card = h('div', 'card');
    card.appendChild(h('h4', '', 'Evaluate'));
    const input = h('input', 'search-input');
    input.style.width = '100%';
    input.placeholder = 'Vesper.get_agent_state("a3")  — or any JS using Vesper / world';
    input.value = 'Vesper.get_world_state()';
    const run = h('button', 'btn mt8', 'Run');
    const out = h('div', 'console-out');
    out.innerHTML = '<div class="faint small">// results appear here as JSON</div>';
    card.appendChild(input); card.appendChild(h('div', 'flex gap6 mt8'));
    const rowBtns = h('div', 'flex gap6 mt8');
    const quick = [
      ['get_world_state', 'Vesper.get_world_state()'],
      ['list_agents', 'Vesper.list_agents()'],
      ['list_orgs', 'Vesper.list_orgs()'],
      ['inspect_compute', 'Vesper.inspect_compute()'],
      ['inspect_evidence', 'Vesper.inspect_evidence(20)'],
      ['discover_world', 'Vesper.discover_world()'],
      ['fabric_status', 'Vesper.fabric_status()'],
      ['fabric_governor', 'Vesper.fabric_governor_execute({agentId: ' + JSON.stringify(w.agentOrder[0] || 'a1') + ', taskType: "analyze", params: {}}).then(r => JSON.stringify(r))'],
      ['fabric_balance', 'Vesper.fabric_credits_balance({agentId: ' + JSON.stringify(w.agentOrder[0] || 'a1') + '}).then(r => JSON.stringify(r))'],
      ['fabric_mcp', 'Vesper.fabric_mcp({agentId: ' + JSON.stringify(w.agentOrder[0] || 'a1') + ', tool: "inspect_evidence", args: {limit: 5}}).then(r => JSON.stringify(r))'],
    ];
    for (const [lbl, code] of quick) {
      const b = h('button', 'btn sm ghost', lbl);
      b.addEventListener('click', () => { input.value = code; runIt(); });
      rowBtns.appendChild(b);
    }
    card.appendChild(rowBtns);
    card.appendChild(run);
    card.appendChild(out);
    const runIt = () => {
      const code = input.value.trim();
      if (!code) return;
      let result;
      try {
        const fn = new Function('Vesper', 'world', 'regionName', 'tickLabel', 'return (' + code + ')');
        result = fn(deps.getVesper(), w, regionName, tickLabel);
        if (result && typeof result.then === 'function') {
          out.innerHTML = '<span class="dim">Promise — awaiting…</span> <span class="spin"></span>';
          result.then((v) => { out.textContent = JSON.stringify(v, null, 2); out.classList.add('json'); }).catch((e) => { out.textContent = 'ERROR: ' + e.message; out.classList.add('err-out'); });
          return;
        }
        out.textContent = JSON.stringify(result, null, 2);
        out.classList.add('json');
      } catch (e) {
        out.textContent = 'ERROR: ' + (e && e.message || e);
        out.classList.add('err-out');
      }
    };
    run.addEventListener('click', runIt);
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') runIt(); });
    view.appendChild(card);
    return view;
  }

  function screenAdmin(w) {
    const view = h('div', 'screen');
    view.appendChild(screenHeader('Admin', 'World control — changes are validated and recorded'));
    const grid = h('div', 'grid cols-2');
    const metaCard = h('div', 'card');
    metaCard.appendChild(h('h4', '', 'World'));
    metaCard.appendChild(kv('Seed', w.meta.seed));
    metaCard.appendChild(kv('World ID', w.meta.id));
    metaCard.appendChild(kv('Tick', w.clock.t));
    metaCard.appendChild(kv('Time', tickFull(w.clock.t)));
    metaCard.appendChild(kv('Version', w.meta.version));
    grid.appendChild(metaCard);
    const ctlCard = h('div', 'card');
    ctlCard.appendChild(h('h4', '', 'Control'));
    const advanceRow = h('div', 'flex gap6');
    const nInput = h('input');
    nInput.type = 'number'; nInput.value = '24'; nInput.style.width = '80px';
    const adv = h('button', 'btn', 'Advance ticks');
    adv.addEventListener('click', () => { deps.advance(parseInt(nInput.value) || 1); });
    advanceRow.appendChild(nInput); advanceRow.appendChild(adv);
    ctlCard.appendChild(advanceRow);
    const saveBtn = h('button', 'btn ghost mt8', 'Save now');
    saveBtn.addEventListener('click', () => deps.saveNow());
    ctlCard.appendChild(saveBtn);
    const resetBtn = h('button', 'btn danger mt8', 'Reset world');
    resetBtn.addEventListener('click', () => {
      openModal((box) => {
        box.appendChild(h('h3', '', 'Reset the world?'));
        box.appendChild(h('p', 'dim small', 'This irreversibly replaces the current world with a fresh one. All agent history is lost.'));
        const seedInput = h('input');
        seedInput.value = 'vesper-' + Math.random().toString(36).slice(2, 10);
        box.appendChild(h('label', 'fld mt12', '')).appendChild(h('span', 'lbl', 'New seed')).appendChild(seedInput);
        const row = h('div', 'flex gap6 mt16');
        const go = h('button', 'btn danger', 'Reset');
        go.addEventListener('click', () => { deps.resetWorld(seedInput.value); closeModal(); });
        const cancel = h('button', 'btn ghost', 'Cancel');
        cancel.addEventListener('click', closeModal);
        row.appendChild(go); row.appendChild(cancel);
        box.appendChild(row);
      });
    });
    ctlCard.appendChild(resetBtn);
    grid.appendChild(ctlCard);
    view.appendChild(grid);
    const fbCard = h('div', 'card mt12');
    fbCard.appendChild(h('h4', '', 'DecentraAI fabric bridge'));
    const fs = statusOf(w);
    fbCard.appendChild(kv('State', fs.state));
    fbCard.appendChild(kv('Endpoint', fs.baseUrl || 'local-only (honest)'));
    fbCard.appendChild(kv('Calls', fs.calls + ' (' + fs.ok + ' ok / ' + fs.fail + ' fail)'));
    fbCard.appendChild(kv('Last probe', fs.latencyMs != null ? fs.latencyMs + 'ms' : '—'));
    if (fs.note) fbCard.appendChild(h('div', 'dim small mt8', fs.note));
    const probeBtn = h('button', 'btn sm mt8', 'Probe fabric');
    probeBtn.addEventListener('click', () => deps.probeFabric());
    fbCard.appendChild(probeBtn);
    view.appendChild(fbCard);
    return view;
  }

  /* ================= inspector ================= */
  function openInspector() {
    if (!sel) { inspector.classList.remove('open'); return; }
    inspector.classList.add('open');
    renderInspector();
    if (map && sel.kind === 'agent') map.focus(sel.id);
  }

  function closeInspector() { inspector.classList.remove('open'); sel = null; }

  function renderInspector() {
    if (!sel) return;
    inspector.innerHTML = '';
    const w = world();
    const head = h('div', 'insp-head');
    const close = h('button', 'close', '✕');
    close.addEventListener('click', closeInspector);
    head.appendChild(close);
    inspector.appendChild(head);
    const tabs = h('div', 'insp-tabs');
    const body = h('div', 'insp-body');
    inspector.appendChild(tabs);
    inspector.appendChild(body);

    const setTab = (name) => { inspectTab = name; renderInspector(); };

    if (sel.kind === 'agent') {
      const a = w.agents[sel.id];
      if (!a) { closeInspector(); return; }
      const av = h('span', 'big-av', a.avatar); av.style.background = a.color; av.style.color = '#061018';
      head.insertBefore(av, close);
      const nm = h('div', '');
      nm.appendChild(h('div', 'insp-title', a.name));
      nm.appendChild(h('div', 'dim small', ARCH_LABEL[a.archetype] + ' · ' + regionName(w, regionOf(a))));
      head.insertBefore(nm, close);
      const tabsList = ['overview', 'decisions', 'memory', 'compute'];
      for (const t of tabsList) {
        const b = h('button', 'insp-tab' + (inspectTab === t ? ' on' : ''), t);
        b.addEventListener('click', () => setTab(t));
        tabs.appendChild(b);
      }
      if (inspectTab === 'overview') body.appendChild(agentOverview(w, a));
      else if (inspectTab === 'decisions') body.appendChild(agentDecisions(w, a));
      else if (inspectTab === 'memory') body.appendChild(agentMemory(w, a));
      else body.appendChild(agentCompute(w, a));
    } else if (sel.kind === 'org') {
      const o = w.orgs[sel.id];
      if (!o) { closeInspector(); return; }
      const av = h('span', 'big-av', '◉'); av.style.background = o.color || '#5cc8ff';
      head.insertBefore(av, close);
      const nm = h('div', '');
      nm.appendChild(h('div', 'insp-title', o.name));
      nm.appendChild(h('div', 'dim small', o.type + ' · ' + o.members.length + ' members'));
      head.insertBefore(nm, close);
      body.appendChild(orgOverview(w, o));
    } else if (sel.kind === 'region') {
      const r = w.regions.find(x => x.id === sel.id);
      if (!r) { closeInspector(); return; }
      head.appendChild(h('div', 'insp-title', r.name || 'Uncharted'));
      body.appendChild(regionOverview(w, r));
    } else if (sel.kind === 'city') {
      const c = w.map.cities.find(x => x.id === sel.id);
      if (!c) { closeInspector(); return; }
      head.appendChild(h('div', 'insp-title', c.name));
      const m = w.markets[c.marketId];
      body.appendChild(marketOverview(w, m));
    } else if (sel.kind === 'contract') {
      const ct = w.contracts[sel.id];
      if (!ct) { closeInspector(); return; }
      head.appendChild(h('div', 'insp-title', ct.title));
      body.appendChild(contractOverview(w, ct));
    }
  }

  function agentOverview(w, a) {
    const d = h('div', '');
    const objCard = h('div', 'card');
    objCard.appendChild(h('div', 'k', 'Current objective'));
    objCard.appendChild(h('div', 'obj big mt8', a.planGoal || '—'));
    objCard.appendChild(h('div', 'dim small mt8', (a.planKey || '') + ' · status: ' + a.status));
    const why = h('button', 'btn mt8', 'Why did it do this?');
    why.addEventListener('click', () => openWhyModal(a));
    objCard.appendChild(why);
    d.appendChild(objCard);

    const grid = h('div', 'grid cols-2 mt12');
    // Wallet v2 — three layers: personal state, economic stocks, social capital.
    const invCard = h('div', 'card');
    invCard.appendChild(h('h4', '', 'Wallet'));
    const state = h('div', 'mt8');
    state.appendChild(h('div', 'dim small tt-dim', 'STATE'));
    state.appendChild(kv('Energy', Math.round(a.energy || 0) + '/100'));
    state.appendChild(kv('Focus', Math.round(a.focus || 0) + '/100'));
    state.appendChild(kv('Morale', Math.round(a.morale || 0) + '/100'));
    const eco = h('div', 'mt8');
    eco.appendChild(h('div', 'dim small tt-dim', 'ECONOMIC'));
    eco.appendChild(kv('Credits', fmtMoney(a.credits || 0)));
    eco.appendChild(kv('Compute', Math.round(a.compute || 0) + ' ◍'));
    eco.appendChild(kv('Data', Math.round(a.data || 0) + ' ◉'));
    const soc = h('div', 'mt8');
    soc.appendChild(h('div', 'dim small tt-dim', 'SOCIAL'));
    soc.appendChild(kv('Reputation', Math.round(a.reputation || 0) + '/100'));
    const expTop = Object.entries(a.experience || {}).sort((a, b) => b[1] - a[1]).slice(0, 3).map(([k, v]) => k + ' ' + Math.round(v)).join(' · ');
    soc.appendChild(kv('Top experience', expTop || '—'));
    const trustPartners = Object.keys(a.trust || {}).slice(0, 3).map(id => id.slice(-8)).join(' · ');
    soc.appendChild(kv('Trusted', trustPartners || '—'));
    invCard.appendChild(state);
    invCard.appendChild(eco);
    invCard.appendChild(soc);
    grid.appendChild(invCard);
    const repCard = h('div', 'card');
    repCard.appendChild(h('h4', '', 'Reputation'));
    repCard.appendChild(kv('Score', a.rep.score));
    repCard.appendChild(kv('Reliability', a.rep.reliability));
    repCard.appendChild(kv('Cooperation', a.rep.cooperation));
    repCard.appendChild(kv('Contribution', a.rep.contribution));
    repCard.appendChild(kv('Disputes', a.rep.disputes));
    grid.appendChild(repCard);
    d.appendChild(grid);

    const valCard = h('div', 'card mt12');
    valCard.appendChild(h('h4', '', 'Value to the civilization'));
    const s = a.stats || {};
    const totalRow = h('div', 'kv-row');
    totalRow.appendChild(h('span', 'k', 'Total contribution'));
    const totalV = h('span', 'v');
    totalV.id = 'valTotal';
    totalV.textContent = fmt(agentValue(a)) + ' vp';
    totalRow.appendChild(totalV);
    valCard.appendChild(totalRow);
    valCard.appendChild(kv('Credits earned', fmt(Math.round(s.earned || 0))));
    valCard.appendChild(kv('Taxes paid', fmt(Math.round(s.taxesPaid || 0))));
    valCard.appendChild(kv('Contracts completed', (s.contracts || 0) + ''));
    valCard.appendChild(kv('Discoveries', (s.discoveries || 0) + ''));
    valCard.appendChild(kv('Breakthroughs', (s.breakthroughs || 0) + ''));
    valCard.appendChild(kv('Buildings erected', (s.built || 0) + ''));
    valCard.appendChild(kv('Gathered / mined', Math.round(s.produced || 0) + ' units'));
    valCard.appendChild(kv('Market volume', Math.round(s.tradedVol || 0) + ' units'));
    valCard.appendChild(kv('Compute jobs run', (s.computeJobs || 0) + ''));
    d.appendChild(valCard);

    if (a.real) {
      const fab = h('div', 'card mt12');
      fab.appendChild(h('h4', '', 'Fabric Agent'));
      fab.appendChild(kv('Agent ID', a.agentId || a.id));
      fab.appendChild(kv('Role', a.role || 'generalist'));
      fab.appendChild(kv('Node', a.nodeName || (a.remote ? 'remote' : 'local')));
      fab.appendChild(kv('Source', a.remote ? 'remote fabric node' : 'this node'));
      // REAL wallet: spendable quota from the fabric's authoritative ledger.
      const eco = (w.fabric && w.fabric.realEconomy) || null;
      const realBal = eco && eco.agents && eco.agents[a.agentId || a.id];
      fab.appendChild(kv('REAL quota (fabric)', realBal ? fmt(realBal.spendable) + ' units' : (eco && eco.attached ? '0 units' : '—')));
      fab.appendChild(h('div', 'dim small mt8', 'Income is credited only for verified fabric work. Upkeep, taxes and compute spend drain the wallet.'));
      if (a.description) fab.appendChild(h('div', 'dim small mt8', a.description));
      if (a.capabilities && a.capabilities.length) {
        const capRow = h('div', 'mt8');
        for (const c of a.capabilities.slice(0, 10)) {
          capRow.appendChild(tag(c, ''));
        }
        fab.appendChild(capRow);
      }
      if (a.tools && a.tools.length) {
        const tl = h('div', 'mt8 small dim');
        tl.textContent = 'Tools: ' + a.tools.slice(0, 8).join(' · ');
        fab.appendChild(tl);
      }
      d.appendChild(fab);
    }

    const skCard = h('div', 'card mt12');
    skCard.appendChild(h('h4', '', 'Skills'));
    for (const [k, v] of Object.entries(a.skills)) {
      const rw = h('div', 'kv-row');
      rw.appendChild(h('span', 'k', k));
      rw.appendChild(h('span', 'v', v.toFixed(1)));
      skCard.appendChild(rw);
      const b = h('div', 'bar mt8'); b.appendChild(h('i'));
      b.firstChild.style.width = (v / 5) * 100 + '%';
      skCard.appendChild(b);
    }
    d.appendChild(skCard);

    const perCard = h('div', 'card mt12');
    perCard.appendChild(h('h4', '', 'Personality'));
    for (const [k, v] of Object.entries(a.personality)) {
      perCard.appendChild(kv(k, Math.round(v * 100) + '%'));
    }
    d.appendChild(perCard);

    const relCard = h('div', 'card mt12');
    relCard.appendChild(h('h4', '', 'Relationships'));
    if (!Object.keys(a.relations).length) relCard.appendChild(h('div', 'empty', 'No relationships yet.'));
    for (const [k, v] of Object.entries(a.relations)) {
      relCard.appendChild(kv(k, v));
    }
    d.appendChild(relCard);

    const achCard = h('div', 'card mt12');
    achCard.appendChild(h('h4', '', 'Achievements'));
    if (!a.achievements.length) achCard.appendChild(h('div', 'empty', 'No achievements yet.'));
    for (const c of a.achievements.slice(-12)) {
      achCard.appendChild(kv(fmtTick(c.tick), c.detail));
    }
    d.appendChild(achCard);
    return d;
  }

  function openWhyModal(a) {
    openModal((box) => {
      box.appendChild(h('h3', '', 'Why did ' + a.name + ' do it?'));
      box.appendChild(h('div', 'dim small mb12', 'Decision trace — every step recorded, none fabricated.'));
      const w = world();
      const decs = w.decisions.filter(d => d.agentId === a.id).slice(-8);
      if (!decs.length) box.appendChild(h('div', 'empty', 'No recorded decisions yet.'));
      for (const d of decs) {
        const st = h('div', 'trace-step' + (d.phase === 'act' ? ' ev' : ''));
        st.appendChild(h('div', 'ts-ts', fmtTick(d.tick) + ' · ' + d.phase));
        st.appendChild(h('div', 'ts-ttl', d.action ? 'Action: ' + d.action : 'Plan: ' + (d.chosen ? d.chosen.key : '') + ' — ' + (d.chosen ? d.chosen.label : '')));
        if (d.observation && d.observation.length) {
          const obs = h('div', 'ts-body');
          obs.appendChild(h('span', 'faint', 'Observed: '));
          obs.appendChild(h('span', '', d.observation.join(' · ')));
          st.appendChild(obs);
        }
        if (d.evaluation && d.evaluation.length) {
          const ev = h('div', 'ts-body');
          ev.appendChild(h('span', 'faint', 'Evaluated: '));
          ev.appendChild(h('span', '', d.evaluation.slice(0, 3).map(c => c.key + '(' + c.score.toFixed(1) + ')').join(' · ')));
          st.appendChild(ev);
        }
        if (d.plan && d.plan.length) {
          const pl = h('div', 'ts-body');
          pl.appendChild(h('span', 'faint', 'Planned: '));
          pl.appendChild(h('span', '', d.plan.join(' → ')));
          st.appendChild(pl);
        }
        if (d.result) {
          const re = h('div', 'ts-body');
          re.appendChild(h('span', 'faint', 'Result: '));
          re.appendChild(h('span', '', String(d.result) + (d.reward ? ' · +' + d.reward + ' Cr' : '')));
          st.appendChild(re);
        }
        box.appendChild(st);
      }
      const evs = w.events.filter(e => e.actor === a.id).slice(-4);
      if (evs.length) {
        box.appendChild(h('div', 'mt16 mb8 faint small', 'Related events'));
        for (const e of evs) {
          const r = h('div', 'feed-row');
          r.appendChild(h('span', 'ft', fmtTick(e.tick)));
          r.appendChild(h('span', 'msg', e.detail || ''));
          box.appendChild(r);
        }
      }
    });
  }

  function agentDecisions(w, a) {
    const d = h('div', '');
    const decs = w.decisions.filter(x => x.agentId === a.id).reverse().slice(0, 30);
    if (!decs.length) { d.appendChild(h('div', 'empty', 'No decisions recorded yet.')); return d; }
    for (const dc of decs) {
      const st = h('div', 'trace-step');
      st.appendChild(h('div', 'ts-ts', fmtTick(dc.tick) + ' · ' + dc.phase + (dc.action ? ' · ' + dc.action : '')));
      st.appendChild(h('div', 'ts-ttl', (dc.chosen ? dc.chosen.key + ': ' : '') + (dc.chosen ? dc.chosen.label : (dc.action || ''))));
      if (dc.result) st.appendChild(h('div', 'ts-body', 'Result: ' + dc.result + (dc.reward ? ' (+' + dc.reward + ' Cr)' : '')));
      d.appendChild(st);
    }
    return d;
  }

  function agentMemory(w, a) {
    const d = h('div', '');
    const mems = [...a.memory].sort((x, y) => y.importance - x.importance).slice(0, 40);
    if (!mems.length) { d.appendChild(h('div', 'empty', 'No memories yet.')); return d; }
    for (const m of mems) {
      const c = h('div', 'card mb8');
      c.appendChild(h('div', 'dim small', m.type + ' · imp ' + m.importance.toFixed(1) + ' · ' + fmtTick(m.tick)));
      c.appendChild(h('div', 'mt8', m.text));
      if (m.tags && m.tags.length) {
        c.appendChild(h('div', 'mt8'));
        for (const t of m.tags.slice(0, 4)) c.appendChild(tag(t, 'dim'));
      }
      d.appendChild(c);
    }
    return d;
  }

  function agentCompute(w, a) {
    const d = h('div', '');
    const grid = h('div', 'grid cols-2');
    const usg = h('div', 'card');
    usg.appendChild(h('h4', '', 'Usage'));
    usg.appendChild(kv('Compute credits spent', a.computeTrack.usage));
    usg.appendChild(kv('Earned contributing', a.computeTrack.earned));
    usg.appendChild(kv('Is contributor', contributing(w, a.id) ? 'yes' : 'no'));
    usg.appendChild(kv('Last result tick', a.computeTrack.lastResultTick));
    grid.appendChild(usg);
    const res = h('div', 'card');
    res.appendChild(h('h4', '', 'Job results'));
    if (!Object.keys(a.computeTrack.results || {}).length) res.appendChild(h('div', 'empty', 'No compute results.'));
    for (const [task, r] of Object.entries(a.computeTrack.results || {})) {
      res.appendChild(kv(task, r.executionId + ' · ' + JSON.stringify(r.result).slice(0, 60)));
    }
    grid.appendChild(res);
    d.appendChild(grid);
    const jobs = w.compute.jobOrder.map(id => w.compute.jobs[id]).filter(j => j.requester === a.id).reverse().slice(0, 20);
    if (jobs.length) {
      const jc = h('div', 'card mt12');
      jc.appendChild(h('h4', '', 'Jobs'));
      for (const j of jobs) jc.appendChild(kv(j.executionId + ' · ' + j.taskType, j.status + ' · ◍' + j.budget));
      d.appendChild(jc);
    }
    return d;
  }

  function orgOverview(w, o) {
    const d = h('div', '');
    const head = h('div', 'card');
    head.appendChild(kv('Leader', w.agents[o.leaderId] ? w.agents[o.leaderId].name : '—'));
    head.appendChild(kv('Founded', fmtTick(o.createdTick)));
    head.appendChild(kv('Power', powerOf(w, 'org:' + o.id)));
    head.appendChild(kv('Reputation', o.rep));
    d.appendChild(head);
    const grid = h('div', 'grid cols-2 mt12');
    const memCard = h('div', 'card');
    memCard.appendChild(h('h4', '', 'Members'));
    for (const mid of o.members) {
      const a = w.agents[mid];
      if (!a) continue;
      const r = h('div', 'ws-row');
      const av = h('span', 'av', a.avatar); av.style.background = a.color;
      r.appendChild(av);
      const nm = h('span', 'nm');
      nm.innerHTML = `<b>${esc(a.name)}</b> <span class="dim">${a.orgRole}</span>`;
      r.appendChild(nm);
      r.addEventListener('click', () => { sel = { kind: 'agent', id: a.id }; openInspector(); });
      memCard.appendChild(r);
    }
    grid.appendChild(memCard);
    const treaCard = h('div', 'card');
    treaCard.appendChild(h('h4', '', 'Treasury'));
    for (const [k, v] of Object.entries(o.treasury)) {
      treaCard.appendChild(kv(k, Number.isInteger(v) ? v : v.toFixed(1)));
    }
    grid.appendChild(treaCard);
    d.appendChild(grid);
    const terrCard = h('div', 'card mt12');
    terrCard.appendChild(h('h4', '', 'Territory'));
    if (!o.territory.length) terrCard.appendChild(h('div', 'empty', 'No territory.'));
    for (const rid of o.territory) {
      const r = w.regions.find(x => x.id === rid);
      if (r) terrCard.appendChild(kv(r.name, r.infra.relays > 0 ? 'relay' : 'wild'));
    }
    d.appendChild(terrCard);
    const polCard = h('div', 'card mt12');
    polCard.appendChild(h('h4', '', 'Policies'));
    for (const [k, v] of Object.entries(o.policies)) polCard.appendChild(kv(k, String(v)));
    d.appendChild(polCard);
    const histCard = h('div', 'card mt12');
    histCard.appendChild(h('h4', '', 'History'));
    for (const e of o.history.slice(-20)) {
      histCard.appendChild(kv(fmtTick(e.tick) + ' · ' + e.kind, e.detail));
    }
    d.appendChild(histCard);
    return d;
  }

  function regionDistrict(w, r) {
    if (!r.explored) return 'wild';
    if (r.cityId) return 'market';
    if (r.infra.labs > 0) return 'institute';
    if (r.infra.factories > 0 || r.infra.refineries > 0) return 'industry';
    if ((w.fact.disputes || []).some(d => d.state === 'active' && d.regionId === r.id)) return 'arena';
    if (r.prod.food >= 2 || r.resources.food > 520) return 'fields';
    if (r.resources.rare > 320 || r.resources.materials > 540) return 'mine';
    if (r.population > 0) return 'homes';
    return 'wild';
  }
  function regionOverview(w, r) {
    const d = h('div', '');
    const c = h('div', 'card');
    c.appendChild(kv('Biome', BIOME_INFO[r.biome] ? BIOME_INFO[r.biome].label : r.biome));
    const dist = DISTRICT_LEGEND.find(([l]) => l === regionDistrict(w, r));
    if (dist) {
      const dr = h('div', 'kv-row');
      dr.appendChild(h('span', 'k', 'District'));
      const dv = h('span', 'v');
      dv.innerHTML = `<span class="sw" style="background:${dist[1]};display:inline-block;vertical-align:middle"></span> ${dist[0]}`;
      dr.appendChild(dv);
      c.appendChild(dr);
    }
    c.appendChild(kv('Danger', r.danger));
    c.appendChild(kv('Explored', r.explored ? 'yes' : 'no'));
    c.appendChild(kv('Owner', r.owner ? (w.orgs[r.owner] ? w.orgs[r.owner].name : r.owner) : '—'));
    c.appendChild(kv('Population', r.population));
    d.appendChild(c);
    const workers = h('div', 'card mt12');
    workers.appendChild(h('h4', '', 'Who works here'));
    const here = w.agentOrder.map(id => w.agents[id]).filter(a => a && regionOf(a) === r.id);
    if (!here.length) workers.appendChild(h('div', 'empty', 'No agents right now.'));
    for (const a of here.slice(0, 10)) {
      const row = h('div', 'ws-row');
      const av = h('span', 'av', a.avatar); av.style.background = a.color;
      const nm = h('span', 'nm');
      nm.innerHTML = `<b>${esc(a.name)}</b> <span class="dim">${ARCH_LABEL[a.archetype]}</span>`;
      const val = h('span', 'num'); val.innerHTML = fmt(a.credits || 0) + ' <span class="faint">Cr</span>';
      row.appendChild(av); row.appendChild(nm); row.appendChild(h('div', 'f1')); row.appendChild(val);
      row.title = a.status || 'working';
      row.addEventListener('click', () => { sel = { kind: 'agent', id: a.id }; openInspector(); });
      workers.appendChild(row);
      workers.appendChild(h('div', 'ws-sub', (a.status || 'working') + (a.planKey ? ' · ' + a.planKey : '')));
    }
    d.appendChild(workers);
    const act = h('div', 'card mt12');
    act.appendChild(h('h4', '', 'Recent activity here'));
    const acts = (w.activity || []).filter(ac => ac.regionId === r.id).slice(-8).reverse();
    if (!acts.length) act.appendChild(h('div', 'empty', 'Nothing recorded yet.'));
    for (const ac of acts) {
      const row = h('div', 'act-row sm');
      const av = h('span', 'act-av', ac.avatar); av.style.background = ac.color;
      row.appendChild(h('span', 'act-t', fmtTick(ac.t)));
      row.appendChild(av);
      const who = h('span', 'act-who'); who.innerHTML = `<b>${esc(ac.name)}</b>`;
      row.appendChild(who); row.appendChild(h('span', 'act-verb', ac.verb));
      const det = h('span', 'act-det'); det.textContent = (ac.detail || '').slice(0, 64);
      row.appendChild(det);
      row.addEventListener('click', () => { sel = { kind: 'agent', id: ac.agentId }; openInspector(); });
      act.appendChild(row);
    }
    d.appendChild(act);
    const res = h('div', 'card mt12');
    res.appendChild(h('h4', '', 'Resources'));
    for (const [k, v] of Object.entries(r.resources)) res.appendChild(kv(k, Math.round(v)));
    d.appendChild(res);
    const infra = h('div', 'card mt12');
    infra.appendChild(h('h4', '', 'Infrastructure'));
    for (const [k, v] of Object.entries(r.infra)) infra.appendChild(kv(k, v));
    d.appendChild(infra);
    return d;
  }

  function marketOverview(w, m) {
    if (!m) { const e = h('div', 'empty', 'No market data.'); return e; }
    const d = h('div', '');
    const c = h('div', 'card');
    c.appendChild(h('h4', '', 'Prices'));
    for (const r of MKT_RES) c.appendChild(kv(RES_LABEL[r], m.prices[r].toFixed(1) + ' Cr'));
    d.appendChild(c);
    const s = h('div', 'card mt12');
    s.appendChild(h('h4', '', 'History'));
    for (const r of MKT_RES) {
      s.appendChild(sparkSvg(m.history[r] || [], 400, 120, '#5cc8ff'));
    }
    d.appendChild(s);
    return d;
  }

  function contractOverview(w, ct) {
    const d = h('div', '');
    const c = h('div', 'card');
    c.appendChild(kv('State', ct.state));
    c.appendChild(kv('Issuer', ct.issuer === 'world' ? 'World' : (w.agents[ct.issuer] ? w.agents[ct.issuer].name : ct.issuer)));
    c.appendChild(kv('Objective', JSON.stringify(ct.objective)));
    c.appendChild(kv('Reward', fmtMoney(ct.reward.credits || 0) + (ct.reward.compute ? ' · ◍' + ct.reward.compute : '') + (ct.reward.data ? ' · ◉' + ct.reward.data : '')));
    c.appendChild(kv('Deadline', fmtTick(ct.deadlineTick)));
    c.appendChild(kv('Risk', Math.round(ct.risk * 100) + '%'));
    c.appendChild(kv('Progress', Math.round(ct.progress) + ' / ' + ct.target));
    if (ct.claimants && ct.claimants.length) c.appendChild(kv('Claimants', ct.claimants.join(', ')));
    if (ct.completedBy) c.appendChild(kv('Completed by', w.agents[ct.completedBy] ? w.agents[ct.completedBy].name : ct.completedBy));
    if (ct.completedTick != null) c.appendChild(kv('Completed', fmtTick(ct.completedTick)));
    d.appendChild(c);
    return d;
  }

  /* ================= modal ================= */
  function openModal(builder) {
    modal.hidden = false;
    modal.innerHTML = '';
    const box = h('div', 'modal-box');
    builder(box);
    const x = h('button', 'btn sm ghost mt12', 'Close');
    x.addEventListener('click', closeModal);
    box.appendChild(x);
    modal.appendChild(box);
  }
  function closeModal() { modal.hidden = true; modal.innerHTML = ''; }

  /* ================= public refresh ================= */
  let refreshCount = 0;
  let forceFull = false;
  const valPulseSeen = new Map();
  function refresh() {
    const w = world();
    if (!w) return;
    renderTopbar();
    renderRail();
    renderTicker();
    if (curScreen === 'world' || curScreen === 'map') {
      if (map) map.update(w, sel, { full: forceFull || refreshCount % 20 === 0 });
      forceFull = false;
      const hud = stage.querySelector('.map-status');
      if (hud) hud.innerHTML = `${w.map.landRegions.length} regions<br>${w.map.cities.length} cities · ${w.map.routes.length} routes<br>${(map.fronts || 0)} fronts · ${(map.disputes || 0)} disputes`;
      const lifeEl = stage.querySelector('#lifeVal');
      if (lifeEl) lifeEl.textContent = fmt(map.getLife());
    }
    if (w.clock.t !== lastT) {
      lastT = w.clock.t;
      if (curScreen === 'world') {
        const side = stage.querySelector('.world-side');
        if (side) side.replaceWith(buildWorldSidebar());
      } else if (curScreen !== 'map') {
        const view = stage.querySelector('.view');
        if (view) view.replaceWith(buildScreen(curScreen));
      }
      renderInspector();
      updateClock();
    }
    if (sel && sel.kind === 'agent') {
      const a = w.agents[sel.id];
      if (a && a.stats) {
        const key = a.id;
        const prev = valPulseSeen.get(key);
        const now = a.stats.earned || 0;
        if (prev != null && now > prev) {
          valPulseSeen.set(key, now);
          const vt = document.querySelector('#valTotal');
          if (vt) { vt.classList.remove('pulse'); void vt.offsetWidth; vt.classList.add('pulse'); }
        } else if (prev == null) valPulseSeen.set(key, now);
      }
    }
    refreshCount++;
  }

  function esc(s) {
    const d = document.createElement('div');
    d.textContent = s == null ? '' : String(s);
    return d.innerHTML;
  }

  renderTopbar();
  renderRail();
  renderScreen();
  refresh();

  return { nav, refresh, toast, openInspector, closeInspector, openModal, closeModal, markDirty() { forceFull = true; }, get currentScreen() { return curScreen; } };
}
