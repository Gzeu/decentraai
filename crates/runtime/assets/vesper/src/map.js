import { agentValue } from './core.js';

const SVG = 'http://www.w3.org/2000/svg';
const el = (tag, attrs) => { const e = document.createElementNS(SVG, tag); for (const k in attrs) e.setAttribute(k, attrs[k]); return e; };
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const hashH = (s) => { let h = 0; for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0; return (h % 1000) / 1000; };
const fmt = (n) => (isFinite(n) ? (Math.round(n) || 0).toLocaleString('en-US') : '0');

const BIOME_FILL = {
  ocean: 'none',
  plains: '#5a925f',
  forest: '#33906b',
  mountain: '#92a8cf',
  desert: '#b3924f',
  tundra: '#8098ba',
  wetland: '#45a08d',
};
const ZONE_KIND = {
  anomaly: { label: 'Anomaly', color: '#c67bff', sym: '◈' },
  combat: { label: 'Combat Zone', color: '#ff5470', sym: '⚠' },
  research: { label: 'Research Site', color: '#5cc8ff', sym: '◉' },
};
const INFRA_SYM = { labs: '◉', factories: '▲', relays: '◍', defense: '◆', refineries: '⬢' };
const INFRA_LABEL = { labs: 'Lab', factories: 'Factory', relays: 'Relay', defense: 'Defense', refineries: 'Refinery' };
const RES_DOT = { food: '#55e6a4', energy: '#ffd166', materials: '#9d8cff', rare: '#ff6b6b' };

const EV_INFO = {
  discovery: ['✦', '#55e6a4'], breakthrough: ['◉', '#5cc8ff'], 'compute-used': ['⬢', '#9d8cff'],
  construction: ['▲', '#ffd166'], conflict: ['⚔', '#ff5470'], 'org-founded': ['◈', '#9d8cff'],
  'org-collapsed': ['✕', '#ff5470'], 'market-crash': ['⬡', '#ffb454'], shortage: ['⚠', '#ffb454'],
  'political-conflict': ['⚔', '#ff5470'], 'territory-expanded': ['⬢', '#ff9e5c'],
  'resource-discovery': ['◆', '#55e6a4'], 'infra-failure': ['✕', '#ff5470'], anomaly: ['◉', '#c67bff'],
  'anomaly-spread': ['◉', '#c67bff'], prosperity: ['◈', '#ffd166'], migration: ['↷', '#92a4c0'],
  'contract-posted': ['⚑', '#5cc8ff'], genesis: ['✦', '#ffd166'],
};
const ARCH_SHAPE = {
  explorer: 'tri', merchant: 'diamond', trader: 'diamond', scientist: 'hex', researcher: 'hex',
  engineer: 'square', builder: 'square', strategist: 'star', diplomat: 'star',
  mercenary: 'tri', guardian: 'shield', opportunist: 'pent',
};
const SHAPE_D = {
  tri: 'M0,-6.5 L5.6,5 L-5.6,5 Z',
  diamond: 'M0,-6.5 L5,0 L0,6.5 L-5,0 Z',
  hex: 'M0,-6 L5.2,-3 L5.2,3 L0,6 L-5.2,3 L-5.2,-3 Z',
  square: 'M-5,-5 L5,-5 L5,5 L-5,5 Z',
  star: 'M0,-6.5 L1.8,-2 L6.5,-2 L2.7,0.8 L4.2,5.8 L0,2.8 L-4.2,5.8 L-2.7,0.8 L-6.5,-2 L-1.8,-2 Z',
  shield: 'M0,-6.5 L5,-4.5 L5,1.5 C5,4.5 2.5,6 0,6.5 C-2.5,6 -5,4.5 -5,1.5 L-5,-4.5 Z',
  pent: 'M0,-6.2 L5.9,-1.9 L3.6,5 L-3.6,5 L-5.9,-1.9 Z',
  circle: 'M0,-6 A6,6 0 1,1 0,6 A6,6 0 1,1 0,-6',
};
const STATUS_GLYPH = {
  traveling: '→', gathering: '✂', mining: '⛏', researching: '◉', building: '▲', trading: '⬡',
  resting: '☾', patrolling: '◇', contracting: '⚑', contesting: '⚔', sabotaging: '✕', 'founding-org': '◈',
};
const DISTRICT = {
  market: { color: '#ffd166', label: 'Market' },
  industry: { color: '#ff8a5c', label: 'Industry' },
  institute: { color: '#5cc8ff', label: 'Institute' },
  fields: { color: '#55e6a4', label: 'Fields' },
  mine: { color: '#c67bff', label: 'Mine' },
  arena: { color: '#ff5470', label: 'Arena' },
  homes: { color: '#8fa3c4', label: 'Homes' },
  wild: { color: '#4d6270', label: 'Wild' },
};

const rgb = (hex) => { const n = parseInt(hex.slice(1), 16); return [(n >> 16) & 255, (n >> 8) & 255, n & 255]; };
const rgba = (hex, a) => { const c = rgb(hex); return `rgba(${c[0]},${c[1]},${c[2]},${a})`; };
const mix = (a, b, t = 0.5) => { const A = rgb(a), B = rgb(b); return `rgb(${Math.round(A[0] + (B[0] - A[0]) * t)},${Math.round(A[1] + (B[1] - A[1]) * t)},${Math.round(A[2] + (B[2] - A[2]) * t)})`; };
const pointInPoly = (px, py, poly) => {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const xi = poly[i][0], yi = poly[i][1], xj = poly[j][0], yj = poly[j][1];
    if ((yi > py) !== (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
};

export function createMap(container, opts) {
  const onSelect = opts.onSelect || (() => {});

  const tooltip = document.createElement('div');
  tooltip.className = 'map-tip';
  tooltip.style.display = 'none';
  container.appendChild(tooltip);

  const svg = el('svg', { class: 'worldmap map' });
  container.appendChild(svg);

  const fx = document.createElement('canvas');
  fx.className = 'map-fx';
  container.appendChild(fx);
  const fxctx = fx.getContext('2d');

  const mm = document.createElement('canvas');
  mm.className = 'map-mm';
  mm.title = 'Civilization minimap — click to jump';
  container.appendChild(mm);
  const mmctx = mm.getContext('2d');
  const MM_W = 200, MM_H = 132; // desktop minimap size; phone uses 118x78 in applySvgSize

  const defs = el('defs', {});
  svg.appendChild(defs);
  defs.innerHTML = `
    <radialGradient id="oceanGrad" cx="50%" cy="42%" r="80%">
      <stop offset="0%" stop-color="#1f5d9e"/><stop offset="70%" stop-color="#15508c"/><stop offset="100%" stop-color="#0f3f70"/>
    </radialGradient>
    <linearGradient id="terrainShade" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#ffffff" stop-opacity="0.08"/>
      <stop offset="55%" stop-color="#ffffff" stop-opacity="0"/>
      <stop offset="100%" stop-color="#000000" stop-opacity="0.18"/>
    </linearGradient>
    <pattern id="hatchRed" patternUnits="userSpaceOnUse" width="9" height="9" patternTransform="rotate(45)">
      <line x1="0" y1="0" x2="0" y2="9" stroke="#ff5470" stroke-width="1.3" opacity="0.5"/>
    </pattern>
    <pattern id="hatchGold" patternUnits="userSpaceOnUse" width="9" height="9" patternTransform="rotate(-45)">
      <line x1="0" y1="0" x2="0" y2="9" stroke="#ffd166" stroke-width="1.3" opacity="0.45"/>
    </pattern>
    <filter id="glowA" x="-60%" y="-60%" width="220%" height="220%">
      <feGaussianBlur stdDeviation="3" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
    <filter id="glowSoft" x="-80%" y="-80%" width="260%" height="260%">
      <feGaussianBlur stdDeviation="5" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
  `;

  const root = el('g', {});
  svg.appendChild(root);
  const stat = el('g', {}); root.appendChild(stat);
  const decorG = el('g', { 'pointer-events': 'none' }); stat.appendChild(decorG);
  const settleG = el('g', { 'pointer-events': 'none' }); root.appendChild(settleG);
  const trailG = el('g', { 'pointer-events': 'none' }); root.appendChild(trailG);
  const dyn = el('g', {}); root.appendChild(dyn);
  const evG = el('g', { 'pointer-events': 'none' }); root.appendChild(evG);
  const agentG = el('g', {}); root.appendChild(agentG);

  let view = { x: 0, y: 0, k: 1 };
  let B = { minX: 0, minY: 0, maxX: 1000, maxY: 620 };
  let world = null;
  let sel = null;
  let drag = null;
  let userMoved = false;
  let hoveredAgent = null;
  let frontCount = 0;
  let disputeCount = 0;
  let ro = null;

  const S = {
    follow: true,
    followAgent: null,
    filter: 'all',
    fit: null,
    quiet: 0,
    pipe: null,
    pipeLive: false,
    pipeExecs: -1,
    mm: { sc: 1, ox: 0, oy: 0 },
    lastEm: {},
  };

  const regionGeom = new Map();
  const cityGeom = [];
  const infraGeom = [];
  const routeGeom = [];
  const agentGels = new Map();
  const agentUi = new Map();
  let particles = [];
  let flashes = [];
  const evSeen = new Set();
  let lastActSeen = 0;
  let lastFrame = 0;

  /* ================= helpers ================= */
  function ownerOf(w, rid) {
    const r = w.regions.find(x => x.id === rid);
    if (r && r.owner) return r.owner;
    for (const oid of w.orgOrder) {
      const o = w.orgs[oid];
      if (o && o.territory && o.territory.includes(rid)) return oid;
    }
    return null;
  }
  function orgColor(w, oid) {
    const o = w.orgs[oid];
    return (o && o.color) || '#9d8cff';
  }
  function regionOf(a) {
    return a.loc.travel ? a.loc.travel.path[a.loc.travel.idx] : a.loc.regionId;
  }
  function jitter(id) {
    let h = 0;
    for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return { dx: (h % 17) - 8, dy: ((h >> 4) % 17) - 8 };
  }
  function esc(s) {
    const d = document.createElement('div');
    d.textContent = s == null ? '' : String(s);
    return d.innerHTML;
  }
  function districtOf(w, r) {
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
  function settlePos(r) {
    return { x: r.x + (hashH(r.id + 'shx') * 2 - 1) * 14, y: r.y + (hashH(r.id + 'shy') * 2 - 1) * 14 };
  }
  function stageOf(r) {
    const p = r.population || 0;
    let st = 0;
    if (p >= 120) st = 4; else if (p >= 60) st = 3; else if (p >= 30) st = 2; else if (p >= 8) st = 1;
    if (r.cityId) st = Math.max(st, 3);
    return st;
  }
  function lifeScore(w) {
    let s = 0;
    for (const id of w.agentOrder) { const a = w.agents[id]; if (a) s += agentValue(a); }
    s += w.stats.built * 20 + w.stats.explored * 30 + w.stats.research * 2 + w.stats.completedContracts * 25;
    for (const k in (w.fact.tech || {})) s += (w.fact.tech[k].level || 0) * 500;
    return Math.round(s);
  }
  function envOf(t) {
    const h = ((t % 24) + 24) % 24;
    let dayF;
    if (h < 6 || h >= 19) dayF = 0;
    else if (h < 12.5) dayF = (h - 6) / 6.5;
    else dayF = 1 - (h - 12.5) / 6.5;
    dayF = clamp(dayF, 0, 1);
    const doy = Math.floor(t / 24) % 365;
    const season = doy < 80 ? 'winter' : doy < 171 ? 'spring' : doy < 263 ? 'summer' : 'autumn';
    return { h, dayF, doy, season };
  }

  /* ================= view / camera ================= */
  function applySvgSize() {
    const cw = svg.clientWidth || container.clientWidth || 1000;
    const ch = svg.clientHeight || container.clientHeight || 620;
    if (svg.__cw !== cw || svg.__ch !== ch) {
      svg.__cw = cw; svg.__ch = ch;
      svg.setAttribute('viewBox', `0 0 ${cw} ${ch}`);
      const dpr = window.devicePixelRatio || 1;
      const mmw = cw < 640 ? 118 : 200, mmh = cw < 640 ? 78 : 132;
      fx.width = Math.round(cw * dpr); fx.height = Math.round(ch * dpr);
      fx.style.width = cw + 'px'; fx.style.height = ch + 'px';
      mm.width = Math.round(mmw * dpr); mm.height = Math.round(mmh * dpr);
      mm.style.width = mmw + 'px'; mm.style.height = mmh + 'px';
    }
  }
  function applyView() {
    root.setAttribute('transform', `translate(${view.x} ${view.y}) scale(${view.k})`);
  }
  function computeFit() {
    const cw = svg.clientWidth || container.clientWidth || 1000;
    const ch = svg.clientHeight || container.clientHeight || 620;
    const bw = Math.max(1, B.maxX - B.minX), bh = Math.max(1, B.maxY - B.minY);
    const pad = 46;
    const k = Math.max(0.6, Math.min(Math.min((cw - pad * 2) / bw, (ch - pad * 2) / bh), 3));
    return { x: cw / 2 - (B.minX + B.maxX) / 2 * k, y: ch / 2 - (B.minY + B.maxY) / 2 * k, k };
  }
  function initView() {
    S.fit = computeFit();
    view.x = S.fit.x; view.y = S.fit.y; view.k = S.fit.k;
    applyView();
  }
  function zoomAt(cx, cy, factor) {
    const nk = clamp(view.k * factor, 0.6, 8);
    const kr = nk / view.k;
    view.x = cx - (cx - view.x) * kr;
    view.y = cy - (cy - view.y) * kr;
    view.k = nk;
    userMoved = true;
    applyView();
  }
  function worldToScreen(wx, wy) {
    return { x: view.x + wx * view.k, y: view.y + wy * view.k };
  }
  function screenToWorld(px, py) {
    return { x: (px - view.x) / view.k, y: (py - view.y) / view.k };
  }

  function watchSize() {
    if (ro) return;
    ro = new ResizeObserver(() => {
      applySvgSize();
      if (!userMoved) initView();
    });
    ro.observe(container);
  }

  svg.addEventListener('wheel', (e) => {
    e.preventDefault();
    const rect = svg.getBoundingClientRect();
    zoomAt(e.clientX - rect.left, e.clientY - rect.top, e.deltaY < 0 ? 1.22 : 0.82);
  }, { passive: false });

  svg.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    drag = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y, moved: false };
    svg.setPointerCapture(e.pointerId);
  });
  svg.addEventListener('pointermove', (e) => {
    if (drag) {
      const dx = e.clientX - drag.x, dy = e.clientY - drag.y;
      if (Math.abs(dx) + Math.abs(dy) > 3) { drag.moved = true; userMoved = true; }
      if (drag.moved) { view.x = drag.vx + dx; view.y = drag.vy + dy; applyView(); }
    } else if (hoveredAgent) {
      moveTipClient(e.clientX, e.clientY);
    }
  });
  svg.addEventListener('pointerup', (e) => {
    if (drag && !drag.moved) {
      const rect = svg.getBoundingClientRect();
      const pt = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
      const hit = hitTest(pt);
      if (hit) {
        if (hit.kind === 'agent') { S.followAgent = hit.id; S.follow = true; userMoved = false; }
        else { S.followAgent = null; S.follow = false; userMoved = true; }
        onSelect(hit);
      }
    }
    drag = null;
  });
  svg.addEventListener('pointerleave', () => { tooltip.style.display = 'none'; hoveredAgent = null; });

  function moveTipClient(cx, cy) {
    const rect = svg.getBoundingClientRect();
    tooltip.style.left = (cx - rect.left + 14) + 'px';
    tooltip.style.top = (cy - rect.top + 12) + 'px';
  }
  function showTipAt(hit, wx, wy) {
    tooltip.innerHTML = hit.tip || '';
    tooltip.style.display = 'block';
    const s = worldToScreen(wx, wy);
    tooltip.style.left = (s.x + 14) + 'px';
    tooltip.style.top = (s.y + 12) + 'px';
  }

  /* ================= tooltips ================= */
  function regionTip(w, r) {
    const owner = ownerOf(w, r.id);
    const org = owner ? w.orgs[owner] : null;
    const city = r.cityId ? (w.map.cities.find(c => c.id === r.cityId) || {}).name : null;
    const infra = Object.keys(INFRA_SYM).filter(k => r.infra[k] > 0).map(k => INFRA_LABEL[k]).join(', ');
    const dispute = (w.fact.disputes || []).find(d => d.state === 'active' && d.regionId === r.id);
    const dk = districtOf(w, r);
    const dLabel = DISTRICT[dk] ? DISTRICT[dk].label : dk;
    let workers = 0;
    if (r.explored) {
      for (const id of w.agentOrder) { const a = w.agents[id]; if (a && regionOf(a) === r.id) workers++; }
    }
    let out = `<b>${r.name || 'Uncharted'}</b> <span class="tt-dim">${r.biome}</span><br>
      <span class="tt-dim">${org ? `Owned by <b style="color:${org.color}">${esc(org.name)}</b>` : city ? city : 'Wilderness'}</span><br>
      <span class="tt-dim">${dLabel} · Pop ${r.population || 0} · Danger ${r.danger}${infra ? ' · ' + infra : ''}</span>
      <span class="tt-dim"><br>${workers} agent(s) here · Rare ${Math.round(r.resources.rare)}</span>`;
    if (dispute) {
      const a = w.orgs[dispute.orgA], b = w.orgs[dispute.orgB];
      out += `<br><span style="color:#ffd166">⚔ Disputed: ${esc(a ? a.name : dispute.orgA)} vs ${esc(b ? b.name : dispute.orgB)}</span>`;
    }
    return out;
  }
  function cityTip(w, c) {
    const m = w.markets[c.marketId];
    const isCap = [...(w.orgOrder || [])].some(oid => { const o = w.orgs[oid]; return o && o.territory && o.territory[0] === c.regionId; });
    const r = w.regions.find(x => x.id === c.regionId);
    return `<b>${c.name}</b> <span class="tt-dim">${isCap ? 'Capital' : 'City'} · pop ${(r ? r.population : c.population) || 0}</span><br>
      <span class="tt-dim">${m ? Object.keys(m.prices).map(k => k[0].toUpperCase() + ': ' + m.prices[k].toFixed(1)).join(' · ') : ''}</span>`;
  }
  function agentTip(w, a) {
    const rid = regionOf(a);
    const r = w.regions.find(x => x.id === rid);
    const org = a.org && w.orgs[a.org] ? w.orgs[a.org].name : 'Independent';
    return `<b>${a.name}</b> <span class="tt-dim">${a.archetype}</span><br>
      <span class="tt-dim">${r ? r.name : rid} · ${a.status || ''}</span><br>
      <span class="tt-dim">👛 ${fmt(a.inv.credits || 0)} Cr · ${agentValue(a)} vp · ${esc(org)}</span><br>
      <span class="tt-dim">${a.planGoal || ''}</span>`;
  }

  /* ================= static terrain ================= */
  function buildEdges(regions) {
    const edgeMap = new Map();
    const keyOf = (a, b) => {
      const r2 = v => Math.round(v * 100) / 100;
      const ax = r2(a[0]), ay = r2(a[1]), bx = r2(b[0]), by = r2(b[1]);
      return (ax < bx || (ax === bx && ay < by)) ? `${ax},${ay}|${bx},${by}` : `${bx},${by}|${ax},${ay}`;
    };
    for (const r of regions) {
      const pts = r.poly;
      if (!pts || pts.length < 3) continue;
      for (let i = 0; i < pts.length; i++) {
        const a = pts[i], b = pts[(i + 1) % pts.length];
        const k = keyOf(a, b);
        let e = edgeMap.get(k);
        if (!e) { e = { a, b, regions: [] }; edgeMap.set(k, e); }
        if (!e.regions.includes(r.id)) e.regions.push(r.id);
      }
    }
    return edgeMap;
  }

  function buildStatic(w) {
    if (!ro) { watchSize(); applySvgSize(); setTimeout(initView, 0); }
    stat.innerHTML = '';
    regionGeom.clear(); cityGeom.length = 0; infraGeom.length = 0; routeGeom.length = 0;
    frontCount = 0;

    let minX = 1e9, minY = 1e9, maxX = -1e9, maxY = -1e9;
    for (const r of w.map.landRegions) for (const [x, y] of r.poly) {
      if (x < minX) minX = x; if (x > maxX) maxX = x;
      if (y < minY) minY = y; if (y > maxY) maxY = y;
    }
    if (minX < maxX && minY < maxY) B = { minX, minY, maxX, maxY };

    const bg = el('rect', { x: 0, y: 0, width: 1000, height: 620, fill: 'url(#oceanGrad)' });
    stat.appendChild(bg);

    const graticule = el('g', { opacity: 0.05 });
    for (let gx = 0; gx <= 1000; gx += 200) {
      graticule.appendChild(el('line', { x1: gx, y1: 0, x2: gx, y2: 620, stroke: '#8fb4e8', 'stroke-width': 1, 'stroke-dasharray': '1 10' }));
    }
    for (let gy = 0; gy <= 620; gy += 160) {
      graticule.appendChild(el('line', { x1: 0, y1: gy, x2: 1000, y2: gy, stroke: '#8fb4e8', 'stroke-width': 1, 'stroke-dasharray': '1 10' }));
    }
    stat.appendChild(graticule);

    const cx0 = (B.minX + B.maxX) / 2, cy0 = (B.minY + B.maxY) / 2;
    const span = Math.max(B.maxX - B.minX, B.maxY - B.minY);
    const contour = el('g', { opacity: 0.5 });
    [0.62, 0.78, 0.94].forEach((rr, i) => {
      contour.appendChild(el('ellipse', { cx: cx0, cy: cy0, rx: span * rr * 0.78, ry: span * rr * 0.62, fill: 'none', stroke: `rgba(92,200,255,${0.16 - i * 0.045})`, 'stroke-width': 1 }));
    });
    stat.appendChild(contour);

    const disputedIds = new Set((w.fact.disputes || []).filter(d => d.state === 'active').map(d => d.regionId));

    const pathOf = (d, fill, sw) => {
      const p = el('path', { d });
      if (fill) p.setAttribute('fill', fill);
      p.setAttribute('stroke', 'none');
      if (sw) p.setAttribute('stroke-width', sw);
      return p;
    };

    const rEls = [];
    for (const r of w.regions) {
      if (!r.land) continue;
      const d = r.poly.length ? 'M' + r.poly.map(p => p[0] + ',' + p[1]).join('L') + 'Z' : '';
      const g = el('g', { 'data-region': r.id });
      const owner = ownerOf(w, r.id);
      const undiscovered = !r.explored;

      if (undiscovered) {
        const p = pathOf(d, '#0e1320');
        p.setAttribute('class', 'fog');
        p.setAttribute('stroke', 'rgba(148,178,226,0.3)');
        p.setAttribute('stroke-width', 0.8);
        p.setAttribute('stroke-dasharray', '3 3');
        g.appendChild(p);
      } else {
        const baseFill = owner ? mix(BIOME_FILL[r.biome] || '#111', '#0a0e16', 0.18) : (BIOME_FILL[r.biome] || '#111');
        const base = pathOf(d, baseFill);
        base.setAttribute('stroke', owner ? rgba(orgColor(w, owner), 0.5) : 'rgba(148,178,226,0.1)');
        base.setAttribute('stroke-width', owner ? 1.2 : 0.7);
        g.appendChild(base);
        g.appendChild(pathOf(d, 'url(#terrainShade)'));
        if (owner) {
          const tint = pathOf(d, rgba(orgColor(w, owner), 0.08));
          tint.setAttribute('class', 'terr');
          g.appendChild(tint);
        }
        if (disputedIds.has(r.id)) {
          const hat = pathOf(d, 'url(#hatchRed)');
          hat.setAttribute('class', 'disputed');
          g.appendChild(hat);
        }
      }

      if (undiscovered) {
        const t = el('text', { x: r.x, y: r.y, 'text-anchor': 'middle', fill: '#46587a', 'font-size': 18, 'font-family': 'Rajdhani', 'font-weight': 700, opacity: 0.9 });
        t.textContent = '?';
        g.appendChild(t);
      } else if (r.name) {
        const t = el('text', { x: r.x, y: r.y + 3, 'text-anchor': 'middle', fill: 'rgba(190,210,240,0.42)', 'font-size': 8.5, 'font-family': 'Rajdhani', 'letter-spacing': '1.5' });
        t.textContent = r.name.toUpperCase();
        g.appendChild(t);
      }

      const hov = () => {
        highlightTerritory(r.id);
        showTipAt({ kind: 'region', id: r.id, tip: regionTip(w, r) }, r.x, r.y);
      };
      g.addEventListener('pointerenter', hov);
      g.addEventListener('pointerleave', () => { clearHighlight(); tooltip.style.display = 'none'; });
      rEls.push(g);
      regionGeom.set(r.id, { id: r.id, x: r.x, y: r.y, g, path: r.poly.length ? g.children[0] : null });
    }
    for (const g of rEls) stat.appendChild(g);

    const edgeMap = buildEdges(w.map.landRegions);
    const fronts = el('g', { 'pointer-events': 'none' });
    const coastG = el('g', { 'pointer-events': 'none' });
    for (const e of edgeMap.values()) {
      if (e.regions.length === 2) {
        const [ra, rb] = e.regions;
        const oa = ownerOf(w, ra), ob = ownerOf(w, rb);
        const line = el('line', { x1: e.a[0], y1: e.a[1], x2: e.b[0], y2: e.b[1] });
        if (oa && ob && oa !== ob) {
          fronts.appendChild(el('line', { x1: e.a[0], y1: e.a[1], x2: e.b[0], y2: e.b[1], stroke: '#fff', 'stroke-width': 5.5, opacity: 0.26, filter: 'url(#glowSoft)' }));
          line.setAttribute('class', 'front-rival');
          line.setAttribute('stroke', '#ff9e5c');
          line.setAttribute('stroke-width', 2.8);
          line.setAttribute('filter', 'url(#glowA)');
          frontCount++;
        } else if ((oa && !ob) || (!oa && ob)) {
          const oc = oa || ob;
          line.setAttribute('class', 'front-edge');
          line.setAttribute('stroke', rgba(orgColor(w, oc), 0.85));
          line.setAttribute('stroke-width', 1.1);
          line.setAttribute('stroke-dasharray', '3 5');
        } else {
          continue;
        }
        fronts.appendChild(line);
      } else if (e.regions.length === 1) {
        const r = w.regions.find(x => x.id === e.regions[0]);
        if (r && r.land && r.explored) {
          coastG.appendChild(el('line', { x1: e.a[0], y1: e.a[1], x2: e.b[0], y2: e.b[1], class: 'coast', stroke: 'rgba(120,205,255,0.55)', 'stroke-width': 1.4 }));
        }
      }
    }
    stat.appendChild(fronts);
    stat.appendChild(coastG);

    for (const rt of w.map.routes) {
      const a = w.regions.find(x => x.id === rt.a), b = w.regions.find(x => x.id === rt.b);
      if (!a || !b) continue;
      const line = el('line', { x1: a.x, y1: a.y, x2: b.x, y2: b.y, 'pointer-events': 'none' });
      if (rt.kind === 'pipe') {
        line.setAttribute('class', 'route-pipe');
        line.setAttribute('stroke', 'rgba(92,200,255,0.5)');
        line.setAttribute('stroke-width', 1.3);
      } else if (rt.kind === 'shipping') {
        line.setAttribute('stroke', 'rgba(157,140,255,0.35)');
        line.setAttribute('stroke-width', 0.9);
        line.setAttribute('stroke-dasharray', '4 5');
      } else {
        line.setAttribute('stroke', 'rgba(148,178,226,0.12)');
        line.setAttribute('stroke-width', 0.7);
      }
      stat.appendChild(line);
      routeGeom.push(line);
    }

    const capitals = new Set();
    for (const oid of w.orgOrder) {
      const o = w.orgs[oid];
      if (!o || !o.territory || !o.territory.length) continue;
      const home = w.regions.find(r => r.id === o.territory[0]);
      if (home && home.cityId) capitals.add(home.cityId);
    }

    for (const c of w.map.cities) {
      const g = el('g', { 'pointer-events': 'none' });
      const isCap = capitals.has(c.id);
      const owner = ownerOf(w, c.regionId);
      const col = isCap ? orgColor(w, owner) : '#6fd6ff';
      g.appendChild(el('circle', { cx: c.x, cy: c.y, r: isCap ? 22 : 17, fill: rgba(col, 0.07), class: 'city-pulse' }));
      g.appendChild(el('circle', { cx: c.x, cy: c.y, r: isCap ? 14 : 11, fill: 'none', stroke: rgba(col, isCap ? 0.5 : 0.16), 'stroke-width': 1 }));
      const ctr = el('circle', { cx: c.x, cy: c.y, r: isCap ? 6 : 5, fill: isCap ? rgba(col, 0.25) : '#0d1830', stroke: col, 'stroke-width': isCap ? 2 : 1.6, filter: 'url(#glowA)' });
      g.appendChild(ctr);
      if (isCap) {
        const star = el('text', { x: c.x, y: c.y + 4.5, 'text-anchor': 'middle', 'font-size': 12, fill: col });
        star.textContent = '★';
        g.appendChild(star);
      }
      const label = el('text', { x: c.x, y: c.y - (isCap ? 19 : 13), 'text-anchor': 'middle', fill: isCap ? col : '#b7d9f2', 'font-size': isCap ? 12 : 10.5, 'font-family': 'Rajdhani', 'letter-spacing': '1.5' });
      label.textContent = c.name.toUpperCase();
      g.appendChild(label);
      g.addEventListener('pointerenter', () => showTipAt({ kind: 'city', id: c.id, tip: cityTip(w, c) }, c.x, c.y));
      g.addEventListener('pointerleave', () => { tooltip.style.display = 'none'; });
      stat.appendChild(g);
      cityGeom.push({ id: c.id, x: c.x, y: c.y, g });
    }

    const resG = el('g', { 'pointer-events': 'none', opacity: 0.55 });
    for (const r of w.map.landRegions) {
      for (const n of r.nodes || []) {
        let dom = 'materials', best = -1;
        for (const k of Object.keys(RES_DOT)) {
          if ((n.stock[k] || 0) > best) { best = n.stock[k]; dom = k; }
        }
        resG.appendChild(el('circle', { cx: n.x, cy: n.y, r: 2, fill: RES_DOT[dom] }));
      }
    }
    stat.appendChild(resG);

    for (const r of w.regions) {
      if (!r.land) continue;
      const syms = Object.keys(INFRA_SYM).filter(k => r.infra[k] > 0);
      let off = 0;
      for (const s of syms) {
        const g = el('g', { 'pointer-events': 'none' });
        g.appendChild(el('circle', { cx: r.x + 12, cy: r.y - 8 + off, r: 6, fill: 'rgba(4,8,16,0.75)', stroke: 'rgba(148,178,226,0.18)', 'stroke-width': 0.6 }));
        const t = el('text', { x: r.x + 12, y: r.y - 5 + off, 'text-anchor': 'middle', 'font-size': 7.5, fill: INFRA_SYM[s] === '◉' ? '#5cc8ff' : INFRA_SYM[s] === '▲' ? '#ffb454' : '#8fa3c4' });
        t.textContent = INFRA_SYM[s];
        g.appendChild(t);
        stat.appendChild(g);
        infraGeom.push(g);
        off += 13;
      }
    }

    for (const r of w.map.landRegions) {
      if (!r.explored) continue;
      glyphDecor(decorG, r);
    }
    disputeCount = disputedIds.size;

    S.sea = [];
    const sstep = 48;
    for (let gy = 0; gy * sstep < 620; gy++) {
      const row = [];
      for (let gx = 0; gx * sstep < 1000; gx++) {
        const px = gx * sstep + sstep / 2, py = gy * sstep + sstep / 2;
        let onLand = false;
        for (const r of w.map.landRegions) {
          if (pointInPoly(px, py, r.poly)) { onLand = true; break; }
        }
        if (!onLand) row.push({ x: px, y: py });
      }
      if (row.length) S.sea.push(row);
    }
  }

  function highlightTerritory(rid) {
    const owner = ownerOf(world, rid);
    if (!owner) return;
    for (const [id2, geo] of regionGeom) {
      if (id2 === rid) continue;
      const o2 = ownerOf(world, id2);
      geo.g.classList.toggle('dimmed', o2 !== owner);
    }
  }
  function clearHighlight() {
    for (const [, geo] of regionGeom) geo.g.classList.remove('dimmed');
  }

  /* ================= living layers ================= */
  function glyphDecor(g, r) {
    switch (r.biome) {
      case 'forest': {
        const cnt = 5 + Math.floor(hashH(r.id + 'fa') * 5);
        for (let i = 0; i < cnt; i++) {
          const px = r.x + (hashH(r.id + 'fx' + i) * 2 - 1) * 34;
          const py = r.y + (hashH(r.id + 'fy' + i) * 2 - 1) * 26;
          const s = 1.6 + hashH(r.id + 'fs' + i) * 2.2;
          g.appendChild(el('path', { d: `M${px},${py} L${px - s},${py + s * 1.4} L${px + s},${py + s * 1.4} Z`, fill: 'rgba(22,64,48,0.8)' }));
        }
        break;
      }
      case 'mountain': {
        const cnt = 3 + Math.floor(hashH(r.id + 'ma') * 4);
        for (let i = 0; i < cnt; i++) {
          const px = r.x + (hashH(r.id + 'mx' + i) * 2 - 1) * 36;
          const py = r.y + (hashH(r.id + 'my' + i) * 2 - 1) * 26;
          const s = 3 + hashH(r.id + 'ms' + i) * 3;
          g.appendChild(el('path', { d: `M${px - s},${py + s * 0.9} L${px},${py - s * 0.7} L${px + s},${py + s * 0.9} Z`, fill: 'rgba(120,140,175,0.42)' }));
          g.appendChild(el('path', { d: `M${px - s * 0.3},${py - s * 0.05} L${px},${py - s * 0.7} L${px + s * 0.3},${py - s * 0.05} Z`, fill: 'rgba(235,244,255,0.4)' }));
        }
        break;
      }
      case 'desert': {
        const cnt = 6 + Math.floor(hashH(r.id + 'da') * 5);
        for (let i = 0; i < cnt; i++) {
          const px = r.x + (hashH(r.id + 'dx' + i) * 2 - 1) * 38;
          const py = r.y + (hashH(r.id + 'dy' + i) * 2 - 1) * 28;
          g.appendChild(el('circle', { cx: px, cy: py, r: 1 + hashH(r.id + 'ds' + i) * 1.6, fill: 'rgba(224,196,140,0.38)' }));
        }
        break;
      }
      case 'wetland': {
        const cnt = 3 + Math.floor(hashH(r.id + 'wa') * 4);
        for (let i = 0; i < cnt; i++) {
          const px = r.x + (hashH(r.id + 'wx' + i) * 2 - 1) * 30;
          const py = r.y + (hashH(r.id + 'wy' + i) * 2 - 1) * 22;
          g.appendChild(el('line', { x1: px, y1: py + 3, x2: px, y2: py - 3, stroke: 'rgba(80,180,160,0.55)', 'stroke-width': 1 }));
        }
        break;
      }
      case 'tundra': {
        const cnt = 4 + Math.floor(hashH(r.id + 'ta') * 4);
        for (let i = 0; i < cnt; i++) {
          const px = r.x + (hashH(r.id + 'tx' + i) * 2 - 1) * 36;
          const py = r.y + (hashH(r.id + 'ty' + i) * 2 - 1) * 26;
          g.appendChild(el('circle', { cx: px, cy: py, r: 1, fill: 'rgba(220,235,255,0.45)' }));
        }
        break;
      }
      default: {
        const cnt = 4 + Math.floor(hashH(r.id + 'pa') * 4);
        for (let i = 0; i < cnt; i++) {
          const px = r.x + (hashH(r.id + 'px' + i) * 2 - 1) * 36;
          const py = r.y + (hashH(r.id + 'py' + i) * 2 - 1) * 26;
          g.appendChild(el('line', { x1: px, y1: py + 2, x2: px, y2: py - 1, stroke: 'rgba(120,200,140,0.35)', 'stroke-width': 0.9 }));
        }
      }
    }
  }

  function glyphDistrict(g, r, kind) {
    const hx = (hashH(r.id + 'dgx') * 2 - 1) * 26;
    const hy = (hashH(r.id + 'dgy') * 2 - 1) * 18 + 6;
    const x = r.x + hx, y = r.y + hy;
    const c = DISTRICT[kind].color;
    switch (kind) {
      case 'market':
        for (let i = -1; i <= 1; i++) {
          const tx = x + i * 6, ty = y + 3;
          g.appendChild(el('path', { d: `M${tx - 4},${ty + 4} L${tx},${ty - 4} L${tx + 4},${ty + 4} Z`, fill: 'none', stroke: c, 'stroke-width': 1.1, opacity: 0.85 }));
        }
        break;
      case 'industry':
        g.appendChild(el('rect', { x: x - 5, y: y - 2, width: 10, height: 7, fill: 'rgba(10,16,28,0.9)', stroke: c, 'stroke-width': 1, opacity: 0.9 }));
        g.appendChild(el('rect', { x: x + 1.5, y: y - 7, width: 2.4, height: 5, fill: c, opacity: 0.8 }));
        break;
      case 'institute':
        g.appendChild(el('path', { d: `M${x},${y - 7} L${x + 6},${y} L${x},${y + 7} L${x - 6},${y} Z`, fill: 'rgba(92,200,255,0.12)', stroke: c, 'stroke-width': 1.2, opacity: 0.95, filter: 'url(#glowA)' }));
        break;
      case 'fields':
        for (let i = 0; i < 4; i++) g.appendChild(el('line', { x1: x - 6, y1: y - 5 + i * 3, x2: x + 6, y2: y - 5 + i * 3, stroke: c, 'stroke-width': 0.9, opacity: 0.7 }));
        break;
      case 'mine':
        g.appendChild(el('line', { x1: x - 4, y1: y + 5, x2: x + 4, y2: y - 5, stroke: c, 'stroke-width': 1.4, opacity: 0.9 }));
        g.appendChild(el('line', { x1: x + 4, y1: y - 5, x2: x + 8, y2: y - 1, stroke: c, 'stroke-width': 1.4, opacity: 0.9 }));
        g.appendChild(el('rect', { x: x - 1.5, y: y - 7, width: 3, height: 6, fill: c, opacity: 0.85 }));
        g.appendChild(el('line', { x1: x - 4, y1: y - 6, x2: x + 4, y2: y - 6, stroke: c, 'stroke-width': 1, opacity: 0.7 }));
        g.appendChild(el('line', { x1: x - 4, y1: y - 4, x2: x + 4, y2: y - 4, stroke: c, 'stroke-width': 1, opacity: 0.7 }));
        break;
      case 'arena':
        g.appendChild(el('line', { x1: x - 5, y1: y - 5, x2: x + 5, y2: y + 5, stroke: c, 'stroke-width': 1.5, opacity: 0.9 }));
        g.appendChild(el('line', { x1: x + 5, y1: y - 5, x2: x - 5, y2: y + 5, stroke: c, 'stroke-width': 1.5, opacity: 0.9 }));
        break;
      case 'homes':
        for (let i = -1; i <= 1; i++) {
          const hx2 = x + i * 5, hy2 = y;
          g.appendChild(el('rect', { x: hx2 - 3, y: hy2 - 2, width: 6, height: 5, fill: 'rgba(10,16,28,0.9)', stroke: c, 'stroke-width': 0.9, opacity: 0.85 }));
          g.appendChild(el('path', { d: `M${hx2 - 3.5},${hy2 - 2} L${hx2},${hy2 - 5} L${hx2 + 3.5},${hy2 - 2} Z`, fill: 'none', stroke: c, 'stroke-width': 0.9, opacity: 0.7 }));
        }
        break;
      default:
        g.appendChild(el('circle', { cx: x, cy: y, r: 1.6, fill: 'rgba(120,140,170,0.5)' }));
    }
  }

  function glyphSettlement(g, r, stage) {
    if (stage < 1) return;
    const pos = settlePos(r);
    const x = pos.x, y = pos.y;
    const house = (bx, by, s) => {
      g.appendChild(el('rect', { x: bx - 3 * s, y: by - 2 * s, width: 6 * s, height: 5 * s, fill: '#0f1a2e', stroke: 'rgba(255,196,107,0.55)', 'stroke-width': 0.8 }));
      g.appendChild(el('path', { d: `M${bx - 3.5 * s},${by - 2 * s} L${bx},${by - 5.2 * s} L${bx + 3.5 * s},${by - 2 * s} Z`, fill: 'rgba(255,196,107,0.25)', stroke: 'rgba(255,196,107,0.6)', 'stroke-width': 0.8 }));
    };
    if (stage >= 3) {
      g.appendChild(el('circle', { cx: x, cy: y, r: (stage + 2) * 4, fill: 'none', stroke: 'rgba(255,196,107,0.2)', 'stroke-width': 1 }));
    }
    switch (stage) {
      case 1: house(x, y + 1, 1); break;
      case 2: house(x - 4, y + 1, 1); house(x + 4, y + 1, 1); break;
      case 3: house(x - 7, y + 1, 1); house(x, y + 1, 1); house(x + 7, y + 1, 1); house(x - 3, y - 4, 1); house(x + 4, y - 4, 1); break;
      case 4:
        g.appendChild(el('rect', { x: x - 12, y: y - 6, width: 24, height: 13, fill: '#0d1526', stroke: 'rgba(255,196,107,0.7)', 'stroke-width': 1 }));
        for (const [tx, ty] of [[-12, y - 6], [12, y - 6], [-12, y + 7], [12, y + 7]]) {
          g.appendChild(el('rect', { x: tx - 3, y: ty - 3, width: 6, height: 6, fill: '#0d1526', stroke: 'rgba(255,196,107,0.7)', 'stroke-width': 1 }));
        }
        g.appendChild(el('rect', { x: x - 2, y: y + 4, width: 4, height: 4, fill: 'rgba(255,196,107,0.5)' }));
        break;
    }
  }

  function renderSettles(w) {
    settleG.innerHTML = '';
    for (const r of w.map.landRegions) {
      if (!r.explored) continue;
      glyphDistrict(settleG, r, districtOf(w, r));
      glyphSettlement(settleG, r, stageOf(r));
    }
  }

  function renderTrails(w) {
    trailG.innerHTML = '';
    const tr = w.map.trail;
    if (!tr) return;
    for (const key in tr) {
      const wgt = tr[key];
      if (wgt <= 0) continue;
      const [a, b] = key.split('|');
      const ra = w.regions.find(x => x.id === a), rb = w.regions.find(x => x.id === b);
      if (!ra || !rb || !ra.explored || !rb.explored) continue;
      const op = Math.min(1, wgt / 12) * 0.5;
      const sw = 0.5 + wgt * 0.055;
      trailG.appendChild(el('line', { x1: ra.x, y1: ra.y, x2: rb.x, y2: rb.y, stroke: 'rgba(0,0,0,0.45)', 'stroke-width': sw + 1.6, opacity: op * 0.6, 'stroke-linecap': 'round' }));
      trailG.appendChild(el('line', { x1: ra.x, y1: ra.y, x2: rb.x, y2: rb.y, stroke: 'rgba(255,218,158,0.9)', 'stroke-width': sw, opacity: op, 'stroke-linecap': 'round' }));
    }
  }

  function renderDyn(w) {
    dyn.innerHTML = '';
    for (const z of w.zones) {
      const r = w.regions.find(x => x.id === z.regionId);
      if (!r) continue;
      const zk = ZONE_KIND[z.kind] || ZONE_KIND.anomaly;
      dyn.appendChild(el('circle', { cx: r.x, cy: r.y, r: 34, fill: zk.color, opacity: 0.07 }));
      dyn.appendChild(el('circle', { cx: r.x, cy: r.y, r: 34, fill: 'none', stroke: zk.color, 'stroke-width': 1, opacity: 0.5, 'stroke-dasharray': '5 5', filter: 'url(#glowSoft)' }));
      const lbl = el('text', { x: r.x, y: r.y + 4, 'text-anchor': 'middle', 'font-size': 10, fill: zk.color });
      lbl.textContent = zk.sym + ' ' + zk.label.toUpperCase();
      dyn.appendChild(lbl);
    }
    for (const d of (w.fact.disputes || [])) {
      if (d.state !== 'active') continue;
      const r = w.regions.find(x => x.id === d.regionId);
      if (!r) continue;
      dyn.appendChild(el('circle', { cx: r.x, cy: r.y, r: 26, fill: 'none', stroke: '#ffd166', 'stroke-width': 1.6, class: 'dz-ring', filter: 'url(#glowA)' }));
      const lbl = el('text', { x: r.x, y: r.y - 28, 'text-anchor': 'middle', 'font-size': 11, fill: '#ffd166', class: 'dz-flag' });
      lbl.textContent = '⚔ DISPUTE';
      dyn.appendChild(lbl);
    }
    for (const ct of Object.values(w.contracts)) {
      if (ct.state !== 'active') continue;
      const target = contractTarget(w, ct);
      if (!target) continue;
      dyn.appendChild(el('circle', { cx: target.x, cy: target.y, r: 13, fill: 'none', stroke: '#ffd166', 'stroke-width': 1.4, 'stroke-dasharray': '3 3', opacity: 0.8, filter: 'url(#glowA)' }));
    }
    for (const aid of w.agentOrder) {
      const a = w.agents[aid];
      if (!a || a.status !== 'building') continue;
      const st = a.plans && a.plans[a.stepIx];
      const rid = (st && st.regionId) || a.loc.regionId;
      const r = w.regions.find(x => x.id === rid);
      if (!r) continue;
      const p = clamp((st && st.progress || 0) / Math.max(1, st && st.ticks || 1), 0, 1);
      const pos = settlePos(r);
      const circ = 2 * Math.PI * 16;
      dyn.appendChild(el('circle', { cx: pos.x, cy: pos.y, r: 16, fill: 'none', stroke: '#ffd166', 'stroke-width': 2.6, opacity: 0.9, filter: 'url(#glowA)', 'stroke-dasharray': `${(circ * p).toFixed(1)} ${(circ * 2).toFixed(1)}`, transform: 'rotate(-90 16 16)' }));
      const ic = el('text', { x: pos.x, y: pos.y + 4, 'text-anchor': 'middle', 'font-size': 10, fill: '#ffd166' });
      ic.textContent = '▲';
      dyn.appendChild(ic);
    }
  }

  function contractTarget(w, ct) {
    const obj = ct.objective;
    if (obj.kind === 'deliver') { const c = w.map.cities.find(x => x.id === obj.toCityId); return c; }
    if (obj.kind === 'explore' || obj.kind === 'investigate' || obj.kind === 'build' || obj.kind === 'defend') return w.regions.find(r => r.id === obj.regionId);
    if (obj.kind === 'research') return w.map.landRegions.find(r => r.infra.labs > 0);
    return null;
  }

  /* ================= event flashes ================= */
  function createFlash(e, rid) {
    const r = world.regions.find(x => x.id === rid);
    if (!r) return;
    const info = EV_INFO[e.type] || ['◆', '#9d8cff'];
    const g = el('g', { class: 'ev-flash', 'pointer-events': 'none', transform: `translate(${r.x} ${r.y})` });
    const ring = el('circle', { cx: 0, cy: 0, r: 10, fill: 'none', stroke: info[1], 'stroke-width': 1.4 });
    const icon = el('text', { x: 0, y: 4, 'text-anchor': 'middle', 'font-size': 15, fill: info[1], opacity: 0 });
    icon.textContent = info[0];
    const label = el('text', { x: 0, y: -10, 'text-anchor': 'middle', 'font-size': 9, fill: info[1], opacity: 0, 'font-family': 'Rajdhani', 'letter-spacing': '1px' });
    label.textContent = (e.type.replace(/-/g, ' ')).toUpperCase().slice(0, 22);
    g.appendChild(ring); g.appendChild(icon); g.appendChild(label);
    evG.appendChild(g);
    flashes.push({ g, ring, icon, label, x: r.x, y: r.y, born: performance.now() });
    if (e.type === 'discovery') burst(r.x, r.y, 16, '#55e6a4', 'stardust');
    else if (e.type === 'breakthrough' || e.type === 'compute-used') burst(r.x, r.y, 10, '#5cc8ff', 'spark');
    else if (e.type === 'construction') burst(r.x, r.y, 8, '#ffd166', 'chip');
    else if (e.type === 'conflict' || e.type === 'political-conflict' || e.type === 'org-collapsed' || e.type === 'infra-failure') burst(r.x, r.y, 12, '#ff5470', 'spark');
    else if (e.type === 'org-founded') burst(r.x, r.y, 14, '#9d8cff', 'stardust');
  }
  function scanEvents(w) {
    const evs = w.events;
    const start = Math.max(0, evs.length - 24);
    let added = 0;
    for (let i = start; i < evs.length; i++) {
      const e = evs[i];
      if (!e || evSeen.has(e.id)) continue;
      evSeen.add(e.id);
      if (e.tick < w.clock.t - 6) continue;
      let rid = e.regionId;
      if (!rid && e.actor && w.agents[e.actor]) rid = regionOf(w.agents[e.actor]);
      if (!rid || added >= 5) continue;
      createFlash(e, rid);
      added++;
    }
  }
  function updateFlashes(now) {
    for (let i = flashes.length - 1; i >= 0; i--) {
      const f = flashes[i];
      const age = (now - f.born) / 1000;
      if (age >= 6) { f.g.remove(); flashes.splice(i, 1); continue; }
      const a = 1 - age / 6;
      f.g.setAttribute('transform', `translate(${f.x} ${f.y - age * 5}) scale(${1 + age * 0.5})`);
      f.ring.setAttribute('r', (10 + age * 8).toFixed(1));
      f.ring.setAttribute('opacity', (a * 0.8).toFixed(2));
      f.icon.setAttribute('opacity', Math.min(1, a * 2.2).toFixed(2));
      f.label.setAttribute('opacity', (a > 0.6 ? ((1 - a) / 0.4) : 0).toFixed(2));
    }
  }

  /* ================= particles ================= */
  function spawn(p) { if (particles.length < 240) particles.push(Object.assign({ life: 0, maxLife: 1.2, grav: 0, fade: 1, size: 2 }, p)); }
  function burst(x, y, n, color, kind) {
    for (let i = 0; i < n; i++) {
      const ang = Math.random() * Math.PI * 2;
      const sp = 8 + Math.random() * 22;
      spawn({ x, y, vx: Math.cos(ang) * sp, vy: Math.sin(ang) * sp - 6, color, kind, maxLife: 0.7 + Math.random() * 0.7, size: 1.5 + Math.random() * 2, fade: 0.9 });
    }
  }
  function scanActivity(w) {
    const acts = w.activity || [];
    const from = Math.max(lastActSeen, acts.length - 6);
    for (let i = from; i < acts.length; i++) {
      const ac = acts[i];
      if (!ac) continue;
      const rid = ac.regionId;
      const r = rid && w.regions.find(x => x.id === rid);
      if (!r) continue;
      if (ac.kind === 'explore') burst(r.x, r.y, 12, '#55e6a4', 'stardust');
      else if (ac.kind === 'build') burst(r.x, r.y, 8, '#ffd166', 'chip');
      else if (ac.kind === 'contest' || ac.kind === 'sabotage') burst(r.x, r.y, 10, '#ff5470', 'spark');
      else if (ac.kind === 'research') burst(r.x, r.y, 6, '#5cc8ff', 'spark');
      else if (ac.kind === 'trade') {
        const c = ac.cityId ? w.map.cities.find(x => x.id === ac.cityId) : null;
        const a = w.agents[ac.agentId];
        const src = a ? agentUi.get(a.id) : null;
        if (c && src) spawn({ kind: 'orb', x: src.x, y: src.y, tx: c.x, ty: c.y, color: '#ffd166', maxLife: 1.6, size: 2.4, fade: 1 });
      } else if (ac.kind === 'compute') {
        const a = w.agents[ac.agentId];
        const src = a ? agentUi.get(a.id) : null;
        if (src) spawn({ kind: 'orb', x: src.x, y: src.y, tx: src.x + (Math.random() * 2 - 1) * 6, ty: src.y - 60, color: '#5cc8ff', maxLife: 1.3, size: 2, fade: 1 });
      } else if (ac.kind === 'gather' || ac.kind === 'mine') {
        burst(r.x + (hashH(ac.agentId + i) * 2 - 1) * 20, r.y + 12, 3, ac.kind === 'mine' ? '#c67bff' : '#55e6a4', 'spark');
      }
    }
    lastActSeen = acts.length;
  }
  function emitters(now) {
    if (!world) return;
    for (const r of world.map.landRegions) {
      if (!r.explored) continue;
      const last = S.lastEm[r.id] || 0;
      if (now - last < 700) continue;
      let kind = null, col = null;
      if (r.infra.factories > 0) { kind = 'smoke'; col = '#9aa5b8'; }
      else if (r.infra.refineries > 0) { kind = 'ember'; col = '#ff8a5c'; }
      else if (r.infra.labs > 0 && Math.random() < 0.6) { kind = 'stardust'; col = '#5cc8ff'; }
      else if ((world.fact.disputes || []).some(d => d.state === 'active' && d.regionId === r.id) && Math.random() < 0.8) { kind = 'ember'; col = '#ff5470'; }
      if (!kind) continue;
      S.lastEm[r.id] = now;
      const ex = r.x + (hashH(r.id + 'ex') * 2 - 1) * 16;
      const ey = r.y - 10 + (hashH(r.id + 'ey') * 2 - 1) * 10;
      spawn({ x: ex, y: ey, vx: (Math.random() - 0.5) * 2, vy: kind === 'smoke' ? -6 : -12, kind, color: col, maxLife: kind === 'smoke' ? 2.4 : 1.6, size: kind === 'smoke' ? 3.5 : 1.6, fade: 1 });
    }
    if (now - (S.lastWave || 0) > 1100) {
      S.lastWave = now;
      const oceans = world.regions.filter(x => !x.land);
      if (oceans.length) {
        const o = oceans[Math.floor(hashH('wv' + Math.floor(now / 1100)) * 997) % oceans.length];
        spawn({ x: o.x + (Math.random() - 0.5) * 24, y: o.y + (Math.random() - 0.5) * 10, vx: (Math.random() - 0.5) * 8, vy: (Math.random() - 0.5) * 1, kind: 'wave', color: '#8fc8ff', maxLife: 2.6, size: 1.4, fade: 0.45 });
      }
    }
  }
  function updateParticles(dt) {
    for (let i = particles.length - 1; i >= 0; i--) {
      const p = particles[i];
      p.life += dt / 1000;
      if (p.life >= p.maxLife) { particles.splice(i, 1); continue; }
      if (p.kind === 'orb') {
        const dx = p.tx - p.x, dy = p.ty - p.y;
        const d = Math.hypot(dx, dy);
        const step = (d / p.maxLife) * (dt / 1000);
        if (d <= step + 0.1) { p.x = p.tx; p.y = p.ty; }
        else { p.x += dx / d * step; p.y += dy / d * step; }
        if (!p.arrived && p.life >= p.maxLife * 0.92) { p.arrived = 1; burst(p.tx, p.ty, 4, p.color, 'spark'); }
        continue;
      }
      p.x += p.vx * dt / 1000;
      p.y += p.vy * dt / 1000;
      if (p.grav) p.vy += p.grav * dt / 1000;
      if (p.kind === 'smoke') { p.vx *= 0.985; p.vy *= 0.985; p.size += dt * 0.004; }
    }
  }
  function drawParticles(ctx) {
    for (const p of particles) {
      const a = clamp(1 - p.life / p.maxLife, 0, 1) * (p.fade || 1);
      if (a <= 0) continue;
      ctx.globalAlpha = a;
      if (p.kind === 'smoke') {
        ctx.fillStyle = 'rgba(160,172,192,0.55)';
        ctx.beginPath(); ctx.arc(p.x, p.y, p.size * (0.6 + p.life * 0.8), 0, 7); ctx.fill();
      } else if (p.kind === 'ember') {
        ctx.fillStyle = p.color;
        ctx.beginPath(); ctx.arc(p.x, p.y, p.size * (0.7 + Math.sin(p.life * 18) * 0.3), 0, 7); ctx.fill();
      } else if (p.kind === 'stardust') {
        ctx.fillStyle = p.color;
        const tw = 0.6 + Math.sin(p.life * 22 + p.x) * 0.4;
        ctx.beginPath(); ctx.arc(p.x, p.y, p.size * tw, 0, 7); ctx.fill();
      } else if (p.kind === 'spark') {
        ctx.strokeStyle = p.color; ctx.lineWidth = 1;
        ctx.beginPath(); ctx.moveTo(p.x, p.y); ctx.lineTo(p.x - p.vx * 0.03, p.y - p.vy * 0.03); ctx.stroke();
      } else if (p.kind === 'chip') {
        ctx.fillStyle = p.color;
        ctx.fillRect(p.x - 1.5, p.y - 1.5, 3, 3);
      } else if (p.kind === 'snow') {
        ctx.fillStyle = 'rgba(230,240,255,0.8)';
        ctx.beginPath(); ctx.arc(p.x, p.y, p.size, 0, 7); ctx.fill();
      } else if (p.kind === 'wave') {
        ctx.strokeStyle = p.color; ctx.lineWidth = 1;
        ctx.beginPath(); ctx.moveTo(p.x - 6, p.y); ctx.quadraticCurveTo(p.x, p.y - 2, p.x + 6, p.y); ctx.stroke();
      } else {
        ctx.fillStyle = p.color;
        ctx.beginPath(); ctx.arc(p.x, p.y, p.size, 0, 7); ctx.fill();
      }
    }
    ctx.globalAlpha = 1;
  }

  /* ================= agents ================= */
  function computeTarget(a) {
    const tgt = { x: 0, y: 0, heading: null, traveling: false };
    if (a.loc.travel && a.loc.travel.path) {
      const path = a.loc.travel.path;
      const idx = a.loc.travel.idx;
      const A = world.regions.find(x => x.id === path[idx]);
      const B = world.regions.find(x => x.id === path[idx + 1]);
      if (A && B) {
        const leg = Math.hypot(B.x - A.x, B.y - A.y) || 1;
        const tt = clamp((a.loc.travel.dist || 0) / leg, 0, 1);
        tgt.x = A.x + (B.x - A.x) * tt;
        tgt.y = A.y + (B.y - A.y) * tt;
        tgt.heading = Math.atan2(B.y - A.y, B.x - A.x);
        tgt.traveling = true;
      }
    }
    if (!tgt.traveling) {
      const rid = a.loc.regionId;
      const r = world.regions.find(x => x.id === rid);
      if (r) {
        const j = jitter(a.id);
        tgt.x = r.x + j.dx; tgt.y = r.y + j.dy;
      }
    }
    return tgt;
  }
  function ensureAgent(a) {
    let gel = agentGels.get(a.id);
    if (gel) return gel;
    const shape = SHAPE_D[ARCH_SHAPE[a.archetype]] || SHAPE_D.circle;
    const g = el('g', { 'data-agent': a.id, class: 'agent-gel', transform: 'translate(0 0)' });
    const halo = el('circle', { class: 'agent-halo', r: 9, fill: a.color, opacity: 0.3, filter: 'url(#glowSoft)' });
    const fring = el('circle', { r: 13, fill: 'none', stroke: '#ffffff', 'stroke-width': 1, 'stroke-dasharray': '3 3', opacity: 0 });
    const body = el('path', { d: shape, fill: rgba(a.color, 0.22), stroke: a.color, 'stroke-width': 1.6, filter: 'url(#glowA)' });
    const avatar = el('text', { x: 0, y: 3.4, 'text-anchor': 'middle', 'font-size': 9.5, fill: a.color, 'font-family': 'Rajdhani' });
    avatar.textContent = a.avatar;
    const wedge = el('path', { d: 'M7,-3.2 L11,0 L7,3.2 Z', fill: a.color, opacity: 0, 'pointer-events': 'none' });
    const job = el('text', { x: 0, y: -12, 'text-anchor': 'middle', 'font-size': 8.5, fill: '#ffd166', 'font-family': 'Rajdhani', opacity: 0, 'pointer-events': 'none' });
    const name = el('text', { x: 0, y: 17, 'text-anchor': 'middle', 'font-size': 9, fill: '#d6e4f7', 'font-family': 'Rajdhani', opacity: 0, 'pointer-events': 'none', 'letter-spacing': '0.5px' });
    name.textContent = a.name;
    g.appendChild(halo); g.appendChild(fring); g.appendChild(body); g.appendChild(avatar); g.appendChild(wedge); g.appendChild(job); g.appendChild(name);
    const ent = { g, halo, fring, body, avatar, wedge, job, name };
    g.addEventListener('pointerenter', (ev) => {
      hoveredAgent = a.id;
      const ui = agentUi.get(a.id);
      showTipAt({ kind: 'agent', id: a.id, tip: agentTip(world, a) }, ui ? ui.x : a.loc.regionId && world.regions.find(x => x.id === a.loc.regionId).x || 0, ui ? ui.y : 0);
    });
    g.addEventListener('pointerleave', () => { if (hoveredAgent === a.id) hoveredAgent = null; tooltip.style.display = 'none'; });
    agentG.appendChild(g);
    const tgt = computeTarget(a);
    agentUi.set(a.id, { x: tgt.x, y: tgt.y, phase: Math.random() * 6.28 });
    agentGels.set(a.id, ent);
    return ent;
  }
  function updateAgents(now) {
    const w = world;
    if (!w) return;
    const k = view.k;
    const isSelId = (sel && sel.kind === 'agent') ? sel.id : null;
    for (const aid of w.agentOrder) {
      const a = w.agents[aid];
      if (!a) continue;
      const ent = ensureAgent(a);
      const ui = agentUi.get(aid);
      const tgt = computeTarget(a);
      ui.x += (tgt.x - ui.x) * 0.22;
      ui.y += (tgt.y - ui.y) * 0.22;
      if (Math.abs(tgt.x - ui.x) < 0.04 && Math.abs(tgt.y - ui.y) < 0.04) { ui.x = tgt.x; ui.y = tgt.y; }
      const affil = (a.org && w.orgs[a.org]) ? w.orgs[a.org].color : a.color;
      const vis = (S.filter === 'all' || a.archetype === S.filter);
      ent.g.style.opacity = vis ? 1 : 0;
      ent.g.style.display = vis ? '' : 'none';
      ent.g.setAttribute('transform', `translate(${ui.x.toFixed(2)} ${ui.y.toFixed(2)})`);
      const isSel = aid === isSelId;
      const scale = clamp(1 + (a.wealth || 0) / 7000, 1, 1.5);
      ent.g.setAttribute('data-sel', isSel ? 1 : 0);
      ent.body.setAttribute('transform', `scale(${scale.toFixed(3)})`);
      ent.avatar.setAttribute('transform', `scale(${scale.toFixed(3)})`);
      ent.halo.setAttribute('fill', affil);
      ent.halo.setAttribute('r', ((isSel ? 12 : 9) + Math.sin(now / 180 + ui.phase) * 1.2).toFixed(1));
      ent.halo.setAttribute('stroke', isSel ? '#ffffff' : 'none');
      ent.halo.setAttribute('stroke-width', isSel ? 1 : 0);
      const following = S.followAgent === aid;
      ent.fring.setAttribute('opacity', following ? 0.85 : 0);
      ent.fring.setAttribute('stroke', affil);
      if (tgt.heading != null) {
        ent.wedge.setAttribute('transform', `rotate(${(tgt.heading * 180 / Math.PI).toFixed(1)})`);
        ent.wedge.setAttribute('opacity', 0.9);
        ent.wedge.setAttribute('fill', affil);
      } else {
        ent.wedge.setAttribute('opacity', 0);
      }
      const jg = STATUS_GLYPH[a.status] || '';
      ent.job.textContent = jg;
      ent.job.setAttribute('opacity', (k >= 1.4 || hoveredAgent === aid) ? 0.95 : 0);
      ent.name.setAttribute('opacity', (k >= 1.6 || hoveredAgent === aid) ? 0.92 : 0);
      ent.body.setAttribute('stroke', a.color);
      ent.avatar.setAttribute('fill', a.color);
    }
    for (const [aid, ent] of agentGels) {
      if (!w.agents[aid]) { ent.g.remove(); agentGels.delete(aid); agentUi.delete(aid); }
    }
  }

  /* ================= camera logic ================= */
  function hotTarget() {
    if (S.followAgent && world.agents[S.followAgent]) {
      const ui = agentUi.get(S.followAgent);
      if (ui) return { x: ui.x, y: ui.y, k: 2.7 };
    }
    const acts = world.activity || [];
    for (let i = acts.length - 1; i >= 0 && i >= acts.length - 12; i--) {
      const ac = acts[i];
      if (ac.t < world.clock.t - 5) break;
      if (!ac.agentId || !world.agents[ac.agentId]) continue;
      const ui = agentUi.get(ac.agentId);
      if (ui) return { x: ui.x, y: ui.y, k: 2.0 };
    }
    const evs = world.events;
    for (let i = evs.length - 1; i >= 0 && i >= evs.length - 14; i--) {
      const e = evs[i];
      if (!e || e.t < world.clock.t - 8) break;
      if (e.regionId) { const r = world.regions.find(x => x.id === e.regionId); if (r) return { x: r.x, y: r.y, k: 1.7 }; }
    }
    return null;
  }
  function frameCamera() {
    if (!world || !S.follow || userMoved || drag) return;
    const cw = svg.clientWidth || 1000, ch = svg.clientHeight || 620;
    const hot = hotTarget();
    if (hot) {
      const k = clamp(hot.k, 1.3, 3.2);
      view.x += (cw / 2 - hot.x * k - view.x) * 0.05;
      view.y += (ch / 2 - hot.y * k - view.y) * 0.05;
      view.k += (k - view.k) * 0.05;
      S.quiet = 0;
    } else {
      S.quiet++;
      if (S.quiet > 14 && S.fit) {
        view.x += (S.fit.x - view.x) * 0.04;
        view.y += (S.fit.y - view.y) * 0.04;
        view.k += (S.fit.k - view.k) * 0.04;
      }
    }
    applyView();
  }

  /* ================= fx overlay ================= */
  function roundRect(ctx, x, y, w, h, r) {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.arcTo(x + w, y, x + w, y + h, r);
    ctx.arcTo(x + w, y + h, x, y + h, r);
    ctx.arcTo(x, y + h, x, y, r);
    ctx.arcTo(x, y, x + w, y, r);
    ctx.closePath();
  }
  function computePipeVals(w) {
    const jobs = w.compute.jobOrder || [];
    let modelDone = 0, lastTask = 'IDLE';
    const n = Math.min(jobs.length, 400);
    for (let i = jobs.length - 1; i >= jobs.length - n; i--) {
      const j = w.compute.jobs[jobs[i]];
      if (!j) continue;
      if (j.status === 'done') {
        if (lastTask === 'IDLE') lastTask = (j.taskType || 'JOB').toUpperCase().slice(0, 5);
        if (j.capability === 'model') modelDone++;
      }
    }
    const ec = (w.balances['compute-pool'] || {}).computeCredits || 0;
    return {
      gov: lastTask,
      model: modelDone ? modelDone + ' jobs' : 'IDLE',
      cpu: (w.compute.stats.execs || 0) + ' exec',
      evid: (w.evidence && w.evidence.count) || 0,
      econ: ec >= 1000 ? (ec / 1000).toFixed(1) + 'k' : String(Math.round(ec)),
    };
  }
  function drawPipeline(ctx, cw, ch, now) {
    if (cw < 640) return;
    const w = world;
    if (!w || !w.compute) return;
    const x0 = cw / 2 - 150, y0 = ch - 32, ww = 300, h = 24;
    ctx.fillStyle = 'rgba(10,17,32,0.9)';
    roundRect(ctx, x0 - 8, y0 - 6, ww + 16, h + 12, 7); ctx.fill();
    ctx.strokeStyle = 'rgba(110,190,255,0.35)'; ctx.stroke();
    const nodes = [
      ['GOV', S.pipe.gov], ['MODEL', S.pipe.model], ['CPU', S.pipe.cpu],
      ['EVID', S.pipe.evid], ['ECON', S.pipe.econ],
    ];
    const nw = ww / nodes.length;
    const live = S.pipeLive;
    for (let i = 0; i < nodes.length; i++) {
      const [lbl, val] = nodes[i];
      const nx = x0 + i * nw + nw / 2;
      ctx.fillStyle = 'rgba(176,200,235,0.95)';
      ctx.font = '600 8px JetBrains Mono, monospace';
      ctx.textAlign = 'center';
      ctx.fillText(lbl, nx, y0 + 3);
      ctx.fillStyle = '#f8fbff';
      ctx.font = '700 9px JetBrains Mono, monospace';
      ctx.fillText(String(val), nx, y0 + 15);
      if (i < nodes.length - 1) {
        const lx = nx + nw / 2 - 6;
        ctx.fillStyle = 'rgba(92,200,255,0.3)';
        ctx.fillRect(lx, y0 + 4, 12, 1.6);
        if (live) {
          const o = (now / 700 + i * 0.2) % 1;
          ctx.fillStyle = 'rgba(92,200,255,0.95)';
          ctx.beginPath(); ctx.arc(lx + o * 12, y0 + 4.7, 1.7, 0, 7); ctx.fill();
        }
      }
    }
  }
  function drawSeaCurrents(ctx, now) {
    if (!S.sea || !S.sea.length) return;
    const t = now * 0.00035;
    const n = S.sea.length;
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    for (let i = 0; i < n; i++) {
      const row = S.sea[i];
      const off = Math.sin(t * 1.2 + i * 1.9) * 15;
      const hl = (i + Math.floor(t * 2)) % 3 === 0;
      ctx.strokeStyle = hl ? 'rgba(180,220,255,0.30)' : 'rgba(150,205,255,0.14)';
      ctx.lineWidth = hl ? 3.2 : 2.2;
      ctx.lineCap = 'round';
      ctx.beginPath();
      let started = false;
      for (const p of row) {
        const y = p.y + Math.sin(p.x * 0.018 + t * 9 + i * 2.1) * 4 + off;
        if (!started) { ctx.moveTo(p.x, y); started = true; }
        else ctx.lineTo(p.x, y);
      }
      ctx.stroke();
    }
    ctx.restore();
  }
  function drawFx(now) {
    const cw = fx.width, ch = fx.height;
    const dpr = window.devicePixelRatio || 1;
    const k = view.k;
    fxctx.setTransform(1, 0, 0, 1, 0, 0);
    fxctx.clearRect(0, 0, cw, ch);
    fxctx.save();
    fxctx.setTransform(k * dpr, 0, 0, k * dpr, view.x * dpr, view.y * dpr);
    drawParticles(fxctx);
    drawSeaCurrents(fxctx, now);
    fxctx.restore();
    if (!world) return;
    const env = envOf(world.clock.t);
    const night = 1 - env.dayF;
    fxctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (night > 0.02) {
      fxctx.fillStyle = `rgba(2,6,16,${(night * 0.32).toFixed(3)})`;
      fxctx.fillRect(0, 0, fx.clientWidth, fx.clientHeight);
      fxctx.globalCompositeOperation = 'lighter';
      for (const r of world.map.landRegions) {
        if (!r.explored) continue;
        const pop = r.population || 0;
        if (pop <= 0 && !r.cityId) continue;
        const s = worldToScreen(r.x, r.y);
        const g = fxctx.createRadialGradient(s.x, s.y, 0, s.x, s.y, 40 + Math.min(80, pop));
        g.addColorStop(0, `rgba(255,170,80,${(0.1 * night).toFixed(3)})`);
        g.addColorStop(1, 'rgba(255,170,80,0)');
        fxctx.fillStyle = g;
        fxctx.fillRect(s.x - 100, s.y - 100, 200, 200);
      }
      fxctx.globalCompositeOperation = 'source-over';
    }
    const season = env.season;
    if (season === 'winter') { fxctx.fillStyle = 'rgba(190,215,245,0.09)'; fxctx.fillRect(0, 0, fx.clientWidth, fx.clientHeight); }
    else if (season === 'summer') { fxctx.fillStyle = 'rgba(255,224,150,0.05)'; fxctx.fillRect(0, 0, fx.clientWidth, fx.clientHeight); }
    else if (season === 'autumn') { fxctx.fillStyle = 'rgba(255,170,120,0.045)'; fxctx.fillRect(0, 0, fx.clientWidth, fx.clientHeight); }
    const vg = fxctx.createRadialGradient(fx.clientWidth / 2, fx.clientHeight / 2, Math.min(fx.clientWidth, fx.clientHeight) * 0.45, fx.clientWidth / 2, fx.clientHeight / 2, Math.max(fx.clientWidth, fx.clientHeight) * 0.78);
    vg.addColorStop(0, 'rgba(0,0,0,0)');
    vg.addColorStop(1, 'rgba(0,0,0,0.42)');
    fxctx.fillStyle = vg;
    fxctx.fillRect(0, 0, fx.clientWidth, fx.clientHeight);
    if (S.pipe) drawPipeline(fxctx, fx.clientWidth, fx.clientHeight, now);
  }

  /* ================= minimap ================= */
  function drawMinimap(now) {
    if (!world) return;
    const dpr = window.devicePixelRatio || 1;
    const W = mm.width / dpr, H = mm.height / dpr;
    const bw = Math.max(1, B.maxX - B.minX), bh = Math.max(1, B.maxY - B.minY);
    const sc = Math.min((W - 14) / bw, (H - 14) / bh);
    const ox = (W - bw * sc) / 2 - B.minX * sc;
    const oy = (H - bh * sc) / 2 - B.minY * sc;
    S.mm = { sc, ox, oy };
    const px = (wx) => ox + wx * sc, py = (wy) => oy + wy * sc;
    mmctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    mmctx.clearRect(0, 0, W, H);
    mmctx.fillStyle = '#05080f';
    mmctx.fillRect(0, 0, W, H);
    const tr = world.map.trail || {};
    for (const key in tr) {
      const wgt = tr[key];
      if (wgt <= 0) continue;
      const [a, b] = key.split('|');
      const ra = world.regions.find(x => x.id === a), rb = world.regions.find(x => x.id === b);
      if (!ra || !rb || !ra.explored || !rb.explored) continue;
      mmctx.strokeStyle = `rgba(255,218,158,${Math.min(1, wgt / 12) * 0.3})`;
      mmctx.lineWidth = 0.6;
      mmctx.beginPath(); mmctx.moveTo(px(ra.x), py(ra.y)); mmctx.lineTo(px(rb.x), py(rb.y)); mmctx.stroke();
    }
    for (const r of world.map.landRegions) {
      if (r.poly.length < 3) continue;
      const owner = ownerOf(world, r.id);
      const fill = owner ? rgba(orgColor(world, owner), 0.5) : (r.explored ? 'rgba(30,48,74,0.85)' : 'rgba(14,19,32,0.9)');
      mmctx.fillStyle = fill;
      mmctx.strokeStyle = 'rgba(148,178,226,0.14)';
      mmctx.lineWidth = 0.4;
      mmctx.beginPath();
      r.poly.forEach((p, i) => { if (i === 0) mmctx.moveTo(px(p[0]), py(p[1])); else mmctx.lineTo(px(p[0]), py(p[1])); });
      mmctx.closePath();
      mmctx.fill();
      mmctx.stroke();
    }
    for (const c of world.map.cities) {
      mmctx.fillStyle = '#8fd6ff';
      mmctx.beginPath(); mmctx.arc(px(c.x), py(c.y), 2, 0, 7); mmctx.fill();
    }
    for (const aid of world.agentOrder) {
      const a = world.agents[aid];
      if (!a) continue;
      const ui = agentUi.get(aid);
      if (!ui) continue;
      mmctx.fillStyle = (a.org && world.orgs[a.org]) ? world.orgs[a.org].color : a.color;
      mmctx.beginPath(); mmctx.arc(px(ui.x), py(ui.y), 1.6, 0, 7); mmctx.fill();
    }
    const x0 = (0 - view.x) / view.k, y0 = (0 - view.y) / view.k;
    const x1 = x0 + fx.clientWidth / view.k, y1 = y0 + fx.clientHeight / view.k;
    mmctx.strokeStyle = 'rgba(255,255,255,0.55)';
    mmctx.lineWidth = 0.7;
    mmctx.strokeRect(px(x0), py(y0), (x1 - x0) * sc, (y1 - y0) * sc);
  }
  function minimapClick(e) {
    if (!world) return;
    const rect = mm.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    const { sc, ox, oy } = S.mm;
    const wx = (mx - ox) / sc, wy = (my - oy) / sc;
    const cw = svg.clientWidth || 1000, ch = svg.clientHeight || 620;
    view.x = cw / 2 - wx * view.k;
    view.y = ch / 2 - wy * view.k;
    userMoved = true;
    S.follow = false;
    applyView();
  }
  mm.addEventListener('click', minimapClick);

  /* ================= hit test ================= */
  function hitTest(pt) {
    for (const [aid, ent] of agentGels) {
      if (ent.g.style.display === 'none') continue;
      const ui = agentUi.get(aid);
      if (ui && Math.hypot(ui.x - pt.x, ui.y - pt.y) < 13) {
        const a = world.agents[aid];
        if (a) return { kind: 'agent', id: aid };
      }
    }
    for (let i = cityGeom.length - 1; i >= 0; i--) {
      const cg = cityGeom[i];
      if (Math.hypot(cg.x - pt.x, cg.y - pt.y) < 20) {
        const c = world.map.cities.find(x => x.id === cg.id);
        if (c) return { kind: 'city', id: cg.id };
      }
    }
    for (const [id, rg] of regionGeom) {
      const r = world.regions.find(x => x.id === id);
      if (!r || !r.land) continue;
      if (Math.hypot(rg.x - pt.x, rg.y - pt.y) < 30) return { kind: 'region', id };
    }
    return null;
  }

  /* ================= update ================= */
  function update(w, selArg, opts) {
    const fresh = w !== world;
    world = w;
    sel = selArg || null;
    if (fresh) {
      evSeen.clear();
      for (const f of flashes) f.g.remove();
      flashes.length = 0;
      lastActSeen = 0;
      agentG.innerHTML = '';
      agentGels.clear();
      agentUi.clear();
      buildStatic(w);
      if (!world.map.trail) world.map.trail = {};
      initView();
    } else if (opts && opts.full) {
      buildStatic(w);
    }
    renderSettles(w);
    renderTrails(w);
    renderDyn(w);
    scanEvents(w);
    scanActivity(w);
    S.pipe = computePipeVals(w);
    S.pipeLive = w.compute.stats.execs !== S.pipeExecs;
    S.pipeExecs = w.compute.stats.execs;
    for (const [id, rg] of regionGeom) {
      const isSel = selArg && selArg.kind === 'region' && selArg.id === id;
      if (rg.path) {
        rg.path.setAttribute('stroke', isSel ? '#5cc8ff' : 'rgba(148,178,226,0.1)');
        rg.path.setAttribute('stroke-width', isSel ? 1.6 : 0.7);
      }
    }
  }

  function loop(now) {
    requestAnimationFrame(loop);
    if (!world) return;
    applySvgSize();
    const dt = Math.min(80, now - (lastFrame || now));
    lastFrame = now;
    updateAgents(now);
    updateFlashes(now);
    emitters(now);
    updateParticles(dt);
    frameCamera();
    const env = envOf(world.clock.t);
    if (env.season === 'winter' && Math.random() < 0.4) {
      const cw = svg.clientWidth || 1000, ch = svg.clientHeight || 620;
      const wx0 = (0 - view.x) / view.k, wx1 = (cw - view.x) / view.k;
      const wy0 = (0 - view.y) / view.k, wy1 = (ch - view.y) / view.k;
      spawn({ x: wx0 + Math.random() * (wx1 - wx0), y: wy0 + Math.random() * (wy1 - wy0), vx: 4 + Math.random() * 6, vy: 14 + Math.random() * 12, kind: 'snow', color: '#e6f0ff', maxLife: 4, size: 1 + Math.random() * 1.4, fade: 1 });
    }
    decorG.setAttribute('opacity', clamp((view.k - 1.05) * 1.4, 0, 0.9).toFixed(2));
    settleG.setAttribute('opacity', clamp(0.6 + (view.k - 1.2) * 0.5, 0.6, 1).toFixed(2));
    drawFx(now);
    if (now - (S._mmt || 0) > 220) { S._mmt = now; drawMinimap(now); }
  }
  requestAnimationFrame(loop);

  return {
    update,
    zoomAt,
    fit() {
      if (!S.fit) S.fit = computeFit();
      view.x = S.fit.x; view.y = S.fit.y; view.k = S.fit.k;
      userMoved = true;
      S.follow = false;
      applyView();
    },
    get view() { return view; },
    get fronts() { return frontCount; },
    get disputes() { return disputeCount; },
    setFilter(k) { S.filter = k || 'all'; },
    setFollow(v) {
      S.follow = !!v;
      if (S.follow) userMoved = false;
      else { S.followAgent = null; }
    },
    getFollow() { return S.follow; },
    focus(id) {
      S.followAgent = id;
      S.follow = true;
      userMoved = false;
    },
    getLife() { return world ? lifeScore(world) : 0; },
    showMinimap(b) { mm.style.display = b ? 'block' : 'none'; },
    setFilterLabel() {},
  };
}
